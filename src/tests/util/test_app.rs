use std::net::SocketAddr;
use std::sync::Arc;

use axum::Router;
use status_list_portal::config::{self, Config};
use status_list_portal::util::logging::LogFormat;
use status_list_portal::{Env, ServerContext, ServerContextInner, build_handler};

use super::MockAnonymousUser;

struct TestAppInner {
    router: Router,
}

/// The application under test, with every route and middleware.
#[derive(Clone)]
pub struct TestApp(Arc<TestAppInner>);

impl TestApp {
    pub fn init() -> TestAppBuilder {
        TestAppBuilder {
            config: simple_config(),
        }
    }

    pub fn router(&self) -> &Router {
        &self.0.router
    }
}

pub struct TestAppBuilder {
    config: Config,
}

impl TestAppBuilder {
    /// Builds the app and returns it with a user who is not signed in.
    pub async fn empty(self) -> (TestApp, MockAnonymousUser) {
        let ctx = ServerContext::new(ServerContextInner {
            config: self.config,
        });
        let app = TestApp(Arc::new(TestAppInner {
            router: build_handler(ctx),
        }));
        let anon = MockAnonymousUser { app: app.clone() };
        (app, anon)
    }
}

fn simple_config() -> Config {
    Config {
        env: Env::Test,
        log_format: LogFormat::Text,
        server: config::Server {
            bind_address: SocketAddr::from(([127, 0, 0, 1], 0)),
        },
    }
}
