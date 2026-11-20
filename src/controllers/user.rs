//! The signed-in user's account.

use axum::Json;
use serde::Serialize;
use utoipa::ToSchema;

use crate::auth::AuthenticatedUser;
use crate::models::TermsAcceptance;
use crate::server::ServerContext;
use crate::util::errors::{ApiErrorResponse, AppResult};

#[derive(Serialize, ToSchema)]
pub struct MeResponse {
    /// GitHub login.
    #[schema(example = "octocat")]
    login: String,
    /// Display name from the GitHub profile.
    #[schema(example = "The Octocat", required = true)]
    name: Option<String>,
    /// Verified primary email on GitHub.
    #[schema(example = "octocat@example.com")]
    email: String,
    /// Whether the user accepted the current terms of use. Registering an issuer needs it.
    terms_accepted: bool,
}

impl std::fmt::Debug for MeResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MeResponse")
            .field("terms_accepted", &self.terms_accepted)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::MeResponse;

    #[test]
    fn debug_hides_account_details() {
        let response = MeResponse {
            login: "private-login".into(),
            name: Some("Private Name".into()),
            email: "private@example.com".into(),
            terms_accepted: true,
        };
        let debug = format!("{response:?}");
        assert!(debug.contains("terms_accepted: true"));
        for secret in ["private-login", "Private Name", "private@example.com"] {
            assert!(!debug.contains(secret), "{debug}");
        }
    }
}

/// The signed-in user.
#[utoipa::path(
    get,
    path = "/api/v1/me",
    tag = "users",
    responses(
        (status = 200, description = "The signed-in user.", body = MeResponse),
        (status = 401, description = "Not signed in.", body = ApiErrorResponse),
    ),
)]
pub async fn me(
    ctx: ServerContext,
    AuthenticatedUser(user): AuthenticatedUser,
) -> AppResult<Json<MeResponse>> {
    let mut conn = ctx.db.get().await?;
    let terms_accepted =
        TermsAcceptance::exists(&mut conn, user.id, &ctx.config.terms.version).await?;
    Ok(Json(MeResponse {
        login: user.gh_login,
        name: user.name,
        email: user.email,
        terms_accepted,
    }))
}
