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

#[tokio::test]
async fn liveness_does_not_depend_on_the_database() {
    let (_app, anon) = TestApp::init().with_unreachable_database().empty().await;

    let response = anon.get("/health/live").await;

    assert_snapshot!(response.status(), @"200 OK");
}

#[tokio::test]
async fn readiness_reports_ok_when_the_database_answers() {
    let (_app, anon) = TestApp::init().empty().await;

    let response = anon.get("/health/ready").await;

    assert_snapshot!(response.status(), @"200 OK");
    assert_eq!(response.json(), json!({ "status": "ok" }));
}

#[tokio::test]
async fn readiness_fails_when_the_database_is_unreachable() {
    let (_app, anon) = TestApp::init().with_unreachable_database().empty().await;

    let response = anon.get("/health/ready").await;

    assert_snapshot!(response.status(), @"503 Service Unavailable");
    assert_eq!(
        response.json(),
        json!({ "errors": [{ "detail": "Service Unavailable" }] })
    );
}
