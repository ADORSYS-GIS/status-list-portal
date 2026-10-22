use crate::util::{RequestHelper, TestApp};
use insta::assert_snapshot;
use serde_json::json;

#[tokio::test]
async fn liveness_reports_ok() {
    let (_app, anon) = TestApp::init().empty().await;

    let response = anon.get("/health/live").await;

    assert_snapshot!(response.status(), @"200 OK");
    assert_eq!(response.json(), json!({ "status": "ok" }));
}
