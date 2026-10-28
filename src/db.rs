//! The portal's Postgres database: its connection pool and its migrations.
//!
//! Connections are not encrypted yet, so the database must sit on a private network.

use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use anyhow::Context;
use deadpool::Runtime;
use diesel::sql_types::{BigInt, Bool};
use diesel::{ConnectionError, ConnectionResult, QueryResult};
use diesel_async::pooled_connection::{AsyncDieselConnectionManager, ManagerConfig};
use diesel_async::{AsyncConnection, AsyncMigrationHarness, AsyncPgConnection, RunQueryDsl};
use diesel_migrations::{EmbeddedMigrations, MigrationHarness, embed_migrations};
use secrecy::ExposeSecret;
use tracing::info;

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

/// Longest the pool's check of an idle connection may take. A connection still busy with
/// an abandoned query is replaced instead of waited for.
const RECYCLE_TIMEOUT: Duration = Duration::from_secs(1);

/// Longest a migration waits for a table lock. Queries on that table would wait behind it.
const MIGRATION_LOCK_TIMEOUT: Duration = Duration::from_secs(5);

/// Key of the advisory lock held while migrations run: "portal" in ASCII. Nothing else in
/// the database may use it.
const MIGRATION_LOCK_KEY: i64 = 0x706f_7274_616c;

#[diesel::declare_sql_function]
extern "SQL" {
    fn pg_try_advisory_lock(key: BigInt) -> Bool;
}

/// Builds the pool. Connections are opened on first use, not here.
pub fn pool(config: &config::Database) -> anyhow::Result<Pool> {
    let mut manager_config = ManagerConfig::default();
    manager_config.custom_setup = Box::new(establish);
    let manager =
        AsyncDieselConnectionManager::new_with_config(config.url.expose_secret(), manager_config);
    Pool::builder(manager)
        .max_size(config.pool_size.get())
        .wait_timeout(Some(CONNECTION_TIMEOUT))
        .create_timeout(Some(CONNECTION_TIMEOUT))
        .recycle_timeout(Some(RECYCLE_TIMEOUT))
        .runtime(Runtime::Tokio1)
        .build()
        .context("failed to build the database pool")
}

/// Opens a pool connection, with [`STATEMENT_TIMEOUT`] set.
fn establish(url: &str) -> BoxFuture<'_, ConnectionResult<AsyncPgConnection>> {
    Box::pin(async move {
        let mut conn = AsyncPgConnection::establish(url).await?;
        set_timeout(&mut conn, "statement_timeout", STATEMENT_TIMEOUT)
            .await
            .map_err(ConnectionError::CouldntSetupConfiguration)?;
        Ok(conn)
    })
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
pub async fn run_migrations(database_url: &str) -> anyhow::Result<Vec<String>> {
    let mut conn = tokio::time::timeout(
        CONNECTION_TIMEOUT,
        AsyncPgConnection::establish(database_url),
    )
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
