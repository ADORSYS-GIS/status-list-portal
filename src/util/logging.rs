//! Log lines on stdout, or stderr for operator commands. [`crate::telemetry`] installs them
//! with the exporters.

use anyhow::Context;
use serde::Deserialize;
use tracing::Subscriber;
use tracing::level_filters::LevelFilter;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::fmt::writer::BoxMakeWriter;
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::{EnvFilter, Layer};

/// How log lines are written.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogFormat {
    /// Human-readable lines.
    #[default]
    Text,
    /// One JSON object per line, for log collectors. A line written in an exported span
    /// carries its trace and span ids, so the logs and traces of a request join up.
    Json,
}

/// Where log lines go.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogOutput {
    /// The server's standard output.
    Stdout,
    /// Standard error, so a command's own output stays apart.
    Stderr,
}

impl LogOutput {
    fn writer(self) -> BoxMakeWriter {
        match self {
            Self::Stdout => BoxMakeWriter::new(std::io::stdout),
            Self::Stderr => BoxMakeWriter::new(std::io::stderr),
        }
    }
}

/// The layer that writes log lines to `output`, as `format` says, for the events [`filter`]
/// lets through.
pub(crate) fn console_layer<S>(
    format: LogFormat,
    output: LogOutput,
) -> anyhow::Result<Box<dyn Layer<S> + Send + Sync + 'static>>
where
    S: Subscriber + for<'span> LookupSpan<'span>,
{
    Ok(lines_layer(format, output.writer())
        .with_filter(filter()?)
        .boxed())
}

/// The layer that writes every event to `writer` as a line, as `format` says.
pub fn lines_layer<S, W>(format: LogFormat, writer: W) -> Box<dyn Layer<S> + Send + Sync + 'static>
where
    S: Subscriber + for<'span> LookupSpan<'span>,
    W: for<'writer> MakeWriter<'writer> + Send + Sync + 'static,
{
    match format {
        LogFormat::Text => tracing_subscriber::fmt::layer().with_writer(writer).boxed(),
        // The span list holds the fields of every span the event is in, the current one's
        // included, so the current span is not written again on its own.
        LogFormat::Json => json_subscriber::fmt::layer()
            .with_writer(writer)
            .flatten_event(true)
            .with_current_span(false)
            .with_flat_span_list(true)
            .with_opentelemetry_ids(true)
            .boxed(),
    }
}

/// `RUST_LOG`, in tracing `EnvFilter` syntax (default `info`).
pub(crate) fn filter() -> anyhow::Result<EnvFilter> {
    EnvFilter::builder()
        .with_default_directive(LevelFilter::INFO.into())
        .from_env()
        .context("RUST_LOG is not a valid log filter")
}
