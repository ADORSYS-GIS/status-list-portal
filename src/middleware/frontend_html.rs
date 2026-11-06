//! Serves the page that boots the frontend.
//!
//! The frontend routes in the browser, so every page has the same HTML: the build's fallback
//! page. Browser navigations outside the backend's own paths get it; every other request
//! goes on to the router.
//!
//! The page and its policy are read once, at startup: a new build needs a restart.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Context;
use axum::body::Bytes;
use axum::extract::{Request, State};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use http::header::{ACCEPT, ALLOW, CACHE_CONTROL, CONTENT_SECURITY_POLICY, CONTENT_TYPE};
use http::{HeaderMap, HeaderValue, Method};

use crate::util::errors;

/// The page the frontend's build writes for every path that it routes itself.
const FALLBACK_PAGE: &str = "200.html";
/// Paths the backend answers itself.
const BACKEND_PATHS: [&str; 2] = ["/api/", "/health/"];
/// How the build states its policy, with the hashes of the page's inline scripts.
const POLICY_TAG: &str = r#"<meta http-equiv="content-security-policy" content=""#;

/// The built frontend.
#[derive(Debug)]
pub struct Frontend {
    pub(super) dir: PathBuf,
    html: Bytes,
    content_security_policy: HeaderValue,
}

impl Frontend {
    /// Reads the build in `dir`. Fails when the frontend is not built, or was built without
    /// its content security policy.
    pub fn load(dir: &Path) -> anyhow::Result<Self> {
        let page = dir.join(FALLBACK_PAGE);
        let html = std::fs::read_to_string(&page).with_context(|| {
            format!("failed to read {}; is the frontend built?", page.display())
        })?;
        let policy = page_policy(&html)
            .with_context(|| format!("{} states no content security policy", page.display()))?;
        // A policy in a `<meta>` tag cannot forbid framing; the header can.
        let content_security_policy = HeaderValue::try_from(format!(
            "{policy}; frame-ancestors 'none'"
        ))
        .with_context(|| format!("the policy in {} is not a valid header", page.display()))?;
        Ok(Self {
            dir: dir.to_owned(),
            html: Bytes::from(html),
            content_security_policy,
        })
    }
}

pub(super) fn is_backend_path(path: &str) -> bool {
    BACKEND_PATHS.iter().any(|prefix| path.starts_with(prefix))
}

pub(super) async fn serve(
    State(frontend): State<Arc<Frontend>>,
    request: Request,
    next: Next,
) -> Response {
    if is_backend_path(request.uri().path()) || !accepts_html(request.headers()) {
        return next.run(request).await;
    }
    if !matches!(*request.method(), Method::GET | Method::HEAD) {
        let allow = [(ALLOW, HeaderValue::from_static("GET, HEAD"))];
        return (allow, errors::method_not_allowed()).into_response();
    }

    let headers = [
        (
            CONTENT_TYPE,
            HeaderValue::from_static("text/html; charset=utf-8"),
        ),
        // The page names the current build's assets, so browsers check for a new one.
        (CACHE_CONTROL, HeaderValue::from_static("no-cache")),
        (
            CONTENT_SECURITY_POLICY,
            frontend.content_security_policy.clone(),
        ),
    ];
    (headers, frontend.html.clone()).into_response()
}

fn accepts_html(headers: &HeaderMap) -> bool {
    headers
        .get_all(ACCEPT)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .any(|value| value.contains("text/html"))
}

/// The policy that the page's `<meta http-equiv="content-security-policy">` states.
fn page_policy(html: &str) -> Option<&str> {
    let start = html.find(POLICY_TAG)? + POLICY_TAG.len();
    let length = html[start..].find('"')?;
    Some(&html[start..start + length])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_policy_is_read_from_the_page() {
        let html = r#"<head><meta http-equiv="content-security-policy" content="script-src 'self' 'sha256-abc='"></head>"#;

        assert_eq!(page_policy(html), Some("script-src 'self' 'sha256-abc='"));
    }

    #[test]
    fn a_page_without_a_policy_has_none() {
        assert_eq!(page_policy("<head></head>"), None);
    }

    #[test]
    fn the_backend_keeps_its_paths() {
        assert!(is_backend_path("/api/v1/me"));
        assert!(is_backend_path("/health/live"));
        assert!(!is_backend_path("/issuers/new"));
        assert!(!is_backend_path("/apiary"));
    }
}
