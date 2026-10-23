//! Sandbox issuers, within their limits.
//!
//! An account may have a few active issuers and register a few more over time; the sandbox
//! as a whole holds a fixed number of active ones. Every change that adds an active issuer
//! counts under one lock, so two requests cannot take the last place together. When
//! registrations need an operator's approval, a developer's request waits in the portal's
//! database until an operator approves or rejects it.

use diesel::OptionalExtension;
use diesel_async::{AnsiTransactionManager, AsyncPgConnection, TransactionManager};
use http::StatusCode;
use time::{Duration, OffsetDateTime};
use tracing::warn;

use crate::admin_api::{
    Issuer, IssuerQuery, IssuerSource, IssuerState, IssuerTier, NewIssuer, Owner, UnknownAlgorithm,
};
use crate::config;
use crate::db;
use crate::models::{NewRegistration, Registration, RegistrationState, TermsAcceptance, User};
use crate::server::ServerContext;
use crate::util::errors::{self, AppResult};

/// An issuer's lifetime ends this long before the longest the server allows, in case its
/// clock runs behind the portal's.
const LIFETIME_MARGIN: Duration = Duration::minutes(5);

/// Registrations that wait for approval. Each becomes an active issuer when approved.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Pending {
    pub(crate) of_account: usize,
    pub(crate) in_sandbox: usize,
}

/// When an issuer registered or renewed now stops being active.
pub fn expiry(sandbox: &config::Sandbox) -> OffsetDateTime {
    let lifetime = Duration::days(sandbox.lifetime_days.get().into());
    OffsetDateTime::now_utc() + lifetime - LIFETIME_MARGIN
}

/// Registering needs the current terms of use accepted.
pub async fn ensure_terms_accepted(
    conn: &mut AsyncPgConnection,
    user: &User,
    terms: &config::Terms,
) -> AppResult<()> {
    if !TermsAcceptance::exists(conn, user.id, &terms.version).await? {
        return Err(errors::forbidden("Accept the current terms of use first."));
    }
    Ok(())
}

/// Creates an issuer for `owner`, if the limits leave room for it.
pub async fn register(
    ctx: &ServerContext,
    conn: &mut AsyncPgConnection,
    owner: &Owner,
    issuer: &NewIssuer,
) -> AppResult<Issuer> {
    with_limits_lock(conn, async |_| {
        ensure_may_add(ctx, owner, 0, Pending::default()).await?;
        AppResult::Ok(ctx.admin_api.issuers_of(owner).create(issuer).await?)
    })
    .await
}

/// Records what `user` asks for, for an operator to approve, if the limits leave room for
/// it. Pending registrations count as active issuers, and rejected ones as registered.
pub async fn request(
    ctx: &ServerContext,
    conn: &mut AsyncPgConnection,
    user: &User,
    issuer: &NewIssuer,
) -> AppResult<Registration> {
    let owner = Owner::github(user.gh_id);
    let alg = issuer.alg.to_string();
    let public_key = serde_json::to_value(&issuer.public_key)?;
    with_limits_lock(conn, async |conn| {
        let without_issuer = Registration::count_without_issuer(conn, user.id).await?;
        let pending = Pending {
            of_account: Registration::count_pending(conn, Some(user.id)).await?,
            in_sandbox: Registration::count_pending(conn, None).await?,
        };
        ensure_may_add(ctx, &owner, without_issuer, pending).await?;
        let registration = NewRegistration {
            user_id: user.id,
            display_name: &issuer.display_name,
            alg: &alg,
            public_key: &public_key,
        };
        AppResult::Ok(registration.insert(conn).await?)
    })
    .await
}

/// Approves a pending registration: creates its issuer for the developer who asked, if the
/// limits leave room for it now.
///
/// The registration stays locked until the approval is recorded, so a withdrawal or a
/// rejection meanwhile waits, then finds it decided. If the issuer cannot be created, the
/// registration stays pending.
pub async fn approve(
    ctx: &ServerContext,
    conn: &mut AsyncPgConnection,
    id: i64,
) -> AppResult<Issuer> {
    with_limits_lock(conn, async |conn| {
        let registration = Registration::find_for_update(conn, id)
            .await
            .optional()?
            .ok_or_else(|| errors::custom(StatusCode::NOT_FOUND, "No registration has this id."))?;
        if registration.state() != RegistrationState::Pending {
            return Err(errors::custom(
                StatusCode::CONFLICT,
                "This registration was decided already.",
            ));
        }
        let user = User::find(conn, registration.user_id).await?;
        if !TermsAcceptance::exists(conn, user.id, &ctx.config.terms.version).await? {
            return Err(errors::forbidden(
                "The developer has not accepted the current terms of use.",
            ));
        }
        let alg = registration
            .alg
            .parse()
            .map_err(|error: UnknownAlgorithm| errors::internal(error.to_string()))?;
        let new_issuer = NewIssuer {
            display_name: registration.display_name,
            public_key: serde_json::from_value(registration.public_key)?,
            alg,
            expires_at: Some(expiry(&ctx.config.sandbox)),
        };

        let owner = Owner::github(user.gh_id);
        ensure_may_add(ctx, &owner, 0, Pending::default()).await?;
        let issuer = ctx.admin_api.issuers_of(&owner).create(&new_issuer).await?;
        Registration::approve(conn, id, &issuer.id).await?;
        AppResult::Ok(issuer)
    })
    .await
}

/// Runs `work` in a transaction that holds the sandbox limits lock, so two issuers cannot
/// both take the last free place. A request that finds the lock taken gives up at once
/// rather than wait with a database connection. An error rolls the transaction back.
///
/// The connection stays checked out while `work` calls the admin API, so the lock holds.
pub(crate) async fn with_limits_lock<T>(
    conn: &mut AsyncPgConnection,
    work: impl AsyncFnOnce(&mut AsyncPgConnection) -> AppResult<T>,
) -> AppResult<T> {
    AnsiTransactionManager::begin_transaction(conn).await?;
    let result = async {
        if !db::try_lock_issuer_limits(conn).await? {
            return Err(errors::custom(
                StatusCode::SERVICE_UNAVAILABLE,
                "Another issuer is being registered. Please try again in a moment.",
            ));
        }
        work(conn).await
    }
    .await;

    match result {
        Ok(value) => {
            AnsiTransactionManager::commit_transaction(conn).await?;
            Ok(value)
        }
        Err(error) => {
            if let Err(rollback) = AnsiTransactionManager::rollback_transaction(conn).await {
                warn!(error = %rollback, "failed to roll back the sandbox limits transaction");
            }
            Err(error)
        }
    }
}

/// Refuses another issuer for `owner` when the account registered as many as it may, or
/// when it or the sandbox has no room for another active one. `requested` registrations
/// that created no issuer count as registered, and `pending` ones as active.
async fn ensure_may_add(
    ctx: &ServerContext,
    owner: &Owner,
    requested: usize,
    pending: Pending,
) -> AppResult<()> {
    // One read answers both limits on the account: when it holds fewer issuers than the
    // account may register, it holds all of them.
    let registered = registered_issuers(ctx, owner).await?;
    ensure_registrations_left(&ctx.config.sandbox, registered.len() + requested)?;
    let active = registered
        .iter()
        .filter(|issuer| issuer.state == IssuerState::Active)
        .count();
    ensure_room(ctx, active, pending).await
}

/// The account's sandbox issuers in every state: all of them, or at least as many as it
/// may register.
async fn registered_issuers(ctx: &ServerContext, owner: &Owner) -> AppResult<Vec<Issuer>> {
    let query = sandbox_issuers(Some(owner), None);
    let most = ctx.config.sandbox.max_registrations_per_account;
    Ok(ctx.admin_api.first_issuers(&query, most).await?)
}

/// How many active issuers the account has, as far as the limit on them needs to know.
pub(crate) async fn count_active(ctx: &ServerContext, owner: &Owner) -> AppResult<usize> {
    let query = sandbox_issuers(Some(owner), Some(IssuerState::Active));
    let per_account = ctx.config.sandbox.max_issuers_per_account;
    Ok(ctx.admin_api.count_issuers(&query, per_account).await?)
}

fn sandbox_issuers(owner: Option<&Owner>, state: Option<IssuerState>) -> IssuerQuery {
    IssuerQuery {
        source: Some(IssuerSource::Portal),
        tier: Some(IssuerTier::Sandbox),
        owner: owner.cloned(),
        state,
        ..IssuerQuery::default()
    }
}

/// Refuses another registration when the account registered as many issuers as it may.
/// Deleted issuers count, since the admin API keeps them.
fn ensure_registrations_left(sandbox: &config::Sandbox, registered: usize) -> AppResult<()> {
    let most = sandbox.max_registrations_per_account;
    if registered >= most {
        return Err(errors::forbidden(format!(
            "This account cannot register more sandbox issuers: it has registered {most}, \
             deleted and rejected ones included."
        )));
    }
    Ok(())
}

/// Refuses another active issuer when the account, which has `active` already, or the
/// sandbox as a whole, has as many as allowed. `pending` registrations count as active.
pub(crate) async fn ensure_room(
    ctx: &ServerContext,
    active: usize,
    pending: Pending,
) -> AppResult<()> {
    let limits = &ctx.config.sandbox;
    let per_account = limits.max_issuers_per_account;
    if active + pending.of_account >= per_account {
        return Err(errors::custom(
            StatusCode::CONFLICT,
            format!("An account can have {per_account} active sandbox issuers. Delete one first."),
        ));
    }
    let in_total = limits.max_active_issuers;
    let in_sandbox = sandbox_issuers(None, Some(IssuerState::Active));
    if ctx.admin_api.count_issuers(&in_sandbox, in_total).await? + pending.in_sandbox >= in_total {
        return Err(errors::custom(
            StatusCode::SERVICE_UNAVAILABLE,
            "The sandbox has no room for another issuer right now. Please try again later.",
        ));
    }
    Ok(())
}
