//! Backend of the Status List developer portal.
//!
//! The portal is the backend for its own frontend. It signs developers in and calls the
//! Status List Server's admin API on their behalf. Routes are declared in the `router`
//! module and implemented in [`controllers`].

pub mod admin_api;
pub mod auth;
pub mod client_address;
pub mod config;
pub mod controllers;
pub mod db;
pub mod github;
pub mod metrics;
pub mod middleware;
pub mod models;
pub mod openapi;
pub mod rate_limiter;
mod router;
pub mod sandbox;
/// Diesel table definitions generated from the database migrations.
pub mod schema;
pub mod server;
pub mod telemetry;
pub mod util;

pub use crate::server::{ServerContext, ServerContextInner};

use std::net::SocketAddr;

use serde::Deserialize;
use tokio::net::TcpListener;

/// Where the server runs. Some settings have safe defaults only outside production.
///
/// Production is the default, so a deployment that forgets `APP_ENV` keeps production's
/// checks.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Env {
    Development,
    /// Set by the test harness; never read from the environment.
    #[serde(skip)]
    Test,
    #[default]
    Production,
}

impl Env {
    /// The name `APP_ENV` gives it.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Development => "development",
            Self::Test => "test",
            Self::Production => "production",
        }
    }
}

/// Builds the router with every route and middleware.
pub fn build_handler(ctx: ServerContext) -> axum::Router {
    let origin = ctx.config.server.origin.clone();
    let metrics = ctx.metrics.clone();
    let frontend = ctx.frontend.clone();
    middleware::apply_middleware(router::build_router(ctx), &origin, metrics, frontend)
}

/// Serves requests on `listener` until `shutdown` completes, then lets the requests in
/// progress finish. Each request knows its peer's address: the client's, or that of the proxy
/// that forwards for it.
pub async fn serve(
    listener: TcpListener,
    ctx: ServerContext,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> std::io::Result<()> {
    let service = build_handler(ctx).into_make_service_with_connect_info::<SocketAddr>();
    axum::serve(listener, service)
        .with_graceful_shutdown(shutdown)
        .await
}
