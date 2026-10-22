use std::net::SocketAddr;
use std::num::NonZeroUsize;
use std::sync::Arc;

use axum::Router;
use secrecy::SecretString;
use status_list_portal::config::{self, Config};
use status_list_portal::util::logging::LogFormat;
use status_list_portal::{Env, ServerContext, ServerContextInner, build_handler, db};

use super::{MockAnonymousUser, TestDatabase};

/// Nothing listens on port 1, so connections are refused at once.
const UNREACHABLE_DATABASE_URL: &str = "postgres://portal@127.0.0.1:1/portal";

struct TestAppInner {
    router: Router,
    /// Kept so the database lives as long as the app.
    _db: Option<TestDatabase>,
}

/// The application under test, with every route and middleware, backed by a database of its
/// own.
#[derive(Clone)]
pub struct TestApp(Arc<TestAppInner>);

impl TestApp {
    pub fn init() -> TestAppBuilder {
        TestAppBuilder {
            unreachable_database: false,
        }
    }

    pub fn router(&self) -> &Router {
        &self.0.router
    }
}

pub struct TestAppBuilder {
    unreachable_database: bool,
}

impl TestAppBuilder {
    /// Points the app at a database server that refuses connections.
    pub fn with_unreachable_database(self) -> Self {
        Self {
            unreachable_database: true,
        }
    }

    /// Builds the app and returns it with a user who is not signed in.
    pub async fn empty(self) -> (TestApp, MockAnonymousUser) {
        let (test_db, database_url) = if self.unreachable_database {
            (None, UNREACHABLE_DATABASE_URL.to_string())
        } else {
            let test_db = TestDatabase::new().await;
            let url = test_db.url().to_string();
            (Some(test_db), url)
        };

        let config = simple_config(database_url);
        let db = db::pool(&config.database).expect("failed to build the database pool");
        let ctx = ServerContext::new(ServerContextInner { config, db });
        let app = TestApp(Arc::new(TestAppInner {
            router: build_handler(ctx),
            _db: test_db,
        }));
        let anon = MockAnonymousUser { app: app.clone() };
        (app, anon)
    }
}

fn simple_config(database_url: String) -> Config {
    Config {
        env: Env::Test,
        log_format: LogFormat::Text,
        server: config::Server {
            bind_address: SocketAddr::from(([127, 0, 0, 1], 0)),
        },
        database: config::Database {
            url: SecretString::from(database_url),
            pool_size: NonZeroUsize::new(2).unwrap(),
        },
    }
}
