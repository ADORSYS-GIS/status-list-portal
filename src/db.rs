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
use tokio_postgres::config::SslMode;
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
        let (connection_string, mode) = normalize_sslmode(database.url.expose_secret())?;
        if production
            && mode
                .as_deref()
                .is_some_and(|mode| mode != "require" && mode != "verify-full")
        {
            anyhow::bail!("production APP_DATABASE__URL must not weaken Postgres TLS");
        }
        let mut config: tokio_postgres::Config = connection_string
            .parse()
            .context("invalid APP_DATABASE__URL Postgres connection string")?;
        if production {
            config.ssl_mode(SslMode::Require);
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
        if production && roots.is_empty() {
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

/// Tokio Postgres accepts URI and libpq key/value connection strings, but does not
/// recognize libpq's `verify-full`. The Rustls connector verifies the chain and host
/// for every TLS mode, so `require` is the equivalent here. Preserve every other byte:
/// URL form encoding changes `%20` into `+`, which Tokio Postgres reads differently.
fn normalize_sslmode(input: &str) -> anyhow::Result<(String, Option<String>)> {
    if input.starts_with("postgres://") || input.starts_with("postgresql://") {
        let Some((prefix, query)) = input.split_once('?') else {
            return Ok((input.to_owned(), None));
        };
        let mut mode = None;
        let mut segments = Vec::new();
        for segment in query.split('&') {
            let (key, _) = segment.split_once('=').unwrap_or((segment, ""));
            if url::form_urlencoded::parse(segment.as_bytes())
                .next()
                .is_some_and(|(name, _)| name == "sslmode")
            {
                if mode.is_some() {
                    anyhow::bail!("APP_DATABASE__URL must specify sslmode at most once");
                }
                let decoded = url::form_urlencoded::parse(segment.as_bytes())
                    .next()
                    .map(|(_, value)| value.into_owned())
                    .unwrap_or_default();
                mode = Some(decoded.clone());
                if decoded == "verify-full" {
                    segments.push(format!("{key}=require"));
                    continue;
                }
            }
            segments.push(segment.to_owned());
        }
        return Ok((format!("{prefix}?{}", segments.join("&")), mode));
    }

    // Scan only the parameter boundaries; let tokio-postgres validate the full
    // key/value grammar. Quoted values may contain spaces or escaped quotes.
    let bytes = input.as_bytes();
    let mut index = 0;
    let mut mode = None;
    let mut replacement = None;
    while index < bytes.len() {
        while index < bytes.len() && bytes[index].is_ascii_whitespace() {
            index += 1;
        }
        let key_start = index;
        while index < bytes.len() && !bytes[index].is_ascii_whitespace() && bytes[index] != b'=' {
            index += 1;
        }
        let key = &input[key_start..index];
        while index < bytes.len() && bytes[index].is_ascii_whitespace() {
            index += 1;
        }
        if index >= bytes.len() || bytes[index] != b'=' {
            break;
        }
        index += 1;
        while index < bytes.len() && bytes[index].is_ascii_whitespace() {
            index += 1;
        }
        let value_start = index;
        let quoted = index < bytes.len() && bytes[index] == b'\'';
        if quoted {
            index += 1;
        }
        while index < bytes.len() {
            if bytes[index] == b'\\' {
                index = (index + 2).min(bytes.len());
            } else if quoted && bytes[index] == b'\'' {
                index += 1;
                break;
            } else if !quoted && bytes[index].is_ascii_whitespace() {
                break;
            } else {
                index += 1;
            }
        }
        if key == "sslmode" {
            if mode.is_some() {
                anyhow::bail!("APP_DATABASE__URL must specify sslmode at most once");
            }
            let value = input[value_start..index].trim_matches('\'');
            mode = Some(value.to_owned());
            if value == "verify-full" {
                replacement = Some(value_start..index);
            }
        }
    }
    let mut normalized = input.to_owned();
    if let Some(range) = replacement {
        normalized.replace_range(range, "require");
    }
    Ok((normalized, mode))
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
    use super::normalize_sslmode;
    use crate::{Env, config};
    use secrecy::SecretString;
    use std::num::NonZeroUsize;

    #[test]
    fn verify_full_keeps_other_query_values_byte_for_byte() {
        let input = "postgres://u:p@db/portal?application_name=portal%20prod&options=-c%20search_path%3Dportal&sslmode=verify-full";
        let (normalized, mode) = normalize_sslmode(input).unwrap();
        assert_eq!(mode.as_deref(), Some("verify-full"));
        let config: tokio_postgres::Config = normalized.parse().unwrap();
        assert_eq!(config.get_application_name(), Some("portal prod"));
        assert_eq!(config.get_options(), Some("-c search_path=portal"));
    }

    #[test]
    fn accepts_multi_host_and_key_value_connection_strings() {
        for input in [
            "postgres://u:p@h1:5432,h2:5432/db?target_session_attrs=read-write",
            "host=db user=portal dbname=portal",
            "host=db user=portal dbname=portal sslmode='verify-full'",
        ] {
            let (normalized, _) = normalize_sslmode(input).unwrap();
            let _: tokio_postgres::Config = normalized.parse().unwrap();
        }
    }

    #[test]
    fn invalid_postgres_options_fail_when_the_pool_is_built() {
        for input in [
            "postgres://db/portal?sslrootcert=/tmp/ca.pem",
            "postgres://db/portal?sslmode=verify-ca",
        ] {
            let database = config::Database {
                url: SecretString::from(input),
                tls_ca_file: None,
                pool_size: NonZeroUsize::MIN,
            };
            assert!(super::pool(&database, Env::Development).is_err(), "{input}");
        }
    }
}
