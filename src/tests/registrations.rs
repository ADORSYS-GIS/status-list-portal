use diesel::QueryResult;
use diesel_async::AsyncConnection;
use insta::assert_snapshot;
use serde_json::{Value, json};
use status_list_portal::models::{NewRegistration, Registration};
use status_list_portal::{config, db, sandbox};
use wiremock::matchers::{method, path};
use wiremock::{Mock, ResponseTemplate};

use crate::util::admin_api::{
    Counts, ISSUER_ID, issuer_json, issuer_response, new_issuer_body, owner, public_jwk_json,
    sandbox_counts,
};
use crate::util::{MockCookieUser, RequestHelper, Response, TestApp};

fn approval_needed() -> config::Sandbox {
    config::Sandbox {
        manual_approval: true,
        ..config::Sandbox::default()
    }
}

async fn ask_for_an_issuer(user: &MockCookieUser) -> Response {
    user.post("/api/v1/issuers", &new_issuer_body()).await
}

async fn registrations_of(user: &MockCookieUser) -> Vec<Value> {
    let response = user.get("/api/v1/registrations").await;
    assert_eq!(response.status(), 200);
    response.json()["registrations"]
        .as_array()
        .expect("no registrations")
        .clone()
}

/// A registration as if `user` had asked for it, without the checks a request runs.
async fn db_new_registration(app: &TestApp, user: &MockCookieUser) -> Registration {
    NewRegistration {
        user_id: user.user().id,
        display_name: "Acme Wallet Issuer",
        alg: "ES256",
        public_key: &public_jwk_json(),
    }
    .insert(&mut app.db_conn().await)
    .await
    .expect("failed to create the registration")
}

async fn reject(app: &TestApp, registration: i64, reason: Option<&str>) {
    let rejected = Registration::reject(&mut app.db_conn().await, registration, reason)
        .await
        .unwrap();
    assert!(rejected, "the registration was not pending");
}

#[tokio::test]
async fn with_approval_on_a_registration_waits_for_an_operator() {
    let (app, _anon) = TestApp::init()
        .with_sandbox(approval_needed())
        .empty()
        .await;
    let user = app.db_new_registrant("octocat").await;
    sandbox_counts(app.admin_api(), user.user(), Counts::default()).await;

    let response = ask_for_an_issuer(&user).await;

    assert_snapshot!(response.status(), @"202 Accepted");
    let registration = response.json();
    assert_eq!(registration["state"], "pending");
    assert_eq!(registration["display_name"], "Acme Wallet Issuer");
    assert_eq!(registration["alg"], "ES256");
    assert_eq!(registration["issuer_id"], Value::Null);
    assert_eq!(registration["decided_at"], Value::Null);
    assert_eq!(registrations_of(&user).await, vec![registration.clone()]);
    let received = app.admin_api().received_requests().await.unwrap();
    assert!(
        received
            .iter()
            .all(|request| request.method.as_str() == "GET")
    );
    let id = registration["id"].as_i64().unwrap();
    let stored = Registration::find(&mut app.db_conn().await, id)
        .await
        .unwrap();
    assert_eq!(stored.public_key, public_jwk_json());
}

#[tokio::test]
async fn without_approval_nothing_waits() {
    let (app, _anon) = TestApp::init().empty().await;
    let user = app.db_new_registrant("octocat").await;
    sandbox_counts(app.admin_api(), user.user(), Counts::default()).await;
    Mock::given(method("POST"))
        .respond_with(issuer_response(
            201,
            "\"1\"",
            &issuer_json(user.user().gh_id),
        ))
        .mount(app.admin_api())
        .await;

    let response = ask_for_an_issuer(&user).await;

    assert_snapshot!(response.status(), @"201 Created");
    assert!(registrations_of(&user).await.is_empty());
}

#[tokio::test]
async fn pending_registrations_count_as_active_issuers() {
    let (app, _anon) = TestApp::init()
        .with_sandbox(approval_needed())
        .empty()
        .await;
    let user = app.db_new_registrant("octocat").await;
    let counts = Counts {
        active: 1,
        registered: 1,
        in_sandbox: 1,
    };
    sandbox_counts(app.admin_api(), user.user(), counts).await;
    assert_eq!(ask_for_an_issuer(&user).await.status(), 202);

    let response = ask_for_an_issuer(&user).await;

    assert_snapshot!(response.status(), @"409 Conflict");
}

#[tokio::test]
async fn rejected_registrations_count_as_registered() {
    let sandbox = config::Sandbox {
        max_registrations_per_account: 2,
        ..approval_needed()
    };
    let (app, _anon) = TestApp::init().with_sandbox(sandbox).empty().await;
    let user = app.db_new_registrant("octocat").await;
    sandbox_counts(app.admin_api(), user.user(), Counts::default()).await;
    let first = db_new_registration(&app, &user).await;
    reject(&app, first.id, None).await;
    assert_eq!(ask_for_an_issuer(&user).await.status(), 202);

    let response = ask_for_an_issuer(&user).await;

    assert_snapshot!(response.status(), @"403 Forbidden");
}

#[tokio::test]
async fn an_operator_approves_a_registration() {
    let (app, _anon) = TestApp::init()
        .with_sandbox(approval_needed())
        .empty()
        .await;
    let user = app.db_new_registrant("octocat").await;
    sandbox_counts(app.admin_api(), user.user(), Counts::default()).await;
    Mock::given(method("POST"))
        .and(path("/admin/v1/issuers"))
        .respond_with(issuer_response(
            201,
            "\"1\"",
            &issuer_json(user.user().gh_id),
        ))
        .expect(1)
        .mount(app.admin_api())
        .await;
    let registration = db_new_registration(&app, &user).await;

    let issuer = sandbox::approve(app.ctx(), &mut app.db_conn().await, registration.id).await;

    assert_eq!(issuer.unwrap().id, ISSUER_ID);
    let requests = app.admin_api().received_requests().await.unwrap();
    let post = requests
        .iter()
        .find(|request| request.method.as_str() == "POST")
        .unwrap();
    let sent: Value = serde_json::from_slice(&post.body).unwrap();
    assert_eq!(sent["owner"], owner(user.user()));
    assert_eq!(sent["public_key"], public_jwk_json());
    assert_eq!(sent["display_name"], "Acme Wallet Issuer");
    assert_eq!(sent["alg"], "ES256");
    let shown = &registrations_of(&user).await[0];
    assert_eq!(shown["state"], "approved");
    assert_eq!(shown["issuer_id"], ISSUER_ID);
    assert!(shown["decided_at"].is_string());
}

#[tokio::test]
async fn approving_needs_room_too() {
    let (app, _anon) = TestApp::init()
        .with_sandbox(approval_needed())
        .empty()
        .await;
    let user = app.db_new_registrant("octocat").await;
    let counts = Counts {
        active: 2,
        registered: 2,
        in_sandbox: 2,
    };
    sandbox_counts(app.admin_api(), user.user(), counts).await;
    let registration = db_new_registration(&app, &user).await;

    let error = sandbox::approve(app.ctx(), &mut app.db_conn().await, registration.id)
        .await
        .unwrap_err();

    assert_snapshot!(error, @"An account can have 2 active sandbox issuers. Delete one first.");
    assert_eq!(registrations_of(&user).await[0]["state"], "pending");
}

#[tokio::test]
async fn a_registration_whose_issuer_fails_stays_pending() {
    let (app, _anon) = TestApp::init()
        .with_sandbox(approval_needed())
        .empty()
        .await;
    let user = app.db_new_registrant("octocat").await;
    sandbox_counts(app.admin_api(), user.user(), Counts::default()).await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(503).set_body_json(json!({
            "error": "service_unavailable",
        })))
        .mount(app.admin_api())
        .await;
    let registration = db_new_registration(&app, &user).await;

    let result = sandbox::approve(app.ctx(), &mut app.db_conn().await, registration.id).await;

    assert!(result.is_err());
    assert_eq!(registrations_of(&user).await[0]["state"], "pending");
}

#[tokio::test]
async fn approving_needs_the_current_terms() {
    let (app, _anon) = TestApp::init()
        .with_sandbox(approval_needed())
        .empty()
        .await;
    let user = app.db_new_user("octocat").await;
    let registration = db_new_registration(&app, &user).await;

    let error = sandbox::approve(app.ctx(), &mut app.db_conn().await, registration.id)
        .await
        .unwrap_err();

    assert_snapshot!(error, @"The developer has not accepted the current terms of use.");
    assert!(
        app.admin_api()
            .received_requests()
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn pending_registrations_of_every_account_count_against_the_sandbox() {
    let sandbox = config::Sandbox {
        max_active_issuers: 1,
        ..approval_needed()
    };
    let (app, _anon) = TestApp::init().with_sandbox(sandbox).empty().await;
    let someone_else = app.db_new_registrant("hubot").await;
    db_new_registration(&app, &someone_else).await;
    let user = app.db_new_registrant("octocat").await;
    sandbox_counts(app.admin_api(), user.user(), Counts::default()).await;

    let response = ask_for_an_issuer(&user).await;

    assert_snapshot!(response.status(), @"503 Service Unavailable");
}

#[tokio::test]
async fn a_request_under_way_holds_back_others() {
    let (app, _anon) = TestApp::init()
        .with_sandbox(approval_needed())
        .empty()
        .await;
    let user = app.db_new_registrant("octocat").await;
    let mut conn = app.db_conn().await;

    conn.transaction(async |conn| {
        assert!(db::try_lock_issuer_limits(conn).await?);

        let response = ask_for_an_issuer(&user).await;

        assert_snapshot!(response.status(), @"503 Service Unavailable");
        QueryResult::Ok(())
    })
    .await
    .unwrap();
    assert!(registrations_of(&user).await.is_empty());
}

#[tokio::test]
async fn decided_registrations_are_not_approved() {
    let (app, _anon) = TestApp::init()
        .with_sandbox(approval_needed())
        .empty()
        .await;
    let user = app.db_new_registrant("octocat").await;
    let registration = db_new_registration(&app, &user).await;
    reject(&app, registration.id, None).await;

    let error = sandbox::approve(app.ctx(), &mut app.db_conn().await, registration.id)
        .await
        .unwrap_err();

    assert_snapshot!(error, @"This registration was decided already.");
    assert!(
        app.admin_api()
            .received_requests()
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn developers_see_why_an_operator_rejected() {
    let (app, _anon) = TestApp::init()
        .with_sandbox(approval_needed())
        .empty()
        .await;
    let user = app.db_new_registrant("octocat").await;
    let registration = db_new_registration(&app, &user).await;

    reject(
        &app,
        registration.id,
        Some("Use a key you made for this sandbox."),
    )
    .await;

    let shown = &registrations_of(&user).await[0];
    assert_eq!(shown["state"], "rejected");
    assert_eq!(
        shown["rejection_reason"],
        "Use a key you made for this sandbox."
    );
    assert_eq!(shown["issuer_id"], Value::Null);
}

#[tokio::test]
async fn developers_withdraw_a_pending_registration() {
    let (app, _anon) = TestApp::init()
        .with_sandbox(approval_needed())
        .empty()
        .await;
    let user = app.db_new_registrant("octocat").await;
    let registration = db_new_registration(&app, &user).await;

    let response = user
        .delete(&format!("/api/v1/registrations/{}", registration.id))
        .await;

    assert_snapshot!(response.status(), @"204 No Content");
    assert!(registrations_of(&user).await.is_empty());
}

#[tokio::test]
async fn another_users_registration_is_out_of_reach() {
    let (app, _anon) = TestApp::init()
        .with_sandbox(approval_needed())
        .empty()
        .await;
    let owner = app.db_new_registrant("octocat").await;
    let someone_else = app.db_new_registrant("hubot").await;
    let registration = db_new_registration(&app, &owner).await;

    let response = someone_else
        .delete(&format!("/api/v1/registrations/{}", registration.id))
        .await;

    assert_snapshot!(response.status(), @"404 Not Found");
    assert!(registrations_of(&someone_else).await.is_empty());
    assert_eq!(registrations_of(&owner).await.len(), 1);
}

#[tokio::test]
async fn decided_registrations_cannot_be_withdrawn() {
    let (app, _anon) = TestApp::init()
        .with_sandbox(approval_needed())
        .empty()
        .await;
    let user = app.db_new_registrant("octocat").await;
    let registration = db_new_registration(&app, &user).await;
    reject(&app, registration.id, None).await;

    let response = user
        .delete(&format!("/api/v1/registrations/{}", registration.id))
        .await;

    assert_snapshot!(response.status(), @"409 Conflict");
}

#[tokio::test]
async fn ids_that_are_not_numbers_are_not_found() {
    let (app, _anon) = TestApp::init().empty().await;
    let user = app.db_new_user("octocat").await;

    let response = user.delete("/api/v1/registrations/latest").await;

    assert_snapshot!(response.status(), @"404 Not Found");
}

#[tokio::test]
async fn anonymous_users_are_asked_to_sign_in() {
    let (_app, anon) = TestApp::init().empty().await;

    assert_eq!(anon.get("/api/v1/registrations").await.status(), 401);
    assert_eq!(anon.delete("/api/v1/registrations/1").await.status(), 401);
}

#[tokio::test]
async fn the_admin_api_failing_leaves_nothing_waiting() {
    let (app, _anon) = TestApp::init()
        .with_sandbox(approval_needed())
        .empty()
        .await;
    let user = app.db_new_registrant("octocat").await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(503).set_body_json(json!({
            "error": "service_unavailable",
        })))
        .mount(app.admin_api())
        .await;

    let response = ask_for_an_issuer(&user).await;

    assert_snapshot!(response.status(), @"503 Service Unavailable");
    assert!(registrations_of(&user).await.is_empty());
}
