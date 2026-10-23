//! State shared by every request handler.

use crate::admin_api::AdminApi;
use crate::config::Config;
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
        let db = db::pool(&config.database)?;
        let github = GitHub::new(&config.github, &config.server.origin)?;
        let admin_api = AdminApi::new(&config.admin_api)?;
        let frontend = match &config.server.frontend_dir {
            Some(dir) => Some(Arc::new(Frontend::load(dir)?)),
            None => None,
        };
        Ok(Self::new(ServerContextInner {
            config,
            db,
            github,
            admin_api,
            frontend,
        }))
    }
}

/// Lets handlers read and write encrypted cookies with `PrivateCookieJar`.
impl FromRef<ServerContext> for Key {
    fn from_ref(ctx: &ServerContext) -> Self {
        ctx.config.session.key.key().clone()
    }
}
