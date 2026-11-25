//! Rate limits on issuer and key changes, and on signing in.

use axum::body::Body;
use http::header::COOKIE;
use http::{Method, StatusCode};
use serde_json::json;
use status_list_portal::auth::SIGN_IN_COOKIE;
use status_list_portal::config::{RateLimit, RateLimits};
use status_list_portal::rate_limiter::LimitedAction;
use tokio::net::TcpListener;
use tokio::task::JoinSet;
use url::Url;
use wiremock::matchers::{body_partial_json, method, path};
use wiremock::{Mock, ResponseTemplate};

use crate::util::admin_api::{
    Counts, issuer_json, issuer_response, new_issuer_body, owner, sandbox_counts,
};
use crate::util::{MockAnonymousUser, MockCookieUser, RequestHelper, Response, TestApp};

/// A sign-in that did not start here.
const MADE_UP_CALLBACK: &str = "/api/v1/session/github/callback?code=code&state=state";

/// Once, then once an hour, for every action.
fn once_an_hour() -> RateLimits {
    let limit = RateLimit::new(1, 3600);
    RateLimits {
        register: limit,
        renew: limit,
        add_key: limit,
        revoke_key: limit,
        delete: limit,
        sign_in: limit,
    }
}

async fn accept_registrations(app: &TestApp, user: &MockCookieUser) {
    sandbox_counts(app.admin_api(), user.user(), Counts::default()).await;
    Mock::given(method("POST"))
        .and(path("/admin/v1/issuers"))
        .and(body_partial_json(json!({ "owner": owner(user.user()) })))
        .respond_with(issuer_response(
            201,
            "\"1\"",
            &issuer_json(user.user().gh_id),
        ))
        .mount(app.admin_api())
        .await;
}

/// Starts a sign-in and comes back from GitHub as if from `client`, forwarded by a proxy.
/// GitHub, which the tests do not stand in for here, fails to trade the code.
async fn sign_in_from(anon: &MockAnonymousUser, client: &str) -> Response {
    let started = anon.get("/api/v1/session/github").await;
    let authorize_url = Url::parse(started.header("location").unwrap()).unwrap();
    let (_, state) = authorize_url
        .query_pairs()
        .find(|(name, _)| name == "state")
        .unwrap();
    let cookie = started.cookie(SIGN_IN_COOKIE).unwrap();
    let request = anon
        .request_builder(
            Method::GET,
            &format!("/api/v1/session/github/callback?code=code&state={state}"),
        )
        .header("x-forwarded-for", client)
        .header(COOKIE, format!("{}={}", cookie.name(), cookie.value()))
        .body(Body::empty())
        .unwrap();
    anon.run(request).await
}

/// The browser went to the home page, which explains why signing in failed.
#[track_caller]
fn assert_sign_in_failed(response: &Response, reason: &str) {
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let location = format!("/?sign_in_error={reason}");
    assert_eq!(response.header("location"), Some(location.as_str()));
}

#[track_caller]
fn assert_refused(response: &Response, detail: &str) {
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(response.header("retry-after"), Some("3600"));
    let expected = format!("{detail} Please try again later.");
    assert_eq!(
        response.json(),
        json!({ "errors": [{ "code": "rate_limited", "detail": expected }] })
    );
}

#[tokio::test]
async fn registrations_are_limited_per_developer() {
    let (app, _anon) = TestApp::init()
        .with_rate_limits(once_an_hour())
        .empty()
        .await;
    let octocat = app.db_new_registrant("octocat").await;
    let hubot = app.db_new_registrant("hubot").await;
    accept_registrations(&app, &octocat).await;
    accept_registrations(&app, &hubot).await;

    let first = octocat.post("/api/v1/issuers", &new_issuer_body()).await;
    let second = octocat.post("/api/v1/issuers", &new_issuer_body()).await;
    let other = hubot.post("/api/v1/issuers", &new_issuer_body()).await;

    assert_eq!(first.status(), StatusCode::CREATED);
    assert_refused(&second, "You registered too many issuers in a short time.");
    assert_eq!(other.status(), StatusCode::CREATED);
    let metrics = app.metrics();
    let register = [("portal.rate_limit.action", "register")];
    assert_eq!(metrics.sum("portal.rate_limited", &register), 1);
}

#[tokio::test]
async fn each_action_has_its_own_limit() {
    let (app, _anon) = TestApp::init()
        .with_rate_limits(once_an_hour())
        .empty()
        .await;
    let user = app.db_new_registrant("octocat").await;
    Mock::given(method("GET"))
        .and(path("/admin/v1/issuers/unknown"))
        .respond_with(
            ResponseTemplate::new(404).set_body_json(json!({ "error": "issuer_not_found" })),
        )
        .mount(app.admin_api())
        .await;

    let renewed = user.post("/api/v1/issuers/unknown/renew", &json!({})).await;
    let renewed_again = user.post("/api/v1/issuers/unknown/renew", &json!({})).await;
    let deleted = user.delete("/api/v1/issuers/unknown").await;
    let deleted_again = user.delete("/api/v1/issuers/unknown").await;

    // A refused or failed request uses up a token too.
    assert_eq!(renewed.status(), StatusCode::NOT_FOUND);
    assert_refused(
        &renewed_again,
        "You renewed issuers too often in a short time.",
    );
    assert_eq!(deleted.status(), StatusCode::NOT_FOUND);
    assert_refused(
        &deleted_again,
        "You deleted too many issuers in a short time.",
    );
}

#[tokio::test]
async fn sign_ins_are_limited_per_client_network() {
    let (_app, anon) = TestApp::init()
        .with_rate_limits(once_an_hour())
        .behind_a_proxy()
        .empty()
        .await;

    let first = sign_in_from(&anon, "203.0.113.7").await;
    let second = sign_in_from(&anon, "203.0.113.7").await;
    let other = sign_in_from(&anon, "198.51.100.4").await;

    assert_sign_in_failed(&first, "unavailable");
    assert_sign_in_failed(&second, "too_many");
    assert_sign_in_failed(&other, "unavailable");
}

#[tokio::test]
async fn sign_ins_that_did_not_start_here_use_no_token() {
    let (_app, anon) = TestApp::init()
        .with_rate_limits(once_an_hour())
        .empty()
        .await;

    for _ in 0..3 {
        let made_up = anon.get(MADE_UP_CALLBACK).await;
        assert_sign_in_failed(&made_up, "expired");
    }
    let started = sign_in_from(&anon, "203.0.113.7").await;

    assert_sign_in_failed(&started, "unavailable");
}

#[tokio::test]
async fn clients_name_themselves_only_through_trusted_proxies() {
    let (app, anon) = TestApp::init()
        .with_rate_limits(once_an_hour())
        .empty()
        .await;

    let first = sign_in_from(&anon, "203.0.113.7").await;
    let made_up = sign_in_from(&anon, "198.51.100.4").await;

    assert_sign_in_failed(&first, "unavailable");
    assert_sign_in_failed(&made_up, "too_many");
    let sign_in = [("portal.rate_limit.action", "sign_in")];
    assert_eq!(app.metrics().sum("portal.rate_limited", &sign_in), 1);
}

#[tokio::test]
async fn concurrent_requests_take_turns() {
    let limits = RateLimits {
        register: RateLimit::new(3, 3600),
        ..once_an_hour()
    };
    let (app, _anon) = TestApp::init().with_rate_limits(limits).empty().await;
    let user_id = app.db_new_user("octocat").await.user().id;
    // Connections of their own, opened beforehand: the pool holds two, and its connect
    // timeout could fail the test while other tests keep the database busy.
    let mut conns = Vec::new();
    for _ in 0..6 {
        conns.push(app.db_conn().await);
    }

    let mut checks = JoinSet::new();
    for mut conn in conns {
        let ctx = app.ctx().clone();
        checks.spawn(async move {
            let check = ctx
                .rate_limiter
                .check(&mut conn, user_id, LimitedAction::Register);
            check.await.is_ok()
        });
    }
    let allowed = checks.join_all().await.into_iter().filter(|ok| *ok).count();

    assert_eq!(allowed, 3);
}

#[tokio::test]
async fn adding_and_revoking_keys_have_independent_buckets() {
    let (app, _anon) = TestApp::init()
        .with_rate_limits(once_an_hour())
        .empty()
        .await;
    let user_id = app.db_new_user("octocat").await.user().id;
    let mut conn = app.db_conn().await;
    let limiter = &app.ctx().rate_limiter;
    assert!(
        limiter
            .check(&mut conn, user_id, LimitedAction::AddKey)
            .await
            .is_ok()
    );
    assert!(
        limiter
            .check(&mut conn, user_id, LimitedAction::AddKey)
            .await
            .is_err()
    );
    assert!(
        limiter
            .check(&mut conn, user_id, LimitedAction::RevokeKey)
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn the_server_knows_each_peer() {
    let (app, _anon) = TestApp::init().empty().await;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(status_list_portal::serve(
        listener,
        app.ctx().clone(),
        std::future::pending(),
    ));

    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let response = client
        .get(format!("http://{address}{MADE_UP_CALLBACK}"))
        .send()
        .await
        .unwrap();
    server.abort();

    // Without the peer's address, the server could not tell sign-ins apart, and would fail.
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(response.headers()["location"], "/?sign_in_error=expired");
}
