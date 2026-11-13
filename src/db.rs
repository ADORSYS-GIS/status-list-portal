//! The portal's Postgres database: its connection pool and its migrations.
//!
//! Production connections use TLS with certificate and hostname verification.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use deadpool::Runtime;
use diesel::sql_types::{BigInt, Bool};
use diesel::{ConnectionError, ConnectionResult, QueryResult};
use diesel_async::pooled_connection::{AsyncDieselConnectionManager, ManagerConfig};
use diesel_async::{AsyncMigrationHarness, AsyncPgConnection, RunQueryDsl};
use diesel_migrations::{EmbeddedMigrations, MigrationHarness, embed_migrations};
use rustls::pki_types::{CertificateDer, pem::PemObject};
use rustls::{ClientConfig, RootCertStore};
use secrecy::ExposeSecret;
use tokio_postgres::config::SslMode;
use tokio_postgres_rustls::MakeRustlsConnect;
use tracing::info;

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
    let manager =
        AsyncDieselConnectionManager::new_with_config(config.url.expose_secret(), manager_config);
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
    url: String,
    tls: ClientConfig,
    production: bool,
}

impl ConnectionOptions {
    fn new(database: &config::Database, env: Env) -> anyhow::Result<Self> {
        let url = database.url.expose_secret();
        let mut parsed =
            url::Url::parse(url).context("APP_DATABASE__URL must be a Postgres URL")?;
        if !matches!(parsed.scheme(), "postgres" | "postgresql") {
            anyhow::bail!("APP_DATABASE__URL must use postgres:// or postgresql://");
        }
        let production = env == Env::Production;
        let modes = parsed
            .query_pairs()
            .filter(|(key, _)| key == "sslmode")
            .map(|(_, value)| value.into_owned())
            .collect::<Vec<_>>();
        if modes.len() > 1 {
            anyhow::bail!("APP_DATABASE__URL must specify sslmode at most once");
        }
        let mode = modes.as_slice().first().map(String::as_str);
        if production && mode.is_some_and(|mode| !matches!(mode, "require" | "verify-full")) {
            anyhow::bail!("production APP_DATABASE__URL must not weaken Postgres TLS");
        }
        if mode == Some("verify-full") {
            let pairs = parsed
                .query_pairs()
                .map(|(key, value)| {
                    let replacement = key == "sslmode";
                    (
                        key.into_owned(),
                        if replacement {
                            "require".to_string()
                        } else {
                            value.into_owned()
                        },
                    )
                })
                .collect::<Vec<_>>();
            parsed.query_pairs_mut().clear().extend_pairs(pairs);
        }
        let mut roots = RootCertStore::empty();
        let native = rustls_native_certs::load_native_certs();
        roots.add_parsable_certificates(native.certs);
        if let Some(path) = &database.tls_ca_file {
            let certs = CertificateDer::pem_file_iter(path)
                .with_context(|| format!("failed to open Postgres CA bundle {}", path.display()))?
                .collect::<Result<Vec<_>, _>>()
                .context("failed to parse Postgres CA bundle")?;
            let (added, _) = roots.add_parsable_certificates(certs);
            if added == 0 {
                anyhow::bail!("Postgres CA bundle has no valid certificates");
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
        Ok(Self {
            url: parsed.to_string(),
            tls,
            production,
        })
    }

    async fn connect(&self) -> ConnectionResult<AsyncPgConnection> {
        let mut url: tokio_postgres::Config =
            self.url.parse().map_err(|error: tokio_postgres::Error| {
                ConnectionError::BadConnection(format!("invalid Postgres URL: {error}"))
            })?;
        if self.production {
            url.ssl_mode(SslMode::Require);
        }
        let (client, conn) = url
            .connect(MakeRustlsConnect::new(self.tls.clone()))
            .await
            .map_err(|error| ConnectionError::BadConnection(error.to_string()))?;
        AsyncPgConnection::try_from_client_and_connection(client, conn).await
    }
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
