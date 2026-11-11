//! The exporters, end to end: from the `OTEL_*` variables to an OTLP endpoint.
//!
//! The exporters read their settings from the environment, and the subscriber they feed is
//! global, so the export runs in a process of its own: this test binary, started again to
//! run [`export_every_signal`] alone.

use std::process::{Command, Output};

use status_list_portal::Env;
use status_list_portal::metrics::{Metrics, SignInOutcome};
use status_list_portal::telemetry::{Process, Telemetry};
use status_list_portal::util::logging::LogFormat;
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Set in the process that runs [`export_every_signal`], which does nothing without it.
const EXPORTING_PROCESS: &str = "PORTAL_TEST_EXPORTING_PROCESS";

#[tokio::test]
async fn every_signal_reaches_the_endpoint() {
    let collector = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&collector)
        .await;

    let output = export_to(&collector).await;

    let report = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.status.success(), "{report}");
    assert!(report.contains("1 passed"), "{report}");
    let received = collector.received_requests().await.unwrap();
    for path in ["/v1/traces", "/v1/metrics", "/v1/logs"] {
        let export = received
            .iter()
            .find(|request| request.url.path() == path)
            .unwrap_or_else(|| panic!("nothing was exported to {path}"));
        assert_eq!(export.headers["content-type"], "application/x-protobuf");
        assert!(!export.body.is_empty(), "{path} received an empty export");
    }
}

/// Runs [`export_every_signal`] in a new process that exports to `collector`, with no other
/// `OTEL_*` variable or `RUST_LOG` than the test's.
async fn export_to(collector: &MockServer) -> Output {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command.args(["--exact", "otlp::export_every_signal", "--nocapture"]);
    for (name, _) in std::env::vars_os() {
        if name.to_string_lossy().starts_with("OTEL_") || name == "RUST_LOG" {
            command.env_remove(name);
        }
    }
    command
        .env(EXPORTING_PROCESS, "1")
        .env("OTEL_EXPORTER_OTLP_ENDPOINT", collector.uri());
    tokio::task::spawn_blocking(move || command.output())
        .await
        .unwrap()
        .expect("failed to start the exporting process")
}

/// Records a span, a log line and a metric, and exports them as the server would.
#[test]
fn export_every_signal() {
    if std::env::var_os(EXPORTING_PROCESS).is_none() {
        return;
    }
    let telemetry = Telemetry::from_env(Env::Test, Process::Server).unwrap();
    telemetry.init_subscriber(LogFormat::Text).unwrap();

    Metrics::new(&telemetry.meter()).sign_in(SignInOutcome::Succeeded);
    tracing::info_span!("request").in_scope(|| tracing::info!("answered"));

    telemetry.shutdown();
}
