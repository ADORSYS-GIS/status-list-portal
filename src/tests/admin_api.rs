use jsonwebtoken::jwk::Jwk;
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Validation, decode, decode_header};
use serde_json::{Value, json};
use status_list_portal::admin_api::{
    AdminApiError, ErrorCode, IssuerChange, IssuerQuery, IssuerSource, IssuerState, IssuerTier,
    KeyState, NewIssuer, Owner, RequestedState, SigningAlgorithm,
};
use time::OffsetDateTime;
use wiremock::matchers::{any, body_json, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use crate::util::{ADMIN_AUDIENCE, ADMIN_KEY_ID, ADMIN_PRINCIPAL, ADMIN_SIGNING_KEY_FILE, TestApp};

const ISSUER_ID: &str = "6f1c2a9e-0b7d-4c1e-9a53-2f8d7e4b1c00";
const ISSUER_PATH: &str = "/admin/v1/issuers/6f1c2a9e-0b7d-4c1e-9a53-2f8d7e4b1c00";
const GH_ID: i64 = 5821374;
const EXPIRES_AT: i64 = 1_792_592_000;

fn octocat() -> Owner {
    Owner::github(GH_ID)
}

fn public_jwk_json() -> Value {
    json!({
        "kty": "EC",
        "crv": "P-256",
        "x": "GtkCNIXEFHQ8j_gwyoS0H9eZQGCI_Iaqm2wUymJvOfQ",
        "y": "D9yk2oGE-yZjX50UqD1j68vpRbEhM-WyuhAWrEPAcyI",
    })
}

/// A sandbox issuer that the GitHub account `gh_id` registered through the portal.
fn issuer_json(gh_id: i64) -> Value {
    json!({
        "id": ISSUER_ID,
        "aggregation_id": "0b8ef1c4-3c6a-4d2e-9f1a-5b7c9d2e4f60",
        "display_name": "Acme Wallet Issuer",
        "source": "portal",
        "tier": "sandbox",
        "state": "active",
        "owner": format!("github|{gh_id}"),
        "created_at": 1_790_000_000,
        "expires_at": EXPIRES_AT,
        "keys": [{
            "kid": "QoRTRSkzlInq0eOmPqJwmZ5OS9pxkN_SIf5KgZP2F8M",
            "alg": "ES256",
            "state": "active",
            "not_before": null,
            "not_after": null,
            "created_at": 1_790_000_000,
            "public_key": public_jwk_json(),
        }],
        "usage": { "lists": 3, "lists_limit": 10, "keys": 1, "keys_limit": 2 },
    })
}

/// An issuer from the server's registry file, which has no owner and never expires.
fn static_issuer_json() -> Value {
    json!({
        "id": "https://issuer.example.com",
        "aggregation_id": "a1d2c3b4-0000-4000-8000-000000000001",
        "source": "static",
        "tier": "standard",
        "state": "active",
        "owner": null,
        "created_at": null,
        "expires_at": null,
        "keys": [],
        "usage": { "lists": 0, "lists_limit": 1000, "keys": 0, "keys_limit": 5 },
    })
}

fn issuer_response(status: u16, etag: &str, gh_id: i64) -> ResponseTemplate {
    ResponseTemplate::new(status)
        .insert_header("etag", etag)
        .set_body_json(issuer_json(gh_id))
}

/// The admin API holds octocat's issuer at version `"7"`.
async fn admin_api_holds_octocats_issuer(admin_api: &MockServer) {
    Mock::given(method("GET"))
        .and(path(ISSUER_PATH))
        .respond_with(issuer_response(200, "\"7\"", GH_ID))
        .mount(admin_api)
        .await;
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
    admin_api_holds_octocats_issuer(app.admin_api()).await;
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
    admin_api_holds_octocats_issuer(app.admin_api()).await;
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
    assert_eq!(issuer.usage.keys_limit, 2);
}

#[tokio::test]
async fn another_accounts_issuer_looks_missing() {
    let (app, _) = TestApp::init().empty().await;
    admin_api_holds_octocats_issuer(app.admin_api()).await;
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
        .respond_with(issuer_response(200, "\"7\"", GH_ID))
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

    for id in ["", ".", ".."] {
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
        .respond_with(issuer_response(201, "\"1\"", GH_ID))
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
async fn suspending_an_issuer_sends_the_version_it_read() {
    let (app, _) = TestApp::init().empty().await;
    admin_api_holds_octocats_issuer(app.admin_api()).await;
    Mock::given(method("PATCH"))
        .and(path(ISSUER_PATH))
        .and(header("if-match", "\"7\""))
        .and(body_json(json!({ "state": "suspended" })))
        .respond_with(issuer_response(200, "\"8\"", GH_ID))
        .expect(1)
        .mount(app.admin_api())
        .await;
    let owner = octocat();
    let suspend = IssuerChange::State(RequestedState::Suspended);

    let issuer = app
        .ctx()
        .admin_api
        .issuers_of(&owner)
        .update(ISSUER_ID, &suspend)
        .await;

    assert!(issuer.unwrap().is_some());
}

#[tokio::test]
async fn renewing_an_issuer_sends_the_new_expiry() {
    let (app, _) = TestApp::init().empty().await;
    admin_api_holds_octocats_issuer(app.admin_api()).await;
    Mock::given(method("PATCH"))
        .and(path(ISSUER_PATH))
        .and(header("if-match", "\"7\""))
        .and(body_json(json!({ "expires_at": EXPIRES_AT })))
        .respond_with(issuer_response(200, "\"8\"", GH_ID))
        .expect(1)
        .mount(app.admin_api())
        .await;
    let owner = octocat();
    let renew = IssuerChange::ExpiresAt(expires_at());

    let issuer = app
        .ctx()
        .admin_api
        .issuers_of(&owner)
        .update(ISSUER_ID, &renew)
        .await;

    assert!(issuer.unwrap().is_some());
}

#[tokio::test]
async fn another_accounts_issuer_is_never_changed() {
    let (app, _) = TestApp::init().empty().await;
    admin_api_holds_octocats_issuer(app.admin_api()).await;
    Mock::given(method("PATCH"))
        .respond_with(issuer_response(200, "\"8\"", GH_ID))
        .expect(0)
        .mount(app.admin_api())
        .await;
    let someone_else = Owner::github(GH_ID + 1);
    let suspend = IssuerChange::State(RequestedState::Suspended);

    let issuer = app
        .ctx()
        .admin_api
        .issuers_of(&someone_else)
        .update(ISSUER_ID, &suspend)
        .await;

    assert_eq!(issuer.unwrap(), None);
}

#[tokio::test]
async fn a_change_racing_another_is_refused() {
    let (app, _) = TestApp::init().empty().await;
    admin_api_holds_octocats_issuer(app.admin_api()).await;
    Mock::given(method("PATCH"))
        .respond_with(
            ResponseTemplate::new(412)
                .insert_header("etag", "\"8\"")
                .set_body_json(json!({ "error": "precondition_failed" })),
        )
        .mount(app.admin_api())
        .await;
    let owner = octocat();
    let renew = IssuerChange::ExpiresAt(expires_at());

    let error = app
        .ctx()
        .admin_api
        .issuers_of(&owner)
        .update(ISSUER_ID, &renew)
        .await
        .unwrap_err();

    assert_eq!(error.code(), Some(ErrorCode::PreconditionFailed));
}

#[tokio::test]
async fn an_account_lists_only_its_own_issuers() {
    let (app, _) = TestApp::init().empty().await;
    Mock::given(method("GET"))
        .and(path("/admin/v1/issuers"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "issuers": [issuer_json(GH_ID)],
            "next_cursor": null,
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
    assert_eq!(page.next_cursor, None);
    let requests = app.admin_api().received_requests().await.unwrap();
    assert_eq!(
        requests[0].url.query(),
        Some("owner=github%7C5821374&cursor=page-2")
    );
}

#[tokio::test]
async fn listing_every_issuer_sends_only_the_filters_set() {
    let (app, _) = TestApp::init().empty().await;
    Mock::given(method("GET"))
        .and(path("/admin/v1/issuers"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "issuers": [static_issuer_json(), issuer_json(GH_ID)],
            "next_cursor": "page-2",
        })))
        .mount(app.admin_api())
        .await;
    let query = IssuerQuery {
        tier: Some(IssuerTier::Sandbox),
        state: Some(IssuerState::Active),
        limit: Some(100),
        ..IssuerQuery::default()
    };

    let page = app.ctx().admin_api.list_issuers(&query).await.unwrap();

    let static_issuer = &page.issuers[0];
    assert_eq!(static_issuer.source, IssuerSource::Static);
    assert_eq!(static_issuer.owner, None);
    assert_eq!(static_issuer.expires_at, None);
    assert_eq!(static_issuer.display_name, None);
    assert_eq!(page.next_cursor.as_deref(), Some("page-2"));
    let requests = app.admin_api().received_requests().await.unwrap();
    assert_eq!(
        requests[0].url.query(),
        Some("tier=sandbox&state=active&limit=100")
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
async fn an_issuer_without_an_etag_is_unexpected() {
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
