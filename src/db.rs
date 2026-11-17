//! The portal's Postgres database: its connection pool and its migrations.
//!
//! Production connections use TLS with certificate and hostname verification.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use deadpool::Runtime;
use diesel::migration::MigrationSource;
use diesel::pg::Pg;
use diesel::sql_types::{BigInt, Bool, Text};
use diesel::{ConnectionError, ConnectionResult, QueryResult, QueryableByName};
use diesel_async::pooled_connection::{AsyncDieselConnectionManager, ManagerConfig};
use diesel_async::{AsyncMigrationHarness, AsyncPgConnection, RunQueryDsl};
use diesel_migrations::{EmbeddedMigrations, MigrationHarness, embed_migrations};
use rustls::pki_types::{CertificateDer, pem::PemObject};
use rustls::{ClientConfig, RootCertStore};
use secrecy::ExposeSecret;
use tokio_postgres::config::{Host, SslMode};
use tokio_postgres_rustls::MakeRustlsConnect;
use tracing::{info, warn};

use crate::Env;
use crate::config;

/// Pool of connections to the portal's database.
pub type Pool = diesel_async::pooled_connection::deadpool::Pool<AsyncPgConnection>;

type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// The migrations in `migrations/`, compiled into the binary.
pub const MIGRATIONS: EmbeddedMigrations = embed_migrations!("migrations");

/// How long to wait for a connection, from the pool or from the server, before giving up.
const CONNECTION_TIMEOUT: Duration = Duration::from_secs(5);

/// Longest a query from the pool may run before the server cancels it. A handler that gave
/// up on a query drops the future, not the query, and its connection returns to the pool.
const STATEMENT_TIMEOUT: Duration = Duration::from_secs(10);

/// Longest a pool connection may sit in an open transaction. A request cut off inside one
/// leaves it open, and the pool only notices when it next hands the connection out; until
/// then, its locks would stay held. The server ends such a session instead.
const IDLE_IN_TRANSACTION_TIMEOUT: Duration = Duration::from_secs(60);

/// Longest the pool's check of an idle connection may take. A connection still busy with
/// an abandoned query is replaced instead of waited for.
const RECYCLE_TIMEOUT: Duration = Duration::from_secs(1);

/// Longest a migration waits for a table lock. Queries on that table would wait behind it.
const MIGRATION_LOCK_TIMEOUT: Duration = Duration::from_secs(5);

/// Key of the advisory lock held while migrations run: "portal" in ASCII. Nothing else in
/// the database may use it.
const MIGRATION_LOCK_KEY: i64 = 0x706f_7274_616c;
/// Key of the advisory lock held while an issuer takes a place within the sandbox limits:
/// "issuer" in ASCII.
const ISSUER_LIMITS_LOCK_KEY: i64 = 0x6973_7375_6572;

#[diesel::declare_sql_function]
extern "SQL" {
    fn pg_try_advisory_lock(key: BigInt) -> Bool;
    fn pg_try_advisory_xact_lock(key: BigInt) -> Bool;
}

/// Takes the sandbox limits lock until the transaction ends, unless another transaction
/// holds it. Whoever holds it is the only one adding an active issuer, on every replica.
pub async fn try_lock_issuer_limits(conn: &mut AsyncPgConnection) -> QueryResult<bool> {
    diesel::select(pg_try_advisory_xact_lock(ISSUER_LIMITS_LOCK_KEY))
        .get_result(conn)
        .await
}

/// Builds the pool. Connections are opened on first use, not here.
pub fn pool(config: &config::Database, env: Env) -> anyhow::Result<Pool> {
    build_pool(config, env, config.pool_size.get())
}

/// Keeps health checks independent of application pool contention.
pub fn readiness_pool(config: &config::Database, env: Env) -> anyhow::Result<Pool> {
    build_pool(config, env, 1)
}

fn build_pool(config: &config::Database, env: Env, max_size: usize) -> anyhow::Result<Pool> {
    let options = Arc::new(ConnectionOptions::new(config, env)?);
    let mut manager_config = ManagerConfig::default();
    manager_config.custom_setup = Box::new(move |_| establish(options.clone()));
    // The custom setup owns the parsed connection configuration. The manager's
    // URL argument is unused; keeping a credential-bearing copy there would
    // unnecessarily expose it through another String.
    let manager = AsyncDieselConnectionManager::new_with_config("", manager_config);
    Pool::builder(manager)
        .max_size(max_size)
        .wait_timeout(Some(CONNECTION_TIMEOUT))
        .create_timeout(Some(CONNECTION_TIMEOUT))
        .recycle_timeout(Some(RECYCLE_TIMEOUT))
        .runtime(Runtime::Tokio1)
        .build()
        .context("failed to build the database pool")
}

/// Opens a pool connection, with [`STATEMENT_TIMEOUT`] and [`IDLE_IN_TRANSACTION_TIMEOUT`]
/// set.
fn establish(
    options: Arc<ConnectionOptions>,
) -> BoxFuture<'static, ConnectionResult<AsyncPgConnection>> {
    Box::pin(async move {
        let mut conn = options.connect().await?;
        for (setting, timeout) in [
            ("statement_timeout", STATEMENT_TIMEOUT),
            (
                "idle_in_transaction_session_timeout",
                IDLE_IN_TRANSACTION_TIMEOUT,
            ),
        ] {
            set_timeout(&mut conn, setting, timeout)
                .await
                .map_err(ConnectionError::CouldntSetupConfiguration)?;
        }
        Ok(conn)
    })
}

struct ConnectionOptions {
    config: tokio_postgres::Config,
    tls: ClientConfig,
}

impl ConnectionOptions {
    fn new(database: &config::Database, env: Env) -> anyhow::Result<Self> {
        let production = env == Env::Production;
        let mut config: tokio_postgres::Config = database
            .url
            .expose_secret()
            .parse()
            .context("invalid APP_DATABASE__URL Postgres connection string (use sslmode=require for verified TLS; verify-full is unsupported)")?;
        if production {
            if config.get_ssl_mode() == SslMode::Disable {
                if !local_database_hosts(&config) {
                    anyhow::bail!(
                        "production APP_DATABASE__URL may disable Postgres TLS only for Unix sockets or loopback hosts"
                    );
                }
            } else {
                // The Rustls connector verifies the CA chain and hostname for Require.
                // Upgrade Prefer so a remote production connection cannot use plaintext.
                config.ssl_mode(SslMode::Require);
            }
        }
        let mut roots = RootCertStore::empty();
        if let Some(path) = &database.tls_ca_file {
            let certs = CertificateDer::pem_file_iter(path)
                .with_context(|| format!("failed to open Postgres CA bundle {}", path.display()))?
                .collect::<Result<Vec<_>, _>>()
                .context("failed to parse Postgres CA bundle")?;
            let (added, ignored) = roots.add_parsable_certificates(certs);
            if added == 0 || ignored != 0 {
                anyhow::bail!(
                    "Postgres CA bundle has {added} valid and {ignored} invalid certificates"
                );
            }
        } else {
            let native = rustls_native_certs::load_native_certs();
            roots.add_parsable_certificates(native.certs);
            if !native.errors.is_empty() {
                warn!(errors = ?native.errors, "some system Postgres CA certificates could not be loaded");
            }
            if roots.is_empty() && !native.errors.is_empty() {
                anyhow::bail!(
                    "failed to load system Postgres CA certificates: {:?}",
                    native.errors
                );
            }
        }
        if production && config.get_ssl_mode() != SslMode::Disable && roots.is_empty() {
            anyhow::bail!("production Postgres TLS needs trusted CA certificates");
        }
        let tls = ClientConfig::builder_with_provider(Arc::new(
            rustls::crypto::aws_lc_rs::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .context("failed to configure Postgres TLS versions")?
        .with_root_certificates(roots)
        .with_no_client_auth();
        Ok(Self { config, tls })
    }

    async fn connect(&self) -> ConnectionResult<AsyncPgConnection> {
        let (client, conn) = self
            .config
            .connect(MakeRustlsConnect::new(self.tls.clone()))
            .await
            .map_err(|error| {
                ConnectionError::BadConnection(format!("{:#}", anyhow::Error::new(error)))
            })?;
        AsyncPgConnection::try_from_client_and_connection(client, conn).await
    }
}

/// An explicit plaintext production connection is safe only within the local host.
fn local_database_hosts(config: &tokio_postgres::Config) -> bool {
    let hosts = config.get_hosts();
    let hostaddrs = config.get_hostaddrs();
    if hosts.is_empty() && hostaddrs.is_empty() {
        return false;
    }
    hosts.iter().all(|host| match host {
        Host::Tcp(name) => {
            name == "localhost"
                || name
                    .parse::<std::net::IpAddr>()
                    .is_ok_and(|ip| ip.is_loopback())
        }
        #[cfg(unix)]
        Host::Unix(_) => true,
    }) && hostaddrs.iter().all(std::net::IpAddr::is_loopback)
}

#[derive(QueryableByName)]
struct AppliedMigration {
    #[diesel(sql_type = Text)]
    version: String,
}

/// A server may receive traffic only after the migration job has finished.
pub async fn ensure_current_schema(conn: &mut AsyncPgConnection) -> anyhow::Result<()> {
    let applied: Vec<AppliedMigration> =
        diesel::sql_query("SELECT version FROM __diesel_schema_migrations")
            .load(conn)
            .await
            .context("failed to inspect schema migrations; run portal migrate first")?;
    let applied = applied
        .into_iter()
        .map(|row| row.version)
        .collect::<std::collections::HashSet<_>>();
    let pending = MigrationSource::<Pg>::migrations(&MIGRATIONS)
        .map_err(anyhow::Error::from_boxed)?
        .into_iter()
        .map(|migration| migration.name().version().to_string())
        .filter(|version| !applied.contains(version))
        .collect::<Vec<_>>();
    if !pending.is_empty() {
        anyhow::bail!(
            "database has pending migrations ({}); run portal migrate first",
            pending.join(", ")
        );
    }
    Ok(())
}

/// Sets a Postgres timeout setting for the rest of the session.
async fn set_timeout(
    conn: &mut AsyncPgConnection,
    setting: &str,
    timeout: Duration,
) -> QueryResult<()> {
    diesel::sql_query(format!("SET {setting} = {}", timeout.as_millis()))
        .execute(conn)
        .await
        .map(drop)
}

/// Applies the migrations the database has not seen yet, and returns their versions.
///
/// An advisory lock is held while they run, so replicas that start together apply each
/// migration once.
pub async fn run_migrations(database: &config::Database, env: Env) -> anyhow::Result<Vec<String>> {
    let options = ConnectionOptions::new(database, env)?;
    let mut conn = tokio::time::timeout(CONNECTION_TIMEOUT, options.connect())
        .await
        .context("timed out connecting to the database")?
        .context("failed to connect to the database")?;

    let locked = diesel::select(pg_try_advisory_lock(MIGRATION_LOCK_KEY))
        .get_result::<bool>(&mut conn)
        .await
        .context("failed to take the migration lock")?;
    if !locked {
        info!("another instance is applying migrations, waiting for it to finish");
        diesel::sql_query("SELECT pg_advisory_lock($1)")
            .bind::<BigInt, _>(MIGRATION_LOCK_KEY)
            .execute(&mut conn)
            .await
            .context("failed to take the migration lock")?;
    }
    // Set once the advisory lock is held, so waiting for that lock is not cut short. A
    // migration that times out fails the start, and the next start tries it again.
    set_timeout(&mut conn, "lock_timeout", MIGRATION_LOCK_TIMEOUT)
        .await
        .context("failed to set the migration lock timeout")?;

    // The harness blocks while it works, which only a blocking task may do on every kind of
    // runtime. The connection closes when the task ends, and the lock goes with it.
    let mut harness = AsyncMigrationHarness::new(conn);
    let applied = tokio::task::spawn_blocking(move || {
        harness
            .run_pending_migrations(MIGRATIONS)
            .map(|versions| versions.iter().map(ToString::to_string).collect::<Vec<_>>())
    })
    .await
    .context("the migration task panicked")?
    .map_err(anyhow::Error::from_boxed)
    .context("failed to run migrations")?;

    for version in &applied {
        info!(%version, "applied migration");
    }
    Ok(applied)
}

#[cfg(test)]
mod connection_string_tests {
    use super::local_database_hosts;
    use crate::{Env, config};
    use secrecy::SecretString;
    use std::num::NonZeroUsize;

    #[test]
    fn parses_query_values_without_reencoding() {
        let input = "postgres://u:p@db/portal?application_name=portal%20prod&options=-c%20search_path%3Dportal&sslmode=require";
        let config: tokio_postgres::Config = input.parse().unwrap();
        assert_eq!(config.get_application_name(), Some("portal prod"));
        assert_eq!(config.get_options(), Some("-c search_path=portal"));
    }

    #[test]
    fn accepts_multi_host_and_key_value_connection_strings() {
        for input in [
            "postgres://u:p@h1:5432,h2:5432/db?target_session_attrs=read-write",
            "host=db user=portal dbname=portal",
            "host=db user=portal dbname=portal sslmode=require",
        ] {
            let _: tokio_postgres::Config = input.parse().unwrap();
        }
    }

    #[test]
    fn invalid_postgres_options_fail_when_the_pool_is_built() {
        for input in [
            "postgres://db/portal?sslrootcert=/tmp/ca.pem",
            "postgres://db/portal?sslmode=verify-ca",
            "postgres://db/portal?sslmode=verify-full",
        ] {
            let database = config::Database {
                url: SecretString::from(input),
                tls_ca_file: None,
                pool_size: NonZeroUsize::MIN,
            };
            assert!(super::pool(&database, Env::Development).is_err(), "{input}");
        }
    }

    #[test]
    fn plaintext_requires_only_local_hosts_and_hostaddrs() {
        for input in [
            "host=localhost sslmode=disable",
            "host=127.0.0.1 sslmode=disable",
            "host=::1 sslmode=disable",
            "host=localhost,127.0.0.1 hostaddr=127.0.0.1,::1 sslmode=disable",
            #[cfg(unix)]
            "host=/var/run/postgresql sslmode=disable",
        ] {
            let config: tokio_postgres::Config = input.parse().unwrap();
            assert!(local_database_hosts(&config), "{input}");
        }
        for input in [
            "host=db sslmode=disable",
            "host=localhost,db sslmode=disable",
            "host=localhost hostaddr=10.0.0.1 sslmode=disable",
            "host=db hostaddr=127.0.0.1 sslmode=disable",
        ] {
            let config: tokio_postgres::Config = input.parse().unwrap();
            assert!(!local_database_hosts(&config), "{input}");
        }
    }
}
