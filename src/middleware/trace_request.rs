//! A span for each request, named after its route as the OpenTelemetry semantic conventions
//! ask.

use std::time::Duration;

use axum::extract::{MatchedPath, Request};
use axum::response::Response;
use http::header::USER_AGENT;
use tower_http::classify::ServerErrorsFailureClass;
use tower_http::trace::{DefaultOnFailure, DefaultOnResponse, OnFailure, OnResponse};
use tracing::field::Empty;
use tracing::{Level, Span};

use crate::metrics::method_name;

/// The longest path and user agent a span records, in bytes. A client chooses both, and the
/// span repeats them in every log line written during the request.
const MAX_CLIENT_VALUE_LEN: usize = 256;

/// Routes of the health checks, which a probe calls every few seconds. Their spans, and
/// the lines that log their responses and failures, are at [`HEALTH_CHECK_LEVEL`], so the
/// default `RUST_LOG` leaves them out, and no trace of them is exported: only `INFO` spans
/// are. The request metrics still count them.
const HEALTH_CHECK_ROUTES: &str = "/health/";

const HEALTH_CHECK_LEVEL: Level = Level::DEBUG;

/// Starts a trace of its own for each request: a trace context the request carries is not
/// trusted, since anyone can send one. The span records the path but never the query,
/// which can carry OAuth codes.
pub fn make_span(request: &Request) -> Span {
    let method = method_name(request.method());
    let route = request
        .extensions()
        .get::<MatchedPath>()
        .map(MatchedPath::as_str);
    let name = match route {
        Some(route) => format!("{method} {route}"),
        None => method.to_string(),
    };
    let user_agent = request
        .headers()
        .get(USER_AGENT)
        .and_then(|value| value.to_str().ok())
        .map(cut_short);
    let path = cut_short(request.uri().path());

    // `tracing` takes a span's level as a constant, so each level needs its own call.
    macro_rules! request_span {
        ($level:expr) => {
            tracing::span!(
                parent: None,
                $level,
                "request",
                otel.name = name,
                otel.kind = "server",
                otel.status_code = Empty,
                http.request.method = method,
                http.route = route,
                http.response.status_code = Empty,
                error.type = Empty,
                url.path = path,
                user_agent.original = user_agent,
            )
        };
    }
    if route.is_some_and(|route| route.starts_with(HEALTH_CHECK_ROUTES)) {
        request_span!(HEALTH_CHECK_LEVEL)
    } else {
        request_span!(Level::INFO)
    }
}

/// `value`, cut at [`MAX_CLIENT_VALUE_LEN`] bytes, on a character boundary.
fn cut_short(value: &str) -> &str {
    &value[..value.floor_char_boundary(MAX_CLIENT_VALUE_LEN)]
}

/// Records the response's status, marks server errors as failures, and logs the response at
/// its span's level.
pub fn on_response(response: &Response, latency: Duration, span: &Span) {
    let status = response.status();
    // Recorded as `i64`: `tracing-opentelemetry` would export a `u64` as a string.
    span.record("http.response.status_code", i64::from(status.as_u16()));
    if status.is_server_error() {
        span.record("otel.status_code", "ERROR");
        span.record("error.type", status.as_str());
    }
    DefaultOnResponse::new()
        .level(level_of(span))
        .on_response(response, latency, span);
}

/// Logs a server error at `ERROR`, but a failing health check at its span's level: the probe
/// reports it, and while the database is down it would log an error every few seconds.
pub fn on_failure(failure: ServerErrorsFailureClass, latency: Duration, span: &Span) {
    let level = if level_of(span) == HEALTH_CHECK_LEVEL {
        HEALTH_CHECK_LEVEL
    } else {
        Level::ERROR
    };
    DefaultOnFailure::new()
        .level(level)
        .on_failure(failure, latency, span);
}

/// The level `span` was made at, which a span keeps even when no layer records it.
fn level_of(span: &Span) -> Level {
    span.metadata()
        .map_or(Level::INFO, |metadata| *metadata.level())
}
