use crate::util::{RequestHelper, TestApp};

#[tokio::test]
async fn every_response_carries_security_headers() {
    let (_app, anon) = TestApp::init().empty().await;

    let response = anon.get("/health/live").await;

    assert_eq!(
        response.header("strict-transport-security"),
        Some("max-age=31536000")
    );
    assert_eq!(response.header("x-content-type-options"), Some("nosniff"));
    assert_eq!(response.header("x-frame-options"), Some("DENY"));
    assert_eq!(response.header("referrer-policy"), Some("no-referrer"));
    assert_eq!(
        response.header("cross-origin-opener-policy"),
        Some("same-origin")
    );
    assert_eq!(
        response.header("cross-origin-resource-policy"),
        Some("same-origin")
    );
    assert_eq!(
        response.header("content-security-policy"),
        Some("default-src 'none'; frame-ancestors 'none'; base-uri 'none'; form-action 'none'")
    );
}

#[tokio::test]
async fn api_responses_are_never_stored() {
    let (_app, anon) = TestApp::init().empty().await;

    let response = anon.get("/api/v1/does-not-exist").await;

    assert_eq!(response.header("cache-control"), Some("no-store"));
}

#[tokio::test]
async fn other_responses_keep_their_own_caching() {
    let (_app, anon) = TestApp::init().empty().await;

    let response = anon.get("/health/live").await;

    assert_eq!(response.header("cache-control"), None);
}
