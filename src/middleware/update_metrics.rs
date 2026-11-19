//! Measures each request.

use std::time::Instant;

use axum::extract::{MatchedPath, Request, State};
use axum::middleware::Next;
use axum::response::Response;

use crate::metrics::Metrics;

pub async fn update_metrics(
    State(metrics): State<Metrics>,
    request: Request,
    next: Next,
) -> Response {
    let method = request.method().clone();
    let route = request.extensions().get::<MatchedPath>().cloned();
    let _active = metrics.active_request(&method);
    let started = Instant::now();

    let response = next.run(request).await;

    metrics.record_request(
        &method,
        route.as_ref().map(MatchedPath::as_str),
        response.status(),
        started.elapsed(),
    );
    response
}
