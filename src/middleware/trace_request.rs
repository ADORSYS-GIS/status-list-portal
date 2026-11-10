//! A span for each request, named after its route as the OpenTelemetry semantic conventions
//! ask.

use std::time::Duration;

use axum::extract::{MatchedPath, Request};
use axum::response::Response;
use http::header::USER_AGENT;
use tower_http::trace::{DefaultOnResponse, OnResponse};
use tracing::field::Empty;
use tracing::{Level, Span};

use crate::metrics::method_name;

/// The longest path and user agent a span records, in bytes. A client chooses both, and the
/// span repeats them in every log line written during the request.
const MAX_CLIENT_VALUE_LEN: usize = 256;

/// Routes of the health checks, which a probe calls every few seconds. Their spans and
/// response lines are `DEBUG`, so the default `RUST_LOG` leaves them out, and no trace of
/// them is exported: only `INFO` spans are. The request metrics still count them.
const HEALTH_CHECK_ROUTES: &str = "/health/";

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
        request_span!(Level::DEBUG)
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
    // A span no layer wants is disabled and has no metadata: a health check's span when
    // `DEBUG` is filtered out, so its response is not logged either.
    let level = span
        .metadata()
        .map_or(Level::DEBUG, |metadata| *metadata.level());
    DefaultOnResponse::new()
        .level(level)
        .on_response(response, latency, span);
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::sync::{Arc, Mutex};

    use axum::Router;
    use axum::body::Body;
    use axum::routing::get;
    use tower::ServiceExt;
    use tower_http::trace::TraceLayer;
    use tracing_subscriber::fmt::MakeWriter;

    use super::*;

    /// Keeps the log lines written to it.
    #[derive(Clone, Default)]
    struct Lines(Arc<Mutex<Vec<u8>>>);

    impl Write for Lines {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
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

    /// What answering a `GET` of `path` logs at `INFO`.
    async fn logged_at_info(path: &str) -> String {
        let lines = Lines::default();
        let subscriber = tracing_subscriber::fmt()
            .with_max_level(Level::INFO)
            .with_writer(lines.clone())
            .finish();
        let _guard = tracing::subscriber::set_default(subscriber);
        let router = Router::new()
            .route("/health/live", get(|| async { "ok" }))
            .route("/api/v1/terms", get(|| async { "terms" }))
            .layer(
                TraceLayer::new_for_http()
                    .make_span_with(make_span)
                    .on_response(on_response),
            );

        let request = Request::get(path).body(Body::empty()).unwrap();
        router.oneshot(request).await.unwrap();

        String::from_utf8(lines.0.lock().unwrap().clone()).unwrap()
    }

    #[tokio::test]
    async fn health_checks_are_not_logged_at_info() {
        assert_eq!(logged_at_info("/health/live").await, "");
    }

    #[tokio::test]
    async fn other_responses_are_logged_at_info() {
        let logged = logged_at_info("/api/v1/terms").await;

        assert!(logged.contains("finished processing request"), "{logged}");
        assert!(logged.contains("GET /api/v1/terms"), "{logged}");
    }
}
