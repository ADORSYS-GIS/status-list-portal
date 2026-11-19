//! Commands for the portal's operators.

mod approve_registration;
mod list_registrations;
mod reject_registration;

use opentelemetry::metrics::Meter;
use status_list_portal::ServerContext;
use status_list_portal::config::Config;
use status_list_portal::telemetry::OPERATOR_COMMAND_SPAN;
use tracing::field::Empty;
use tracing::{Instrument, info_span};

#[derive(Debug, clap::Subcommand)]
pub enum Command {
    /// Lists the registrations that wait for approval, oldest first.
    ListRegistrations,
    ApproveRegistration(approve_registration::Opts),
    RejectRegistration(reject_registration::Opts),
}

/// Runs `command` in a trace of its own, which is always kept.
pub async fn run(config: Config, meter: &Meter, command: Command) -> anyhow::Result<()> {
    let ctx = ServerContext::from_config(config, meter)?;
    let span = info_span!(
        parent: None,
        OPERATOR_COMMAND_SPAN,
        otel.status_code = Empty,
        operator.command = command.name(),
    );
    let result = async {
        match command {
            Command::ListRegistrations => list_registrations::run(&ctx).await,
            Command::ApproveRegistration(opts) => approve_registration::run(&ctx, opts).await,
            Command::RejectRegistration(opts) => reject_registration::run(&ctx, opts).await,
        }
    }
    .instrument(span.clone())
    .await;
    if result.is_err() {
        span.record("otel.status_code", "ERROR");
    }
    result
}

impl Command {
    /// The command as operators type it.
    fn name(&self) -> &'static str {
        match self {
            Self::ListRegistrations => "list-registrations",
            Self::ApproveRegistration(_) => "approve-registration",
            Self::RejectRegistration(_) => "reject-registration",
        }
    }
}
