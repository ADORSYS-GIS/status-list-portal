//! Runs the portal's HTTP server.

use anyhow::Context;
use secrecy::ExposeSecret;
use status_list_portal::admin_api::AdminApi;
use status_list_portal::config::Config;
use status_list_portal::github::GitHub;
use status_list_portal::{ServerContext, ServerContextInner, build_handler, db, util};
use tokio::net::TcpListener;
use tokio::signal;
use tracing::{info, warn};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    load_dotenv()?;
    let config = Config::from_environment()
        .context("invalid configuration (.env.template lists every APP_* setting)")?;
    util::logging::init(config.log_format)?;

    db::run_migrations(config.database.url.expose_secret()).await?;
    let db = db::pool(&config.database)?;
    let github = GitHub::new(&config.github, &config.server.origin)?;
    let admin_api = AdminApi::new(&config.admin_api)?;

    let bind_address = config.server.bind_address;
    let ctx = ServerContext::new(ServerContextInner {
        config,
        db,
        github,
        admin_api,
    });

    let terminate = terminate_signal()?;
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
