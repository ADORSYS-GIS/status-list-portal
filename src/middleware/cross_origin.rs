//! Rejects state-changing requests that a browser sends on behalf of another site.
//!
//! Session cookies are `SameSite=Lax`, which already keeps them off most such requests; this
//! is the second line of defense. Browsers mark every request with `Sec-Fetch-Site` or, in
//! older versions, `Origin`. Requests with neither come from tools that are not browsers,
//! which no other site can drive, so they pass.

use axum::extract::{Request, State};
use axum::middleware::Next;
use axum::response::Response;
use http::header::ORIGIN;

use crate::config::PublicOrigin;
use crate::util::errors;

pub(super) async fn reject_cross_origin(
    State(origin): State<PublicOrigin>,
    request: Request,
    next: Next,
) -> Response {
    if request.method().is_safe() || is_same_origin(&request, &origin) {
        next.run(request).await
    } else {
        errors::forbidden("Requests from other sites are not allowed").response()
    }
}

fn is_same_origin(request: &Request, origin: &PublicOrigin) -> bool {
    let headers = request.headers();
    if let Some(site) = headers.get("sec-fetch-site") {
        // `none` means the user started the request, for example from the address bar.
        return site == "same-origin" || site == "none";
    }
    headers
        .get(ORIGIN)
        .is_none_or(|value| value == origin.as_str())
}
