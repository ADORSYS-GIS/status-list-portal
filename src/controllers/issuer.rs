//! Sandbox issuers: developers register, list, renew and delete their own.
//!
//! Every handler reaches the admin API through the signed-in account's
//! [`OwnedIssuers`](crate::admin_api::OwnedIssuers), so another account's issuer answers
//! `404 Not Found`. Deleting suspends: the admin API keeps every issuer.

mod public_key;

use axum::Json;
use axum::extract::Path;
use axum::response::{IntoResponse, Response};
use http::StatusCode;
use http::header::LOCATION;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use time::{Duration, OffsetDateTime};
use tracing::info;
use url::Url;
use utoipa::{IntoParams, ToSchema};

use crate::admin_api::{
    Issuer, IssuerChange, IssuerKey, IssuerState, IssuerUsage, KeyState, NewIssuer, NewIssuerKey,
    Owner, RequestedState, SigningAlgorithm,
};
use crate::auth::AuthenticatedUser;
use crate::config::{self, PublicOrigin};
use crate::controllers::registration::RegistrationResponse;
use crate::metrics::RegistrationOutcome;
use crate::rate_limiter::LimitedAction;
use crate::sandbox;
use crate::server::ServerContext;
use crate::util::errors::{self, ApiErrorCode, ApiErrorResponse, AppResult};
use crate::util::{JsonBody, QueryParams};

const DISPLAY_NAME_MAX_CHARS: usize = 100;

#[derive(Debug, Serialize, ToSchema)]
pub struct IssuerResponse {
    /// The `iss` of the issuer's management tokens.
    #[schema(example = "6f1c2a9e-0b7d-4c1e-9a53-2f8d7e4b1c00")]
    id: String,
    #[schema(example = "Acme Wallet Issuer", required = true)]
    display_name: Option<String>,
    /// Only `active` issuers can write status lists. Deleted issuers are `suspended`.
    state: IssuerState,
    #[serde(with = "time::serde::rfc3339::option")]
    #[schema(required = true)]
    created_at: Option<OffsetDateTime>,
    /// When the issuer stops being active, unless it is renewed.
    #[serde(with = "time::serde::rfc3339::option")]
    #[schema(required = true)]
    expires_at: Option<OffsetDateTime>,
    /// From when the portal warns that the issuer expires soon: a week before `expires_at`, or
    /// the last quarter of the sandbox lifetime when that is shorter. Renewing works at any
    /// time. `null` when the issuer never expires.
    #[serde(with = "time::serde::rfc3339::option")]
    #[schema(required = true)]
    expires_soon_from: Option<OffsetDateTime>,
    /// Every key of the issuer, revoked ones included, newest first. Only `active` keys may
    /// sign its management tokens.
    keys: Vec<IssuerKeyResponse>,
    /// What the issuer uses of its tier's limits.
    usage: IssuerUsageResponse,
    /// Where the issuer's management tokens go.
    #[schema(example = "https://status.example.com")]
    management_url: String,
    /// The `aud` of the issuer's management tokens.
    #[schema(example = "https://status.example.com")]
    audience: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct IssuerKeyResponse {
    /// The `kid` in the header of the tokens the key signs.
    #[schema(example = "QoRTRSkzlInq0eOmPqJwmZ5OS9pxkN_SIf5KgZP2F8M")]
    kid: String,
    alg: SigningAlgorithm,
    state: KeyState,
}

impl From<IssuerKey> for IssuerKeyResponse {
    fn from(key: IssuerKey) -> Self {
        Self {
            kid: key.kid,
            alg: key.alg,
            state: key.state,
        }
    }
}

/// What the issuer uses of its tier's limits.
#[derive(Debug, Serialize, ToSchema)]
pub struct IssuerUsageResponse {
    /// Status lists the issuer holds.
    #[schema(example = 3)]
    lists: u32,
    /// Most status lists the issuer may hold.
    #[schema(example = 10)]
    lists_limit: u32,
    /// Keys that are not revoked.
    #[schema(example = 1)]
    keys: u32,
    /// Most keys the issuer may have that are not revoked. `null` when there is no limit.
    #[schema(example = 5, required = true)]
    keys_limit: Option<u32>,
}

impl From<IssuerUsage> for IssuerUsageResponse {
    fn from(usage: IssuerUsage) -> Self {
        // Destructured, so that a field the admin API adds is not left out unnoticed.
        let IssuerUsage {
            lists,
            lists_limit,
            keys,
            keys_limit,
        } = usage;
        Self {
            lists,
            lists_limit,
            keys,
            keys_limit,
        }
    }
}

impl IssuerResponse {
    /// `expires_soon_window` is how long before its expiry the issuer is warned about it.
    fn new(
        issuer: Issuer,
        management_api: &config::ManagementApi,
        expires_soon_window: Duration,
    ) -> Self {
        let keys = issuer
            .keys
            .into_iter()
            .map(IssuerKeyResponse::from)
            .collect();
        Self {
            id: issuer.id,
            display_name: issuer.display_name,
            state: issuer.state,
            created_at: issuer.created_at,
            expires_at: issuer.expires_at,
            expires_soon_from: issuer.expires_at.map(|expiry| expiry - expires_soon_window),
            keys,
            usage: issuer.usage.into(),
            management_url: management_api.url.as_str().trim_end_matches('/').to_owned(),
            audience: management_api.audience.clone(),
        }
    }
}

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ListQuery {
    /// The previous page's `next_cursor`.
    cursor: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct IssuerListResponse {
    issuers: Vec<IssuerResponse>,
    /// Pass it as `cursor` to read the next page. `null` on the last page.
    #[schema(required = true)]
    next_cursor: Option<String>,
}

/// The signed-in user's issuers, deleted ones included, a page at a time.
#[utoipa::path(
    get,
    path = "/api/v1/issuers",
    tag = "issuers",
    params(ListQuery),
    responses(
        (status = 200, description = "One page of the user's issuers.", body = IssuerListResponse),
        (status = 400, description = "The query string is invalid.", body = ApiErrorResponse),
        (status = 401, description = "Not signed in.", body = ApiErrorResponse),
        (status = 502, description = "The Status List Server did not complete the request.", body = ApiErrorResponse),
    ),
)]
pub async fn list_issuers(
    ctx: ServerContext,
    AuthenticatedUser(user): AuthenticatedUser,
    QueryParams(query): QueryParams<ListQuery>,
) -> AppResult<Json<IssuerListResponse>> {
    let owner = Owner::github(user.gh_id);
    let page = ctx
        .admin_api
        .issuers_of(&owner)
        .list(query.cursor.as_deref())
        .await?;
    let management_api = &ctx.config.management_api;
    let expires_soon_window = sandbox::expires_soon_window(&ctx.config.sandbox);
    Ok(Json(IssuerListResponse {
        issuers: page
            .issuers
            .into_iter()
            .map(|issuer| IssuerResponse::new(issuer, management_api, expires_soon_window))
            .collect(),
        next_cursor: page.next_cursor,
    }))
}

// No `Debug`: a pasted private key must not reach the logs.
/// A sandbox issuer to register.
#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct NewIssuerRequest {
    /// Shown in the portal and to the server's operators. 1 to 100 characters.
    #[schema(example = "Acme Wallet Issuer")]
    display_name: String,
    /// The algorithm the issuer signs its management tokens with.
    alg: SigningAlgorithm,
    /// The public key that checks them, as a JSON Web Key. Private keys are refused.
    #[schema(value_type = Object)]
    public_key: Value,
}

impl NewIssuerRequest {
    fn into_new_issuer(self, expires_at: OffsetDateTime) -> AppResult<NewIssuer> {
        let display_name = self.display_name.trim();
        let length = display_name.chars().count();
        if length == 0 || length > DISPLAY_NAME_MAX_CHARS || display_name.contains(char::is_control)
        {
            return Err(errors::custom(
                ApiErrorCode::InvalidDisplayName,
                format!(
                    "The display name must be 1 to {DISPLAY_NAME_MAX_CHARS} printable characters."
                ),
            ));
        }
        Ok(NewIssuer {
            display_name: display_name.to_owned(),
            public_key: public_key::parse(&self.public_key, self.alg)?,
            alg: self.alg,
            expires_at: Some(expires_at),
        })
    }
}

/// Registers a sandbox issuer for the signed-in user.
///
/// It stays active for one sandbox lifetime. The account needs the current terms of use
/// accepted and registrations left, and both the account and the sandbox need a free place.
/// When registrations need an operator's approval, the request waits for it instead, and the
/// Status List Server only sees the key then: a key it refuses, or one registered already,
/// fails the approval rather than this request.
#[utoipa::path(
    post,
    path = "/api/v1/issuers",
    tag = "issuers",
    request_body = NewIssuerRequest,
    responses(
        (status = 201, description = "Registered.", body = IssuerResponse, headers(("location" = String))),
        (status = 202, description = "Waiting for an operator's approval.", body = RegistrationResponse),
        (status = 401, description = "Not signed in.", body = ApiErrorResponse),
        (status = 403, description = "The current terms are not accepted, or the account registered as many issuers as it may.", body = ApiErrorResponse),
        (status = 409, description = "The account has as many active issuers as allowed, or the key is registered already.", body = ApiErrorResponse),
        (status = 422, description = "The display name or the public key cannot be used.", body = ApiErrorResponse),
        (status = 429, description = "Too many registrations in a short time.", body = ApiErrorResponse, headers(("retry-after" = u64, description = "Seconds until the user may try again."))),
        (status = 502, description = "The Status List Server did not complete the request.", body = ApiErrorResponse),
        (status = 503, description = "sandbox_issuer_limit_reached, issuer_registration_in_progress, or service_unavailable.", body = ApiErrorResponse),
    ),
)]
pub async fn create_issuer(
    ctx: ServerContext,
    AuthenticatedUser(user): AuthenticatedUser,
    JsonBody(request): JsonBody<NewIssuerRequest>,
) -> AppResult<Response> {
    let mut conn = ctx.db.get().await?;
    sandbox::ensure_terms_accepted(&mut conn, &user, &ctx.config.terms).await?;
    let new_issuer = request.into_new_issuer(sandbox::expiry(&ctx.config.sandbox))?;
    // Only a registration the developer may make counts. A full sandbox or a failing
    // Status List Server still costs a token, so that retries cannot hammer either.
    ctx.rate_limiter
        .check(&mut conn, user.id, LimitedAction::Register)
        .await?;

    if ctx.config.sandbox.manual_approval {
        let registration = sandbox::request(&ctx, &mut conn, &user, &new_issuer).await?;
        info!(user.id, registration.id, "asked for an issuer");
        ctx.metrics.registration(RegistrationOutcome::Requested);
        let body = RegistrationResponse::new(registration)?;
        return Ok((StatusCode::ACCEPTED, Json(body)).into_response());
    }

    let issuer = sandbox::register(&ctx, &mut conn, &user, &new_issuer).await?;
    info!(user.id, issuer.id = %issuer.id, "registered an issuer");
    ctx.metrics.registration(RegistrationOutcome::Created);

    let location = issuer_location(&ctx.config.server.origin, &issuer.id);
    let body = IssuerResponse::new(
        issuer,
        &ctx.config.management_api,
        sandbox::expires_soon_window(&ctx.config.sandbox),
    );
    Ok((StatusCode::CREATED, [(LOCATION, location)], Json(body)).into_response())
}

/// One of the signed-in user's issuers.
#[utoipa::path(
    get,
    path = "/api/v1/issuers/{id}",
    tag = "issuers",
    params(("id" = String, Path, description = "The issuer's id.")),
    responses(
        (status = 200, description = "The issuer.", body = IssuerResponse),
        (status = 401, description = "Not signed in.", body = ApiErrorResponse),
        (status = 404, description = "The user has no issuer with this id.", body = ApiErrorResponse),
        (status = 502, description = "The Status List Server did not complete the request.", body = ApiErrorResponse),
    ),
)]
pub async fn get_issuer(
    ctx: ServerContext,
    AuthenticatedUser(user): AuthenticatedUser,
    Path(id): Path<String>,
) -> AppResult<Json<IssuerResponse>> {
    let owner = Owner::github(user.gh_id);
    let issuer = ctx
        .admin_api
        .issuers_of(&owner)
        .get(&id)
        .await?
        .ok_or_else(errors::not_found)?;
    Ok(Json(IssuerResponse::new(
        issuer.into_issuer(),
        &ctx.config.management_api,
        sandbox::expires_soon_window(&ctx.config.sandbox),
    )))
}

// No `Debug`: a private key accidentally pasted into the request must stay out of logs.
#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct NewIssuerKeyRequest {
    alg: SigningAlgorithm,
    #[schema(value_type = Object)]
    public_key: Value,
}

/// Adds an active key to one of the signed-in user's issuers.
#[utoipa::path(
    post,
    path = "/api/v1/issuers/{id}/keys",
    tag = "issuers",
    params(("id" = String, Path, description = "The issuer's id.")),
    request_body = NewIssuerKeyRequest,
    responses(
        (status = 201, description = "Key added.", body = IssuerKeyResponse),
        (status = 401, body = ApiErrorResponse),
        (status = 404, body = ApiErrorResponse),
        (status = 409, description = "Key limit, key already used, or issuer changed.", body = ApiErrorResponse),
        (status = 422, description = "Invalid or private key.", body = ApiErrorResponse),
        (status = 502, body = ApiErrorResponse),
        (status = 503, body = ApiErrorResponse),
    ),
)]
pub async fn add_issuer_key(
    ctx: ServerContext,
    AuthenticatedUser(user): AuthenticatedUser,
    Path(id): Path<String>,
    JsonBody(request): JsonBody<NewIssuerKeyRequest>,
) -> AppResult<(StatusCode, Json<IssuerKeyResponse>)> {
    let key = NewIssuerKey {
        alg: request.alg,
        public_key: public_key::parse(&request.public_key, request.alg)?,
    };
    let owner = Owner::github(user.gh_id);
    let issuers = ctx.admin_api.issuers_of(&owner);
    let current = issuers.get(&id).await?.ok_or_else(errors::not_found)?;
    let added = issuers
        .add_key(&current, &key)
        .await?
        .ok_or_else(errors::not_found)?;
    info!(user.id, issuer.id = %id, key.id = %added.kid, "added issuer key");
    Ok((StatusCode::CREATED, Json(added.into())))
}

/// Permanently revokes one key of the signed-in user's issuer.
#[utoipa::path(
    post,
    path = "/api/v1/issuers/{id}/keys/{kid}/revoke",
    tag = "issuers",
    params(
        ("id" = String, Path, description = "The issuer's id."),
        ("kid" = String, Path, description = "The key id."),
    ),
    responses(
        (status = 200, description = "Key revoked.", body = IssuerKeyResponse),
        (status = 401, body = ApiErrorResponse),
        (status = 404, body = ApiErrorResponse),
        (status = 409, description = "Issuer changed.", body = ApiErrorResponse),
        (status = 502, body = ApiErrorResponse),
        (status = 503, body = ApiErrorResponse),
    ),
)]
pub async fn revoke_issuer_key(
    ctx: ServerContext,
    AuthenticatedUser(user): AuthenticatedUser,
    Path((id, kid)): Path<(String, String)>,
) -> AppResult<Json<IssuerKeyResponse>> {
    let owner = Owner::github(user.gh_id);
    let issuers = ctx.admin_api.issuers_of(&owner);
    let current = issuers.get(&id).await?.ok_or_else(errors::not_found)?;
    let revoked = issuers
        .revoke_key(&current, &kid)
        .await?
        .ok_or_else(errors::not_found)?;
    info!(user.id, issuer.id = %id, key.id = %kid, "revoked issuer key");
    Ok(Json(revoked.into()))
}

/// Keeps one of the signed-in user's issuers active for another sandbox lifetime.
///
/// An expired issuer becomes active again, so it needs a free place, like a new one.
#[utoipa::path(
    post,
    path = "/api/v1/issuers/{id}/renew",
    tag = "issuers",
    params(("id" = String, Path, description = "The issuer's id.")),
    responses(
        (status = 200, description = "Renewed.", body = IssuerResponse),
        (status = 401, description = "Not signed in.", body = ApiErrorResponse),
        (status = 403, description = "The current terms are not accepted.", body = ApiErrorResponse),
        (status = 404, description = "The user has no issuer with this id.", body = ApiErrorResponse),
        (status = 409, description = "The issuer is deleted, the account has as many active issuers as allowed, or the issuer changed while this request ran.", body = ApiErrorResponse),
        (status = 429, description = "Too many renewals in a short time.", body = ApiErrorResponse, headers(("retry-after" = u64, description = "Seconds until the user may try again."))),
        (status = 502, description = "The Status List Server did not complete the request.", body = ApiErrorResponse),
        (status = 503, description = "sandbox_issuer_limit_reached, issuer_registration_in_progress, or service_unavailable.", body = ApiErrorResponse),
    ),
)]
pub async fn renew_issuer(
    ctx: ServerContext,
    AuthenticatedUser(user): AuthenticatedUser,
    Path(id): Path<String>,
) -> AppResult<Json<IssuerResponse>> {
    // The connection goes back to the pool before the admin API is called. Only an expired
    // issuer needs one again, for the lock.
    let mut conn = ctx.db.get().await?;
    sandbox::ensure_terms_accepted(&mut conn, &user, &ctx.config.terms).await?;
    ctx.rate_limiter
        .check(&mut conn, user.id, LimitedAction::Renew)
        .await?;
    drop(conn);
    let owner = Owner::github(user.gh_id);
    let issuers = ctx.admin_api.issuers_of(&owner);
    let current = issuers.get(&id).await?.ok_or_else(errors::not_found)?;
    let renewal = IssuerChange::ExpiresAt(sandbox::expiry(&ctx.config.sandbox));

    let renewed = match current.state {
        // No lock: an issuer that expires before the change lands becomes active again
        // without a free place. That takes the sandbox one issuer over at most, which is
        // accepted.
        IssuerState::Active => issuers.update(&current, &renewal).await?,
        IssuerState::Expired => {
            let mut conn = ctx.db.get().await?;
            sandbox::with_limits_lock(&mut conn, async |conn| {
                sandbox::ensure_room_to_renew(&ctx, conn, user.id, &owner).await?;
                AppResult::Ok(issuers.update(&current, &renewal).await?)
            })
            .await?
        }
        IssuerState::Suspended => {
            return Err(errors::custom(
                ApiErrorCode::IssuerNotRenewable,
                "This issuer is deleted or suspended, so it cannot be renewed.",
            ));
        }
    };
    let renewed = renewed.ok_or_else(errors::not_found)?;
    info!(user.id, issuer.id = %renewed.id, "renewed an issuer");
    Ok(Json(IssuerResponse::new(
        renewed,
        &ctx.config.management_api,
        sandbox::expires_soon_window(&ctx.config.sandbox),
    )))
}

/// Deletes one of the signed-in user's issuers: suspends it.
#[utoipa::path(
    delete,
    path = "/api/v1/issuers/{id}",
    tag = "issuers",
    params(("id" = String, Path, description = "The issuer's id.")),
    responses(
        (status = 204, description = "Deleted, now or before."),
        (status = 401, description = "Not signed in.", body = ApiErrorResponse),
        (status = 404, description = "The user has no issuer with this id.", body = ApiErrorResponse),
        (status = 409, description = "The issuer changed while this request ran.", body = ApiErrorResponse),
        (status = 429, description = "Too many deletions in a short time.", body = ApiErrorResponse, headers(("retry-after" = u64, description = "Seconds until the user may try again."))),
        (status = 502, description = "The Status List Server did not complete the request.", body = ApiErrorResponse),
    ),
)]
pub async fn delete_issuer(
    ctx: ServerContext,
    AuthenticatedUser(user): AuthenticatedUser,
    Path(id): Path<String>,
) -> AppResult<StatusCode> {
    // The connection goes back to the pool before the admin API is called.
    let mut conn = ctx.db.get().await?;
    ctx.rate_limiter
        .check(&mut conn, user.id, LimitedAction::Delete)
        .await?;
    drop(conn);
    let owner = Owner::github(user.gh_id);
    let issuers = ctx.admin_api.issuers_of(&owner);
    let current = issuers.get(&id).await?.ok_or_else(errors::not_found)?;
    if current.state == IssuerState::Suspended {
        return Ok(StatusCode::NO_CONTENT);
    }
    let suspend = IssuerChange::State(RequestedState::Suspended);
    issuers
        .update(&current, &suspend)
        .await?
        .ok_or_else(errors::not_found)?;
    info!(user.id, issuer.id = %id, "deleted an issuer");
    Ok(StatusCode::NO_CONTENT)
}

/// The issuer's URL on the portal, for the `Location` header.
fn issuer_location(origin: &PublicOrigin, id: &str) -> String {
    let mut url =
        Url::parse(&origin.url_for("/api/v1/issuers")).expect("the origin is an http(s) URL");
    url.path_segments_mut()
        .expect("an http(s) URL has a path")
        .push(id);
    url.into()
}
