use insta::assert_snapshot;
use serde_json::json;
use status_list_portal::models::TermsAcceptance;

use crate::util::{ORIGIN, RequestHelper, TERMS_VERSION, TestApp};

#[tokio::test]
async fn anyone_reads_the_current_terms() {
    let (_app, anon) = TestApp::init().empty().await;

    let response = anon.get("/api/v1/terms").await;

    assert_snapshot!(response.status(), @"200 OK");
    assert_eq!(
        response.json(),
        json!({ "version": TERMS_VERSION, "url": format!("{ORIGIN}/terms") })
    );
}

#[tokio::test]
async fn accepting_the_current_terms_is_recorded() {
    let (app, _anon) = TestApp::init().empty().await;
    let user = app.db_new_user("octocat").await;
    assert_eq!(user.get("/api/v1/me").await.json()["terms_accepted"], false);

    let response = user
        .put("/api/v1/me/terms", &json!({ "version": TERMS_VERSION }))
        .await;

    assert_snapshot!(response.status(), @"204 No Content");
    assert_eq!(user.get("/api/v1/me").await.json()["terms_accepted"], true);
}

#[tokio::test]
async fn accepting_twice_changes_nothing() {
    let (app, _anon) = TestApp::init().empty().await;
    let user = app.db_new_user("octocat").await;
    let body = json!({ "version": TERMS_VERSION });
    user.put("/api/v1/me/terms", &body).await;

    let response = user.put("/api/v1/me/terms", &body).await;

    assert_snapshot!(response.status(), @"204 No Content");
}

#[tokio::test]
async fn terms_that_changed_since_reading_are_not_accepted() {
    let (app, _anon) = TestApp::init().empty().await;
    let user = app.db_new_user("octocat").await;

    let response = user
        .put("/api/v1/me/terms", &json!({ "version": "2020-01-01" }))
        .await;

    assert_snapshot!(response.status(), @"409 Conflict");
    assert_eq!(user.get("/api/v1/me").await.json()["terms_accepted"], false);
}

#[tokio::test]
async fn accepting_older_terms_does_not_count() {
    let (app, _anon) = TestApp::init().empty().await;
    let user = app.db_new_user("octocat").await;
    TermsAcceptance {
        user_id: user.user().id,
        terms_version: "2020-01-01",
        email: "octocat@example.com",
    }
    .record(&mut app.db_conn().await)
    .await
    .unwrap();

    let response = user.get("/api/v1/me").await;

    assert_eq!(response.json()["terms_accepted"], false);
}

#[tokio::test]
async fn anonymous_users_cannot_accept() {
    let (_app, anon) = TestApp::init().empty().await;

    let response = anon
        .put("/api/v1/me/terms", &json!({ "version": TERMS_VERSION }))
        .await;

    assert_snapshot!(response.status(), @"401 Unauthorized");
}

#[tokio::test]
async fn unreadable_bodies_get_the_api_error_format() {
    let (app, _anon) = TestApp::init().empty().await;
    let user = app.db_new_user("octocat").await;

    let response = user
        .put(
            "/api/v1/me/terms",
            &json!({ "version": TERMS_VERSION, "accepted": true }),
        )
        .await;

    assert_snapshot!(response.status(), @"422 Unprocessable Entity");
    let detail = &response.json()["errors"][0]["detail"];
    assert!(detail.as_str().unwrap().contains("accepted"), "{detail}");
}
