//! Issuers as the mock admin API returns them.

use serde_json::{Value, json};
use status_list_portal::models::User;
use time::{Duration, OffsetDateTime};
use wiremock::matchers::{method, path, query_param, query_param_is_missing};
use wiremock::{Mock, MockServer, ResponseTemplate};

pub const ISSUER_ID: &str = "6f1c2a9e-0b7d-4c1e-9a53-2f8d7e4b1c00";
pub const CREATED_AT: i64 = 1_790_000_000;
pub const EXPIRES_AT: i64 = 1_792_592_000;

/// The public key of the test issuers, from `test_data/admin-keys/es256.pem`.
pub fn public_jwk_json() -> Value {
    json!({
        "kty": "EC",
        "crv": "P-256",
        "x": "GtkCNIXEFHQ8j_gwyoS0H9eZQGCI_Iaqm2wUymJvOfQ",
        "y": "D9yk2oGE-yZjX50UqD1j68vpRbEhM-WyuhAWrEPAcyI",
    })
}

/// An active sandbox issuer that the GitHub account `gh_id` registered through the portal.
pub fn issuer_json(gh_id: i64) -> Value {
    json!({
        "id": ISSUER_ID,
        "aggregation_id": "0b8ef1c4-3c6a-4d2e-9f1a-5b7c9d2e4f60",
        "display_name": "Acme Wallet Issuer",
        "source": "portal",
        "tier": "sandbox",
        "state": "active",
        "owner": format!("github|{gh_id}"),
        "created_at": CREATED_AT,
        "expires_at": EXPIRES_AT,
        "keys": [{
            "kid": "QoRTRSkzlInq0eOmPqJwmZ5OS9pxkN_SIf5KgZP2F8M",
            "alg": "ES256",
            "state": "active",
            "not_before": null,
            "not_after": null,
            "created_at": CREATED_AT,
            "public_key": public_jwk_json(),
        }],
        "usage": { "lists": 3, "lists_limit": 10, "keys": 1, "keys_limit": null },
    })
}

/// An issuer from the server's registry file, which has no owner and never expires.
pub fn static_issuer_json() -> Value {
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
        "usage": { "lists": 0, "lists_limit": 1000, "keys": 0, "keys_limit": null },
    })
}

pub fn issuer_path(issuer: &Value) -> String {
    let id = issuer["id"].as_str().expect("the issuer has no id");
    format!("/admin/v1/issuers/{id}")
}

pub fn issuer_response(status: u16, version: &str, issuer: &Value) -> ResponseTemplate {
    ResponseTemplate::new(status)
        .insert_header("issuer-version", version)
        .set_body_json(issuer)
}

/// One page of issuers, the last one.
pub fn page_response(issuers: &[Value]) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({ "issuers": issuers, "next_cursor": null }))
}

/// The admin API holds `issuer` at version `7`.
pub async fn admin_api_holds(admin_api: &MockServer, issuer: &Value) {
    Mock::given(method("GET"))
        .and(path(issuer_path(issuer)))
        .respond_with(issuer_response(200, "7", issuer))
        .mount(admin_api)
        .await;
}

/// The admin API's owner for `user`'s issuers.
pub fn owner(user: &User) -> String {
    format!("github|{}", user.gh_id)
}

/// What a developer sends to register an issuer, with a key member the portal drops.
pub fn new_issuer_body() -> Value {
    let mut public_key = public_jwk_json();
    public_key["kid"] = json!("my-key");
    json!({ "display_name": " Acme Wallet Issuer ", "alg": "ES256", "public_key": public_key })
}

/// Sandbox issuers as the admin API counts them.
#[derive(Default)]
pub struct Counts {
    /// The user's active issuers.
    pub active: usize,
    /// The user's issuers in any state: the active ones, then deleted ones.
    pub registered: usize,
    /// Active issuers of every account.
    pub in_sandbox: usize,
}

pub async fn sandbox_counts(admin_api: &MockServer, user: &User, counts: Counts) {
    let own = |count| page_response(&vec![issuer_json(user.gh_id); count]);
    let mut deleted = issuer_json(user.gh_id);
    deleted["state"] = json!("suspended");
    let mut registered = vec![issuer_json(user.gh_id); counts.active];
    registered.resize(counts.registered, deleted);
    Mock::given(method("GET"))
        .and(path("/admin/v1/issuers"))
        .and(query_param("owner", owner(user)))
        .and(query_param("state", "active"))
        .and(query_param("tier", "sandbox"))
        .and(query_param("source", "portal"))
        .respond_with(own(counts.active))
        .mount(admin_api)
        .await;
    Mock::given(method("GET"))
        .and(path("/admin/v1/issuers"))
        .and(query_param("owner", owner(user)))
        .and(query_param_is_missing("state"))
        .and(query_param("tier", "sandbox"))
        .and(query_param("source", "portal"))
        .respond_with(page_response(&registered))
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
pub fn assert_one_lifetime(body: &Value) {
    let expires_at = body["expires_at"].as_i64().expect("no expires_at");
    let expires_at = OffsetDateTime::from_unix_timestamp(expires_at).unwrap();
    let expected = OffsetDateTime::now_utc() + Duration::days(30) - Duration::minutes(5);
    assert!(
        (expected - expires_at).abs() < Duration::minutes(1),
        "{expires_at}"
    );
}
