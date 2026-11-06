use axum::body::Body;
use http::Method;
use http::header::{ACCEPT, ACCEPT_ENCODING};
use insta::assert_snapshot;
use serde_json::json;

use crate::util::{MockAnonymousUser, RequestHelper, Response, TestApp};

const HTML: &str = "text/html,application/xhtml+xml";

async fn request(
    anon: &MockAnonymousUser,
    method: Method,
    path: &str,
    headers: &[(http::HeaderName, &str)],
) -> Response {
    let builder = headers.iter().fold(
        anon.request_builder(method, path),
        |builder, (name, value)| builder.header(name, *value),
    );
    anon.run(builder.body(Body::empty()).unwrap()).await
}

#[tokio::test]
async fn browser_navigations_get_the_page_with_its_policy() {
    let (_app, anon) = TestApp::init().with_frontend().empty().await;

    for path in ["/", "/issuers/6f1c2a9e", "/issuers/new"] {
        let response = request(&anon, Method::GET, path, &[(ACCEPT, HTML)]).await;

        assert_eq!(response.status(), 200, "{path}");
        assert_eq!(
            response.header("content-type"),
            Some("text/html; charset=utf-8")
        );
        assert_eq!(
            response.header("content-security-policy"),
            Some("script-src 'self' 'sha256-test='; frame-ancestors 'none'")
        );
        assert_eq!(response.header("cache-control"), Some("no-cache"));
        assert!(response.text().contains("The portal"), "{path}");
    }
}

#[tokio::test]
async fn built_files_are_served_and_cached_for_good() {
    let (_app, anon) = TestApp::init().with_frontend().empty().await;

    let response = request(&anon, Method::GET, "/_app/immutable/app.js", &[]).await;

    assert_snapshot!(response.status(), @"200 OK");
    assert_eq!(
        response.header("cache-control"),
        Some("public, max-age=31536000, immutable")
    );
    assert_eq!(response.text(), "console.log(\"portal\");\n");
}

#[tokio::test]
async fn precompressed_files_are_served_to_browsers_that_accept_them() {
    let (_app, anon) = TestApp::init().with_frontend().empty().await;

    let response = request(
        &anon,
        Method::GET,
        "/_app/immutable/app.js",
        &[(ACCEPT_ENCODING, "gzip")],
    )
    .await;

    assert_snapshot!(response.status(), @"200 OK");
    assert_eq!(response.header("content-encoding"), Some("gzip"));
}

#[tokio::test]
async fn other_files_are_checked_for_changes() {
    let (_app, anon) = TestApp::init().with_frontend().empty().await;

    let response = request(&anon, Method::GET, "/robots.txt", &[]).await;

    assert_snapshot!(response.status(), @"200 OK");
    assert_eq!(response.header("cache-control"), Some("no-cache"));
}

#[tokio::test]
async fn the_backend_keeps_its_paths() {
    let (_app, anon) = TestApp::init().with_frontend().empty().await;

    let api = request(&anon, Method::GET, "/api/v1/nothing", &[(ACCEPT, HTML)]).await;
    let health = request(&anon, Method::GET, "/health/live", &[(ACCEPT, HTML)]).await;

    assert_snapshot!(api.status(), @"404 Not Found");
    assert_eq!(api.header("content-type"), Some("application/json"));
    assert_snapshot!(health.status(), @"200 OK");
    assert_eq!(health.header("content-type"), Some("application/json"));
}

#[tokio::test]
async fn pages_only_answer_reads() {
    let (_app, anon) = TestApp::init().with_frontend().empty().await;

    let response = request(&anon, Method::POST, "/issuers", &[(ACCEPT, HTML)]).await;

    assert_snapshot!(response.status(), @"405 Method Not Allowed");
    assert_eq!(response.header("allow"), Some("GET, HEAD"));
    assert_eq!(
        response.json(),
        json!({ "errors": [{ "detail": "Method Not Allowed" }] })
    );
}

#[tokio::test]
async fn without_a_frontend_only_the_api_answers() {
    let (_app, anon) = TestApp::init().empty().await;

    let response = request(&anon, Method::GET, "/", &[(ACCEPT, HTML)]).await;

    assert_snapshot!(response.status(), @"404 Not Found");
    assert_eq!(response.header("content-type"), Some("application/json"));
}
