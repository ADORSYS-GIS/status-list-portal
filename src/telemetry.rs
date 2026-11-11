//! Logs, traces and metrics.
//!
//! Log lines always go to stdout. When the standard `OTEL_*` variables configure an OTLP
//! endpoint, traces, metrics and logs also go there, over HTTP with protobuf. The
//! OpenTelemetry SDK reads the other variables itself: headers, timeouts, compression,
//! sampling and resource attributes. `docs/telemetry.md` lists what the portal records.

mod sampler;
mod signals;

pub use sampler::{OPERATOR_COMMAND_SPAN, sampler};

use std::time::Duration;

use anyhow::{Context, anyhow};
use opentelemetry::KeyValue;
use opentelemetry::metrics::noop::NoopMeterProvider;
use opentelemetry::metrics::{Meter, MeterProvider};
use opentelemetry::trace::TracerProvider;
use opentelemetry_appender_tracing::layer::OpenTelemetryTracingBridge;
use opentelemetry_otlp::{LogExporter, MetricExporter, SpanExporter};
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::error::OTelSdkResult;
use opentelemetry_sdk::logs::SdkLoggerProvider;
use opentelemetry_sdk::metrics::{PeriodicReader, SdkMeterProvider};
use opentelemetry_sdk::resource::{
    EnvResourceDetector, ResourceDetector, TelemetryResourceDetector,
};
use opentelemetry_sdk::trace::{self, SdkTracerProvider};
use tracing::{Subscriber, warn};
use tracing_subscriber::Layer;
use tracing_subscriber::filter::{FilterExt, LevelFilter, Targets};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::util::SubscriberInitExt;
use uuid::Uuid;

use crate::Env;
use crate::util::logging::{self, LogFormat, LogOutput};
use signals::Signals;

/// The name of the portal's spans, metrics and logs: the service, unless `OTEL_SERVICE_NAME`
/// names it otherwise, and the instrumentation scope.
pub const SERVICE_NAME: &str = "status-list-portal";

/// Spans and events from these targets are never exported: the exporters' HTTP clients and
/// the SDK would otherwise report on their own exports.
const UNEXPORTED_TARGETS: [&str; 4] = ["opentelemetry", "hyper", "h2", "reqwest"];

/// The process that sends the telemetry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Process {
    /// The server, which exports every signal.
    Server,
    /// An operator's command, which exports no metrics: a process that lasts a second would
    /// only add series that end at once.
    Command,
}

impl Process {
    fn log_output(self) -> LogOutput {
        match self {
            Self::Server => LogOutput::Stdout,
            Self::Command => LogOutput::Stderr,
        }
    }
}

/// The exporters that the `OTEL_*` variables configure. Shut them down before the process
/// exits, or the telemetry they still hold is lost.
pub struct Telemetry {
    process: Process,
    tracer_provider: Option<SdkTracerProvider>,
    meter_provider: Option<SdkMeterProvider>,
    logger_provider: Option<SdkLoggerProvider>,
    /// How long the last exports may take on shutdown.
    shutdown_timeout: Duration,
}

impl Telemetry {
    /// Builds an exporter for each signal the `OTEL_*` variables configure. Fails when they
    /// ask for something the portal does not support, such as gRPC.
    pub fn from_env(env: Env, process: Process) -> anyhow::Result<Self> {
        let var = |name: &str| std::env::var(name).ok();
        let signals = Signals::from_lookup(var)?;
        let signals = Signals {
            metrics: signals.metrics && process == Process::Server,
            ..signals
        };
        let resource = resource(env, var("OTEL_SERVICE_NAME"));
        Ok(Self {
            process,
            tracer_provider: signals
                .traces
                .then(|| tracer_provider(&resource))
                .transpose()?,
            meter_provider: signals
                .metrics
                .then(|| meter_provider(&resource))
                .transpose()?,
            logger_provider: signals
                .logs
                .then(|| logger_provider(&resource))
                .transpose()?,
            shutdown_timeout: signals.shutdown_timeout(var),
        })
    }

    /// Where the portal records its metrics. Without a metrics exporter, nothing is kept.
    pub fn meter(&self) -> Meter {
        match &self.meter_provider {
            Some(provider) => provider.meter(SERVICE_NAME),
            None => NoopMeterProvider::new().meter(SERVICE_NAME),
        }
    }

    /// Installs the global subscriber: log lines go to stdout, or stderr for an operator's
    /// command, and spans and log records to their exporters.
    pub fn init_subscriber(&self, format: LogFormat) -> anyhow::Result<()> {
        let logs = match &self.logger_provider {
            Some(provider) => Some(log_layer(provider)?),
            None => None,
        };
        tracing_subscriber::registry()
            .with(logging::console_layer(format, self.process.log_output())?)
            .with(self.tracer_provider.as_ref().map(trace_layer))
            .with(logs)
            .try_init()
            .map_err(|error| anyhow!("failed to install the log subscriber: {error}"))
    }

    /// Exports what the exporters still hold, then stops them. Each waits up to the longest
    /// export timeout of the exported signals, and they stop side by side: one after the
    /// other, they could outlast the time a container gets to stop.
    pub fn shutdown(self) {
        let timeout = self.shutdown_timeout;
        std::thread::scope(|scope| {
            if let Some(provider) = &self.tracer_provider {
                scope.spawn(|| report_shutdown("trace", provider.shutdown_with_timeout(timeout)));
            }
            if let Some(provider) = &self.meter_provider {
                // The SDK's metric reader waits 5 seconds at most, whatever the timeout.
                scope.spawn(|| report_shutdown("metric", provider.shutdown_with_timeout(timeout)));
            }
            if let Some(provider) = &self.logger_provider {
                scope.spawn(|| report_shutdown("log", provider.shutdown_with_timeout(timeout)));
            }
        });
    }
}

fn report_shutdown(signal: &str, result: OTelSdkResult) {
    if let Err(error) = result {
        warn!(%error, "failed to shut down the {signal} exporter");
    }
}

/// The layer that turns spans into OpenTelemetry spans for `provider`. Spans at `INFO` and
/// above are exported whatever `RUST_LOG` says.
pub fn trace_layer<S>(provider: &SdkTracerProvider) -> impl Layer<S>
where
    S: Subscriber + for<'span> LookupSpan<'span>,
{
    tracing_opentelemetry::layer()
        .with_tracer(provider.tracer(SERVICE_NAME))
        .with_filter(exported_targets(LevelFilter::INFO))
}

/// The layer that exports log records: those `RUST_LOG` lets through to stdout.
fn log_layer<S>(provider: &SdkLoggerProvider) -> anyhow::Result<impl Layer<S>>
where
    S: Subscriber + for<'span> LookupSpan<'span>,
{
    let filter = logging::filter()?.and(exported_targets(LevelFilter::TRACE));
    Ok(OpenTelemetryTracingBridge::new(provider).with_filter(filter))
}

/// Every target at `level` and above, but the [`UNEXPORTED_TARGETS`].
fn exported_targets(level: LevelFilter) -> Targets {
    UNEXPORTED_TARGETS
        .into_iter()
        .fold(Targets::new().with_default(level), |targets, target| {
            targets.with_target(target, LevelFilter::OFF)
        })
}

/// What the telemetry is about. `service_name` comes from `OTEL_SERVICE_NAME`, and
/// `OTEL_RESOURCE_ATTRIBUTES` may override the other attributes.
fn resource(env: Env, service_name: Option<String>) -> Resource {
    let detectors: [Box<dyn ResourceDetector>; 2] = [
        Box::new(TelemetryResourceDetector),
        Box::new(EnvResourceDetector::new()),
    ];
    let builder = Resource::builder_empty()
        .with_service_name(SERVICE_NAME)
        .with_attributes([
            KeyValue::new("service.version", env!("CARGO_PKG_VERSION")),
            // Tells replicas apart, so their series do not mix.
            KeyValue::new("service.instance.id", Uuid::new_v4().to_string()),
            KeyValue::new("deployment.environment.name", env.as_str()),
        ])
        .with_detectors(&detectors);
    match service_name.filter(|name| !name.is_empty()) {
        Some(name) => builder.with_service_name(name),
        None => builder,
    }
    .build()
}

fn tracer_provider(resource: &Resource) -> anyhow::Result<SdkTracerProvider> {
    let exporter = SpanExporter::builder()
        .with_http()
        .build()
        .context("failed to build the OTLP trace exporter")?;
    Ok(SdkTracerProvider::builder()
        .with_resource(resource.clone())
        // The baseline is the sampler that `OTEL_TRACES_SAMPLER` configures.
        .with_sampler(sampler(trace::Config::default().sampler))
        .with_batch_exporter(exporter)
        .build())
}

fn meter_provider(resource: &Resource) -> anyhow::Result<SdkMeterProvider> {
    let exporter = MetricExporter::builder()
        .with_http()
        .build()
        .context("failed to build the OTLP metric exporter")?;
    Ok(SdkMeterProvider::builder()
        .with_resource(resource.clone())
        .with_reader(PeriodicReader::builder(exporter).build())
        .build())
}

fn logger_provider(resource: &Resource) -> anyhow::Result<SdkLoggerProvider> {
    let exporter = LogExporter::builder()
        .with_http()
        .build()
        .context("failed to build the OTLP log exporter")?;
    Ok(SdkLoggerProvider::builder()
        .with_resource(resource.clone())
        .with_batch_exporter(exporter)
        .build())
}

#[cfg(test)]
mod tests {
    use super::*;
    use opentelemetry::{Key, Value};

    fn attribute(resource: &Resource, key: &'static str) -> Option<Value> {
        resource.get(&Key::from_static_str(key))
    }

    #[test]
    fn the_resource_names_the_portal_and_where_it_runs() {
        let resource = resource(Env::Production, None);

        assert_eq!(
            attribute(&resource, "service.name"),
            Some(SERVICE_NAME.into())
        );
        assert_eq!(
            attribute(&resource, "service.version"),
            Some(env!("CARGO_PKG_VERSION").into())
        );
        assert_eq!(
            attribute(&resource, "deployment.environment.name"),
            Some("production".into())
        );
        assert!(attribute(&resource, "telemetry.sdk.language").is_some());
    }

    #[test]
    fn otel_service_name_renames_the_service() {
        let renamed = resource(Env::Development, Some("portal-staging".to_string()));
        assert_eq!(
            attribute(&renamed, "service.name"),
            Some("portal-staging".into())
        );
        let unnamed = resource(Env::Development, Some(String::new()));
        assert_eq!(
            attribute(&unnamed, "service.name"),
            Some(SERVICE_NAME.into())
        );
    }

    #[test]
    fn each_process_is_its_own_instance() {
        let first = resource(Env::Production, None);
        let second = resource(Env::Production, None);
        assert_ne!(
            attribute(&first, "service.instance.id"),
            attribute(&second, "service.instance.id")
        );
    }
}
