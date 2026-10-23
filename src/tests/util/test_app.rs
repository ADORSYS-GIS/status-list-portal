use std::net::SocketAddr;
use std::num::NonZeroUsize;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

use axum::Router;
use axum::response::IntoResponse;
use axum_extra::extract::PrivateCookieJar;
use axum_extra::extract::cookie::Cookie;
use diesel_async::AsyncPgConnection;
use http::header::SET_COOKIE;
use secrecy::SecretString;
use status_list_portal::admin_api::SigningAlgorithm;
use status_list_portal::auth::Session;
use status_list_portal::config::{self, Config, PublicOrigin, SessionKey};
use status_list_portal::models::{NewUser, TermsAcceptance, User};
use status_list_portal::util::logging::LogFormat;
use status_list_portal::{Env, ServerContext, build_handler};
use url::Url;
use wiremock::MockServer;

use super::{MockAnonymousUser, MockCookieUser, TestDatabase};

/// Where the tests pretend browsers reach the portal.
pub const ORIGIN: &str = "https://portal.example.com";
/// Where the tests pretend issuers reach the management API.
pub const MANAGEMENT_URL: &str = "https://status.example.com";
pub const TERMS_VERSION: &str = "2026-10-22";

/// Nothing listens on port 1, so connections are refused at once.
const UNREACHABLE_DATABASE_URL: &str = "postgres://portal@127.0.0.1:1/portal";
/// What the app signs its admin tokens as, and with.
pub const ADMIN_PRINCIPAL: &str = "portal";
pub const ADMIN_AUDIENCE: &str = "test-admin-audience";
pub const ADMIN_KEY_ID: &str = "test-admin-key";
pub const ADMIN_SIGNING_KEY_FILE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/test_data/admin-keys/es256.pem"
);

static NEXT_GH_ID: AtomicI64 = AtomicI64::new(1);

struct TestAppInner {
    router: Router,
    /// Stands in for GitHub's sign-in pages and API.
    github: MockServer,
    /// Stands in for the Status List Server's admin API.
    admin_api: MockServer,
    ctx: ServerContext,
    session_key: SessionKey,
    db: Option<TestDatabase>,
}

/// The application under test, with every route and middleware, backed by a database of its
/// own.
#[derive(Clone)]
pub struct TestApp(Arc<TestAppInner>);

impl TestApp {
    pub fn init() -> TestAppBuilder {
        TestAppBuilder {
            unreachable_database: false,
            sandbox: config::Sandbox::default(),
        }
    }

    pub fn router(&self) -> &Router {
        &self.0.router
    }

    pub fn github(&self) -> &MockServer {
        &self.0.github
    }

    pub fn admin_api(&self) -> &MockServer {
        &self.0.admin_api
    }

    /// The state the app's handlers share, such as its admin API client.
    pub fn ctx(&self) -> &ServerContext {
        &self.0.ctx
    }

    pub async fn db_conn(&self) -> AsyncPgConnection {
        self.0
            .db
            .as_ref()
            .expect("the app has no database")
            .connect()
            .await
    }

    /// Creates a user and returns them signed in.
    pub async fn db_new_user(&self, login: &str) -> MockCookieUser {
        let email = format!("{login}@example.com");
        let new_user = NewUser {
            gh_id: NEXT_GH_ID.fetch_add(1, Ordering::Relaxed),
            gh_login: login,
            name: None,
            email: &email,
        };
        let user = new_user
            .insert_or_update(&mut self.db_conn().await)
            .await
            .expect("failed to create the user");
        let cookie = self.session_cookie(&user);
        MockCookieUser {
            app: self.clone(),
            user,
            cookie,
        }
    }

    /// Creates a user who may register issuers: they accepted the current terms of use.
    pub async fn db_new_registrant(&self, login: &str) -> MockCookieUser {
        let user = self.db_new_user(login).await;
        self.accept_terms(user.user()).await;
        user
    }

    /// Records that `user` accepted the current terms of use.
    pub async fn accept_terms(&self, user: &User) {
        TermsAcceptance {
            user_id: user.id,
            terms_version: TERMS_VERSION,
            email: &user.email,
        }
        .record(&mut self.db_conn().await)
        .await
        .expect("failed to record the acceptance");
    }

    /// A `Cookie` header with a new session for `user`, encrypted the way the app does it.
    pub fn session_cookie(&self, user: &User) -> String {
        self.cookie_for(Session::new(user))
    }

    /// A `Cookie` header that carries `session`, encrypted the way the app does it.
    pub fn cookie_for(&self, session: Session) -> String {
        let jar =
            PrivateCookieJar::new(self.0.session_key.key().clone()).add(session.into_cookie());
        let response = jar.into_response();
        let set_cookie = response.headers()[SET_COOKIE]
            .to_str()
            .expect("cookies are ASCII");
        let cookie = Cookie::parse(set_cookie).expect("the jar writes valid cookies");
        format!("{}={}", cookie.name(), cookie.value())
    }
}

pub struct TestAppBuilder {
    unreachable_database: bool,
    sandbox: config::Sandbox,
}

impl TestAppBuilder {
    /// Points the app at a database server that refuses connections.
    pub fn with_unreachable_database(self) -> Self {
        Self {
            unreachable_database: true,
            ..self
        }
    }

    /// Limits sandbox issuers as `sandbox` says, instead of with the defaults.
    pub fn with_sandbox(self, sandbox: config::Sandbox) -> Self {
        Self { sandbox, ..self }
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
        let github = MockServer::start().await;
        let github_url = Url::parse(&github.uri()).expect("the mock server has a valid URL");
        let admin_api = MockServer::start().await;
        let admin_api_url = Url::parse(&admin_api.uri()).expect("the mock server has a valid URL");
        let session_key = SessionKey::generate();

        let config = Config {
            sandbox: self.sandbox,
            ..simple_config(
                database_url,
                &github_url,
                admin_api_url,
                session_key.clone(),
            )
        };
        let ctx = ServerContext::from_config(config).expect("failed to build the server context");
        let app = TestApp(Arc::new(TestAppInner {
            router: build_handler(ctx.clone()),
            github,
            admin_api,
            ctx,
            session_key,
            db: test_db,
        }));
        let anon = MockAnonymousUser { app: app.clone() };
        (app, anon)
    }
}

fn simple_config(
    database_url: String,
    github_url: &Url,
    admin_api_url: Url,
    session_key: SessionKey,
) -> Config {
    Config {
        env: Env::Test,
        log_format: LogFormat::Text,
        server: config::Server {
            bind_address: SocketAddr::from(([127, 0, 0, 1], 0)),
            origin: PublicOrigin::parse(ORIGIN).expect("the origin is valid"),
        },
        database: config::Database {
            url: SecretString::from(database_url),
            pool_size: NonZeroUsize::new(2).unwrap(),
        },
        session: config::Session { key: session_key },
        github: config::GitHub {
            client_id: "test-client-id".to_string(),
            client_secret: SecretString::from("test-client-secret"),
            web_url: github_url.clone(),
            api_url: github_url.clone(),
        },
        admin_api: config::AdminApi {
            url: admin_api_url,
            principal: ADMIN_PRINCIPAL.to_string(),
            audience: ADMIN_AUDIENCE.to_string(),
            signing_key_file: ADMIN_SIGNING_KEY_FILE.into(),
            key_id: ADMIN_KEY_ID.to_string(),
            algorithm: SigningAlgorithm::Es256,
        },
        management_api: config::ManagementApi {
            url: Url::parse(MANAGEMENT_URL).expect("the URL is valid"),
            audience: MANAGEMENT_URL.to_string(),
        },
        sandbox: config::Sandbox::default(),
        terms: config::Terms {
            version: TERMS_VERSION.to_string(),
            url: Url::parse(&format!("{ORIGIN}/terms")).expect("the URL is valid"),
        },
    }
}
