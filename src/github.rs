//! Signing in with GitHub: the OAuth web flow, with PKCE, and the account it yields.
//!
//! The portal asks only for the `user:email` scope and drops GitHub's token once it has read
//! the account.

use std::time::Duration;

use anyhow::Context;
use http::header::ACCEPT;
use oauth2::basic::{BasicClient, BasicErrorResponse};
use oauth2::{
    AccessToken, AuthType, AuthUrl, AuthorizationCode, ClientId, ClientSecret, CsrfToken,
    EndpointNotSet, EndpointSet, HttpClientError, PkceCodeChallenge, PkceCodeVerifier, RedirectUrl,
    RequestTokenError, Scope, TokenResponse, TokenUrl,
};
use secrecy::ExposeSecret;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use url::Url;

use crate::config::{self, PublicOrigin};

/// Where GitHub sends the browser back. The OAuth app's callback URL must point here.
pub const CALLBACK_PATH: &str = "/api/v1/session/github/callback";

/// Read access to the account's email addresses. The profile is public.
const SCOPE: &str = "user:email";
const API_VERSION: &str = "2022-11-28";
/// A sign-in makes two rounds of calls, which must fit in the server's request timeout.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
/// What GitHub answers a code with when it is wrong, expired or already used. Any other
/// OAuth error points at the OAuth app's settings.
const BAD_CODE: &str = "bad_verification_code";

/// Client of GitHub's authorization and token endpoints.
type OAuthClient =
    BasicClient<EndpointSet, EndpointNotSet, EndpointNotSet, EndpointNotSet, EndpointSet>;
type TokenError = RequestTokenError<HttpClientError<reqwest::Error>, BasicErrorResponse>;

/// Client of GitHub's OAuth endpoints and REST API.
pub struct GitHub {
    oauth: OAuthClient,
    http: reqwest::Client,
    user_url: Url,
    emails_url: Url,
}

/// What the browser keeps, in a cookie, while it is away at GitHub.
#[derive(Debug, Serialize, Deserialize)]
pub struct PendingSignIn {
    /// GitHub echoes it back, which proves the callback belongs to a sign-in this browser
    /// started.
    pub state: String,
    /// GitHub only trades the code for a token when shown this secret (PKCE).
    pub code_verifier: String,
}

/// A GitHub account, as GitHub reports it at sign-in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Account {
    pub id: i64,
    pub login: String,
    pub name: Option<String>,
    /// Primary email, which GitHub has verified.
    pub email: String,
}

#[derive(Debug, thiserror::Error)]
pub enum SignInError {
    /// GitHub refused the code, for example because it expired or was already used.
    #[error("GitHub rejected the authorization code")]
    Rejected,
    /// GitHub refused the OAuth app itself, such as its client secret or callback URL.
    #[error("GitHub refused the OAuth app: {0}")]
    Refused(String),
    #[error("GitHub did not issue a token")]
    Token(#[source] TokenError),
    #[error("GitHub request failed")]
    Request(#[from] reqwest::Error),
    /// The account has no verified primary email, which signing in needs.
    #[error("the GitHub account has no verified primary email")]
    NoVerifiedEmail,
}

#[derive(Deserialize)]
struct UserResponse {
    id: i64,
    login: String,
    name: Option<String>,
}

#[derive(Deserialize)]
struct EmailResponse {
    email: String,
    primary: bool,
    verified: bool,
}

impl GitHub {
    pub fn new(config: &config::GitHub, origin: &PublicOrigin) -> anyhow::Result<Self> {
        let oauth = BasicClient::new(ClientId::new(config.client_id.clone()))
            .set_client_secret(ClientSecret::new(
                config.client_secret.expose_secret().to_owned(),
            ))
            // GitHub documents the client credentials as form fields.
            .set_auth_type(AuthType::RequestBody)
            .set_auth_uri(AuthUrl::from_url(
                config.web_url.join("login/oauth/authorize")?,
            ))
            .set_token_uri(TokenUrl::from_url(
                config.web_url.join("login/oauth/access_token")?,
            ))
            .set_redirect_uri(RedirectUrl::new(origin.url_for(CALLBACK_PATH))?);
        let http = reqwest::Client::builder()
            .user_agent(env!("CARGO_PKG_NAME"))
            .timeout(REQUEST_TIMEOUT)
            // A redirect could carry the code or the token somewhere else.
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .context("failed to build the GitHub client")?;
        Ok(Self {
            oauth,
            http,
            user_url: config.api_url.join("user")?,
            emails_url: config.api_url.join("user/emails")?,
        })
    }

    /// Starts a sign-in: where to send the browser, and what it must keep until it returns.
    pub fn start_sign_in(&self) -> (Url, PendingSignIn) {
        let (challenge, verifier) = PkceCodeChallenge::new_random_sha256();
        let (url, state) = self
            .oauth
            .authorize_url(CsrfToken::new_random)
            .add_scope(Scope::new(SCOPE.to_string()))
            .set_pkce_challenge(challenge)
            .url();
        let pending = PendingSignIn {
            state: state.into_secret(),
            code_verifier: verifier.into_secret(),
        };
        (url, pending)
    }

    /// Finishes a sign-in: trades the code GitHub sent back for the account that signed in.
    pub async fn finish_sign_in(
        &self,
        code: &str,
        pending: &PendingSignIn,
    ) -> Result<Account, SignInError> {
        let token = self.exchange_code(code, pending).await?;
        let (user, emails) = tokio::try_join!(
            self.get::<UserResponse>(&self.user_url, &token),
            self.get::<Vec<EmailResponse>>(&self.emails_url, &token),
        )?;
        let email = emails
            .into_iter()
            .find(|email| email.primary && email.verified)
            .map(|email| email.email)
            .ok_or(SignInError::NoVerifiedEmail)?;
        Ok(Account {
            id: user.id,
            login: user.login,
            name: user.name,
            email,
        })
    }

    async fn exchange_code(
        &self,
        code: &str,
        pending: &PendingSignIn,
    ) -> Result<AccessToken, SignInError> {
        let result = self
            .oauth
            .exchange_code(AuthorizationCode::new(code.to_owned()))
            .set_pkce_verifier(PkceCodeVerifier::new(pending.code_verifier.clone()))
            .request_async(&self.http)
            .await;
        match result {
            Ok(response) => Ok(response.access_token().clone()),
            Err(RequestTokenError::ServerResponse(response)) => Err(refusal(&response)),
            // GitHub refuses a code with `200 OK` and an OAuth error, which is no token.
            Err(RequestTokenError::Parse(error, body)) => {
                match serde_json::from_slice::<BasicErrorResponse>(&body) {
                    Ok(response) => Err(refusal(&response)),
                    Err(_) => Err(SignInError::Token(RequestTokenError::Parse(error, body))),
                }
            }
            Err(error) => Err(SignInError::Token(error)),
        }
    }

    async fn get<T: DeserializeOwned>(
        &self,
        url: &Url,
        token: &AccessToken,
    ) -> Result<T, reqwest::Error> {
        self.http
            .get(url.clone())
            .bearer_auth(token.secret())
            .header(ACCEPT, "application/vnd.github+json")
            .header("x-github-api-version", API_VERSION)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await
    }
}

fn refusal(response: &BasicErrorResponse) -> SignInError {
    let error = response.error().to_string();
    if error == BAD_CODE {
        SignInError::Rejected
    } else {
        SignInError::Refused(error)
    }
}
