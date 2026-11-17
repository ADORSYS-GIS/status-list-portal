//! The portal: its HTTP server, and commands for its operators.

mod admin;
mod server;

use anyhow::Context;
use clap::Parser;
use status_list_portal::config::{Config, MigrationConfig};
use status_list_portal::db;
use status_list_portal::util;
use status_list_portal::util::logging::LogOutput;

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

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let command = Command::parse();
    load_dotenv()?;
    match command {
        Command::Migrate => {
            let config = MigrationConfig::from_environment().context(
                "invalid migration configuration (.env.template lists the APP_DATABASE settings)",
            )?;
            util::logging::init(config.log_format, LogOutput::Stderr)?;
            db::run_migrations(&config.database, config.env).await?;
            Ok(())
        }
        Command::Server => server::run(load_config(LogOutput::Stdout)?).await,
        Command::Admin(command) => admin::run(load_config(LogOutput::Stderr)?, command).await,
    }
}

fn load_config(output: LogOutput) -> anyhow::Result<Config> {
    let config = Config::from_environment()
        .context("invalid configuration (.env.template lists every APP_* setting)")?;
    util::logging::init(config.log_format, output)?;
    Ok(config)
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
