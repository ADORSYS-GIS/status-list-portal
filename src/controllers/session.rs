//! Signing in with GitHub, and signing out.

use std::net::IpAddr;

use axum::extract::Query;
use axum::response::Redirect;
use axum_extra::extract::PrivateCookieJar;
use http::StatusCode;
use serde::Deserialize;
use tracing::{error, info, warn};
use utoipa::IntoParams;

use crate::auth::{self, SESSION_COOKIE, SIGN_IN_COOKIE, Session};
use crate::client_address::ClientAddress;
use crate::github::{Account, PendingSignIn, SignInError};
use crate::metrics::SignInOutcome;
use crate::models::{NewUser, User};
use crate::server::ServerContext;
use crate::util::errors::{self, AppResult};

/// What GitHub sends back when the user did not approve the portal.
const ACCESS_DENIED: &str = "access_denied";

/// Starts signing in: sends the browser to GitHub to approve the portal.
#[utoipa::path(
    get,
    path = "/api/v1/session/github",
    tag = "session",
    responses(
        (status = 303, description = "Redirects to GitHub.", headers(("location" = String))),
    ),
)]
pub async fn begin(ctx: ServerContext, jar: PrivateCookieJar) -> (PrivateCookieJar, Redirect) {
    let (url, pending) = ctx.github.start_sign_in();
    (
        jar.add(auth::sign_in_cookie(&pending)),
        Redirect::to(url.as_str()),
    )
}

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct CallbackQuery {
    /// Code to trade for a token, when the user approved the portal.
    code: Option<String>,
    /// The `state` the sign-in started with.
    state: Option<String>,
    /// Why GitHub did not approve, for example `access_denied`.
    error: Option<String>,
}

/// Why a sign-in failed, as the home page reads it from its `sign_in_error` parameter.
#[derive(Debug, Clone, Copy)]
enum Failure {
    /// The user did not approve the portal on GitHub.
    Denied,
    /// The sign-in expired, was already used, or did not start in this browser.
    Expired,
    NoVerifiedEmail,
    /// Too many sign-ins came from the client's network in a short time.
    TooMany,
    /// GitHub or the portal failed; signing in again later may work.
    Unavailable,
}

impl Failure {
    fn redirect(self) -> Redirect {
        let reason = match self {
            Self::Denied => "denied",
            Self::Expired => "expired",
            Self::NoVerifiedEmail => "no_verified_email",
            Self::TooMany => "too_many",
            Self::Unavailable => "unavailable",
        };
        Redirect::to(&format!("/?sign_in_error={reason}"))
    }

    fn outcome(self) -> SignInOutcome {
        match self {
            Self::Denied => SignInOutcome::Denied,
            Self::Expired => SignInOutcome::Expired,
            Self::NoVerifiedEmail => SignInOutcome::NoVerifiedEmail,
            Self::TooMany => SignInOutcome::RateLimited,
            Self::Unavailable => SignInOutcome::Failed,
        }
    }
}

/// Finishes signing in when GitHub sends the browser back, then goes to the home page.
///
/// The browser is navigating, not fetching, so failures also go to the home page, which
/// explains them.
#[utoipa::path(
    get,
    path = "/api/v1/session/github/callback",
    tag = "session",
    params(CallbackQuery),
    responses(
        (status = 303, description = "Redirects to the home page. After a failure, its \
            `sign_in_error` parameter says why: `denied`, `expired`, `no_verified_email`, \
            `too_many` or `unavailable`.", headers(("location" = String))),
    ),
)]
pub async fn callback(
    ctx: ServerContext,
    ClientAddress(client): ClientAddress,
    jar: PrivateCookieJar,
    Query(query): Query<CallbackQuery>,
) -> (PrivateCookieJar, Redirect) {
    let pending = auth::pending_sign_in(&jar)
        .filter(|pending| query.state.as_deref() == Some(pending.state.as_str()));
    let result = match &pending {
        Some(pending) => sign_in(&ctx, client, pending, query).await,
        None => Err(Failure::Expired),
    };
    ctx.metrics.sign_in(match result {
        Ok(_) => SignInOutcome::Succeeded,
        Err(failure) => failure.outcome(),
    });

    // A link that is not this sign-in's must not cancel it.
    let jar = match pending {
        Some(_) => jar.remove(auth::removal(SIGN_IN_COOKIE)),
        None => jar,
    };
    match result {
        Ok(user) => (
            jar.add(Session::new(&user).into_cookie()),
            Redirect::to("/"),
        ),
        Err(failure) => (jar, failure.redirect()),
    }
}

async fn sign_in(
    ctx: &ServerContext,
    client: IpAddr,
    pending: &PendingSignIn,
    query: CallbackQuery,
) -> Result<User, Failure> {
    match query.error {
        Some(error) if error == ACCESS_DENIED => {
            info!("a sign-in was not approved on GitHub");
            return Err(Failure::Denied);
        }
        Some(error) => {
            // Debug-formatted: the value comes from the query string, so it is escaped.
            error!(
                ?error,
                "GitHub refused to start a sign-in; check the OAuth app"
            );
            return Err(Failure::Unavailable);
        }
        None => {}
    }
    let code = query.code.ok_or(Failure::Expired)?;
    // Each sign-in calls GitHub, so a client that sent many could get the portal's OAuth app
    // throttled for everyone. Only a sign-in that started here counts: one made up does not
    // reach GitHub, and must not use up the tokens of a network that others share.
    if ctx.rate_limiter.check_sign_in(client).is_err() {
        return Err(Failure::TooMany);
    }

    let account = ctx
        .github
        .finish_sign_in(&code, pending)
        .await
        .map_err(|error| match error {
            SignInError::Rejected => {
                info!("GitHub rejected a sign-in's code");
                Failure::Expired
            }
            SignInError::NoVerifiedEmail => {
                info!("a sign-in had no verified primary email");
                Failure::NoVerifiedEmail
            }
            SignInError::Refused(error) => {
                error!(%error, "GitHub refused to finish a sign-in; check the OAuth app");
                Failure::Unavailable
            }
            error => {
                warn!(error.message = %errors::describe(&error), "sign-in failed");
                Failure::Unavailable
            }
        })?;

    let user = save(ctx, &account).await.map_err(|error| {
        error!(error.message = %format!("{error:#}"), "failed to save a signed-in account");
        Failure::Unavailable
    })?;
    info!(user.id, "signed in");
    Ok(user)
}

async fn save(ctx: &ServerContext, account: &Account) -> anyhow::Result<User> {
    let mut conn = ctx.db.get().await?;
    let user = NewUser {
        gh_id: account.id,
        gh_login: &account.login,
        name: account.name.as_deref(),
        email: &account.email,
    }
    .insert_or_update(&mut conn)
    .await?;
    Ok(user)
}

/// Signs out of every browser at once.
#[utoipa::path(
    delete,
    path = "/api/v1/session",
    tag = "session",
    responses(
        (status = 204, description = "Signed out, or was not signed in."),
    ),
)]
pub async fn sign_out(
    ctx: ServerContext,
    jar: PrivateCookieJar,
) -> AppResult<(PrivateCookieJar, StatusCode)> {
    if let Some(session) = Session::from_jar(&jar) {
        let mut conn = ctx.db.get().await?;
        User::end_sessions(&mut conn, session.user_id, session.epoch).await?;
    }
    Ok((
        jar.remove(auth::removal(SESSION_COOKIE)),
        StatusCode::NO_CONTENT,
    ))
}
