//! Commands for the portal's operators.

mod approve_registration;
mod list_registrations;
mod reject_registration;

use status_list_portal::ServerContext;
use status_list_portal::config::Config;

#[derive(Debug, clap::Subcommand)]
pub enum Command {
    /// Lists the registrations that wait for approval, oldest first.
    ListRegistrations,
    ApproveRegistration(approve_registration::Opts),
    RejectRegistration(reject_registration::Opts),
}

pub async fn run(config: Config, command: Command) -> anyhow::Result<()> {
    let ctx = ServerContext::from_config(config)?;
    match command {
        Command::ListRegistrations => list_registrations::run(&ctx).await,
        Command::ApproveRegistration(opts) => approve_registration::run(&ctx, opts).await,
        Command::RejectRegistration(opts) => reject_registration::run(&ctx, opts).await,
    }
}
