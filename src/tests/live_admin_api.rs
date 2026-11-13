//! The admin API client and the issuer routes against a real Status List Server, rather than
//! the stand-in the other tests use, so that a change in the server's contract fails here.
//!
//! The tests run when `LIVE_ADMIN_API_URL` names the server's admin listener; they pass
//! without checking anything otherwise. `scripts/live-status-list-server.sh` starts a server
//! for them. Every test registers issuers with keys and owners of its own, so the tests can
//! run again against the same server.

use std::sync::atomic::{AtomicI64, Ordering};

use aws_lc_rs::signature::{ECDSA_P256_SHA256_FIXED_SIGNING, EcdsaKeyPair, KeyPair};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use http::StatusCode;
use serde_json::{Value, json};
use status_list_portal::admin_api::{
    AdminApi, AdminApiError, ErrorCode, IssuerChange, IssuerQuery, IssuerSource, IssuerState,
    IssuerTier, KeyState, NewIssuer, Owner, PublicJwk, RequestedState, SigningAlgorithm,
};
use status_list_portal::config;
use time::format_description::well_known::Rfc3339;
use time::{Duration, OffsetDateTime};
use url::Url;

use crate::util::{
    ADMIN_AUDIENCE, ADMIN_KEY_ID, ADMIN_PRINCIPAL, ADMIN_SIGNING_KEY_FILE, RequestHelper, TestApp,
};

/// GitHub account ids no earlier run used: they start from the time the tests started.
static NEXT_GH_ID: AtomicI64 = AtomicI64::new(0);

/// The real server's admin listener, or `None` when the tests are not to run.
fn live_url() -> Option<Url> {
    let Some(url) = std::env::var("LIVE_ADMIN_API_URL")
        .ok()
        .filter(|url| !url.is_empty())
    else {
        eprintln!("LIVE_ADMIN_API_URL is not set: skipping the test against a real server");
        return None;
    };
    Some(Url::parse(&url).expect("LIVE_ADMIN_API_URL is not a URL"))
}

/// A client that signs in as the portal's admin principal, with tokens for `audience`.
fn client(url: &Url, audience: &str) -> AdminApi {
    AdminApi::new(&config::AdminApi {
        url: url.clone(),
        principal: ADMIN_PRINCIPAL.to_string(),
        audience: audience.to_string(),
        signing_key_file: ADMIN_SIGNING_KEY_FILE.into(),
        key_id: ADMIN_KEY_ID.to_string(),
        algorithm: SigningAlgorithm::Es256,
    })
    .expect("failed to build the admin API client")
}

/// A GitHub account that owns no issuers yet. The ids count up from the time the tests
/// started, in microseconds, so runs against the same server never share an owner.
fn new_gh_id() -> i64 {
    let start = i64::try_from(OffsetDateTime::now_utc().unix_timestamp_nanos() / 1000)
        .expect("the time fits in microseconds");
    let _ = NEXT_GH_ID.compare_exchange(0, start, Ordering::Relaxed, Ordering::Relaxed);
    NEXT_GH_ID.fetch_add(1, Ordering::Relaxed)
}

/// A P-256 public key that no issuer was registered with.
fn new_public_key() -> PublicJwk {
    let key =
        EcdsaKeyPair::generate(&ECDSA_P256_SHA256_FIXED_SIGNING).expect("failed to generate a key");
    // An uncompressed point: 0x04, then x, then y.
    let point = key.public_key().as_ref();
    assert_eq!(point.len(), 65);
    serde_json::from_value(json!({
        "kty": "EC",
        "crv": "P-256",
        "x": URL_SAFE_NO_PAD.encode(&point[1..33]),
        "y": URL_SAFE_NO_PAD.encode(&point[33..]),
    }))
    .unwrap()
}

fn new_issuer() -> NewIssuer {
    NewIssuer {
        display_name: "Acme Wallet Issuer".to_string(),
        public_key: new_public_key(),
        alg: SigningAlgorithm::Es256,
        expires_at: None,
    }
}

#[track_caller]
fn assert_refused(result: Result<impl std::fmt::Debug, AdminApiError>, expected: ErrorCode) {
    match result {
        Err(error) => assert_eq!(error.code(), Some(expected), "{error:?}"),
        Ok(value) => panic!("expected {expected}, got {value:?}"),
    }
}

/// Suspends what a test registered, so that it does not hold a place in the sandbox.
async fn suspend(api: &AdminApi, owner: &Owner, id: &str) {
    let issuers = api.issuers_of(owner);
    let current = issuers.get(id).await.unwrap().expect("the issuer is gone");
    issuers
        .update(&current, &IssuerChange::State(RequestedState::Suspended))
        .await
        .unwrap();
}

#[tokio::test]
async fn created_issuers_are_sandbox_issuers_of_their_owner() {
    let Some(url) = live_url() else { return };
    let api = client(&url, ADMIN_AUDIENCE);
    let owner = Owner::github(new_gh_id());
    let new_issuer = new_issuer();

    let issuer = api.issuers_of(&owner).create(&new_issuer).await.unwrap();

    assert_eq!(issuer.display_name.as_deref(), Some("Acme Wallet Issuer"));
    assert_eq!(issuer.source, IssuerSource::Portal);
    assert_eq!(issuer.tier, IssuerTier::Sandbox);
    assert_eq!(issuer.state, IssuerState::Active);
    assert_eq!(issuer.owner.as_ref(), Some(&owner));
    assert!(issuer.created_at.is_some());
    let expires_at = issuer.expires_at.expect("a sandbox issuer expires");
    let expected = OffsetDateTime::now_utc() + Duration::days(30);
    assert!(
        (expected - expires_at).abs() < Duration::minutes(1),
        "{expires_at}"
    );
    let [key] = issuer.keys.as_slice() else {
        panic!("expected one key, got {:?}", issuer.keys);
    };
    assert!(!key.kid.is_empty());
    assert_eq!(key.alg, SigningAlgorithm::Es256);
    assert_eq!(key.state, KeyState::Active);
    assert_eq!(key.public_key, new_issuer.public_key);
    assert_eq!(issuer.usage.lists, 0);
    assert_eq!(issuer.usage.keys, 1);

    suspend(&api, &owner, &issuer.id).await;
}

#[tokio::test]
async fn an_issuer_is_read_by_its_owner_only() {
    let Some(url) = live_url() else { return };
    let api = client(&url, ADMIN_AUDIENCE);
    let owner = Owner::github(new_gh_id());
    let someone_else = Owner::github(new_gh_id());
    let created = api.issuers_of(&owner).create(&new_issuer()).await.unwrap();

    let read = api.issuers_of(&owner).get(&created.id).await.unwrap();
    let theirs = api
        .issuers_of(&someone_else)
        .get(&created.id)
        .await
        .unwrap();
    let unknown = api
        .issuers_of(&owner)
        .get("00000000-0000-4000-8000-000000000000")
        .await
        .unwrap();

    let read = read.expect("the owner cannot read the issuer");
    assert_eq!(read.into_issuer(), created);
    assert_eq!(theirs, None);
    assert_eq!(unknown, None);

    suspend(&api, &owner, &created.id).await;
}

#[tokio::test]
async fn issuers_are_listed_by_owner_state_tier_and_source() {
    let Some(url) = live_url() else { return };
    let api = client(&url, ADMIN_AUDIENCE);
    let owner = Owner::github(new_gh_id());
    let issuers = api.issuers_of(&owner);
    let mut ids = Vec::new();
    for _ in 0..3 {
        ids.push(issuers.create(&new_issuer()).await.unwrap().id);
    }
    suspend(&api, &owner, &ids[2]).await;
    let query = |state| IssuerQuery {
        owner: Some(owner.clone()),
        state,
        tier: Some(IssuerTier::Sandbox),
        source: Some(IssuerSource::Portal),
        ..IssuerQuery::default()
    };

    let all = api.first_issuers(&query(None), 100).await.unwrap();
    let active = api
        .count_issuers(&query(Some(IssuerState::Active)), 100)
        .await;
    let suspended = api
        .count_issuers(&query(Some(IssuerState::Suspended)), 100)
        .await;
    // Pages of two, of which the first is enough.
    let at_least_two = api.count_issuers(&query(None), 2).await;
    let page = issuers.list(None).await.unwrap();

    let mut listed: Vec<_> = all.iter().map(|issuer| issuer.id.clone()).collect();
    listed.sort();
    ids.sort();
    assert_eq!(listed, ids);
    assert!(
        all.iter()
            .all(|issuer| issuer.owner.as_ref() == Some(&owner))
    );
    assert_eq!(active.unwrap(), 2);
    assert_eq!(suspended.unwrap(), 1);
    assert_eq!(at_least_two.unwrap(), 2);
    assert_eq!(page.issuers.len(), 3);
    assert_eq!(page.next_cursor, None);

    for id in &ids {
        let current = issuers.get(id).await.unwrap().unwrap();
        if current.state == IssuerState::Active {
            suspend(&api, &owner, id).await;
        }
    }
}

#[tokio::test]
async fn changes_apply_to_the_version_they_were_read_at() {
    let Some(url) = live_url() else { return };
    let api = client(&url, ADMIN_AUDIENCE);
    let owner = Owner::github(new_gh_id());
    let issuers = api.issuers_of(&owner);
    let created = issuers.create(&new_issuer()).await.unwrap();
    let read = issuers.get(&created.id).await.unwrap().unwrap();
    let renewal = OffsetDateTime::now_utc() + Duration::days(20);

    let renewed = issuers
        .update(&read, &IssuerChange::ExpiresAt(renewal))
        .await;
    let stale = issuers
        .update(&read, &IssuerChange::State(RequestedState::Suspended))
        .await;

    let renewed = renewed.unwrap().expect("the issuer is gone");
    assert_eq!(
        renewed.expires_at.map(OffsetDateTime::unix_timestamp),
        Some(renewal.unix_timestamp())
    );
    assert_refused(stale, ErrorCode::PreconditionFailed);

    let current = issuers.get(&created.id).await.unwrap().unwrap();
    let suspended = issuers
        .update(&current, &IssuerChange::State(RequestedState::Suspended))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(suspended.state, IssuerState::Suspended);
    let current = issuers.get(&created.id).await.unwrap().unwrap();
    let resumed = issuers
        .update(&current, &IssuerChange::State(RequestedState::Active))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(resumed.state, IssuerState::Active);

    suspend(&api, &owner, &created.id).await;
}

#[tokio::test]
async fn refusals_carry_the_codes_the_portal_branches_on() {
    let Some(url) = live_url() else { return };
    let api = client(&url, ADMIN_AUDIENCE);
    let owner = Owner::github(new_gh_id());
    let issuers = api.issuers_of(&owner);
    let registered = new_issuer();
    let created = issuers.create(&registered).await.unwrap();

    let same_key = issuers.create(&registered).await;
    let mut off_the_curve = new_issuer();
    let y = off_the_curve.public_key.y.as_mut().unwrap();
    // Only two points have this x: another y leaves the curve.
    let first = if y.starts_with('A') { "B" } else { "A" };
    y.replace_range(..1, first);
    let off_the_curve = issuers.create(&off_the_curve).await;
    let too_long = issuers
        .create(&NewIssuer {
            expires_at: Some(OffsetDateTime::now_utc() + Duration::days(31)),
            ..new_issuer()
        })
        .await;
    let read = issuers.get(&created.id).await.unwrap().unwrap();
    let renewed_too_long = issuers
        .update(
            &read,
            &IssuerChange::ExpiresAt(OffsetDateTime::now_utc() + Duration::days(31)),
        )
        .await;
    let wrong_audience = client(&url, "another-audience")
        .issuers_of(&owner)
        .get(&created.id)
        .await;

    assert_refused(same_key, ErrorCode::KeyMaterialInUse);
    assert_refused(off_the_curve, ErrorCode::InvalidPublicJwk);
    assert_refused(too_long, ErrorCode::InvalidExpiry);
    assert_refused(renewed_too_long, ErrorCode::InvalidExpiry);
    assert_refused(wrong_audience, ErrorCode::InvalidToken);

    suspend(&api, &owner, &created.id).await;
}

/// Checks that an issuer as the portal shows it stays active for one sandbox lifetime from
/// now, give or take the portal's margin.
#[track_caller]
fn assert_one_lifetime(issuer: &Value) {
    let expires_at = issuer["expires_at"].as_str().expect("no expires_at");
    let expires_at = OffsetDateTime::parse(expires_at, &Rfc3339).unwrap();
    let expected = OffsetDateTime::now_utc() + Duration::days(30);
    assert!(
        (expected - expires_at).abs() < Duration::minutes(10),
        "{expires_at}"
    );
}

/// What a developer sends to register an issuer with a key of its own.
fn new_issuer_body() -> Value {
    json!({
        "display_name": "Acme Wallet Issuer",
        "alg": "ES256",
        "public_key": new_public_key(),
    })
}

#[tokio::test]
async fn developers_register_renew_and_delete_issuers() {
    let Some(url) = live_url() else { return };
    let (app, _) = TestApp::init().with_admin_api_url(url).empty().await;
    let user = app.db_new_user_with_gh_id("octocat", new_gh_id()).await;
    app.accept_terms(user.user()).await;

    let registered = user.post("/api/v1/issuers", &new_issuer_body()).await;
    assert_eq!(
        registered.status(),
        StatusCode::CREATED,
        "{}",
        registered.text()
    );
    let registered = registered.json();
    let id = registered["id"].as_str().expect("no id").to_owned();
    let issuer_url = format!("/api/v1/issuers/{id}");
    assert_eq!(registered["state"], "active");
    assert_one_lifetime(&registered);

    let listed = user.get("/api/v1/issuers").await;
    assert_eq!(listed.status(), StatusCode::OK, "{}", listed.text());
    assert_eq!(listed.json()["issuers"], json!([registered]));

    let read = user.get(&issuer_url).await;
    assert_eq!(read.status(), StatusCode::OK, "{}", read.text());
    assert_eq!(read.json(), registered);

    let renewed = user.post(&format!("{issuer_url}/renew"), &json!({})).await;
    assert_eq!(renewed.status(), StatusCode::OK, "{}", renewed.text());
    assert_one_lifetime(&renewed.json());

    let deleted = user.delete(&issuer_url).await;
    assert_eq!(
        deleted.status(),
        StatusCode::NO_CONTENT,
        "{}",
        deleted.text()
    );
    let read = user.get(&issuer_url).await;
    assert_eq!(read.json()["state"], "suspended");
    let deleted_again = user.delete(&issuer_url).await;
    assert_eq!(deleted_again.status(), StatusCode::NO_CONTENT);
}
