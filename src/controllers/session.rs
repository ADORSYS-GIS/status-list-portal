//! Signing in with GitHub, and signing out.

use axum::extract::Query;
use axum::response::Redirect;
use axum_extra::extract::PrivateCookieJar;
use http::StatusCode;
use serde::Deserialize;
use tracing::{info, warn};
use utoipa::IntoParams;

use crate::auth::{self, SESSION_COOKIE, SIGN_IN_COOKIE, Session};
use crate::github::SignInError;
use crate::models::{NewUser, User};
use crate::server::ServerContext;
use crate::util::errors::{self, ApiErrorResponse, AppResult};

const START_AGAIN: &str = "This sign-in cannot be completed. Please sign in again.";
const NEEDS_EMAIL: &str = "Signing in needs a verified primary email on your GitHub account. \
                           Add one at https://github.com/settings/emails, then sign in again.";

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

/// Finishes signing in when GitHub sends the browser back, then goes to the home page.
#[utoipa::path(
    get,
    path = "/api/v1/session/github/callback",
    tag = "session",
    params(CallbackQuery),
    responses(
        (status = 303, description = "Signed in; redirects to the home page.", headers(("location" = String))),
        (status = 400, description = "The sign-in was refused, expired or did not start here.", body = ApiErrorResponse),
        (status = 403, description = "The GitHub account has no verified primary email.", body = ApiErrorResponse),
        (status = 502, description = "GitHub could not be reached.", body = ApiErrorResponse),
    ),
)]
pub async fn callback(
    ctx: ServerContext,
    jar: PrivateCookieJar,
    Query(query): Query<CallbackQuery>,
) -> AppResult<(PrivateCookieJar, Redirect)> {
    let pending = auth::pending_sign_in(&jar).ok_or_else(|| errors::bad_request(START_AGAIN))?;
    if query.state.as_deref() != Some(pending.state.as_str()) {
        return Err(errors::bad_request(START_AGAIN));
    }
    if let Some(error) = query.error {
        // Debug-formatted: the value comes from the query string, so it is escaped.
        info!(?error, "GitHub did not approve a sign-in");
        return Err(errors::bad_request(START_AGAIN));
    }
    let code = query.code.ok_or_else(|| errors::bad_request(START_AGAIN))?;

    let account =
        ctx.github
            .finish_sign_in(&code, &pending)
            .await
            .map_err(|error| match error {
                SignInError::Rejected(reason) => {
                    info!(%reason, "GitHub rejected a sign-in");
                    errors::bad_request(START_AGAIN)
                }
                SignInError::NoVerifiedEmail => {
                    info!("a sign-in had no verified primary email");
                    errors::forbidden(NEEDS_EMAIL)
                }
                error => {
                    warn!(error = %errors::describe(&error), "sign-in failed");
                    errors::bad_gateway("GitHub could not be reached. Please try again later.")
                }
            })?;

    let mut conn = ctx.db.get().await?;
    let user = NewUser {
        gh_id: account.id,
        gh_login: &account.login,
        name: account.name.as_deref(),
        email: &account.email,
    }
    .insert_or_update(&mut conn)
    .await?;
    info!(user.id, "signed in");

    let jar = jar
        .remove(auth::removal(SIGN_IN_COOKIE))
        .add(Session::new(&user).into_cookie());
    Ok((jar, Redirect::to("/")))
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
