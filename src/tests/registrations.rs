use std::time::Duration;

use diesel::QueryResult;
use diesel::sql_types::BigInt;
use diesel_async::{AsyncConnection, RunQueryDsl};
use insta::assert_snapshot;
use serde_json::{Value, json};
use status_list_portal::models::{NewRegistration, Registration};
use status_list_portal::{config, db, sandbox};
use wiremock::matchers::{method, path, query_param_is_missing};
use wiremock::{Mock, ResponseTemplate};

use crate::util::admin_api::{
    Counts, ISSUER_ID, admin_api_holds, assert_one_lifetime, issuer_json, issuer_response,
    new_issuer_body, owner, page_response, public_jwk_json, sandbox_counts,
};
use crate::util::{MockCookieUser, RequestHelper, Response, TestApp};

/// Long enough for another decision to try its luck while the admin API answers.
const SLOW_ANSWER: Duration = Duration::from_millis(500);

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

async fn reject(app: &TestApp, registration: i64, reason: &str) {
    sandbox::reject(&mut app.db_conn().await, registration, reason)
        .await
        .expect("the registration was not rejected");
}

async fn withdraw(user: &MockCookieUser, registration: i64) -> Response {
    user.delete(&format!("/api/v1/registrations/{registration}"))
        .await
}

/// The admin API answers an issuer's creation for `user` after a while.
async fn slow_creation(app: &TestApp, user: &MockCookieUser, status: u16) {
    let body = if status == 201 {
        issuer_json(user.user().gh_id)
    } else {
        json!({ "error": "service_unavailable" })
    };
    Mock::given(method("POST"))
        .respond_with(issuer_response(status, "\"1\"", &body).set_delay(SLOW_ANSWER))
        .expect(1)
        .mount(app.admin_api())
        .await;
}

/// Waits until the admin API has been asked to create an issuer.
async fn creation_under_way(app: &TestApp) {
    for _ in 0..200 {
        let requests = app.admin_api().received_requests().await.unwrap();
        if requests
            .iter()
            .any(|request| request.method.as_str() == "POST")
        {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("the admin API was not asked to create an issuer");
}

fn detail(response: &Response) -> Value {
    response.json()["errors"][0]["detail"].clone()
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
async fn with_approval_on_private_keys_are_refused_at_once() {
    let (app, _anon) = TestApp::init()
        .with_sandbox(approval_needed())
        .empty()
        .await;
    let user = app.db_new_registrant("octocat").await;
    let mut body = new_issuer_body();
    body["public_key"]["d"] = json!("c2VjcmV0");

    let response = user.post("/api/v1/issuers", &body).await;

    assert_snapshot!(response.status(), @"422 Unprocessable Entity");
    assert!(registrations_of(&user).await.is_empty());
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
    reject(&app, first.id, "Not now.").await;
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
    assert_one_lifetime(&sent);
    let shown = &registrations_of(&user).await[0];
    assert_eq!(shown["state"], "approved");
    assert_eq!(shown["issuer_id"], ISSUER_ID);
    assert!(shown["decided_at"].is_string());
}

#[tokio::test]
async fn approving_again_records_the_issuer_an_unanswered_approval_created() {
    let (app, _anon) = TestApp::init()
        .with_sandbox(approval_needed())
        .empty()
        .await;
    let user = app.db_new_registrant("octocat").await;
    let registration = db_new_registration(&app, &user).await;
    let mut created = issuer_json(user.user().gh_id);
    created["created_at"] = json!(registration.created_at.unix_timestamp() + 1);
    Mock::given(method("GET"))
        .and(path("/admin/v1/issuers"))
        .and(query_param_is_missing("state"))
        .respond_with(page_response(&[created]))
        .with_priority(1)
        .mount(app.admin_api())
        .await;
    let counts = Counts {
        active: 2,
        registered: 2,
        in_sandbox: 2,
    };
    sandbox_counts(app.admin_api(), user.user(), counts).await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(app.admin_api())
        .await;

    let issuer = sandbox::approve(app.ctx(), &mut app.db_conn().await, registration.id).await;

    assert_eq!(issuer.unwrap().id, ISSUER_ID);
    let shown = &registrations_of(&user).await[0];
    assert_eq!(shown["state"], "approved");
    assert_eq!(shown["issuer_id"], ISSUER_ID);
}

#[tokio::test]
async fn a_second_registration_of_a_key_does_not_take_the_first_ones_issuer() {
    let (app, _anon) = TestApp::init()
        .with_sandbox(approval_needed())
        .empty()
        .await;
    let user = app.db_new_registrant("octocat").await;
    let first = db_new_registration(&app, &user).await;
    let second = db_new_registration(&app, &user).await;
    let mut created = issuer_json(user.user().gh_id);
    created["created_at"] = json!(second.created_at.unix_timestamp() + 1);
    Mock::given(method("GET"))
        .and(path("/admin/v1/issuers"))
        .and(query_param_is_missing("state"))
        .respond_with(page_response(&[created]))
        .with_priority(1)
        .mount(app.admin_api())
        .await;
    sandbox_counts(app.admin_api(), user.user(), Counts::default()).await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(409).set_body_json(json!({
            "error": "key_material_in_use",
        })))
        .expect(1)
        .mount(app.admin_api())
        .await;
    let mut conn = app.db_conn().await;
    assert!(
        Registration::approve(&mut conn, first.id, ISSUER_ID)
            .await
            .unwrap()
    );

    let error = sandbox::approve(app.ctx(), &mut conn, second.id)
        .await
        .unwrap_err();

    assert_snapshot!(error, @"This public key is already registered, maybe for an issuer you deleted, since a deleted issuer keeps its key. Make a new key pair, or, if you just registered this one, check your issuers.");
    let shown = registrations_of(&user).await;
    assert_eq!(shown[0]["id"], second.id);
    assert_eq!(shown[0]["state"], "pending");
}

#[tokio::test]
async fn an_older_issuer_with_the_key_is_not_taken_for_the_approval() {
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
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(409).set_body_json(json!({
            "error": "key_material_in_use",
        })))
        .expect(1)
        .mount(app.admin_api())
        .await;
    let registration = db_new_registration(&app, &user).await;

    let result = sandbox::approve(app.ctx(), &mut app.db_conn().await, registration.id).await;

    assert!(result.is_err());
    assert_eq!(registrations_of(&user).await[0]["state"], "pending");
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

    assert_snapshot!(error, @"An account can have 2 active sandbox issuers, registrations waiting for approval included. Delete one, or withdraw a registration, first.");
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
    assert_eq!(withdraw(&user, registration.id).await.status(), 204);
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
async fn pending_registrations_of_other_accounts_leave_the_sandbox_alone() {
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

    assert_snapshot!(response.status(), @"202 Accepted");
}

#[tokio::test]
async fn pending_registrations_count_when_approval_is_turned_off() {
    let (app, _anon) = TestApp::init().empty().await;
    let user = app.db_new_registrant("octocat").await;
    let counts = Counts {
        active: 1,
        registered: 1,
        in_sandbox: 1,
    };
    sandbox_counts(app.admin_api(), user.user(), counts).await;
    db_new_registration(&app, &user).await;

    let response = ask_for_an_issuer(&user).await;

    assert_snapshot!(response.status(), @"409 Conflict");
    assert!(
        app.admin_api()
            .received_requests()
            .await
            .unwrap()
            .iter()
            .all(|request| request.method.as_str() == "GET")
    );
}

#[tokio::test]
async fn renewing_an_expired_issuer_leaves_pending_registrations_their_place() {
    let (app, _anon) = TestApp::init()
        .with_sandbox(approval_needed())
        .empty()
        .await;
    let user = app.db_new_registrant("octocat").await;
    let mut expired = issuer_json(user.user().gh_id);
    expired["state"] = json!("expired");
    admin_api_holds(app.admin_api(), &expired).await;
    let counts = Counts {
        active: 1,
        registered: 2,
        in_sandbox: 1,
    };
    sandbox_counts(app.admin_api(), user.user(), counts).await;
    db_new_registration(&app, &user).await;

    let response = user
        .post(&format!("/api/v1/issuers/{ISSUER_ID}/renew"), &json!({}))
        .await;

    assert_snapshot!(response.status(), @"409 Conflict");
}

#[tokio::test]
async fn approving_leaves_out_the_registration_it_approves() {
    let sandbox = config::Sandbox {
        max_issuers_per_account: 1,
        max_registrations_per_account: 1,
        ..approval_needed()
    };
    let (app, _anon) = TestApp::init().with_sandbox(sandbox).empty().await;
    let user = app.db_new_registrant("octocat").await;
    sandbox_counts(app.admin_api(), user.user(), Counts::default()).await;
    Mock::given(method("POST"))
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
    reject(&app, registration.id, "Not now.").await;

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
        " Use a key you made for this sandbox. ",
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
async fn a_rejection_needs_a_reason_on_one_line() {
    let (app, _anon) = TestApp::init()
        .with_sandbox(approval_needed())
        .empty()
        .await;
    let user = app.db_new_registrant("octocat").await;
    let registration = db_new_registration(&app, &user).await;
    let mut conn = app.db_conn().await;

    for reason in [" ", "Two\nlines", &"a".repeat(501)] {
        let error = sandbox::reject(&mut conn, registration.id, reason)
            .await
            .unwrap_err();

        assert_eq!(
            error.to_string(),
            "The reason must be 1 to 500 characters, on one line."
        );
    }
    assert_eq!(registrations_of(&user).await[0]["state"], "pending");
}

#[tokio::test]
async fn the_database_keeps_rejections_without_a_reason_out() {
    let (app, _anon) = TestApp::init()
        .with_sandbox(approval_needed())
        .empty()
        .await;
    let user = app.db_new_registrant("octocat").await;
    let registration = db_new_registration(&app, &user).await;
    let mut conn = app.db_conn().await;

    let blank = Registration::reject(&mut conn, registration.id, " ").await;
    let missing = diesel::sql_query("UPDATE registrations SET decided_at = now() WHERE id = $1")
        .bind::<BigInt, _>(registration.id)
        .execute(&mut conn)
        .await;

    assert!(blank.is_err());
    assert!(missing.is_err());
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
async fn another_users_registration_is_never_locked() {
    let (app, _anon) = TestApp::init()
        .with_sandbox(approval_needed())
        .empty()
        .await;
    let owner = app.db_new_registrant("octocat").await;
    let registration = db_new_registration(&app, &owner).await;
    let someone_else = app.db_new_registrant("hubot").await;
    let mut conn = app.db_conn().await;

    conn.transaction(async |conn| {
        let theirs =
            Registration::try_lock(conn, registration.id, Some(someone_else.user().id)).await?;

        assert!(theirs.is_none());
        let mut other = app.db_conn().await;
        let free = Registration::try_lock(&mut other, registration.id, None).await?;
        assert!(free.is_some(), "the registration was locked");
        QueryResult::Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn decided_registrations_cannot_be_withdrawn() {
    let (app, _anon) = TestApp::init()
        .with_sandbox(approval_needed())
        .empty()
        .await;
    let user = app.db_new_registrant("octocat").await;
    let registration = db_new_registration(&app, &user).await;
    reject(&app, registration.id, "Not now.").await;

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

#[tokio::test]
async fn a_withdrawal_during_an_approval_is_refused() {
    let (app, _anon) = TestApp::init()
        .with_sandbox(approval_needed())
        .empty()
        .await;
    let user = app.db_new_registrant("octocat").await;
    sandbox_counts(app.admin_api(), user.user(), Counts::default()).await;
    slow_creation(&app, &user, 201).await;
    let registration = db_new_registration(&app, &user).await;
    let mut conn = app.db_conn().await;

    let (approved, withdrawn) = tokio::join!(
        sandbox::approve(app.ctx(), &mut conn, registration.id),
        async {
            creation_under_way(&app).await;
            withdraw(&user, registration.id).await
        },
    );

    assert_snapshot!(withdrawn.status(), @"409 Conflict");
    assert_snapshot!(detail(&withdrawn), @r#""This registration is being decided. Please try again in a moment.""#);
    assert_eq!(approved.unwrap().id, ISSUER_ID);
    assert_eq!(registrations_of(&user).await[0]["state"], "approved");
}

#[tokio::test]
async fn a_withdrawal_after_a_failed_approval_goes_through() {
    let (app, _anon) = TestApp::init()
        .with_sandbox(approval_needed())
        .empty()
        .await;
    let user = app.db_new_registrant("octocat").await;
    sandbox_counts(app.admin_api(), user.user(), Counts::default()).await;
    slow_creation(&app, &user, 503).await;
    let registration = db_new_registration(&app, &user).await;
    let mut conn = app.db_conn().await;

    let (approved, withdrawn) = tokio::join!(
        sandbox::approve(app.ctx(), &mut conn, registration.id),
        async {
            creation_under_way(&app).await;
            withdraw(&user, registration.id).await
        },
    );

    assert_eq!(withdrawn.status(), 409);
    assert!(approved.is_err());
    assert_snapshot!(withdraw(&user, registration.id).await.status(), @"204 No Content");
    assert!(registrations_of(&user).await.is_empty());
}

#[tokio::test]
async fn a_rejection_during_an_approval_is_refused() {
    let (app, _anon) = TestApp::init()
        .with_sandbox(approval_needed())
        .empty()
        .await;
    let user = app.db_new_registrant("octocat").await;
    sandbox_counts(app.admin_api(), user.user(), Counts::default()).await;
    slow_creation(&app, &user, 201).await;
    let registration = db_new_registration(&app, &user).await;
    let (mut approving, mut rejecting) = (app.db_conn().await, app.db_conn().await);

    let (approved, rejected) = tokio::join!(
        sandbox::approve(app.ctx(), &mut approving, registration.id),
        async {
            creation_under_way(&app).await;
            sandbox::reject(&mut rejecting, registration.id, "No.").await
        },
    );

    assert_snapshot!(rejected.unwrap_err(), @"This registration is being decided. Please try again in a moment.");
    assert_eq!(approved.unwrap().id, ISSUER_ID);
    assert_eq!(registrations_of(&user).await[0]["state"], "approved");
}

#[tokio::test]
async fn of_two_approvals_at_once_one_creates_the_issuer() {
    let (app, _anon) = TestApp::init()
        .with_sandbox(approval_needed())
        .empty()
        .await;
    let user = app.db_new_registrant("octocat").await;
    sandbox_counts(app.admin_api(), user.user(), Counts::default()).await;
    slow_creation(&app, &user, 201).await;
    let registration = db_new_registration(&app, &user).await;
    let (mut first, mut second) = (app.db_conn().await, app.db_conn().await);

    let (approved, approved_again) = tokio::join!(
        sandbox::approve(app.ctx(), &mut first, registration.id),
        async {
            creation_under_way(&app).await;
            sandbox::approve(app.ctx(), &mut second, registration.id).await
        },
    );

    assert_snapshot!(approved_again.unwrap_err(), @"Another issuer is being registered. Please try again in a moment.");
    assert_eq!(approved.unwrap().id, ISSUER_ID);
    assert_eq!(registrations_of(&user).await[0]["state"], "approved");
}

#[tokio::test]
async fn a_withdrawal_while_an_operator_rejects_is_refused() {
    let (app, _anon) = TestApp::init()
        .with_sandbox(approval_needed())
        .empty()
        .await;
    let user = app.db_new_registrant("octocat").await;
    let registration = db_new_registration(&app, &user).await;
    let mut conn = app.db_conn().await;

    conn.transaction(async |conn| {
        // Holds the registration as a rejection does until it commits.
        let held = Registration::try_lock(conn, registration.id, None).await?;
        assert!(held.is_some());

        let response = withdraw(&user, registration.id).await;

        assert_snapshot!(response.status(), @"409 Conflict");
        QueryResult::Ok(())
    })
    .await
    .unwrap();
    assert_eq!(registrations_of(&user).await[0]["state"], "pending");
}

#[tokio::test]
async fn a_withdrawn_registration_cannot_be_rejected() {
    let (app, _anon) = TestApp::init()
        .with_sandbox(approval_needed())
        .empty()
        .await;
    let user = app.db_new_registrant("octocat").await;
    let registration = db_new_registration(&app, &user).await;
    assert_eq!(withdraw(&user, registration.id).await.status(), 204);

    let error = sandbox::reject(&mut app.db_conn().await, registration.id, "No.")
        .await
        .unwrap_err();

    assert_snapshot!(error, @"No registration has this id.");
}
