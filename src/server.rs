//! State shared by every request handler.

use crate::config::Config;
use crate::db;
use axum::extract::{FromRequestParts, State};
use derive_more::Deref;
use std::sync::Arc;

/// Components shared across requests.
pub struct ServerContextInner {
    pub config: Config,
    pub db: db::Pool,
}

/// Cheap-to-clone handle to [`ServerContextInner`]. Handlers take it as an extractor.
#[derive(Clone, FromRequestParts, Deref)]
#[from_request(via(State))]
pub struct ServerContext(Arc<ServerContextInner>);

impl ServerContext {
    pub fn new(inner: ServerContextInner) -> Self {
        Self(Arc::new(inner))
    }
}
