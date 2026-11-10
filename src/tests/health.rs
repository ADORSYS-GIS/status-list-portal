use crate::util::{RequestHelper, TestApp};
use insta::assert_snapshot;
use serde_json::json;
use std::time::Duration;

async fn wait_until_ready(anon: &impl RequestHelper) {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if anon.get("/health/ready").await.status().is_success() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("background database check did not become ready");
}

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
    wait_until_ready(&anon).await;

    let response = anon.get("/health/ready").await;

    assert_snapshot!(response.status(), @"200 OK");
    assert_eq!(response.json(), json!({ "status": "ok" }));
}

#[tokio::test]
async fn readiness_stays_available_when_the_request_pool_is_busy() {
    let (app, anon) = TestApp::init().empty().await;
    wait_until_ready(&anon).await;
    let _first = app.ctx().db.get().await.unwrap();
    let _second = app.ctx().db.get().await.unwrap();

    let response = tokio::time::timeout(Duration::from_secs(1), anon.get("/health/ready"))
        .await
        .expect("readiness waited for an application connection");

    assert_eq!(response.status(), http::StatusCode::OK);
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
