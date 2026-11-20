//! The portal: its HTTP server, and commands for its operators.

mod admin;
mod migrate;
mod server;

use std::future::Future;
use std::process::ExitCode;

use anyhow::Context;
use clap::Parser;
use status_list_portal::config::{Config, MigrationConfig};
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
    let (env, log_format, process, failure_message) = match &action {
        Action::Server(config) => (
            config.env,
            config.log_format,
            Process::Server,
            Some("the server stopped"),
        ),
        Action::Migrate(config) => (
            config.env,
            config.log_format,
            Process::Command,
            Some("migration failed"),
        ),
        Action::Admin(config, _) => (config.env, config.log_format, Process::Command, None),
    };
    let telemetry = Telemetry::from_env(env, process)?;
    telemetry.init_subscriber(log_format)?;

    let meter = telemetry.meter();
    let runtime = tokio::runtime::Runtime::new().context("failed to start the async runtime")?;
    let result = runtime.block_on(async {
        match action {
            Action::Server(config) => server::run(config, &meter).await,
            Action::Admin(config, command) => admin::run(config, &meter, command).await,
            Action::Migrate(config) => migrate::run(config).await,
        }
    });
    // The exporters run on threads of their own, and shutting them down blocks until they
    // have sent what they hold, so it waits for the runtime to stop.
    drop(runtime);
    let result = match (result, failure_message) {
        // The server and migration job report failures in the configured log format
        // before the exporters stop.
        (Err(error), Some(message)) => {
            error!(error = %format!("{error:#}"), "{message}");
            Ok(ExitCode::FAILURE)
        }
        (result, _) => result.map(|()| ExitCode::SUCCESS),
    };
    telemetry.shutdown();
    result
}

/// Runs an operator command in its own trace and marks failures on that trace.
async fn run_operator_command<F>(name: &'static str, future: F) -> anyhow::Result<()>
where
    F: Future<Output = anyhow::Result<()>>,
{
    let span = info_span!(
        parent: None,
        OPERATOR_COMMAND_SPAN,
        otel.status_code = Empty,
        operator.command = name,
    );
    let result = future.instrument(span.clone()).await;
    if result.is_err() {
        span.record("otel.status_code", "ERROR");
    }
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
