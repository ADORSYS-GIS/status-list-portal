//! Registrations: the issuers developers asked for while registrations need an operator's
//! approval, and what became of them.

use axum::Json;
use axum::extract::Path;
use http::StatusCode;
use serde::Serialize;
use time::OffsetDateTime;
use tracing::info;
use utoipa::ToSchema;

use crate::admin_api::SigningAlgorithm;
use crate::auth::AuthenticatedUser;
use crate::models::{Registration, RegistrationState};
use crate::sandbox;
use crate::server::ServerContext;
use crate::util::errors::{self, ApiErrorResponse, AppResult};

#[derive(Debug, Serialize, ToSchema)]
pub struct RegistrationResponse {
    #[schema(example = 42)]
    id: i64,
    #[schema(example = "Acme Wallet Issuer")]
    display_name: String,
    alg: SigningAlgorithm,
    state: RegistrationState,
    /// The issuer the approval created.
    #[schema(required = true)]
    issuer_id: Option<String>,
    /// Why an operator rejected the registration.
    #[schema(required = true)]
    rejection_reason: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339::option")]
    #[schema(required = true)]
    decided_at: Option<OffsetDateTime>,
}

impl RegistrationResponse {
    pub(crate) fn new(registration: Registration) -> AppResult<Self> {
        Ok(Self {
            state: registration.state(),
            alg: registration.signing_algorithm()?,
            id: registration.id,
            display_name: registration.display_name,
            issuer_id: registration.issuer_id,
            rejection_reason: registration.rejection_reason,
            created_at: registration.created_at,
            decided_at: registration.decided_at,
        })
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct RegistrationListResponse {
    registrations: Vec<RegistrationResponse>,
}

/// The signed-in user's registrations, newest first. Pending and rejected ones count toward
/// the issuers the account may register.
#[utoipa::path(
    get,
    path = "/api/v1/registrations",
    tag = "issuers",
    responses(
        (status = 200, description = "The user's registrations.", body = RegistrationListResponse),
        (status = 401, description = "Not signed in.", body = ApiErrorResponse),
    ),
)]
pub async fn list_registrations(
    ctx: ServerContext,
    AuthenticatedUser(user): AuthenticatedUser,
) -> AppResult<Json<RegistrationListResponse>> {
    let mut conn = ctx.db.get().await?;
    let registrations = Registration::of_user(&mut conn, user.id)
        .await?
        .into_iter()
        .map(RegistrationResponse::new)
        .collect::<AppResult<_>>()?;
    Ok(Json(RegistrationListResponse { registrations }))
}

/// Withdraws one of the signed-in user's registrations while it waits for approval.
#[utoipa::path(
    delete,
    path = "/api/v1/registrations/{id}",
    tag = "issuers",
    params(("id" = i64, Path, description = "The registration's id.")),
    responses(
        (status = 204, description = "Withdrawn."),
        (status = 401, description = "Not signed in.", body = ApiErrorResponse),
        (status = 404, description = "The user has no registration with this id.", body = ApiErrorResponse),
        (status = 409, description = "The registration was decided already, or is being decided.", body = ApiErrorResponse),
    ),
)]
pub async fn withdraw_registration(
    ctx: ServerContext,
    AuthenticatedUser(user): AuthenticatedUser,
    Path(id): Path<String>,
) -> AppResult<StatusCode> {
    let id: i64 = id.parse().map_err(|_| errors::not_found())?;
    let mut conn = ctx.db.get().await?;
    sandbox::withdraw(&mut conn, user.id, id).await?;
    info!(user.id, registration.id = id, "withdrew a registration");
    Ok(StatusCode::NO_CONTENT)
}
