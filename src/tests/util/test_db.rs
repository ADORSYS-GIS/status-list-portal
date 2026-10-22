//! A throwaway Postgres database for each test.
//!
//! `TEST_DATABASE_URL`, from the environment or from `.env`, names a server where tests may
//! create databases. The database in its path only serves as a prefix for their names; they
//! are created and dropped through the server's `postgres` database.

use std::sync::LazyLock;
use std::sync::atomic::{AtomicU32, Ordering};

use diesel_async::{AsyncConnection, AsyncPgConnection, RunQueryDsl};
use status_list_portal::db;
use url::Url;

const URL_VARIABLE: &str = "TEST_DATABASE_URL";
const MAINTENANCE_DATABASE: &str = "postgres";

/// Leaves room for `_<pid>_<id>` within the 63 bytes Postgres keeps of a name. Longer names
/// are cut short silently, and could then collide.
const MAX_PREFIX_LEN: usize = 40;

static SERVER_URL: LazyLock<Url> = LazyLock::new(server_url);
static NEXT_ID: AtomicU32 = AtomicU32::new(0);

/// A migrated database of the test's own, dropped with this value.
pub struct TestDatabase {
    name: String,
    url: Url,
}

impl TestDatabase {
    pub async fn new() -> Self {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let name = format!("{}_{}_{id}", name_prefix(), std::process::id());
        run_on_server(&[
            // A killed test run can leave a database with this name behind.
            format!("DROP DATABASE IF EXISTS \"{name}\" WITH (FORCE)"),
            format!("CREATE DATABASE \"{name}\""),
        ])
        .await;

        let url = url_for(&name);
        db::run_migrations(url.as_str())
            .await
            .expect("failed to migrate the test database");
        Self { name, url }
    }

    pub fn url(&self) -> &str {
        self.url.as_str()
    }

    pub async fn connect(&self) -> AsyncPgConnection {
        AsyncPgConnection::establish(self.url())
            .await
            .expect("failed to connect to the test database")
    }
}

impl Drop for TestDatabase {
    fn drop(&mut self) {
        // Drop cannot await, and the test's runtime may already be gone, so the database is
        // dropped from a thread with a runtime of its own.
        let statement = format!("DROP DATABASE \"{}\" WITH (FORCE)", self.name);
        let dropped = std::thread::spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("failed to start a runtime")
                .block_on(run_on_server(&[statement]));
        })
        .join();
        if dropped.is_err() {
            eprintln!("failed to drop test database {}", self.name);
        }
    }
}

async fn run_on_server(statements: &[String]) {
    let mut conn = AsyncPgConnection::establish(url_for(MAINTENANCE_DATABASE).as_str())
        .await
        .unwrap_or_else(|error| panic!("failed to connect to {URL_VARIABLE}: {error}"));
    for statement in statements {
        diesel::sql_query(statement)
            .execute(&mut conn)
            .await
            .unwrap_or_else(|error| panic!("`{statement}` failed: {error}"));
    }
}

fn url_for(database: &str) -> Url {
    let mut url = SERVER_URL.clone();
    url.set_path(database);
    url
}

/// The database named in the URL, which test databases are named after.
fn name_prefix() -> &'static str {
    let prefix = SERVER_URL.path().trim_start_matches('/');
    let is_plain = !prefix.is_empty()
        && prefix.len() <= MAX_PREFIX_LEN
        && prefix
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
    assert!(
        is_plain,
        "the database in {URL_VARIABLE} must be named with at most {MAX_PREFIX_LEN} of \
         a-z, 0-9 and _"
    );
    prefix
}

fn server_url() -> Url {
    let url = std::env::var(URL_VARIABLE)
        .ok()
        .or_else(url_from_dotenv)
        .unwrap_or_else(|| panic!("{URL_VARIABLE} must be set, in the environment or in .env"));
    Url::parse(&url).unwrap_or_else(|_| panic!("{URL_VARIABLE} is not a valid URL"))
}

/// Reads the URL from `.env` without changing the process environment, which other test
/// threads may be reading.
fn url_from_dotenv() -> Option<String> {
    dotenvy::dotenv_iter()
        .ok()?
        .map(|item| item.expect(".env is malformed"))
        .find(|(key, _)| key == URL_VARIABLE)
        .map(|(_, value)| value)
}
