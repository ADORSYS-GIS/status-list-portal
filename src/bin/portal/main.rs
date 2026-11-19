//! The portal: its HTTP server, and commands for its operators.

mod admin;
mod server;

use std::process::ExitCode;

use anyhow::Context;
use clap::Parser;
use status_list_portal::config::{Config, MigrationConfig};
use status_list_portal::db;
use status_list_portal::telemetry::{OPERATOR_COMMAND_SPAN, Process, Telemetry};
use tracing::field::Empty;
use tracing::{Instrument, error, info_span};

/// Developer portal for the Status List Server. Every command reads the `APP_*` settings
/// that `.env.template` lists, and the operators' commands expect the database that
/// `portal migrate` migrated.
#[derive(Debug, Parser)]
#[command(name = "portal")]
enum Command {
    /// Runs the HTTP server against an already migrated database.
    Server,
    /// Applies pending database migrations with the migration role.
    Migrate,
    #[command(flatten)]
    Admin(admin::Command),
}

enum Action {
    Server(Config),
    Migrate(MigrationConfig),
    Admin(Config, admin::Command),
}

fn main() -> anyhow::Result<ExitCode> {
    let command = Command::parse();
    load_dotenv()?;
    let action = match command {
        Command::Migrate => {
            let config = MigrationConfig::from_environment().context(
                "invalid migration configuration (.env.template lists the APP_DATABASE settings)",
            )?;
            Action::Migrate(config)
        }
        Command::Server => Action::Server(load_config()?),
        Command::Admin(command) => Action::Admin(load_config()?, command),
    };
    let (env, log_format, process) = match &action {
        Action::Server(config) => (config.env, config.log_format, Process::Server),
        Action::Migrate(config) => (config.env, config.log_format, Process::Command),
        Action::Admin(config, _) => (config.env, config.log_format, Process::Command),
    };
    let telemetry = Telemetry::from_env(env, process)?;
    telemetry.init_subscriber(log_format)?;

    let meter = telemetry.meter();
    let runtime = tokio::runtime::Runtime::new().context("failed to start the async runtime")?;
    let result = runtime.block_on(async {
        match action {
            Action::Server(config) => server::run(config, &meter).await,
            Action::Admin(config, command) => admin::run(config, &meter, command).await,
            Action::Migrate(config) => {
                let span = info_span!(
                    parent: None,
                    OPERATOR_COMMAND_SPAN,
                    otel.status_code = Empty,
                    operator.command = "migrate",
                );
                let result = db::run_migrations(&config.database, config.env)
                    .instrument(span.clone())
                    .await;
                if result.is_err() {
                    span.record("otel.status_code", "ERROR");
                }
                result.map(|_| ())
            }
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

fn load_config() -> anyhow::Result<Config> {
    Config::from_environment()
        .context("invalid configuration (.env.template lists every APP_* setting)")
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
