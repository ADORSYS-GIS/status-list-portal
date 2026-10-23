//! Registrations: the issuers developers asked for while registrations need an operator's
//! approval, and what became of them.

use axum::Json;
use axum::extract::Path;
use diesel::OptionalExtension;
use http::StatusCode;
use serde::Serialize;
use time::OffsetDateTime;
use tracing::info;
use utoipa::ToSchema;

use crate::auth::AuthenticatedUser;
use crate::models::{Registration, RegistrationState};
use crate::server::ServerContext;
use crate::util::errors::{self, ApiErrorResponse, AppResult};

#[derive(Debug, Serialize, ToSchema)]
pub struct RegistrationResponse {
    #[schema(example = 42)]
    id: i64,
    #[schema(example = "Acme Wallet Issuer")]
    display_name: String,
    #[schema(example = "ES256")]
    alg: String,
    state: RegistrationState,
    /// The issuer the approval created.
    issuer_id: Option<String>,
    /// Why an operator rejected the registration.
    rejection_reason: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339::option")]
    decided_at: Option<OffsetDateTime>,
}

impl From<Registration> for RegistrationResponse {
    fn from(registration: Registration) -> Self {
        Self {
            state: registration.state(),
            id: registration.id,
            display_name: registration.display_name,
            alg: registration.alg,
            issuer_id: registration.issuer_id,
            rejection_reason: registration.rejection_reason,
            created_at: registration.created_at,
            decided_at: registration.decided_at,
        }
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
    let registrations = Registration::of_user(&mut conn, user.id).await?;
    Ok(Json(RegistrationListResponse {
        registrations: registrations.into_iter().map(Into::into).collect(),
    }))
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
        (status = 409, description = "The registration was decided already.", body = ApiErrorResponse),
    ),
)]
pub async fn withdraw_registration(
    ctx: ServerContext,
    AuthenticatedUser(user): AuthenticatedUser,
    Path(id): Path<String>,
) -> AppResult<StatusCode> {
    let id: i64 = id.parse().map_err(|_| errors::not_found())?;
    let mut conn = ctx.db.get().await?;
    let registration = Registration::find(&mut conn, id)
        .await
        .optional()?
        .filter(|registration| registration.user_id == user.id)
        .ok_or_else(errors::not_found)?;
    let decided = || {
        errors::custom(
            StatusCode::CONFLICT,
            "This registration was decided already.",
        )
    };
    if registration.state() != RegistrationState::Pending {
        return Err(decided());
    }
    if !Registration::withdraw(&mut conn, id, user.id).await? {
        // Withdrawn or decided since it was read.
        let still_there = Registration::find(&mut conn, id).await.optional()?;
        return Err(match still_there {
            Some(_) => decided(),
            None => errors::not_found(),
        });
    }
    info!(user.id, registration.id = id, "withdrew a registration");
    Ok(StatusCode::NO_CONTENT)
}
