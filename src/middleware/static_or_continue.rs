//! Serves the frontend's built files, precompressed when the browser accepts it. A request
//! for anything else goes on.

use std::path::Path;
use std::sync::Arc;

use axum::body::Body;
use axum::extract::{Request, State};
use axum::middleware::Next;
use axum::response::Response;
use http::header::CACHE_CONTROL;
use http::{HeaderValue, Method, StatusCode};
use tower::ServiceExt;
use tower_http::services::ServeDir;

use super::frontend_html::{Frontend, is_backend_path};

/// Files whose names hold a hash of their content, so they never change.
const IMMUTABLE_PREFIX: &str = "/_app/immutable/";

pub(super) async fn serve(
    State(frontend): State<Arc<Frontend>>,
    request: Request,
    next: Next,
) -> Response {
    let (parts, body) = request.into_parts();
    match serve_file(&frontend.dir, &parts).await {
        Some(response) => response,
        None => next.run(Request::from_parts(parts, body)).await,
    }
}

/// `None` when the request is not for a built file. Pages go through `frontend_html`, which
/// adds their policy.
async fn serve_file(dir: &Path, parts: &http::request::Parts) -> Option<Response> {
    let path = parts.uri.path();
    if !matches!(parts.method, Method::GET | Method::HEAD)
        || is_backend_path(path)
        || path.ends_with('/')
        || path.ends_with(".html")
    {
        return None;
    }

    let mut request = Request::new(());
    *request.method_mut() = parts.method.clone();
    *request.uri_mut() = parts.uri.clone();
    *request.headers_mut() = parts.headers.clone();
    let files = ServeDir::new(dir).precompressed_br().precompressed_gzip();
    let Ok(mut response) = files.oneshot(request).await;
    if response.status() == StatusCode::NOT_FOUND {
        return None;
    }

    let cache_control = if path.starts_with(IMMUTABLE_PREFIX) {
        "public, max-age=31536000, immutable"
    } else {
        "no-cache"
    };
    response
        .headers_mut()
        .insert(CACHE_CONTROL, HeaderValue::from_static(cache_control));
    Some(response.map(Body::new))
}
