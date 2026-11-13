//! State shared by every request handler.

use crate::admin_api::AdminApi;
use crate::config::Config;
use crate::controllers::health::DatabaseReadiness;
use crate::db;
use crate::github::GitHub;
use crate::middleware::Frontend;
use axum::extract::{FromRef, FromRequestParts, State};
use axum_extra::extract::cookie::Key;
use derive_more::Deref;
use std::sync::Arc;

/// Components shared across requests.
pub struct ServerContextInner {
    pub config: Config,
    pub db: db::Pool,
    pub readiness: Option<DatabaseReadiness>,
    pub github: GitHub,
    pub admin_api: AdminApi,
    /// The built frontend, when the server serves it.
    pub frontend: Option<Arc<Frontend>>,
}

/// Cheap-to-clone handle to [`ServerContextInner`]. Handlers take it as an extractor.
#[derive(Clone, FromRequestParts, Deref)]
#[from_request(via(State))]
pub struct ServerContext(Arc<ServerContextInner>);

impl ServerContext {
    pub fn new(inner: ServerContextInner) -> Self {
        Self(Arc::new(inner))
    }

    /// Builds the shared components from the configuration. Opens no connection yet.
    pub fn from_config(config: Config) -> anyhow::Result<Self> {
        let db = db::pool(&config.database, config.env)?;
        let github = GitHub::new(&config.github, &config.server.origin)?;
        let admin_api = AdminApi::new(&config.admin_api)?;
        let frontend = match &config.server.frontend_dir {
            Some(dir) => Some(Arc::new(Frontend::load(dir)?)),
            None => None,
        };
        Ok(Self::new(ServerContextInner {
            config,
            db,
            readiness: None,
            github,
            admin_api,
            frontend,
        }))
    }

    /// Starts database checks for an HTTP server before sharing its context.
    pub fn start_readiness(&mut self) -> anyhow::Result<()> {
        let inner = Arc::get_mut(&mut self.0).ok_or_else(|| {
            anyhow::anyhow!("cannot start readiness after cloning the server context")
        })?;
        if inner.readiness.is_some() {
            anyhow::bail!("database readiness is already running");
        }
        let pool = db::readiness_pool(&inner.config.database, inner.config.env)?;
        inner.readiness = Some(DatabaseReadiness::start(pool));
        Ok(())
    }
}

/// Lets handlers read and write encrypted cookies with `PrivateCookieJar`.
impl FromRef<ServerContext> for Key {
    fn from_ref(ctx: &ServerContext) -> Self {
        ctx.config.session.key.key().clone()
    }
}
