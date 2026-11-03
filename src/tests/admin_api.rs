use std::collections::BTreeMap;

use http::StatusCode;
use jsonwebtoken::jwk::Jwk;
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Validation, decode, decode_header};
use serde_json::{Value, json};
use status_list_portal::admin_api::{
    AdminApi, AdminApiError, ErrorCode, IssuerChange, IssuerQuery, IssuerSource, IssuerState,
    IssuerTier, KeyState, NewIssuer, Owner, RequestedState, SigningAlgorithm,
};
use status_list_portal::config;
use status_list_portal::util::errors::BoxedAppError;
use time::OffsetDateTime;
use tokio::net::TcpListener;
use url::Url;
use wiremock::matchers::{
    any, body_json, header, method, path, query_param, query_param_is_missing,
};
use wiremock::{Mock, MockServer, ResponseTemplate};

use crate::util::admin_api::{
    EXPIRES_AT, ISSUER_ID, admin_api_holds, issuer_json, issuer_response, public_jwk_json,
    static_issuer_json,
};
use crate::util::{ADMIN_AUDIENCE, ADMIN_KEY_ID, ADMIN_PRINCIPAL, ADMIN_SIGNING_KEY_FILE, TestApp};

const ISSUER_PATH: &str = "/admin/v1/issuers/6f1c2a9e-0b7d-4c1e-9a53-2f8d7e4b1c00";
const GH_ID: i64 = 5821374;

fn octocat() -> Owner {
    Owner::github(GH_ID)
}

fn query_pairs(url: &Url) -> BTreeMap<String, String> {
    url.query_pairs().into_owned().collect()
}

fn new_issuer() -> NewIssuer {
    NewIssuer {
        display_name: "Acme Wallet Issuer".to_string(),
        public_key: serde_json::from_value(public_jwk_json()).unwrap(),
        alg: SigningAlgorithm::Es256,
        expires_at: None,
    }
}

fn expires_at() -> OffsetDateTime {
    OffsetDateTime::from_unix_timestamp(EXPIRES_AT).unwrap()
}

/// Checks `token` the way the server does, with the public half of the app's admin key.
#[track_caller]
fn verify_admin_token(token: &str) -> Value {
    let pem = std::fs::read(ADMIN_SIGNING_KEY_FILE).unwrap();
    let private_key = EncodingKey::from_ec_pem(&pem).unwrap();
    let jwk = Jwk::from_encoding_key(&private_key, Algorithm::ES256).unwrap();
    let mut validation = Validation::new(Algorithm::ES256);
    validation.set_audience(&[ADMIN_AUDIENCE]);
    validation.set_issuer(&[ADMIN_PRINCIPAL]);
    validation.set_required_spec_claims(&["iss", "aud", "iat", "exp"]);

    let header = decode_header(token).unwrap();
    assert_eq!(header.typ.as_deref(), Some("statuslist-admin+jwt"));
    assert_eq!(header.kid.as_deref(), Some(ADMIN_KEY_ID));
    let public_key = DecodingKey::from_jwk(&jwk).unwrap();
    decode::<Value>(token, &public_key, &validation)
        .unwrap()
        .claims
}

#[tokio::test]
async fn calls_carry_an_admin_token_the_server_accepts() {
    let (app, _) = TestApp::init().empty().await;
    admin_api_holds(app.admin_api(), &issuer_json(GH_ID)).await;
    let owner = octocat();

    app.ctx()
        .admin_api
        .issuers_of(&owner)
        .get(ISSUER_ID)
        .await
        .unwrap();

    let requests = app.admin_api().received_requests().await.unwrap();
    let authorization = requests[0].headers["authorization"].to_str().unwrap();
    let token = authorization
        .strip_prefix("Bearer ")
        .expect("no bearer token");
    verify_admin_token(token);
}

#[tokio::test]
async fn an_account_gets_its_own_issuer() {
    let (app, _) = TestApp::init().empty().await;
    admin_api_holds(app.admin_api(), &issuer_json(GH_ID)).await;
    let owner = octocat();

    let issuer = app.ctx().admin_api.issuers_of(&owner).get(ISSUER_ID).await;

    let issuer = issuer.unwrap().expect("the issuer is missing");
    assert_eq!(issuer.id, ISSUER_ID);
    assert_eq!(issuer.source, IssuerSource::Portal);
    assert_eq!(issuer.tier, IssuerTier::Sandbox);
    assert_eq!(issuer.state, IssuerState::Active);
    assert_eq!(issuer.owner, Some(owner));
    assert_eq!(issuer.expires_at, Some(expires_at()));
    assert_eq!(issuer.keys[0].alg, SigningAlgorithm::Es256);
    assert_eq!(issuer.keys[0].state, KeyState::Active);
    assert_eq!(issuer.keys[0].public_key, new_issuer().public_key);
    assert_eq!(issuer.usage.keys_limit, None);
}

#[tokio::test]
async fn another_accounts_issuer_looks_missing() {
    let (app, _) = TestApp::init().empty().await;
    admin_api_holds(app.admin_api(), &issuer_json(GH_ID)).await;
    let someone_else = Owner::github(GH_ID + 1);

    let issuer = app
        .ctx()
        .admin_api
        .issuers_of(&someone_else)
        .get(ISSUER_ID)
        .await;

    assert_eq!(issuer.unwrap(), None);
}

#[tokio::test]
async fn a_static_issuer_looks_missing() {
    let (app, _) = TestApp::init().empty().await;
    Mock::given(method("GET"))
        .respond_with(issuer_response(200, "1", &static_issuer_json()))
        .mount(app.admin_api())
        .await;
    let owner = octocat();

    let issuer = app
        .ctx()
        .admin_api
        .issuers_of(&owner)
        .get("https://issuer.example.com")
        .await;

    assert_eq!(issuer.unwrap(), None);
}

#[tokio::test]
async fn an_issuer_the_admin_api_does_not_know_looks_missing() {
    let (app, _) = TestApp::init().empty().await;
    Mock::given(any())
        .respond_with(ResponseTemplate::new(404).set_body_json(json!({
            "error": "issuer_not_found",
        })))
        .mount(app.admin_api())
        .await;
    let owner = octocat();

    let issuer = app.ctx().admin_api.issuers_of(&owner).get(ISSUER_ID).await;

    assert_eq!(issuer.unwrap(), None);
}

#[tokio::test]
async fn an_issuer_id_is_one_path_segment() {
    let (app, _) = TestApp::init().empty().await;
    Mock::given(method("GET"))
        .and(path("/admin/v1/issuers/a%2Fb%3Fc"))
        .respond_with(issuer_response(200, "7", &issuer_json(GH_ID)))
        .expect(1)
        .mount(app.admin_api())
        .await;
    let owner = octocat();

    let issuer = app.ctx().admin_api.issuers_of(&owner).get("a/b?c").await;

    assert!(issuer.unwrap().is_some());
}

#[tokio::test]
async fn ids_one_path_segment_cannot_hold_look_missing() {
    let (app, _) = TestApp::init().empty().await;
    let owner = octocat();
    let issuers = app.ctx().admin_api.issuers_of(&owner);

    for id in ["", ".", "..", &"a".repeat(256)] {
        assert_eq!(issuers.get(id).await.unwrap(), None, "{id:?}");
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
async fn creating_an_issuer_makes_the_account_its_owner() {
    let (app, _) = TestApp::init().empty().await;
    Mock::given(method("POST"))
        .and(path("/admin/v1/issuers"))
        .and(body_json(json!({
            "display_name": "Acme Wallet Issuer",
            "owner": "github|5821374",
            "public_key": public_jwk_json(),
            "alg": "ES256",
        })))
        .respond_with(issuer_response(201, "1", &issuer_json(GH_ID)))
        .expect(1)
        .mount(app.admin_api())
        .await;
    let owner = octocat();

    let issuer = app
        .ctx()
        .admin_api
        .issuers_of(&owner)
        .create(&new_issuer())
        .await;

    assert_eq!(issuer.unwrap().id, ISSUER_ID);
}

#[tokio::test]
async fn creating_an_issuer_sends_its_expiry() {
    let (app, _) = TestApp::init().empty().await;
    Mock::given(method("POST"))
        .and(path("/admin/v1/issuers"))
        .and(body_json(json!({
            "display_name": "Acme Wallet Issuer",
            "owner": "github|5821374",
            "public_key": public_jwk_json(),
            "alg": "ES256",
            "expires_at": EXPIRES_AT,
        })))
        .respond_with(issuer_response(201, "1", &issuer_json(GH_ID)))
        .expect(1)
        .mount(app.admin_api())
        .await;
    let owner = octocat();
    let issuer = NewIssuer {
        expires_at: Some(expires_at()),
        ..new_issuer()
    };

    let issuer = app.ctx().admin_api.issuers_of(&owner).create(&issuer).await;

    assert_eq!(issuer.unwrap().id, ISSUER_ID);
}

#[tokio::test]
async fn suspending_an_issuer_sends_the_version_it_read() {
    let (app, _) = TestApp::init().empty().await;
    admin_api_holds(app.admin_api(), &issuer_json(GH_ID)).await;
    Mock::given(method("PATCH"))
        .and(path(ISSUER_PATH))
        .and(header("if-issuer-version", "7"))
        .and(body_json(json!({ "state": "suspended" })))
        .respond_with(issuer_response(200, "8", &issuer_json(GH_ID)))
        .expect(1)
        .mount(app.admin_api())
        .await;
    let owner = octocat();
    let suspend = IssuerChange::State(RequestedState::Suspended);

    let issuers = app.ctx().admin_api.issuers_of(&owner);
    let current = issuers.get(ISSUER_ID).await.unwrap().expect("no issuer");

    let issuer = issuers.update(&current, &suspend).await;

    assert!(issuer.unwrap().is_some());
}

#[tokio::test]
async fn renewing_an_issuer_sends_the_new_expiry() {
    let (app, _) = TestApp::init().empty().await;
    admin_api_holds(app.admin_api(), &issuer_json(GH_ID)).await;
    Mock::given(method("PATCH"))
        .and(path(ISSUER_PATH))
        .and(header("if-issuer-version", "7"))
        .and(body_json(json!({ "expires_at": EXPIRES_AT })))
        .respond_with(issuer_response(200, "8", &issuer_json(GH_ID)))
        .expect(1)
        .mount(app.admin_api())
        .await;
    let owner = octocat();
    let renew = IssuerChange::ExpiresAt(expires_at());

    let issuers = app.ctx().admin_api.issuers_of(&owner);
    let current = issuers.get(ISSUER_ID).await.unwrap().expect("no issuer");

    let issuer = issuers.update(&current, &renew).await;

    assert!(issuer.unwrap().is_some());
}

#[tokio::test]
async fn another_accounts_issuer_is_never_changed() {
    let (app, _) = TestApp::init().empty().await;
    admin_api_holds(app.admin_api(), &issuer_json(GH_ID)).await;
    Mock::given(method("PATCH"))
        .respond_with(issuer_response(200, "8", &issuer_json(GH_ID)))
        .expect(0)
        .mount(app.admin_api())
        .await;
    let owner = octocat();
    let someone_else = Owner::github(GH_ID + 1);
    let api = &app.ctx().admin_api;
    let read_by_owner = api.issuers_of(&owner).get(ISSUER_ID).await.unwrap();
    let suspend = IssuerChange::State(RequestedState::Suspended);

    let issuer = api
        .issuers_of(&someone_else)
        .update(&read_by_owner.expect("no issuer"), &suspend)
        .await;

    assert_eq!(issuer.unwrap(), None);
}

#[tokio::test]
async fn an_issuer_deleted_before_the_change_looks_missing() {
    let (app, _) = TestApp::init().empty().await;
    admin_api_holds(app.admin_api(), &issuer_json(GH_ID)).await;
    Mock::given(method("PATCH"))
        .respond_with(
            ResponseTemplate::new(404).set_body_json(json!({ "error": "issuer_not_found" })),
        )
        .expect(1)
        .mount(app.admin_api())
        .await;
    let owner = octocat();
    let issuers = app.ctx().admin_api.issuers_of(&owner);
    let current = issuers.get(ISSUER_ID).await.unwrap().expect("no issuer");
    let suspend = IssuerChange::State(RequestedState::Suspended);

    let issuer = issuers.update(&current, &suspend).await;

    assert_eq!(issuer.unwrap(), None);
}

#[tokio::test]
async fn a_change_racing_another_is_refused() {
    let (app, _) = TestApp::init().empty().await;
    admin_api_holds(app.admin_api(), &issuer_json(GH_ID)).await;
    Mock::given(method("PATCH"))
        .respond_with(
            ResponseTemplate::new(412)
                .insert_header("issuer-version", "8")
                .set_body_json(json!({ "error": "precondition_failed" })),
        )
        .mount(app.admin_api())
        .await;
    let owner = octocat();
    let issuers = app.ctx().admin_api.issuers_of(&owner);
    let current = issuers.get(ISSUER_ID).await.unwrap().expect("no issuer");
    let renew = IssuerChange::ExpiresAt(expires_at());

    let error = issuers.update(&current, &renew).await.unwrap_err();

    assert_eq!(error.code(), Some(ErrorCode::PreconditionFailed));
}

#[tokio::test]
async fn an_account_lists_only_its_own_issuers() {
    let (app, _) = TestApp::init().empty().await;
    Mock::given(method("GET"))
        .and(path("/admin/v1/issuers"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            // The admin API should not return another account's issuer; if it does, the
            // client drops it.
            "issuers": [static_issuer_json(), issuer_json(GH_ID), issuer_json(GH_ID + 1)],
            "next_cursor": "page-3",
        })))
        .mount(app.admin_api())
        .await;
    let owner = octocat();

    let page = app
        .ctx()
        .admin_api
        .issuers_of(&owner)
        .list(Some("page-2"))
        .await
        .unwrap();

    assert_eq!(page.issuers.len(), 1);
    assert_eq!(page.issuers[0].owner, Some(owner));
    assert_eq!(page.next_cursor.as_deref(), Some("page-3"));
    let requests = app.admin_api().received_requests().await.unwrap();
    assert_eq!(
        query_pairs(&requests[0].url),
        BTreeMap::from([
            ("owner".to_string(), "github|5821374".to_string()),
            ("cursor".to_string(), "page-2".to_string()),
        ])
    );
}

#[tokio::test]
async fn an_issuer_created_for_another_owner_is_unexpected() {
    let (app, _) = TestApp::init().empty().await;
    Mock::given(method("POST"))
        .respond_with(issuer_response(201, "1", &issuer_json(GH_ID + 1)))
        .mount(app.admin_api())
        .await;
    let owner = octocat();

    let error = app
        .ctx()
        .admin_api
        .issuers_of(&owner)
        .create(&new_issuer())
        .await
        .unwrap_err();

    assert!(matches!(error, AdminApiError::Unexpected(_)), "{error}");
}

#[tokio::test]
async fn counting_reads_pages_until_it_has_counted_enough() {
    let (app, _) = TestApp::init().empty().await;
    let two_issuers = [issuer_json(GH_ID), issuer_json(GH_ID)];
    for (cursor, next) in [(None, "page-2"), (Some("page-2"), "page-3")] {
        let request = Mock::given(method("GET")).and(path("/admin/v1/issuers"));
        let request = match cursor {
            Some(cursor) => request.and(query_param("cursor", cursor)),
            None => request.and(query_param_is_missing("cursor")),
        };
        request
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "issuers": two_issuers,
                "next_cursor": next,
            })))
            .expect(1)
            .mount(app.admin_api())
            .await;
    }

    let count = app
        .ctx()
        .admin_api
        .count_issuers(&IssuerQuery::default(), 3)
        .await
        .unwrap();

    assert_eq!(count, 4);
    let requests = app.admin_api().received_requests().await.unwrap();
    assert!(requests[0].url.query().unwrap().contains("limit=3"));
}

#[tokio::test]
async fn counting_to_zero_asks_nothing() {
    let (app, _) = TestApp::init().empty().await;

    let count = app
        .ctx()
        .admin_api
        .count_issuers(&IssuerQuery::default(), 0)
        .await
        .unwrap();

    assert_eq!(count, 0);
    assert!(
        app.admin_api()
            .received_requests()
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn counting_sends_only_the_filters_set() {
    let (app, _) = TestApp::init().empty().await;
    Mock::given(method("GET"))
        .and(path("/admin/v1/issuers"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "issuers": [static_issuer_json(), issuer_json(GH_ID)],
            "next_cursor": null,
        })))
        .mount(app.admin_api())
        .await;
    let query = IssuerQuery {
        source: Some(IssuerSource::Portal),
        tier: Some(IssuerTier::Sandbox),
        state: Some(IssuerState::Active),
        ..IssuerQuery::default()
    };

    let count = app.ctx().admin_api.count_issuers(&query, 10).await.unwrap();

    assert_eq!(count, 2);
    let requests = app.admin_api().received_requests().await.unwrap();
    assert_eq!(
        query_pairs(&requests[0].url),
        BTreeMap::from([
            ("source".to_string(), "portal".to_string()),
            ("tier".to_string(), "sandbox".to_string()),
            ("state".to_string(), "active".to_string()),
            ("limit".to_string(), "10".to_string()),
        ])
    );
}

#[tokio::test]
async fn refusals_carry_the_error_code() {
    let (app, _) = TestApp::init().empty().await;
    Mock::given(any())
        .respond_with(ResponseTemplate::new(409).set_body_json(json!({
            "error": "key_material_in_use",
            "error_description": "the key is already registered",
        })))
        .mount(app.admin_api())
        .await;
    let owner = octocat();

    let error = app
        .ctx()
        .admin_api
        .issuers_of(&owner)
        .create(&new_issuer())
        .await
        .unwrap_err();

    assert_eq!(error.code(), Some(ErrorCode::KeyMaterialInUse));
    assert_eq!(
        error.to_string(),
        "the admin API answered 409 Conflict key_material_in_use: the key is already registered"
    );
}

#[tokio::test]
async fn codes_added_to_the_contract_later_are_still_refusals() {
    let (app, _) = TestApp::init().empty().await;
    Mock::given(any())
        .respond_with(ResponseTemplate::new(503).set_body_json(json!({ "error": "maintenance" })))
        .mount(app.admin_api())
        .await;
    let owner = octocat();

    let error = app
        .ctx()
        .admin_api
        .issuers_of(&owner)
        .get(ISSUER_ID)
        .await
        .unwrap_err();

    assert_eq!(error.code(), Some(ErrorCode::Unknown));
}

#[tokio::test]
async fn errors_without_a_code_are_unexpected() {
    let (app, _) = TestApp::init().empty().await;
    Mock::given(any())
        .respond_with(ResponseTemplate::new(502).set_body_string("<html>Bad Gateway</html>"))
        .mount(app.admin_api())
        .await;
    let owner = octocat();

    let error = app
        .ctx()
        .admin_api
        .issuers_of(&owner)
        .get(ISSUER_ID)
        .await
        .unwrap_err();

    assert!(matches!(error, AdminApiError::Unexpected(_)), "{error}");
}

#[tokio::test]
async fn an_issuer_without_a_version_is_unexpected() {
    let (app, _) = TestApp::init().empty().await;
    Mock::given(any())
        .respond_with(ResponseTemplate::new(200).set_body_json(issuer_json(GH_ID)))
        .mount(app.admin_api())
        .await;
    let owner = octocat();

    let error = app
        .ctx()
        .admin_api
        .issuers_of(&owner)
        .get(ISSUER_ID)
        .await
        .unwrap_err();

    assert!(matches!(error, AdminApiError::Unexpected(_)), "{error}");
}

#[tokio::test]
async fn a_body_that_does_not_match_the_contract_is_unexpected() {
    let (app, _) = TestApp::init().empty().await;
    Mock::given(any())
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("issuer-version", "7")
                .set_body_json(json!({ "id": ISSUER_ID, "state": "archived" })),
        )
        .mount(app.admin_api())
        .await;
    let owner = octocat();

    let error = app
        .ctx()
        .admin_api
        .issuers_of(&owner)
        .get(ISSUER_ID)
        .await
        .unwrap_err();

    assert!(matches!(error, AdminApiError::Unexpected(_)), "{error}");
}

#[tokio::test]
async fn an_admin_api_that_is_down_is_a_bad_gateway() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    let admin_api = AdminApi::new(&config::AdminApi {
        url: Url::parse(&format!("http://{address}")).unwrap(),
        principal: ADMIN_PRINCIPAL.to_string(),
        audience: ADMIN_AUDIENCE.to_string(),
        signing_key_file: ADMIN_SIGNING_KEY_FILE.into(),
        key_id: ADMIN_KEY_ID.to_string(),
        algorithm: SigningAlgorithm::Es256,
    })
    .unwrap();
    let owner = octocat();

    let error = admin_api
        .issuers_of(&owner)
        .get(ISSUER_ID)
        .await
        .unwrap_err();

    assert!(matches!(error, AdminApiError::Request(_)), "{error}");
    let response = BoxedAppError::from(error).response();
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
}

#[tokio::test]
async fn redirects_are_not_followed() {
    let (app, _) = TestApp::init().empty().await;
    let elsewhere = MockServer::start().await;
    Mock::given(any())
        .respond_with(
            ResponseTemplate::new(307).insert_header("location", elsewhere.uri().as_str()),
        )
        .mount(app.admin_api())
        .await;
    let owner = octocat();

    let error = app
        .ctx()
        .admin_api
        .issuers_of(&owner)
        .get(ISSUER_ID)
        .await
        .unwrap_err();

    assert!(matches!(error, AdminApiError::Unexpected(_)), "{error}");
    assert!(elsewhere.received_requests().await.unwrap().is_empty());
}
