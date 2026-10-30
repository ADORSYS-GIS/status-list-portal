//! Who is making a request: the cookies that carry a session, and the extractor that checks
//! them.
//!
//! Cookies are encrypted with the session key, so browsers can neither read nor forge them.
//! Their `__Host-` prefix makes browsers keep them only when `Secure`, with `Path=/` and no
//! `Domain`, so no subdomain can plant or read them.

use axum::extract::FromRequestParts;
use axum_extra::extract::PrivateCookieJar;
use axum_extra::extract::cookie::{Cookie, SameSite};
use diesel::OptionalExtension;
use http::request::Parts;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use time::{Duration, OffsetDateTime};

use crate::github::PendingSignIn;
use crate::models::User;
use crate::server::ServerContext;
use crate::util::errors::{self, BoxedAppError};

pub const SESSION_COOKIE: &str = "__Host-portal-session";
pub const SIGN_IN_COOKIE: &str = "__Host-portal-sign-in";

const SESSION_LIFETIME: Duration = Duration::days(7);
/// Time to approve the portal on GitHub.
const SIGN_IN_LIFETIME: Duration = Duration::minutes(10);

/// A signed-in browser, as recorded in its session cookie.
#[derive(Debug, Serialize, Deserialize)]
pub struct Session {
    pub user_id: i64,
    /// The user's session epoch when the session started. Once the epoch moves on, the
    /// session is over.
    pub epoch: i32,
    /// Unix time after which the session is over, whatever the browser does with the cookie.
    pub expires_at: i64,
}

impl Session {
    pub fn new(user: &User) -> Self {
        Self {
            user_id: user.id,
            epoch: user.session_epoch,
            expires_at: (OffsetDateTime::now_utc() + SESSION_LIFETIME).unix_timestamp(),
        }
    }

    /// The session the request's cookies carry, unless it has expired.
    pub fn from_jar(jar: &PrivateCookieJar) -> Option<Self> {
        let session: Self = read(jar, SESSION_COOKIE)?;
        (session.expires_at > OffsetDateTime::now_utc().unix_timestamp()).then_some(session)
    }

    pub fn into_cookie(self) -> Cookie<'static> {
        write(SESSION_COOKIE, &self, SESSION_LIFETIME)
    }
}

pub fn sign_in_cookie(pending: &PendingSignIn) -> Cookie<'static> {
    write(SIGN_IN_COOKIE, pending, SIGN_IN_LIFETIME)
}

pub fn pending_sign_in(jar: &PrivateCookieJar) -> Option<PendingSignIn> {
    read(jar, SIGN_IN_COOKIE)
}

/// What to hand `PrivateCookieJar::remove` to delete the cookie called `name`. Browsers only
/// accept the deletion with the same attributes the cookie was set with.
pub fn removal(name: &'static str) -> Cookie<'static> {
    cookie(name, String::new(), Duration::ZERO)
}

fn write<T: Serialize>(name: &'static str, contents: &T, lifetime: Duration) -> Cookie<'static> {
    let value = serde_json::to_string(contents).expect("cookie contents serialize to JSON");
    cookie(name, value, lifetime)
}

fn read<T: DeserializeOwned>(jar: &PrivateCookieJar, name: &str) -> Option<T> {
    serde_json::from_str(jar.get(name)?.value()).ok()
}

fn cookie(name: &'static str, value: String, lifetime: Duration) -> Cookie<'static> {
    Cookie::build((name, value))
        .path("/")
        .secure(true)
        .http_only(true)
        .same_site(SameSite::Lax)
        .max_age(lifetime)
        .build()
}

/// The signed-in user. Requests without a live session get `401 Unauthorized`.
pub struct AuthenticatedUser(pub User);

impl FromRequestParts<ServerContext> for AuthenticatedUser {
    type Rejection = BoxedAppError;

    async fn from_request_parts(
        parts: &mut Parts,
        ctx: &ServerContext,
    ) -> Result<Self, Self::Rejection> {
        let jar =
            PrivateCookieJar::from_headers(&parts.headers, ctx.config.session.key.key().clone());
        let session = Session::from_jar(&jar).ok_or_else(errors::unauthorized)?;
        let mut conn = ctx.db.get().await?;
        User::find(&mut conn, session.user_id)
            .await
            .optional()?
            .filter(|user| user.session_epoch == session.epoch)
            .map(Self)
            .ok_or_else(errors::unauthorized)
    }
}
