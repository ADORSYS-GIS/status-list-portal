//! What the app records: its metrics, and the spans of a test.

use opentelemetry::KeyValue;
use opentelemetry::metrics::{Meter, MeterProvider};
use opentelemetry_sdk::metrics::data::{AggregatedMetrics, MetricData, ResourceMetrics};
use opentelemetry_sdk::metrics::{InMemoryMetricExporter, PeriodicReader, SdkMeterProvider};
use opentelemetry_sdk::trace::{InMemorySpanExporter, Sampler, SdkTracerProvider, SpanData};
use status_list_portal::telemetry;
use std::sync::Once;

use tracing::level_filters::LevelFilter;
use tracing::span::{Attributes, Id, Record};
use tracing::subscriber::{DefaultGuard, Interest};
use tracing::{Event, Metadata, Subscriber};
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

/// Exports the spans this thread records while it lives, as the server would, kept or
/// dropped by the portal's sampler with `baseline`.
pub struct SpanRecorder {
    provider: SdkTracerProvider,
    exporter: InMemorySpanExporter,
    _guard: DefaultGuard,
}

impl SpanRecorder {
    pub fn start(baseline: Sampler) -> Self {
        keep_callsites_undecided();
        let exporter = InMemorySpanExporter::default();
        let provider = SdkTracerProvider::builder()
            .with_sampler(telemetry::sampler(Box::new(baseline)))
            .with_simple_exporter(exporter.clone())
            .build();
        let subscriber = tracing_subscriber::registry().with(telemetry::trace_layer(&provider));
        let guard = tracing::subscriber::set_default(subscriber);
        Self {
            provider,
            exporter,
            _guard: guard,
        }
    }

    /// The spans that ended so far.
    pub fn spans(&self) -> Vec<SpanData> {
        self.provider
            .force_flush()
            .expect("failed to export the spans");
        self.exporter
            .get_finished_spans()
            .expect("failed to read the spans")
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
/// undecided. Without it, a callsite first reached on a thread with no [`SpanRecorder`] can
/// keep "never" as its interest while another thread starts one, and that recorder then
/// misses the callsite's spans.
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
