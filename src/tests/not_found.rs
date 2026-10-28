use crate::util::{RequestHelper, TestApp};
use insta::assert_snapshot;
use serde_json::json;

#[tokio::test]
async fn unknown_api_routes_return_a_json_404() {
    let (_app, anon) = TestApp::init().empty().await;

    let response = anon.get("/api/v1/does-not-exist").await;

    assert_snapshot!(response.status(), @"404 Not Found");
    assert_eq!(
        response.json(),
        json!({ "errors": [{ "detail": "Not Found" }] })
    );
}
