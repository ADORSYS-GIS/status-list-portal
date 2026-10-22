//! The signed-in user's account.

use axum::Json;
use serde::Serialize;
use utoipa::ToSchema;

use crate::auth::AuthenticatedUser;
use crate::util::errors::ApiErrorResponse;

#[derive(Debug, Serialize, ToSchema)]
pub struct MeResponse {
    /// GitHub login.
    #[schema(example = "octocat")]
    login: String,
    /// Display name from the GitHub profile.
    #[schema(example = "The Octocat")]
    name: Option<String>,
    /// Verified primary email on GitHub.
    #[schema(example = "octocat@example.com")]
    email: String,
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
pub async fn me(AuthenticatedUser(user): AuthenticatedUser) -> Json<MeResponse> {
    Json(MeResponse {
        login: user.gh_login,
        name: user.name,
        email: user.email,
    })
}
