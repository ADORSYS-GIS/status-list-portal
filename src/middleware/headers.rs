//! Security headers added to every response.

use axum::extract::Request;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use http::{HeaderMap, HeaderName, HeaderValue, header};

/// Policy for every response that is not the frontend's HTML: it loads nothing and may not
/// be framed.
const API_CONTENT_SECURITY_POLICY: &str =
    "default-src 'none'; frame-ancestors 'none'; base-uri 'none'; form-action 'none'";

const CROSS_ORIGIN_OPENER_POLICY: HeaderName =
    HeaderName::from_static("cross-origin-opener-policy");
const CROSS_ORIGIN_RESOURCE_POLICY: HeaderName =
    HeaderName::from_static("cross-origin-resource-policy");

pub(super) async fn add_common_headers(request: Request, next: Next) -> Response {
    let is_api = request.uri().path().starts_with("/api/");
    let response = next.run(request).await;

    let v = HeaderValue::from_static;
    let mut headers = HeaderMap::new();
    headers.insert(header::STRICT_TRANSPORT_SECURITY, v("max-age=31536000"));
    headers.insert(header::X_CONTENT_TYPE_OPTIONS, v("nosniff"));
    headers.insert(header::X_FRAME_OPTIONS, v("DENY"));
    headers.insert(header::REFERRER_POLICY, v("no-referrer"));
    headers.insert(CROSS_ORIGIN_OPENER_POLICY, v("same-origin"));
    headers.insert(CROSS_ORIGIN_RESOURCE_POLICY, v("same-origin"));

    // A handler that serves HTML sets a policy of its own.
    if !response
        .headers()
        .contains_key(header::CONTENT_SECURITY_POLICY)
    {
        headers.insert(
            header::CONTENT_SECURITY_POLICY,
            v(API_CONTENT_SECURITY_POLICY),
        );
    }

    // API responses carry account data, which no cache may keep.
    if is_api && !response.headers().contains_key(header::CACHE_CONTROL) {
        headers.insert(header::CACHE_CONTROL, v("no-store"));
    }

    (headers, response).into_response()
}
