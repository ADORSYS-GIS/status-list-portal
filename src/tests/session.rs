use std::collections::HashMap;

use axum::body::Body;
use axum_extra::extract::cookie::SameSite;
use diesel::QueryDsl;
use diesel_async::RunQueryDsl;
use http::Method;
use http::header::COOKIE;
use insta::assert_snapshot;
use oauth2::{PkceCodeChallenge, PkceCodeVerifier};
use serde_json::{Value, json};
use status_list_portal::auth::{SESSION_COOKIE, SIGN_IN_COOKIE};
use status_list_portal::schema::users;
use url::Url;
use wiremock::matchers::{body_string_contains, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use crate::util::{MockAnonymousUser, RequestHelper, Response, TestApp};

const CODE: &str = "the-code";
const TOKEN: &str = "gho_the-token";
const CALLBACK_URL: &str = "https://portal.example.com/api/v1/session/github/callback";

/// A sign-in that went to GitHub: what the browser holds when it comes back.
struct SignInAtGitHub {
    authorize_url: Url,
    state: String,
    cookie: String,
}

async fn start_sign_in(anon: &MockAnonymousUser) -> SignInAtGitHub {
    let response = anon.get("/api/v1/session/github").await;
    let authorize_url = Url::parse(response.header("location").expect("no redirect")).unwrap();
    let cookie = response.cookie(SIGN_IN_COOKIE).expect("no sign-in cookie");
    SignInAtGitHub {
        state: query_param(&authorize_url, "state"),
        authorize_url,
        cookie: format!("{}={}", cookie.name(), cookie.value()),
    }
}

/// GitHub sends the browser back with this query.
async fn come_back(anon: &MockAnonymousUser, cookie: &str, query: &str) -> Response {
    let request = anon
        .request_builder(
            Method::GET,
            &format!("/api/v1/session/github/callback?{query}"),
        )
        .header(COOKIE, cookie)
        .body(Body::empty())
        .unwrap();
    anon.run(request).await
}

async fn sign_in(anon: &MockAnonymousUser) -> Response {
    let sign_in = start_sign_in(anon).await;
    let query = format!("code={CODE}&state={}", sign_in.state);
    come_back(anon, &sign_in.cookie, &query).await
}

async fn get_with_cookie(anon: &MockAnonymousUser, path: &str, cookie: &str) -> Response {
    let request = anon
        .request_builder(Method::GET, path)
        .header(COOKIE, cookie)
        .body(Body::empty())
        .unwrap();
    anon.run(request).await
}

fn query_param(url: &Url, name: &str) -> String {
    url.query_pairs()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.into_owned())
        .unwrap_or_else(|| panic!("no {name} in {url}"))
}

/// GitHub trades the code for a token and reports octocat, with these email addresses.
async fn github_approves(github: &MockServer, emails: Value) {
    let bearer = format!("Bearer {TOKEN}");
    Mock::given(method("POST"))
        .and(path("/login/oauth/access_token"))
        .and(body_string_contains(format!("code={CODE}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "access_token": TOKEN,
            "token_type": "bearer",
            "scope": "user:email",
        })))
        .mount(github)
        .await;
    Mock::given(method("GET"))
        .and(path("/user"))
        .and(header("authorization", bearer.as_str()))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": 583231,
            "login": "octocat",
            "name": "The Octocat",
        })))
        .mount(github)
        .await;
    Mock::given(method("GET"))
        .and(path("/user/emails"))
        .and(header("authorization", bearer.as_str()))
        .respond_with(ResponseTemplate::new(200).set_body_json(emails))
        .mount(github)
        .await;
}

fn verified_primary_email() -> Value {
    json!([
        { "email": "octo@example.org", "primary": false, "verified": true },
        { "email": "octocat@example.com", "primary": true, "verified": true },
    ])
}

#[tokio::test]
async fn signing_in_sends_the_browser_to_github() {
    let (_app, anon) = TestApp::init().empty().await;

    let response = anon.get("/api/v1/session/github").await;

    assert_snapshot!(response.status(), @"303 See Other");
    let location = Url::parse(response.header("location").unwrap()).unwrap();
    assert_eq!(location.path(), "/login/oauth/authorize");
    assert_eq!(query_param(&location, "client_id"), "test-client-id");
    assert_eq!(query_param(&location, "redirect_uri"), CALLBACK_URL);
    assert_eq!(query_param(&location, "scope"), "user:email");
    assert_eq!(query_param(&location, "code_challenge_method"), "S256");
    let cookie = response.cookie(SIGN_IN_COOKIE).unwrap();
    assert_eq!(cookie.secure(), Some(true));
    assert_eq!(cookie.http_only(), Some(true));
    assert_eq!(cookie.same_site(), Some(SameSite::Lax));
    assert_eq!(cookie.path(), Some("/"));
}

#[tokio::test]
async fn coming_back_from_github_signs_the_user_in() {
    let (app, anon) = TestApp::init().empty().await;
    github_approves(app.github(), verified_primary_email()).await;

    let response = sign_in(&anon).await;

    assert_snapshot!(response.status(), @"303 See Other");
    assert_eq!(response.header("location"), Some("/"));
    assert_eq!(response.cookie(SIGN_IN_COOKIE).unwrap().value(), "");
    let session = response.cookie(SESSION_COOKIE).expect("no session cookie");
    assert_eq!(session.secure(), Some(true));
    assert_eq!(session.http_only(), Some(true));
    assert_eq!(session.same_site(), Some(SameSite::Lax));

    let cookie = format!("{}={}", session.name(), session.value());
    let me = get_with_cookie(&anon, "/api/v1/me", &cookie).await;
    assert_eq!(
        me.json(),
        json!({ "login": "octocat", "name": "The Octocat", "email": "octocat@example.com" })
    );
}

#[tokio::test]
async fn the_code_is_traded_with_the_pkce_verifier() {
    let (app, anon) = TestApp::init().empty().await;
    github_approves(app.github(), verified_primary_email()).await;
    let sign_in = start_sign_in(&anon).await;

    let query = format!("code={CODE}&state={}", sign_in.state);
    come_back(&anon, &sign_in.cookie, &query).await;

    let requests = app.github().received_requests().await.unwrap();
    let token_request = requests
        .iter()
        .find(|request| request.url.path() == "/login/oauth/access_token")
        .expect("no token request");
    let form: HashMap<String, String> = url::form_urlencoded::parse(&token_request.body)
        .into_owned()
        .collect();
    let verifier = PkceCodeVerifier::new(form["code_verifier"].clone());
    assert_eq!(
        PkceCodeChallenge::from_code_verifier_sha256(&verifier).as_str(),
        query_param(&sign_in.authorize_url, "code_challenge")
    );
    assert_eq!(form["client_id"], "test-client-id");
    assert_eq!(form["client_secret"], "test-client-secret");
    assert_eq!(form["redirect_uri"], CALLBACK_URL);
}

#[tokio::test]
async fn signing_in_needs_a_verified_primary_email() {
    let (app, anon) = TestApp::init().empty().await;
    let unverified =
        json!([{ "email": "octocat@example.com", "primary": true, "verified": false }]);
    github_approves(app.github(), unverified).await;

    let response = sign_in(&anon).await;

    assert_snapshot!(response.status(), @"403 Forbidden");
    let detail = response.json()["errors"][0]["detail"].to_string();
    assert!(
        detail.contains("https://github.com/settings/emails"),
        "{detail}"
    );
    assert!(response.cookie(SESSION_COOKIE).is_none());
    let mut conn = app.db_conn().await;
    let accounts: i64 = users::table.count().get_result(&mut conn).await.unwrap();
    assert_eq!(accounts, 0);
}

#[tokio::test]
async fn coming_back_with_another_state_is_refused() {
    let (app, anon) = TestApp::init().empty().await;
    github_approves(app.github(), verified_primary_email()).await;
    let sign_in = start_sign_in(&anon).await;

    let query = format!("code={CODE}&state=forged");
    let response = come_back(&anon, &sign_in.cookie, &query).await;

    assert_snapshot!(response.status(), @"400 Bad Request");
    assert!(response.cookie(SESSION_COOKIE).is_none());
}

#[tokio::test]
async fn coming_back_without_the_sign_in_cookie_is_refused() {
    let (app, anon) = TestApp::init().empty().await;
    github_approves(app.github(), verified_primary_email()).await;
    let sign_in = start_sign_in(&anon).await;

    let path = format!(
        "/api/v1/session/github/callback?code={CODE}&state={}",
        sign_in.state
    );
    let response = anon.get(&path).await;

    assert_snapshot!(response.status(), @"400 Bad Request");
    assert!(response.cookie(SESSION_COOKIE).is_none());
}

#[tokio::test]
async fn declining_on_github_signs_nobody_in() {
    let (_app, anon) = TestApp::init().empty().await;
    let sign_in = start_sign_in(&anon).await;

    let query = format!("error=access_denied&state={}", sign_in.state);
    let response = come_back(&anon, &sign_in.cookie, &query).await;

    assert_snapshot!(response.status(), @"400 Bad Request");
    assert_eq!(
        response.json(),
        json!({ "errors": [{ "detail": "This sign-in cannot be completed. Please sign in again." }] })
    );
}

#[tokio::test]
async fn a_code_github_rejects_signs_nobody_in() {
    let (app, anon) = TestApp::init().empty().await;
    Mock::given(method("POST"))
        .and(path("/login/oauth/access_token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "error": "bad_verification_code",
            "error_description": "The code passed is incorrect or expired.",
        })))
        .mount(app.github())
        .await;

    let response = sign_in(&anon).await;

    assert_snapshot!(response.status(), @"400 Bad Request");
    assert!(response.cookie(SESSION_COOKIE).is_none());
}

#[tokio::test]
async fn github_failing_is_a_bad_gateway() {
    let (app, anon) = TestApp::init().empty().await;
    Mock::given(method("POST"))
        .and(path("/login/oauth/access_token"))
        .respond_with(ResponseTemplate::new(503))
        .mount(app.github())
        .await;

    let response = sign_in(&anon).await;

    assert_snapshot!(response.status(), @"502 Bad Gateway");
    assert_eq!(
        response.json(),
        json!({ "errors": [{ "detail": "GitHub could not be reached. Please try again later." }] })
    );
}

#[tokio::test]
async fn signing_out_ends_every_session() {
    let (app, anon) = TestApp::init().empty().await;
    let user = app.db_new_user("octocat").await;
    let other_browser = app.session_cookie(user.user());

    let response = user.delete("/api/v1/session").await;

    assert_snapshot!(response.status(), @"204 No Content");
    assert_eq!(response.cookie(SESSION_COOKIE).unwrap().value(), "");
    assert_snapshot!(user.get("/api/v1/me").await.status(), @"401 Unauthorized");
    let elsewhere = get_with_cookie(&anon, "/api/v1/me", &other_browser).await;
    assert_snapshot!(elsewhere.status(), @"401 Unauthorized");
}

#[tokio::test]
async fn signing_out_without_a_session_succeeds() {
    let (_app, anon) = TestApp::init().empty().await;

    let response = anon.delete("/api/v1/session").await;

    assert_snapshot!(response.status(), @"204 No Content");
}
