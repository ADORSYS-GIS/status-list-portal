//! A throwaway Postgres database for each test.
//!
//! `TEST_DATABASE_URL`, from the environment or from `.env`, names a server where tests may
//! create databases. The database in its path only serves as a prefix for their names; they
//! are created and dropped through the server's `postgres` database.

use std::sync::LazyLock;
use std::sync::atomic::{AtomicU32, Ordering};

use diesel_async::{AsyncConnection, AsyncPgConnection, RunQueryDsl};
use secrecy::SecretString;
use status_list_portal::db;
use status_list_portal::{Env, config};
use std::num::NonZeroUsize;
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
    admin_url: Url,
    roles: Option<Roles>,
}

struct Roles {
    app: String,
    migrator: String,
    app_url: Url,
    migration_url: Url,
}

impl TestDatabase {
    pub async fn new() -> Self {
        let mut test_db = Self::unmigrated().await;
        test_db.create_roles().await;
        db::run_migrations(&test_db.migration_config(), Env::Test)
            .await
            .expect("failed to migrate the test database");
        let app = &test_db.roles.as_ref().unwrap().app;
        diesel::sql_query(format!(
            "REVOKE INSERT, UPDATE, DELETE ON __diesel_schema_migrations FROM {app}"
        ))
        .execute(&mut test_db.admin_connect().await)
        .await
        .expect("failed to protect migration history");
        test_db
    }

    /// An empty database, without the migrations.
    pub async fn unmigrated() -> Self {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let name = format!("{}_{}_{id}", name_prefix(), std::process::id());
        run_on_server(&[
            // A killed test run can leave a database with this name behind.
            format!("DROP DATABASE IF EXISTS \"{name}\" WITH (FORCE)"),
            format!("CREATE DATABASE \"{name}\""),
        ])
        .await;

        let admin_url = url_for(&name);
        Self {
            name,
            admin_url,
            roles: None,
        }
    }

    pub fn url(&self) -> &str {
        self.roles
            .as_ref()
            .map_or(&self.admin_url, |roles| &roles.app_url)
            .as_str()
    }

    pub fn config(&self) -> config::Database {
        config::Database {
            url: SecretString::from(self.url()),
            tls_ca_file: None,
            pool_size: NonZeroUsize::MIN,
        }
    }

    pub fn migration_config(&self) -> config::Database {
        let url = self
            .roles
            .as_ref()
            .map_or(&self.admin_url, |roles| &roles.migration_url);
        config::Database {
            url: SecretString::from(url.as_str()),
            tls_ca_file: None,
            pool_size: NonZeroUsize::MIN,
        }
    }

    pub async fn connect(&self) -> AsyncPgConnection {
        AsyncPgConnection::establish(self.url())
            .await
            .expect("failed to connect to the test database")
    }

    pub async fn admin_connect(&self) -> AsyncPgConnection {
        AsyncPgConnection::establish(self.admin_url.as_str())
            .await
            .expect("failed to connect as test administrator")
    }

    async fn create_roles(&mut self) {
        let id = self.name.rsplit_once('_').unwrap().1;
        let pid = std::process::id();
        let app = format!("pa_{pid}_{id}");
        let migrator = format!("pm_{pid}_{id}");
        let mut app_url = self.admin_url.clone();
        app_url.set_username(&app).unwrap();
        app_url.set_password(Some("test-password")).unwrap();
        let mut migration_url = self.admin_url.clone();
        migration_url.set_username(&migrator).unwrap();
        migration_url.set_password(Some("test-password")).unwrap();
        self.roles = Some(Roles {
            app: app.clone(),
            migrator: migrator.clone(),
            app_url,
            migration_url,
        });
        let mut admin = self.admin_connect().await;
        for role in [&app, &migrator] {
            diesel::sql_query(format!("DROP ROLE IF EXISTS {role}"))
                .execute(&mut admin)
                .await
                .expect("failed to clear stale test role");
            diesel::sql_query(format!("CREATE ROLE {role} LOGIN PASSWORD 'test-password'"))
                .execute(&mut admin)
                .await
                .expect("failed to create test role");
        }
        for grant in [
            format!(
                "GRANT CONNECT ON DATABASE \"{}\" TO {app}, {migrator}",
                self.name
            ),
            "REVOKE CREATE ON SCHEMA public FROM PUBLIC".to_owned(),
            format!("GRANT USAGE ON SCHEMA public TO {app}, {migrator}"),
            format!("GRANT CREATE ON SCHEMA public TO {migrator}"),
            format!(
                "ALTER DEFAULT PRIVILEGES FOR ROLE {migrator} IN SCHEMA public GRANT SELECT, INSERT, UPDATE, DELETE ON TABLES TO {app}"
            ),
            format!(
                "ALTER DEFAULT PRIVILEGES FOR ROLE {migrator} IN SCHEMA public GRANT USAGE, SELECT ON SEQUENCES TO {app}"
            ),
        ] {
            diesel::sql_query(grant)
                .execute(&mut admin)
                .await
                .expect("failed to grant test role privileges");
        }
    }
}

impl Drop for TestDatabase {
    fn drop(&mut self) {
        // Drop cannot await, and the test's runtime may already be gone, so the database is
        // dropped from a thread with a runtime of its own.
        let statement = format!("DROP DATABASE \"{}\" WITH (FORCE)", self.name);
        let roles = self.roles.take();
        let dropped = std::thread::spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("failed to start a runtime")
                .block_on(async {
                    run_on_server(&[statement]).await;
                    if let Some(Roles { app, migrator, .. }) = roles {
                        run_on_server(&[
                            format!("DROP ROLE IF EXISTS {app}"),
                            format!("DROP ROLE IF EXISTS {migrator}"),
                        ])
                        .await;
                    }
                });
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
    let url = super::test_setting(URL_VARIABLE)
        .unwrap_or_else(|| panic!("{URL_VARIABLE} must be set, in the environment or in .env"));
    Url::parse(&url).unwrap_or_else(|_| panic!("{URL_VARIABLE} is not a valid URL"))
}
