//! Backend of the Status List developer portal.
//!
//! The portal is the backend for its own frontend. It signs developers in and calls the
//! Status List Server's admin API on their behalf. Routes are declared in the `router`
//! module and implemented in [`controllers`].

pub mod config;
pub mod controllers;
pub mod middleware;
pub mod openapi;
mod router;
pub mod server;
pub mod util;

pub use crate::server::{ServerContext, ServerContextInner};

use serde::Deserialize;

/// Where the server runs. Some settings have safe defaults only outside production.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Env {
    #[default]
    Development,
    /// Set by the test harness; never read from the environment.
    #[serde(skip)]
    Test,
    Production,
}

/// Builds the router with every route and middleware.
pub fn build_handler(ctx: ServerContext) -> axum::Router {
    middleware::apply_middleware(router::build_router(ctx))
}
