//! The HTTP server.

use anyhow::Context;
use secrecy::ExposeSecret;
use status_list_portal::config::Config;
use status_list_portal::{ServerContext, build_handler, db};
use tokio::net::TcpListener;
use tokio::signal;
use tracing::{info, warn};

/// Runs migrations, then serves requests until SIGTERM or Ctrl+C.
pub async fn run(config: Config) -> anyhow::Result<()> {
    db::run_migrations(config.database.url.expose_secret()).await?;
    let bind_address = config.server.bind_address;
    let mut ctx = ServerContext::from_config(config)?;

    let terminate = terminate_signal()?;
    let listener = TcpListener::bind(bind_address)
        .await
        .with_context(|| format!("failed to bind {bind_address}"))?;
    ctx.start_readiness()?;

    info!(%bind_address, "listening");
    axum::serve(listener, build_handler(ctx))
        .with_graceful_shutdown(shutdown_signal(terminate))
        .await
        .context("server error")
}

/// Resolves on SIGTERM, which orchestrators send to stop the server.
#[cfg(unix)]
fn terminate_signal() -> anyhow::Result<impl Future<Output = ()>> {
    use tokio::signal::unix::{SignalKind, signal};

    let mut terminate = signal(SignalKind::terminate()).context("failed to listen for SIGTERM")?;
    Ok(async move {
        terminate.recv().await;
    })
}

/// SIGTERM exists only on Unix. Elsewhere, Ctrl+C stops the server.
#[cfg(not(unix))]
fn terminate_signal() -> anyhow::Result<impl Future<Output = ()>> {
    Ok(std::future::pending())
}

async fn shutdown_signal(terminate: impl Future<Output = ()>) {
    let ctrl_c = async {
        if let Err(error) = signal::ctrl_c().await {
            warn!(%error, "failed to listen for Ctrl+C; only SIGTERM stops the server");
            std::future::pending::<()>().await;
        }
    };

    tokio::select! {
        () = ctrl_c => {}
        () = terminate => {}
    }
    info!("shutting down");
}
