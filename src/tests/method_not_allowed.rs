use crate::util::{RequestHelper, TestApp};
use axum::body::Body;
use http::Method;
use insta::assert_snapshot;
use serde_json::json;

#[tokio::test]
async fn unsupported_methods_return_a_json_405_with_allow() {
    let (_app, anon) = TestApp::init().empty().await;
    let request = anon
        .request_builder(Method::POST, "/health/ready")
        .body(Body::empty())
        .unwrap();

    let response = anon.run(request).await;

    assert_snapshot!(response.status(), @"405 Method Not Allowed");
    assert_eq!(response.header("allow"), Some("GET,HEAD"));
    assert_eq!(
        response.json(),
        json!({ "errors": [{ "detail": "Method Not Allowed" }] })
    );
}
