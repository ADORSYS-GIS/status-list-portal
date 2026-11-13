//! What the portal records about the requests it answers: spans, log records, log lines and
//! metrics.

use axum::body::Body;
use http::header::USER_AGENT;
use http::{Method, StatusCode};
use opentelemetry::trace::{SpanId, SpanKind, Status};
use opentelemetry_sdk::trace::Sampler;
use serde_json::Value;
use wiremock::matchers::{method, path};
use wiremock::{Mock, ResponseTemplate};

use crate::session::{github_approves, sign_in, verified_primary_email};
use crate::util::admin_api::{
    Counts, issuer_json, issuer_response, new_issuer_body, sandbox_counts,
};
use crate::util::telemetry::{TelemetryRecorder, attribute, span};
use crate::util::{MockCookieUser, RequestHelper, TestApp};

const CALLBACK_WITH_SECRETS: &str =
    "/api/v1/session/github/callback?code=secret-code&state=secret-state";

/// Sets up the admin API to accept a registration from `user`.
async fn admin_api_accepts_a_registration(app: &TestApp, user: &MockCookieUser) {
    sandbox_counts(app.admin_api(), user.user(), Counts::default()).await;
    Mock::given(method("POST"))
        .and(path("/admin/v1/issuers"))
        .respond_with(issuer_response(
            201,
            "\"1\"",
            &issuer_json(user.user().gh_id),
        ))
        .mount(app.admin_api())
        .await;
}

#[tokio::test]
async fn requests_are_traced_by_route_without_their_query() {
    let recorder = TelemetryRecorder::start(Sampler::AlwaysOn);
    let (_app, anon) = TestApp::init().empty().await;

    let response = anon.get(CALLBACK_WITH_SECRETS).await;

    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let spans = recorder.spans();
    let request = span(&spans, "GET /api/v1/session/github/callback");
    assert_eq!(request.span_kind, SpanKind::Server);
    assert_eq!(
        attribute(request, "http.route").as_deref(),
        Some("/api/v1/session/github/callback")
    );
    assert_eq!(
        attribute(request, "http.response.status_code").as_deref(),
        Some("303")
    );
    assert!(!format!("{spans:?}").contains("secret"));
}

#[tokio::test]
async fn values_clients_choose_are_cut_short() {
    let recorder = TelemetryRecorder::start(Sampler::AlwaysOn);
    let (_app, anon) = TestApp::init().empty().await;
    let long = "a".repeat(1000);
    let request = anon
        .request_builder(Method::GET, &format!("/{long}"))
        .header(USER_AGENT, &long)
        .body(Body::empty())
        .unwrap();

    let response = anon.run(request).await;

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let spans = recorder.spans();
    let request = span(&spans, "GET");
    assert_eq!(
        attribute(request, "url.path").map(|path| path.len()),
        Some(256)
    );
    let user_agent = attribute(request, "user_agent.original");
    assert_eq!(user_agent.map(|agent| agent.len()), Some(256));
}

#[tokio::test]
async fn registrations_are_always_traced_into_the_admin_api() {
    let recorder = TelemetryRecorder::start(Sampler::AlwaysOff);
    let (app, _anon) = TestApp::init().empty().await;
    let user = app.db_new_registrant("octocat").await;
    admin_api_accepts_a_registration(&app, &user).await;

    let response = user.post("/api/v1/issuers", &new_issuer_body()).await;

    assert_eq!(response.status(), StatusCode::CREATED);
    let spans = recorder.spans();
    let request = span(&spans, "POST /api/v1/issuers");
    let call = span(&spans, "POST /admin/v1/issuers");
    assert_eq!(call.span_kind, SpanKind::Client);
    assert_eq!(call.parent_span_id, request.span_context.span_id());
    assert_eq!(
        attribute(call, "url.template").as_deref(),
        Some("/admin/v1/issuers")
    );

    // The admin API continues the trace from the call's span.
    let received = app.admin_api().received_requests().await.unwrap();
    let create = received
        .iter()
        .find(|request| request.method.as_str() == "POST")
        .unwrap();
    let expected = format!(
        "00-{}-{}-01",
        call.span_context.trace_id(),
        call.span_context.span_id()
    );
    assert_eq!(create.headers["traceparent"], expected.as_str());
}

#[tokio::test]
async fn sign_ins_are_always_traced() {
    let recorder = TelemetryRecorder::start(Sampler::AlwaysOff);
    let (_app, anon) = TestApp::init().empty().await;

    anon.get(CALLBACK_WITH_SECRETS).await;

    span(&recorder.spans(), "GET /api/v1/session/github/callback");
}

#[tokio::test]
async fn health_checks_are_never_traced() {
    let recorder = TelemetryRecorder::start(Sampler::AlwaysOn);
    let (_app, anon) = TestApp::init().empty().await;

    let response = anon.get("/health/live").await;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(recorder.spans(), []);
}

#[tokio::test]
async fn requests_ignore_the_trace_context_they_carry() {
    let recorder = TelemetryRecorder::start(Sampler::AlwaysOn);
    let (_app, anon) = TestApp::init().empty().await;
    let sent_trace = "0af7651916cd43dd8448eb211c80319c";
    let request = anon
        .request_builder(Method::GET, "/api/v1/terms")
        .header(
            "traceparent",
            format!("00-{sent_trace}-b7ad6b7169203331-01"),
        )
        .body(Body::empty())
        .unwrap();

    let response = anon.run(request).await;

    assert_eq!(response.status(), StatusCode::OK);
    let spans = recorder.spans();
    let request = span(&spans, "GET /api/v1/terms");
    assert_ne!(request.span_context.trace_id().to_string(), sent_trace);
    assert_eq!(request.parent_span_id, SpanId::INVALID);
}

#[tokio::test]
async fn health_checks_are_not_logged_at_info() {
    let recorder = TelemetryRecorder::start(Sampler::AlwaysOn);
    let (_app, anon) = TestApp::init().empty().await;

    let response = anon.get("/health/live").await;

    assert_eq!(response.status(), StatusCode::OK);
    assert_no_request_logged(&recorder);
}

#[tokio::test]
async fn failing_health_checks_are_not_logged_at_info() {
    let recorder = TelemetryRecorder::start(Sampler::AlwaysOn);
    let (_app, anon) = TestApp::init().with_unreachable_database().empty().await;

    let response = anon.get("/health/ready").await;

    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_no_request_logged(&recorder);
}

/// Asserts that no response or failure was logged, as a line or as an exported record.
#[track_caller]
fn assert_no_request_logged(recorder: &TelemetryRecorder) {
    let lines = recorder.lines();
    assert!(!lines.contains("tower_http::trace"), "{lines}");
    let logs = recorder.logs();
    assert!(
        !logs.iter().any(|log| log
            .target()
            .is_some_and(|target| target.starts_with("tower_http::trace"))),
        "{logs:?}"
    );
}

#[tokio::test]
async fn other_responses_are_logged_at_info() {
    let recorder = TelemetryRecorder::start(Sampler::AlwaysOn);
    let (_app, anon) = TestApp::init().empty().await;

    anon.get("/api/v1/terms").await;

    let lines = recorder.lines();
    let response = lines
        .lines()
        .find(|line| line.contains("tower_http::trace::on_response"))
        .unwrap_or_else(|| panic!("the response was not logged: {lines}"));
    assert!(response.contains("INFO"), "{response}");
    assert!(response.contains("GET /api/v1/terms"), "{response}");
}

#[tokio::test]
async fn log_records_carry_the_request_trace() {
    let recorder = TelemetryRecorder::start(Sampler::AlwaysOn);
    let (_app, anon) = TestApp::init().empty().await;

    anon.get("/api/v1/terms").await;

    let spans = recorder.spans();
    let request = span(&spans, "GET /api/v1/terms");
    let logs = recorder.logs();
    let response = logs
        .iter()
        .find(|log| {
            log.target()
                .is_some_and(|target| target == "tower_http::trace::on_response")
        })
        .unwrap_or_else(|| panic!("the response was not exported: {logs:?}"));
    let context = response
        .trace_context()
        .expect("the record has no trace context");
    assert_eq!(context.trace_id, request.span_context.trace_id());
    assert_eq!(context.span_id, request.span_context.span_id());
}

#[tokio::test]
async fn json_lines_carry_the_request_trace() {
    let recorder = TelemetryRecorder::json(Sampler::AlwaysOn);
    let (_app, anon) = TestApp::init().empty().await;

    anon.get("/api/v1/terms").await;

    let spans = recorder.spans();
    let request = span(&spans, "GET /api/v1/terms");
    let line = json_line(&recorder, "finished processing request");
    assert_eq!(line["level"], "INFO");
    // The fields of the spans the line is in share one object, the innermost span's winning.
    assert_eq!(line["spans"]["http.route"], "/api/v1/terms");
    assert_eq!(
        line["openTelemetry"]["traceId"],
        request.span_context.trace_id().to_string()
    );
    assert_eq!(
        line["openTelemetry"]["spanId"],
        request.span_context.span_id().to_string()
    );
}

/// The JSON line whose message is `message`.
#[track_caller]
fn json_line(recorder: &TelemetryRecorder, message: &str) -> Value {
    let lines = recorder.lines();
    lines
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).expect("a line is not JSON"))
        .find(|line| line["message"] == message)
        .unwrap_or_else(|| panic!("no line says {message:?}: {lines}"))
}

#[tokio::test]
async fn sign_ins_export_no_personal_data_at_debug() {
    let recorder = TelemetryRecorder::debug(Sampler::AlwaysOn);
    let (app, anon) = TestApp::init().empty().await;
    github_approves(app.github(), verified_primary_email()).await;

    let response = sign_in(&anon).await;

    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let exported = format!("{:?}\n{:?}", recorder.spans(), recorder.logs());
    // The login is in the name and the email address. A database client that logs a query's
    // parameters writes text as a list of bytes.
    for value in ["octocat", "Octocat"] {
        let bytes = format!("{:?}", value.as_bytes());
        let bytes = bytes.trim_matches(['[', ']']);
        assert!(
            !exported.contains(value),
            "{value} was exported: {exported}"
        );
        assert!(
            !exported.contains(bytes),
            "{value} was exported: {exported}"
        );
    }
}

#[tokio::test]
async fn other_requests_follow_the_configured_sampler() {
    let recorder = TelemetryRecorder::start(Sampler::AlwaysOff);
    let (_app, anon) = TestApp::init().empty().await;

    let response = anon.get("/api/v1/terms").await;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(recorder.spans(), []);
}

#[tokio::test]
async fn requests_are_measured_by_route() {
    let (app, anon) = TestApp::init().empty().await;

    anon.get("/api/v1/terms").await;
    anon.get("/nothing/here").await;

    let metrics = app.metrics();
    let terms = [
        ("http.request.method", "GET"),
        ("http.route", "/api/v1/terms"),
        ("http.response.status_code", "200"),
    ];
    assert_eq!(metrics.count("http.server.request.duration", &terms), 1);
    assert_eq!(metrics.count("http.server.request.duration", &[]), 2);
    assert!(metrics.has("http.server.active_requests"));
    assert_eq!(metrics.sum("http.server.active_requests", &[]), 0);
}

#[tokio::test]
async fn registrations_and_admin_api_calls_are_measured() {
    let (app, _anon) = TestApp::init().empty().await;
    let user = app.db_new_registrant("octocat").await;
    admin_api_accepts_a_registration(&app, &user).await;

    let response = user.post("/api/v1/issuers", &new_issuer_body()).await;

    assert_eq!(response.status(), StatusCode::CREATED);
    let metrics = app.metrics();
    assert_eq!(
        metrics.sum("portal.registrations", &[("outcome", "created")]),
        1
    );
    let port = app.admin_api().address().port().to_string();
    let create = [
        ("http.request.method", "POST"),
        ("url.template", "/admin/v1/issuers"),
        ("server.address", "127.0.0.1"),
        ("server.port", port.as_str()),
        ("http.response.status_code", "201"),
    ];
    assert_eq!(metrics.count("http.client.request.duration", &create), 1);
}

#[tokio::test]
async fn refusals_are_counted_by_limit() {
    let (app, _anon) = TestApp::init().empty().await;
    let user = app.db_new_registrant("octocat").await;
    let counts = Counts {
        active: 2,
        registered: 2,
        in_sandbox: 2,
    };
    sandbox_counts(app.admin_api(), user.user(), counts).await;

    let response = user.post("/api/v1/issuers", &new_issuer_body()).await;

    assert_eq!(response.status(), StatusCode::CONFLICT);
    let metrics = app.metrics();
    let limit = [("limit", "issuers_per_account")];
    assert_eq!(metrics.sum("portal.sandbox.refusals", &limit), 1);
    assert_eq!(metrics.sum("portal.registrations", &[]), 0);
}

#[tokio::test]
async fn sign_ins_are_counted_by_outcome() {
    let (app, anon) = TestApp::init().empty().await;

    anon.get(CALLBACK_WITH_SECRETS).await;

    let metrics = app.metrics();
    assert_eq!(metrics.sum("portal.sign_ins", &[("outcome", "expired")]), 1);
    assert_eq!(metrics.sum("portal.sign_ins", &[]), 1);
}

#[tokio::test]
async fn the_database_pool_is_reported() {
    let (app, _anon) = TestApp::init().empty().await;
    let user = app.db_new_user("octocat").await;

    let response = user.get("/api/v1/me").await;

    assert_eq!(response.status(), StatusCode::OK);
    let metrics = app.metrics();
    let idle = [("db.client.connection.state", "idle")];
    assert!(metrics.sum("db.client.connection.count", &idle) >= 1);
    let used = [("db.client.connection.state", "used")];
    assert_eq!(metrics.sum("db.client.connection.count", &used), 0);
    assert!(metrics.has("db.client.connection.pending_requests"));
    assert_eq!(metrics.sum("db.client.connection.pending_requests", &[]), 0);
}

#[tokio::test]
async fn admin_api_failures_are_recorded_as_errors() {
    let recorder = TelemetryRecorder::start(Sampler::AlwaysOn);
    let (app, _anon) = TestApp::init().empty().await;
    let user = app.db_new_registrant("octocat").await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(503))
        .mount(app.admin_api())
        .await;

    let response = user.post("/api/v1/issuers", &new_issuer_body()).await;

    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    let failed = [
        ("http.request.method", "GET"),
        ("url.template", "/admin/v1/issuers"),
        ("http.response.status_code", "503"),
        ("error.type", "503"),
    ];
    let metrics = app.metrics();
    assert_eq!(metrics.count("http.client.request.duration", &failed), 1);
    let answered = [("http.route", "/api/v1/issuers"), ("error.type", "502")];
    assert_eq!(metrics.count("http.server.request.duration", &answered), 1);

    let spans = recorder.spans();
    let request = span(&spans, "POST /api/v1/issuers");
    assert!(matches!(request.status, Status::Error { .. }));
    assert_eq!(attribute(request, "error.type").as_deref(), Some("502"));
    let call = span(&spans, "GET /admin/v1/issuers");
    assert!(matches!(call.status, Status::Error { .. }));
    assert_eq!(attribute(call, "error.type").as_deref(), Some("503"));

    let lines = recorder.lines();
    let failure = lines
        .lines()
        .find(|line| line.contains("tower_http::trace::on_failure"))
        .unwrap_or_else(|| panic!("the failure was not logged: {lines}"));
    assert!(failure.contains("ERROR"), "{failure}");
}
