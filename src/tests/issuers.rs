use diesel::QueryResult;
use diesel_async::AsyncConnection;
use http::{Method, StatusCode};
use insta::assert_snapshot;
use serde_json::{Value, json};
use status_list_portal::{config, db};
use time::{Duration, OffsetDateTime};
use wiremock::matchers::{body_json, header, method, path, query_param, query_param_is_missing};
use wiremock::{Mock, MockServer, ResponseTemplate};

use crate::util::admin_api::{
    ISSUER_ID, admin_api_holds, issuer_json, issuer_path, issuer_response, page_response,
    public_jwk_json,
};
use crate::util::{MANAGEMENT_URL, MockCookieUser, ORIGIN, RequestHelper, TestApp};

const ISSUER_URL: &str = "/api/v1/issuers/6f1c2a9e-0b7d-4c1e-9a53-2f8d7e4b1c00";

/// A user who may register issuers: they accepted the terms.
async fn registrant(app: &TestApp) -> MockCookieUser {
    let user = app.db_new_user("octocat").await;
    app.accept_terms(user.user()).await;
    user
}

fn owner(user: &MockCookieUser) -> String {
    format!("github|{}", user.user().gh_id)
}

fn own_issuer_json(user: &MockCookieUser) -> Value {
    issuer_json(user.user().gh_id)
}

/// The issuer as the portal shows it.
fn shown_issuer_json() -> Value {
    json!({
        "id": ISSUER_ID,
        "display_name": "Acme Wallet Issuer",
        "state": "active",
        "created_at": "2026-09-21T14:13:20Z",
        "expires_at": "2026-10-21T14:13:20Z",
        "keys": [{
            "kid": "QoRTRSkzlInq0eOmPqJwmZ5OS9pxkN_SIf5KgZP2F8M",
            "alg": "ES256",
            "state": "active",
        }],
        "management_url": MANAGEMENT_URL,
        "audience": MANAGEMENT_URL,
    })
}

fn new_issuer_body() -> Value {
    let mut public_key = public_jwk_json();
    public_key["kid"] = json!("my-key");
    json!({ "display_name": " Acme Wallet Issuer ", "alg": "ES256", "public_key": public_key })
}

/// Sandbox issuers as the admin API counts them.
#[derive(Default)]
struct Counts {
    /// The user's active issuers.
    active: usize,
    /// The user's issuers in any state.
    registered: usize,
    /// Active issuers of every account.
    in_sandbox: usize,
}

async fn sandbox_counts(admin_api: &MockServer, user: &MockCookieUser, counts: Counts) {
    let own = |count| page_response(&vec![own_issuer_json(user); count]);
    Mock::given(method("GET"))
        .and(path("/admin/v1/issuers"))
        .and(query_param("owner", owner(user)))
        .and(query_param("state", "active"))
        .respond_with(own(counts.active))
        .mount(admin_api)
        .await;
    Mock::given(method("GET"))
        .and(path("/admin/v1/issuers"))
        .and(query_param("owner", owner(user)))
        .and(query_param_is_missing("state"))
        .and(query_param("tier", "sandbox"))
        .and(query_param("source", "portal"))
        .respond_with(own(counts.registered))
        .mount(admin_api)
        .await;
    Mock::given(method("GET"))
        .and(path("/admin/v1/issuers"))
        .and(query_param_is_missing("owner"))
        .and(query_param("state", "active"))
        .and(query_param("tier", "sandbox"))
        .and(query_param("source", "portal"))
        .respond_with(page_response(&vec![issuer_json(0); counts.in_sandbox]))
        .mount(admin_api)
        .await;
}

/// Checks that an issuer sent as `body` stays active for one sandbox lifetime from now.
#[track_caller]
fn assert_one_lifetime(body: &Value) {
    let expires_at = body["expires_at"].as_i64().expect("no expires_at");
    let expires_at = OffsetDateTime::from_unix_timestamp(expires_at).unwrap();
    let expected = OffsetDateTime::now_utc() + Duration::days(30) - Duration::minutes(5);
    assert!(
        (expected - expires_at).abs() < Duration::minutes(1),
        "{expires_at}"
    );
}

async fn requests_with_method(admin_api: &MockServer, wanted: &str) -> Vec<wiremock::Request> {
    let requests = admin_api.received_requests().await.unwrap();
    requests
        .into_iter()
        .filter(|request| request.method.as_str() == wanted)
        .collect()
}

#[tokio::test]
async fn anonymous_users_are_asked_to_sign_in() {
    let (app, anon) = TestApp::init().empty().await;

    for (method, path) in [
        (Method::GET, "/api/v1/issuers"),
        (Method::POST, "/api/v1/issuers"),
        (Method::GET, ISSUER_URL),
        (Method::DELETE, ISSUER_URL),
        (Method::POST, &format!("{ISSUER_URL}/renew")),
    ] {
        let response = match method {
            Method::GET => anon.get(path).await,
            Method::DELETE => anon.delete(path).await,
            _ => anon.post(path, &new_issuer_body()).await,
        };

        assert_eq!(response.status(), 401, "{method} {path}");
    }
    assert!(
        app.admin_api()
            .received_requests()
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn users_list_their_own_issuers() {
    let (app, _anon) = TestApp::init().empty().await;
    let user = app.db_new_user("octocat").await;
    Mock::given(method("GET"))
        .and(path("/admin/v1/issuers"))
        .and(query_param("owner", owner(&user)))
        .and(query_param("cursor", "page-2"))
        .respond_with(page_response(&[own_issuer_json(&user)]))
        .mount(app.admin_api())
        .await;

    let response = user.get("/api/v1/issuers?cursor=page-2").await;

    assert_snapshot!(response.status(), @"200 OK");
    assert_eq!(
        response.json(),
        json!({ "issuers": [shown_issuer_json()], "next_cursor": null })
    );
}

#[tokio::test]
async fn users_get_their_own_issuer() {
    let (app, _anon) = TestApp::init().empty().await;
    let user = app.db_new_user("octocat").await;
    admin_api_holds(app.admin_api(), &own_issuer_json(&user)).await;

    let response = user.get(ISSUER_URL).await;

    assert_snapshot!(response.status(), @"200 OK");
    assert_eq!(response.json(), shown_issuer_json());
}

#[tokio::test]
async fn another_users_issuer_is_out_of_reach() {
    let (app, _anon) = TestApp::init().empty().await;
    let owner = app.db_new_user("octocat").await;
    let someone_else = registrant(&app).await;
    admin_api_holds(app.admin_api(), &own_issuer_json(&owner)).await;
    sandbox_counts(app.admin_api(), &someone_else, Counts::default()).await;

    let read = someone_else.get(ISSUER_URL).await;
    let renewed = someone_else
        .post(&format!("{ISSUER_URL}/renew"), &json!({}))
        .await;
    let deleted = someone_else.delete(ISSUER_URL).await;

    assert_snapshot!(read.status(), @"404 Not Found");
    assert_snapshot!(renewed.status(), @"404 Not Found");
    assert_snapshot!(deleted.status(), @"404 Not Found");
    assert!(
        requests_with_method(app.admin_api(), "PATCH")
            .await
            .is_empty()
    );
}

#[tokio::test]
async fn registering_creates_an_issuer_the_user_owns() {
    let (app, _anon) = TestApp::init().empty().await;
    let user = registrant(&app).await;
    let counts = Counts {
        active: 1,
        registered: 3,
        in_sandbox: 99,
    };
    sandbox_counts(app.admin_api(), &user, counts).await;
    Mock::given(method("POST"))
        .and(path("/admin/v1/issuers"))
        .respond_with(issuer_response(201, "1", &own_issuer_json(&user)))
        .expect(1)
        .mount(app.admin_api())
        .await;

    let response = user.post("/api/v1/issuers", &new_issuer_body()).await;

    assert_snapshot!(response.status(), @"201 Created");
    assert_eq!(
        response.header("location"),
        Some(format!("{ORIGIN}{ISSUER_URL}").as_str())
    );
    assert_eq!(response.json(), shown_issuer_json());
    let post = &requests_with_method(app.admin_api(), "POST").await[0];
    let sent: Value = serde_json::from_slice(&post.body).unwrap();
    assert_eq!(sent["display_name"], "Acme Wallet Issuer");
    assert_eq!(sent["owner"], owner(&user));
    assert_eq!(sent["public_key"], public_jwk_json());
    assert_eq!(sent["alg"], "ES256");
    assert_one_lifetime(&sent);
}

#[tokio::test]
async fn a_registration_under_way_holds_back_others() {
    let (app, _anon) = TestApp::init().empty().await;
    let user = registrant(&app).await;
    let mut conn = app.db_conn().await;

    conn.transaction(async |conn| {
        assert!(db::try_lock_issuer_limits(conn).await?);

        let response = user.post("/api/v1/issuers", &new_issuer_body()).await;

        assert_snapshot!(response.status(), @"503 Service Unavailable");
        QueryResult::Ok(())
    })
    .await
    .unwrap();
    assert!(
        app.admin_api()
            .received_requests()
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn unknown_members_are_refused() {
    let (app, _anon) = TestApp::init().empty().await;
    let user = registrant(&app).await;
    let mut body = new_issuer_body();
    body["tier"] = json!("standard");

    let response = user.post("/api/v1/issuers", &body).await;

    assert_snapshot!(response.status(), @"422 Unprocessable Entity");
    assert!(
        app.admin_api()
            .received_requests()
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn registering_needs_the_current_terms() {
    let (app, _anon) = TestApp::init().empty().await;
    let user = app.db_new_user("octocat").await;

    let response = user.post("/api/v1/issuers", &new_issuer_body()).await;

    assert_snapshot!(response.status(), @"403 Forbidden");
    assert!(
        app.admin_api()
            .received_requests()
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn private_keys_never_leave_the_portal() {
    let (app, _anon) = TestApp::init().empty().await;
    let user = registrant(&app).await;
    let mut body = new_issuer_body();
    body["public_key"]["d"] = json!("c2VjcmV0");

    let response = user.post("/api/v1/issuers", &body).await;

    assert_snapshot!(response.status(), @"422 Unprocessable Entity");
    assert!(
        app.admin_api()
            .received_requests()
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn a_key_for_another_algorithm_is_refused() {
    let (app, _anon) = TestApp::init().empty().await;
    let user = registrant(&app).await;
    let mut body = new_issuer_body();
    body["alg"] = json!("ES384");

    let response = user.post("/api/v1/issuers", &body).await;

    assert_snapshot!(response.status(), @"422 Unprocessable Entity");
    assert_eq!(
        response.json(),
        json!({ "errors": [{
            "detail": "For ES384, the public key must be an EC key on the P-384 curve, with \"x\" and \"y\"."
        }] })
    );
}

#[tokio::test]
async fn display_names_are_one_to_a_hundred_printable_characters() {
    let (app, _anon) = TestApp::init().empty().await;
    let user = registrant(&app).await;

    for name in [
        "   ".to_string(),
        "x".repeat(101),
        "Acme\nIssuer".to_string(),
    ] {
        let mut body = new_issuer_body();
        body["display_name"] = json!(name);

        let response = user.post("/api/v1/issuers", &body).await;

        assert_eq!(response.status(), 422, "{name:?}");
    }
}

#[tokio::test]
async fn an_account_at_its_limit_registers_no_more() {
    let (app, _anon) = TestApp::init().empty().await;
    let user = registrant(&app).await;
    let counts = Counts {
        active: 2,
        registered: 2,
        in_sandbox: 2,
    };
    sandbox_counts(app.admin_api(), &user, counts).await;

    let response = user.post("/api/v1/issuers", &new_issuer_body()).await;

    assert_snapshot!(response.status(), @"409 Conflict");
    assert_eq!(
        response.json(),
        json!({ "errors": [{
            "detail": "An account can have 2 active sandbox issuers. Delete one first."
        }] })
    );
    assert!(
        requests_with_method(app.admin_api(), "POST")
            .await
            .is_empty()
    );
}

#[tokio::test]
async fn no_registrations_allowed_stops_registering() {
    let sandbox = config::Sandbox {
        max_registrations_per_account: 0,
        ..config::Sandbox::default()
    };
    let (app, _anon) = TestApp::init().with_sandbox(sandbox).empty().await;
    let user = registrant(&app).await;

    let response = user.post("/api/v1/issuers", &new_issuer_body()).await;

    assert_snapshot!(response.status(), @"403 Forbidden");
    assert!(
        app.admin_api()
            .received_requests()
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn no_active_issuers_allowed_stops_registering() {
    let sandbox = config::Sandbox {
        max_issuers_per_account: 0,
        ..config::Sandbox::default()
    };
    let (app, _anon) = TestApp::init().with_sandbox(sandbox).empty().await;
    let user = registrant(&app).await;
    sandbox_counts(app.admin_api(), &user, Counts::default()).await;

    let response = user.post("/api/v1/issuers", &new_issuer_body()).await;

    assert_snapshot!(response.status(), @"409 Conflict");
    assert!(
        requests_with_method(app.admin_api(), "POST")
            .await
            .is_empty()
    );
}

#[tokio::test]
async fn an_account_that_used_its_registrations_registers_no_more() {
    let (app, _anon) = TestApp::init().empty().await;
    let user = registrant(&app).await;
    let counts = Counts {
        registered: 10,
        ..Counts::default()
    };
    sandbox_counts(app.admin_api(), &user, counts).await;

    let response = user.post("/api/v1/issuers", &new_issuer_body()).await;

    assert_snapshot!(response.status(), @"403 Forbidden");
    assert_eq!(
        response.json(),
        json!({ "errors": [{
            "detail": "An account can register 10 sandbox issuers in all, deleted ones \
                       included, and this account has."
        }] })
    );
    assert!(
        requests_with_method(app.admin_api(), "POST")
            .await
            .is_empty()
    );
}

#[tokio::test]
async fn a_full_sandbox_registers_no_more() {
    let sandbox = config::Sandbox {
        max_active_issuers: 3,
        ..config::Sandbox::default()
    };
    let (app, _anon) = TestApp::init().with_sandbox(sandbox).empty().await;
    let user = registrant(&app).await;
    let counts = Counts {
        in_sandbox: 3,
        ..Counts::default()
    };
    sandbox_counts(app.admin_api(), &user, counts).await;

    let response = user.post("/api/v1/issuers", &new_issuer_body()).await;

    assert_snapshot!(response.status(), @"503 Service Unavailable");
    assert!(
        requests_with_method(app.admin_api(), "POST")
            .await
            .is_empty()
    );
}

#[tokio::test]
async fn a_key_registered_already_is_a_conflict() {
    let (app, _anon) = TestApp::init().empty().await;
    let user = registrant(&app).await;
    sandbox_counts(app.admin_api(), &user, Counts::default()).await;
    Mock::given(method("POST"))
        .and(path("/admin/v1/issuers"))
        .respond_with(ResponseTemplate::new(409).set_body_json(json!({
            "error": "key_material_in_use",
            "error_description": "the key is registered for another issuer",
        })))
        .mount(app.admin_api())
        .await;

    let response = user.post("/api/v1/issuers", &new_issuer_body()).await;

    assert_snapshot!(response.status(), @"409 Conflict");
    assert_eq!(
        response.json(),
        json!({ "errors": [{ "detail": "This public key is already registered." }] })
    );
}

#[tokio::test]
async fn renewing_keeps_an_issuer_active_for_another_lifetime() {
    // No counts are mounted: an active issuer keeps its place, so nothing is counted.
    let (app, _anon) = TestApp::init().empty().await;
    let user = registrant(&app).await;
    admin_api_holds(app.admin_api(), &own_issuer_json(&user)).await;
    Mock::given(method("PATCH"))
        .and(path(issuer_path(&own_issuer_json(&user))))
        .and(header("if-issuer-version", "7"))
        .respond_with(issuer_response(200, "8", &own_issuer_json(&user)))
        .expect(1)
        .mount(app.admin_api())
        .await;

    let response = user.post(&format!("{ISSUER_URL}/renew"), &json!({})).await;

    assert_snapshot!(response.status(), @"200 OK");
    let patch = &requests_with_method(app.admin_api(), "PATCH").await[0];
    assert_one_lifetime(&serde_json::from_slice(&patch.body).unwrap());
}

#[tokio::test]
async fn renewing_an_expired_issuer_needs_room() {
    let (app, _anon) = TestApp::init().empty().await;
    let user = registrant(&app).await;
    let mut expired = own_issuer_json(&user);
    expired["state"] = json!("expired");
    admin_api_holds(app.admin_api(), &expired).await;
    let counts = Counts {
        active: 2,
        registered: 3,
        in_sandbox: 2,
    };
    sandbox_counts(app.admin_api(), &user, counts).await;

    let response = user.post(&format!("{ISSUER_URL}/renew"), &json!({})).await;

    assert_snapshot!(response.status(), @"409 Conflict");
    assert!(
        requests_with_method(app.admin_api(), "PATCH")
            .await
            .is_empty()
    );
}

#[tokio::test]
async fn renewing_an_expired_issuer_with_room_makes_it_active() {
    let (app, _anon) = TestApp::init().empty().await;
    let user = registrant(&app).await;
    let mut expired = own_issuer_json(&user);
    expired["state"] = json!("expired");
    admin_api_holds(app.admin_api(), &expired).await;
    // Renewing creates no issuer, so the registrations left do not matter.
    let counts = Counts {
        active: 1,
        registered: 10,
        in_sandbox: 1,
    };
    sandbox_counts(app.admin_api(), &user, counts).await;
    Mock::given(method("PATCH"))
        .respond_with(issuer_response(200, "8", &own_issuer_json(&user)))
        .expect(1)
        .mount(app.admin_api())
        .await;

    let response = user.post(&format!("{ISSUER_URL}/renew"), &json!({})).await;

    assert_snapshot!(response.status(), @"200 OK");
    assert_eq!(response.json()["state"], "active");
}

#[tokio::test]
async fn deleted_issuers_cannot_be_renewed() {
    let (app, _anon) = TestApp::init().empty().await;
    let user = registrant(&app).await;
    let mut deleted = own_issuer_json(&user);
    deleted["state"] = json!("suspended");
    admin_api_holds(app.admin_api(), &deleted).await;

    let response = user.post(&format!("{ISSUER_URL}/renew"), &json!({})).await;

    assert_snapshot!(response.status(), @"409 Conflict");
    assert!(
        requests_with_method(app.admin_api(), "PATCH")
            .await
            .is_empty()
    );
}

#[tokio::test]
async fn renewing_needs_the_current_terms() {
    let (app, _anon) = TestApp::init().empty().await;
    let user = app.db_new_user("octocat").await;

    let response = user.post(&format!("{ISSUER_URL}/renew"), &json!({})).await;

    assert_snapshot!(response.status(), @"403 Forbidden");
}

#[tokio::test]
async fn deleting_suspends_the_issuer() {
    let (app, _anon) = TestApp::init().empty().await;
    let user = app.db_new_user("octocat").await;
    admin_api_holds(app.admin_api(), &own_issuer_json(&user)).await;
    Mock::given(method("PATCH"))
        .and(path(issuer_path(&own_issuer_json(&user))))
        .and(header("if-issuer-version", "7"))
        .and(body_json(json!({ "state": "suspended" })))
        .respond_with(issuer_response(200, "8", &own_issuer_json(&user)))
        .expect(1)
        .mount(app.admin_api())
        .await;

    let response = user.delete(ISSUER_URL).await;

    assert_snapshot!(response.status(), @"204 No Content");
}

#[tokio::test]
async fn deleting_a_deleted_issuer_changes_nothing() {
    let (app, _anon) = TestApp::init().empty().await;
    let user = app.db_new_user("octocat").await;
    let mut deleted = own_issuer_json(&user);
    deleted["state"] = json!("suspended");
    admin_api_holds(app.admin_api(), &deleted).await;

    let response = user.delete(ISSUER_URL).await;

    assert_snapshot!(response.status(), @"204 No Content");
    assert!(
        requests_with_method(app.admin_api(), "PATCH")
            .await
            .is_empty()
    );
}

#[tokio::test]
async fn a_change_made_in_between_is_a_conflict() {
    let (app, _anon) = TestApp::init().empty().await;
    let user = registrant(&app).await;
    admin_api_holds(app.admin_api(), &own_issuer_json(&user)).await;
    Mock::given(method("PATCH"))
        .respond_with(
            ResponseTemplate::new(412).set_body_json(json!({ "error": "precondition_failed" })),
        )
        .mount(app.admin_api())
        .await;

    let renewed = user.post(&format!("{ISSUER_URL}/renew"), &json!({})).await;
    let deleted = user.delete(ISSUER_URL).await;

    for response in [renewed, deleted] {
        assert_eq!(response.status(), StatusCode::CONFLICT);
        assert_eq!(
            response.json(),
            json!({ "errors": [{
                "detail": "The issuer changed while this request ran. Please try again."
            }] })
        );
    }
}

#[tokio::test]
async fn an_unreachable_status_list_server_is_a_bad_gateway() {
    let (app, _anon) = TestApp::init().empty().await;
    let user = app.db_new_user("octocat").await;
    Mock::given(method("GET"))
        .respond_with(
            ResponseTemplate::new(500).set_body_json(json!({ "error": "internal_error" })),
        )
        .mount(app.admin_api())
        .await;

    let response = user.get("/api/v1/issuers").await;

    assert_snapshot!(response.status(), @"502 Bad Gateway");
    assert_eq!(
        response.json(),
        json!({ "errors": [{
            "detail": "The Status List Server did not complete the request. Please try again later."
        }] })
    );
}
