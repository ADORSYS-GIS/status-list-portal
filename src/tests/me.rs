use axum::body::Body;
use http::Method;
use http::header::COOKIE;
use insta::assert_snapshot;
use serde_json::json;
use status_list_portal::auth::{SESSION_COOKIE, Session};
use time::OffsetDateTime;

use crate::util::{RequestHelper, TestApp};

#[tokio::test]
async fn anonymous_users_are_asked_to_sign_in() {
    let (_app, anon) = TestApp::init().empty().await;

    let response = anon.get("/api/v1/me").await;

    assert_snapshot!(response.status(), @"401 Unauthorized");
    assert_eq!(
        response.json(),
        json!({ "errors": [{ "detail": "This action requires signing in" }] })
    );
}

#[tokio::test]
async fn signed_in_users_see_their_account() {
    let (app, _anon) = TestApp::init().empty().await;
    let user = app.db_new_user("octocat").await;

    let response = user.get("/api/v1/me").await;

    assert_snapshot!(response.status(), @"200 OK");
    assert_eq!(
        response.json(),
        json!({ "login": "octocat", "name": null, "email": "octocat@example.com" })
    );
}

#[tokio::test]
async fn forged_session_cookies_are_ignored() {
    let (_app, anon) = TestApp::init().empty().await;
    let request = anon
        .request_builder(Method::GET, "/api/v1/me")
        .header(COOKIE, format!("{SESSION_COOKIE}={{\"user_id\":1}}"))
        .body(Body::empty())
        .unwrap();

    let response = anon.run(request).await;

    assert_snapshot!(response.status(), @"401 Unauthorized");
}

#[tokio::test]
async fn expired_sessions_are_ignored() {
    let (app, anon) = TestApp::init().empty().await;
    let user = app.db_new_user("octocat").await;
    let expired = Session {
        expires_at: OffsetDateTime::now_utc().unix_timestamp() - 1,
        ..Session::new(user.user())
    };
    let request = anon
        .request_builder(Method::GET, "/api/v1/me")
        .header(COOKIE, app.cookie_for(expired))
        .body(Body::empty())
        .unwrap();

    let response = anon.run(request).await;

    assert_snapshot!(response.status(), @"401 Unauthorized");
}
