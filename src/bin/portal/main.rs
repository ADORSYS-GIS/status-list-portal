//! The portal: its HTTP server, and commands for its operators.

mod admin;
mod server;

use std::process::ExitCode;

use anyhow::Context;
use clap::Parser;
use status_list_portal::config::Config;
use status_list_portal::telemetry::{Process, Telemetry};
use tracing::error;

/// Developer portal for the Status List Server. Every command reads the `APP_*` settings
/// that `.env.template` lists, and the operators' commands expect the database that
/// `portal server` migrated.
#[derive(Debug, Parser)]
#[command(name = "portal")]
enum Command {
    /// Runs the HTTP server, after migrating the database.
    Server,
    #[command(flatten)]
    Admin(admin::Command),
}

fn main() -> anyhow::Result<ExitCode> {
    let command = Command::parse();
    load_dotenv()?;
    let config = Config::from_environment()
        .context("invalid configuration (.env.template lists every APP_* setting)")?;
    let process = match command {
        Command::Server => Process::Server,
        Command::Admin(_) => Process::Command,
    };
    let telemetry = Telemetry::from_env(config.env, process)?;
    telemetry.init_subscriber(config.log_format)?;

    let meter = telemetry.meter();
    let runtime = tokio::runtime::Runtime::new().context("failed to start the async runtime")?;
    let result = runtime.block_on(async {
        match command {
            Command::Server => server::run(config, &meter).await,
            Command::Admin(command) => admin::run(config, &meter, command).await,
        }
    });
    // The exporters run on threads of their own, and shutting them down blocks until they
    // have sent what they hold, so it waits for the runtime to stop.
    drop(runtime);
    let result = match result {
        // The server's logs are where its operators look, so the reason it stopped goes
        // there, in their format, and is exported before the exporters stop.
        Err(error) if process == Process::Server => {
            error!(error = %format!("{error:#}"), "the server stopped");
            Ok(ExitCode::FAILURE)
        }
        result => result.map(|()| ExitCode::SUCCESS),
    };
    telemetry.shutdown();
    result
}

/// Loads `.env` when it exists. A missing file is fine; an unreadable one is an error.
fn load_dotenv() -> anyhow::Result<()> {
    match dotenvy::dotenv() {
        Ok(_) => Ok(()),
        Err(error) if error.not_found() => Ok(()),
        Err(error) => Err(error).context("failed to load .env"),
    }
}

#[test]
fn verify_cli() {
    use clap::CommandFactory;
    Command::command().debug_assert();
}
