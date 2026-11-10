//! Middleware applied to every request.

mod cross_origin;
mod headers;

use std::any::Any;
use std::time::Duration;

use axum::Router;
use axum::extract::{DefaultBodyLimit, Request};
use axum::middleware::{Next, from_fn, from_fn_with_state};
use axum::response::Response;
use tower::ServiceBuilder;
use tower_http::catch_panic::CatchPanicLayer;
use tower_http::trace::{DefaultOnResponse, TraceLayer};
use tracing::{Level, Span, info_span, warn};

use crate::config::PublicOrigin;
use crate::util::errors;

/// Requests that take longer are answered with `503 Service Unavailable`.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Largest accepted request body. Portal requests carry at most a public JWK and a few
/// short fields.
const MAX_BODY_SIZE: usize = 16 * 1024;

/// Wraps `router` in every middleware. `origin` is where browsers reach the portal.
pub fn apply_middleware(router: Router, origin: &PublicOrigin) -> Router {
    // Security headers sit outside the other layers, so the responses those layers produce
    // carry them too.
    let middleware = ServiceBuilder::new()
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(make_span)
                .on_response(DefaultOnResponse::new().level(Level::INFO)),
        )
        .layer(from_fn(headers::add_common_headers))
        .layer(from_fn_with_state(
            origin.clone(),
            cross_origin::reject_cross_origin,
        ))
        .layer(CatchPanicLayer::custom(handle_panic))
        .layer(from_fn(time_out));

    router
        .layer(DefaultBodyLimit::max(MAX_BODY_SIZE))
        .layer(middleware)
}

/// Records the method and path only. Query strings can carry OAuth codes, which must not
/// reach the logs.
fn make_span(request: &Request) -> Span {
    info_span!("request", method = %request.method(), path = %request.uri().path())
}

fn handle_panic(_payload: Box<dyn Any + Send + 'static>) -> Response {
    errors::internal("request handler panicked").response()
}

/// Answers `503`, not `408`: a 408 tells clients they were too slow to send the request,
/// and lets them send it again, so a slow request could run twice.
async fn time_out(request: Request, next: Next) -> Response {
    tokio::time::timeout(REQUEST_TIMEOUT, next.run(request))
        .await
        .unwrap_or_else(|_elapsed| {
            warn!("request timed out");
            errors::service_unavailable().response()
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::routing::get;
    use http::StatusCode;
    use serde_json::{Value, json};
    use tower::ServiceExt;

    async fn boom() -> &'static str {
        panic!("secret detail")
    }

    async fn slow() -> &'static str {
        tokio::time::sleep(REQUEST_TIMEOUT * 2).await;
        "done"
    }

    #[tokio::test(start_paused = true)]
    async fn slow_requests_get_a_json_503_with_security_headers() {
        let router = Router::new().route("/slow", get(slow));
        let request = Request::get("/slow").body(Body::empty()).unwrap();

        let origin = PublicOrigin::parse("http://localhost:8080").unwrap();

        let response = apply_middleware(router, &origin)
            .oneshot(request)
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(response.headers()["content-type"], "application/json");
        assert_eq!(response.headers()["x-content-type-options"], "nosniff");
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let body: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(
            body,
            json!({ "errors": [{ "detail": "Service Unavailable" }] })
        );
    }

    #[tokio::test]
    async fn panics_become_a_generic_json_500_with_security_headers() {
        let router = Router::new().route("/boom", get(boom));
        let request = Request::get("/boom").body(Body::empty()).unwrap();

        let origin = PublicOrigin::parse("http://localhost:8080").unwrap();

        let response = apply_middleware(router, &origin)
            .oneshot(request)
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(response.headers()["x-content-type-options"], "nosniff");
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let body: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(
            body,
            json!({ "errors": [{ "detail": "Internal Server Error" }] })
        );
    }
}
