//! Log output.

use anyhow::{Context, anyhow};
use serde::Deserialize;
use tracing::level_filters::LevelFilter;
use tracing_subscriber::EnvFilter;

/// How log lines are written.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogFormat {
    /// Human-readable lines.
    #[default]
    Text,
    /// One JSON object per line, for log collectors.
    Json,
}

/// Installs the global subscriber.
///
/// `RUST_LOG` filters the output, in tracing `EnvFilter` syntax (default `info`).
pub fn init(format: LogFormat) -> anyhow::Result<()> {
    let filter = EnvFilter::builder()
        .with_default_directive(LevelFilter::INFO.into())
        .from_env()
        .context("RUST_LOG is not a valid log filter")?;
    let subscriber = tracing_subscriber::fmt().with_env_filter(filter);

    let installed = match format {
        LogFormat::Text => subscriber.try_init(),
        LogFormat::Json => subscriber.json().try_init(),
    };
    installed.map_err(|error| anyhow!("failed to install the log subscriber: {error}"))
}
