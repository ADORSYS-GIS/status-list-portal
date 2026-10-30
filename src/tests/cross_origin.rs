use axum::body::Body;
use http::Method;
use insta::assert_snapshot;
use serde_json::json;

use crate::util::{MockCookieUser, ORIGIN, RequestHelper, Response, TestApp};

async fn send(user: &MockCookieUser, method: Method, path: &str, header: (&str, &str)) -> Response {
    let request = user
        .request_builder(method, path)
        .header(header.0, header.1)
        .body(Body::empty())
        .unwrap();
    user.run(request).await
}

async fn sign_out(user: &MockCookieUser, header: (&str, &str)) -> Response {
    send(user, Method::DELETE, "/api/v1/session", header).await
}

#[tokio::test]
async fn changes_requested_by_other_sites_are_rejected() {
    let (app, _anon) = TestApp::init().empty().await;
    let user = app.db_new_user("octocat").await;

    let response = sign_out(&user, ("sec-fetch-site", "cross-site")).await;

    assert_snapshot!(response.status(), @"403 Forbidden");
    assert_eq!(
        response.json(),
        json!({ "errors": [{ "detail": "Requests from other sites are not allowed" }] })
    );
    assert_snapshot!(user.get("/api/v1/me").await.status(), @"200 OK");
}

#[tokio::test]
async fn changes_requested_by_subdomains_are_rejected() {
    let (app, _anon) = TestApp::init().empty().await;
    let user = app.db_new_user("octocat").await;

    let response = sign_out(&user, ("sec-fetch-site", "same-site")).await;

    assert_snapshot!(response.status(), @"403 Forbidden");
}

#[tokio::test]
async fn changes_from_another_origin_are_rejected() {
    let (app, _anon) = TestApp::init().empty().await;
    let user = app.db_new_user("octocat").await;

    let response = sign_out(&user, ("origin", "https://evil.example")).await;

    assert_snapshot!(response.status(), @"403 Forbidden");
}

#[tokio::test]
async fn changes_from_the_portal_pass() {
    let (app, _anon) = TestApp::init().empty().await;
    let user = app.db_new_user("octocat").await;

    let by_fetch_metadata = sign_out(&user, ("sec-fetch-site", "same-origin")).await;
    let by_origin = sign_out(&user, ("origin", ORIGIN)).await;

    assert_snapshot!(by_fetch_metadata.status(), @"204 No Content");
    assert_snapshot!(by_origin.status(), @"204 No Content");
}

#[tokio::test]
async fn reads_from_other_sites_pass() {
    let (app, _anon) = TestApp::init().empty().await;
    let user = app.db_new_user("octocat").await;

    let response = send(
        &user,
        Method::GET,
        "/api/v1/me",
        ("sec-fetch-site", "cross-site"),
    )
    .await;

    assert_snapshot!(response.status(), @"200 OK");
}
