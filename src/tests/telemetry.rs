//! What the portal records about the requests it answers: spans and metrics.

use axum::body::Body;
use http::header::USER_AGENT;
use http::{Method, StatusCode};
use opentelemetry::trace::{SpanKind, Status};
use opentelemetry_sdk::trace::Sampler;
use wiremock::matchers::{method, path};
use wiremock::{Mock, ResponseTemplate};

use crate::util::admin_api::{
    Counts, issuer_json, issuer_response, new_issuer_body, sandbox_counts,
};
use crate::util::telemetry::{SpanRecorder, attribute, span};
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
    let recorder = SpanRecorder::start(Sampler::AlwaysOn);
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
    let recorder = SpanRecorder::start(Sampler::AlwaysOn);
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
    let recorder = SpanRecorder::start(Sampler::AlwaysOff);
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
async fn other_requests_follow_the_configured_sampler() {
    let recorder = SpanRecorder::start(Sampler::AlwaysOff);
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
    let create = [
        ("http.request.method", "POST"),
        ("url.template", "/admin/v1/issuers"),
        ("server.address", "127.0.0.1"),
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
    assert_eq!(metrics.sum("portal.sign_ins", &[("outcome", "refused")]), 1);
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
    let recorder = SpanRecorder::start(Sampler::AlwaysOn);
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
}
