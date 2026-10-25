//! A span for each request, named after its route as the OpenTelemetry semantic conventions
//! ask.

use std::time::Duration;

use axum::extract::{MatchedPath, Request};
use axum::response::Response;
use http::header::USER_AGENT;
use tower_http::trace::{DefaultOnResponse, OnResponse};
use tracing::field::Empty;
use tracing::{Level, Span, info_span};

use crate::metrics::method_name;

/// The longest path and user agent a span records, in bytes. A client chooses both, and the
/// span repeats them in every log line written during the request.
const MAX_CLIENT_VALUE_LEN: usize = 256;

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

    info_span!(
        parent: None,
        "request",
        otel.name = name,
        otel.kind = "server",
        otel.status_code = Empty,
        http.request.method = method,
        http.route = route,
        http.response.status_code = Empty,
        error.type = Empty,
        url.path = cut_short(request.uri().path()),
        user_agent.original = user_agent,
    )
}

/// `value`, cut at [`MAX_CLIENT_VALUE_LEN`] bytes, on a character boundary.
fn cut_short(value: &str) -> &str {
    &value[..value.floor_char_boundary(MAX_CLIENT_VALUE_LEN)]
}

/// Records the response's status, marks server errors as failures, and logs the response.
pub fn on_response(response: &Response, latency: Duration, span: &Span) {
    let status = response.status();
    // Recorded as `i64`: `tracing-opentelemetry` would export a `u64` as a string.
    span.record("http.response.status_code", i64::from(status.as_u16()));
    if status.is_server_error() {
        span.record("otel.status_code", "ERROR");
        span.record("error.type", status.as_str());
    }
    DefaultOnResponse::new()
        .level(Level::INFO)
        .on_response(response, latency, span);
}
