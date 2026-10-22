//! Runs the portal's HTTP server.

use anyhow::Context;
use secrecy::ExposeSecret;
use status_list_portal::config::Config;
use status_list_portal::{ServerContext, ServerContextInner, build_handler, db, util};
use tokio::net::TcpListener;
use tokio::signal;
use tokio::signal::unix::{Signal, SignalKind};
use tracing::{info, warn};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    load_dotenv()?;
    let config = Config::from_environment()
        .context("invalid configuration (.env.template lists every APP_* setting)")?;
    util::logging::init(config.log_format)?;

    db::run_migrations(config.database.url.expose_secret()).await?;
    let db = db::pool(&config.database)?;

    let bind_address = config.server.bind_address;
    let ctx = ServerContext::new(ServerContextInner { config, db });

    let terminate =
        signal::unix::signal(SignalKind::terminate()).context("failed to listen for SIGTERM")?;
    let listener = TcpListener::bind(bind_address)
        .await
        .with_context(|| format!("failed to bind {bind_address}"))?;

    info!(%bind_address, "listening");
    axum::serve(listener, build_handler(ctx))
        .with_graceful_shutdown(shutdown_signal(terminate))
        .await
        .context("server error")
}

/// Loads `.env` when it exists. A missing file is fine; an unreadable one is an error.
fn load_dotenv() -> anyhow::Result<()> {
    match dotenvy::dotenv() {
        Ok(_) => Ok(()),
        Err(error) if error.not_found() => Ok(()),
        Err(error) => Err(error).context("failed to load .env"),
    }
}

async fn shutdown_signal(mut terminate: Signal) {
    let ctrl_c = async {
        if let Err(error) = signal::ctrl_c().await {
            warn!(%error, "failed to listen for Ctrl+C; only SIGTERM stops the server");
            std::future::pending::<()>().await;
        }
    };

    tokio::select! {
        () = ctrl_c => {}
        _ = terminate.recv() => {}
    }
    info!("shutting down");
}
