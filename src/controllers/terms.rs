//! The terms of use developers accept before they register an issuer.

use axum::Json;
use http::StatusCode;
use serde::{Deserialize, Serialize};
use tracing::info;
use utoipa::ToSchema;

use crate::auth::AuthenticatedUser;
use crate::models::TermsAcceptance;
use crate::server::ServerContext;
use crate::util::JsonBody;
use crate::util::errors::{self, ApiErrorCode, ApiErrorResponse, AppResult};

#[derive(Debug, Serialize, ToSchema)]
pub struct TermsResponse {
    /// Version of the current terms. Accepting them names it.
    #[schema(example = "2026-10-22")]
    version: String,
    /// Where the current terms are published.
    #[schema(example = "https://portal.example.com/terms")]
    url: String,
}

/// The current terms of use.
#[utoipa::path(
    get,
    path = "/api/v1/terms",
    tag = "terms",
    responses(
        (status = 200, description = "The current terms of use.", body = TermsResponse),
    ),
)]
pub async fn get_terms(ctx: ServerContext) -> Json<TermsResponse> {
    let terms = &ctx.config.terms;
    Json(TermsResponse {
        version: terms.version.clone(),
        url: terms.url.to_string(),
    })
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AcceptTermsRequest {
    /// Version of the terms the user read.
    #[schema(example = "2026-10-22")]
    version: String,
}

/// Accepts the current terms of use on behalf of the signed-in user.
#[utoipa::path(
    put,
    path = "/api/v1/me/terms",
    tag = "terms",
    request_body = AcceptTermsRequest,
    responses(
        (status = 204, description = "Accepted, now or before."),
        (status = 401, description = "Not signed in.", body = ApiErrorResponse),
        (status = 409, description = "The terms changed since the user read them.", body = ApiErrorResponse),
    ),
)]
pub async fn accept_terms(
    ctx: ServerContext,
    AuthenticatedUser(user): AuthenticatedUser,
    JsonBody(request): JsonBody<AcceptTermsRequest>,
) -> AppResult<StatusCode> {
    let current = ctx.config.terms.version.as_str();
    if request.version != current {
        return Err(errors::custom(
            ApiErrorCode::TermsChanged,
            "These terms are no longer current. Please read the current terms first.",
        ));
    }

    let mut conn = ctx.db.get().await?;
    TermsAcceptance {
        user_id: user.id,
        terms_version: current,
        email: &user.email,
    }
    .record(&mut conn)
    .await?;
    info!(
        user.id,
        terms.version = current,
        "accepted the terms of use"
    );
    Ok(StatusCode::NO_CONTENT)
}
