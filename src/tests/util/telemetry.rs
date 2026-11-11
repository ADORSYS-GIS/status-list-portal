//! What the app records: its metrics, and the spans, log records and log lines of a test.

use std::io::Write;
use std::sync::{Arc, Mutex, Once, PoisonError};

use opentelemetry::KeyValue;
use opentelemetry::metrics::{Meter, MeterProvider};
use opentelemetry_sdk::logs::{InMemoryLogExporter, SdkLogRecord, SdkLoggerProvider};
use opentelemetry_sdk::metrics::data::{AggregatedMetrics, MetricData, ResourceMetrics};
use opentelemetry_sdk::metrics::{InMemoryMetricExporter, PeriodicReader, SdkMeterProvider};
use opentelemetry_sdk::trace::{InMemorySpanExporter, Sampler, SdkTracerProvider, SpanData};
use status_list_portal::telemetry;
use status_list_portal::util::logging::{self, LogFormat};
use tracing::level_filters::LevelFilter;
use tracing::span::{Attributes, Id, Record};
use tracing::subscriber::{DefaultGuard, Interest};
use tracing::{Event, Metadata, Subscriber};
use tracing_subscriber::Layer;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::layer::SubscriberExt;

/// Keeps the metrics an app records, for the test to read.
pub struct MetricsRecorder {
    provider: SdkMeterProvider,
    exporter: InMemoryMetricExporter,
}

impl MetricsRecorder {
    pub fn new() -> Self {
        keep_callsites_undecided();
        let exporter = InMemoryMetricExporter::default();
        let provider = SdkMeterProvider::builder()
            .with_reader(PeriodicReader::builder(exporter.clone()).build())
            .build();
        Self { provider, exporter }
    }

    pub fn meter(&self) -> Meter {
        self.provider.meter(telemetry::SERVICE_NAME)
    }

    /// The metrics recorded so far.
    pub fn collect(&self) -> RecordedMetrics {
        self.provider
            .force_flush()
            .expect("failed to collect the metrics");
        let batches = self
            .exporter
            .get_finished_metrics()
            .expect("failed to read the metrics");
        // Every collection holds the totals so far, so the last one is enough.
        RecordedMetrics(batches.into_iter().last())
    }
}

/// The metrics of one collection.
pub struct RecordedMetrics(Option<ResourceMetrics>);

impl RecordedMetrics {
    /// The total of counter `name` over its data points that carry all `attributes`.
    #[track_caller]
    pub fn sum(&self, name: &str, attributes: &[(&str, &str)]) -> i64 {
        match self.metric(name) {
            Some(AggregatedMetrics::U64(MetricData::Sum(sum))) => sum
                .data_points()
                .filter(|point| carries(point.attributes(), attributes))
                .map(|point| i64::try_from(point.value()).unwrap())
                .sum(),
            Some(AggregatedMetrics::I64(MetricData::Sum(sum))) => sum
                .data_points()
                .filter(|point| carries(point.attributes(), attributes))
                .map(|point| point.value())
                .sum(),
            Some(other) => panic!("{name} is not a counter: {other:?}"),
            None => 0,
        }
    }

    /// How many values histogram `name` recorded with all `attributes`.
    #[track_caller]
    pub fn count(&self, name: &str, attributes: &[(&str, &str)]) -> u64 {
        match self.metric(name) {
            Some(AggregatedMetrics::F64(MetricData::Histogram(histogram))) => histogram
                .data_points()
                .filter(|point| carries(point.attributes(), attributes))
                .map(|point| point.count())
                .sum(),
            Some(other) => panic!("{name} is not a histogram: {other:?}"),
            None => 0,
        }
    }

    /// Whether `name` was recorded at all, so that a zero is a value and not a missing
    /// metric.
    pub fn has(&self, name: &str) -> bool {
        self.metric(name).is_some()
    }

    fn metric(&self, name: &str) -> Option<&AggregatedMetrics> {
        self.0
            .iter()
            .flat_map(ResourceMetrics::scope_metrics)
            .flat_map(|scope| scope.metrics())
            .find(|metric| metric.name() == name)
            .map(|metric| metric.data())
    }
}

/// Whether `attributes` hold each of `wanted`, values compared as text.
fn carries<'a>(attributes: impl Iterator<Item = &'a KeyValue>, wanted: &[(&str, &str)]) -> bool {
    let attributes: Vec<_> = attributes.collect();
    wanted.iter().all(|(key, value)| {
        attributes
            .iter()
            .any(|attribute| attribute.key.as_str() == *key && attribute.value.as_str() == *value)
    })
}

/// Records what this thread exports and logs while it lives, as the server would: spans,
/// kept or dropped by the portal's sampler with `baseline`, log records, and log lines at
/// `INFO` and above, the default `RUST_LOG`.
pub struct TelemetryRecorder {
    tracer_provider: SdkTracerProvider,
    spans: InMemorySpanExporter,
    logs: InMemoryLogExporter,
    lines: Lines,
    _guard: DefaultGuard,
}

impl TelemetryRecorder {
    /// Records log lines as text.
    pub fn start(baseline: Sampler) -> Self {
        Self::with_format(baseline, LogFormat::Text)
    }

    /// Records log lines as JSON.
    pub fn json(baseline: Sampler) -> Self {
        Self::with_format(baseline, LogFormat::Json)
    }

    fn with_format(baseline: Sampler, format: LogFormat) -> Self {
        keep_callsites_undecided();
        let spans = InMemorySpanExporter::default();
        let tracer_provider = SdkTracerProvider::builder()
            .with_sampler(telemetry::sampler(Box::new(baseline)))
            .with_simple_exporter(spans.clone())
            .build();
        let logs = InMemoryLogExporter::default();
        let logger_provider = SdkLoggerProvider::builder()
            .with_simple_exporter(logs.clone())
            .build();
        let lines = Lines::default();
        let subscriber = tracing_subscriber::registry()
            .with(logging::lines_layer(format, lines.clone()).with_filter(LevelFilter::INFO))
            .with(telemetry::trace_layer(&tracer_provider))
            .with(telemetry::log_layer(&logger_provider, LevelFilter::INFO));
        let guard = tracing::subscriber::set_default(subscriber);
        Self {
            tracer_provider,
            spans,
            logs,
            lines,
            _guard: guard,
        }
    }

    /// The spans that ended so far.
    pub fn spans(&self) -> Vec<SpanData> {
        self.tracer_provider
            .force_flush()
            .expect("failed to export the spans");
        self.spans
            .get_finished_spans()
            .expect("failed to read the spans")
    }

    /// The log records exported so far.
    pub fn logs(&self) -> Vec<SdkLogRecord> {
        self.logs
            .get_emitted_logs()
            .expect("failed to read the log records")
            .into_iter()
            .map(|log| log.record)
            .collect()
    }

    /// The log lines written so far.
    pub fn lines(&self) -> String {
        let lines = self.lines.0.lock().unwrap_or_else(PoisonError::into_inner);
        String::from_utf8(lines.clone()).expect("log lines are UTF-8")
    }
}

/// Keeps the log lines written to it.
#[derive(Clone, Default)]
struct Lines(Arc<Mutex<Vec<u8>>>);

impl Write for Lines {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for Lines {
    type Writer = Self;

    fn make_writer(&'a self) -> Self {
        self.clone()
    }
}

/// The span named `name`.
#[track_caller]
pub fn span<'a>(spans: &'a [SpanData], name: &str) -> &'a SpanData {
    spans
        .iter()
        .find(|span| span.name == name)
        .unwrap_or_else(|| panic!("no span is named {name:?}"))
}

/// The value of `span`'s attribute `key`, as text.
pub fn attribute(span: &SpanData, key: &str) -> Option<String> {
    span.attributes
        .iter()
        .find(|attribute| attribute.key.as_str() == key)
        .map(|attribute| attribute.value.as_str().into_owned())
}

/// Installs, once, a global subscriber that records nothing and leaves every callsite
/// undecided. Without it, a callsite first reached on a thread with no
/// [`TelemetryRecorder`] can keep "never" as its interest while another thread starts one,
/// and that recorder then misses the callsite's events and spans.
pub fn keep_callsites_undecided() {
    static INSTALLED: Once = Once::new();
    INSTALLED.call_once(|| {
        tracing::subscriber::set_global_default(Undecided)
            .expect("another global subscriber is installed");
    });
}

struct Undecided;

impl Subscriber for Undecided {
    fn register_callsite(&self, _: &'static Metadata<'static>) -> Interest {
        Interest::sometimes()
    }

    fn enabled(&self, _: &Metadata<'_>) -> bool {
        false
    }

    fn max_level_hint(&self) -> Option<LevelFilter> {
        None
    }

    fn new_span(&self, _: &Attributes<'_>) -> Id {
        Id::from_u64(1)
    }

    fn record(&self, _: &Id, _: &Record<'_>) {}

    fn record_follows_from(&self, _: &Id, _: &Id) {}

    fn event(&self, _: &Event<'_>) {}

    fn enter(&self, _: &Id) {}

    fn exit(&self, _: &Id) {}
}
