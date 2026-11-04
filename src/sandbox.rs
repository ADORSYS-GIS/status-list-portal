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
    Issuer, IssuerQuery, IssuerSource, IssuerState, IssuerTier, NewIssuer, Owner, PublicJwk,
    UnknownAlgorithm,
};
use crate::config;
use crate::db;
use crate::models::{NewRegistration, Registration, RegistrationState, TermsAcceptance, User};
use crate::server::ServerContext;
use crate::util::errors::{self, AppResult};

/// An issuer's lifetime ends this long before the longest the server allows, in case its
/// clock runs behind the portal's.
const LIFETIME_MARGIN: Duration = Duration::minutes(5);

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

/// Creates an issuer for `user`, if the limits leave room for it.
pub async fn register(
    ctx: &ServerContext,
    conn: &mut AsyncPgConnection,
    user: &User,
    issuer: &NewIssuer,
) -> AppResult<Issuer> {
    let owner = Owner::github(user.gh_id);
    with_limits_lock(conn, async |conn| {
        ensure_may_add(ctx, conn, user.id, &owner, None).await?;
        AppResult::Ok(ctx.admin_api.issuers_of(&owner).create(issuer).await?)
    })
    .await
}

/// Records what `user` asks for, for an operator to approve, if the limits leave room for
/// it.
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
        ensure_may_add(ctx, conn, user.id, &owner, None).await?;
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
/// rejection meanwhile is refused. If the issuer cannot be created, the registration stays
/// pending. If an earlier approval created the issuer but never heard back, approving again
/// records that issuer.
pub async fn approve(
    ctx: &ServerContext,
    conn: &mut AsyncPgConnection,
    id: i64,
) -> AppResult<Issuer> {
    with_limits_lock(conn, async |conn| {
        let registration = lock_pending(conn, id, None).await?;
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
        let registered = registered_issuers(ctx, &owner).await?;
        let created = created_earlier(&registered, registration.created_at, &new_issuer.public_key);
        let issuer = match created {
            Some(issuer) => {
                warn!(
                    registration.id = id,
                    issuer.id = %issuer.id,
                    "recorded the issuer an earlier approval created"
                );
                issuer.clone()
            }
            None => {
                ensure_limits(ctx, conn, user.id, &registered, Some(id)).await?;
                ctx.admin_api.issuers_of(&owner).create(&new_issuer).await?
            }
        };
        ensure_recorded(Registration::approve(conn, id, &issuer.id).await?)?;
        AppResult::Ok(issuer)
    })
    .await
}

/// Rejects a pending registration. The developer sees `reason`.
pub async fn reject(conn: &mut AsyncPgConnection, id: i64, reason: Option<&str>) -> AppResult<()> {
    in_transaction(conn, async |conn| {
        lock_pending(conn, id, None).await?;
        ensure_recorded(Registration::reject(conn, id, reason).await?)
    })
    .await
}

/// Withdraws a registration of the user `user_id` while it waits for approval.
pub async fn withdraw(conn: &mut AsyncPgConnection, user_id: i64, id: i64) -> AppResult<()> {
    in_transaction(conn, async |conn| {
        lock_pending(conn, id, Some(user_id)).await?;
        ensure_recorded(Registration::withdraw(conn, id).await?)
    })
    .await
}

/// Locks a pending registration until the transaction ends, so only one decision lands. A
/// registration another decision holds is refused at once rather than waited for with a
/// database connection. With `user_id`, only that user's registration is found.
async fn lock_pending(
    conn: &mut AsyncPgConnection,
    id: i64,
    user_id: Option<i64>,
) -> AppResult<Registration> {
    let found =
        |registration: &Registration| user_id.is_none_or(|user_id| registration.user_id == user_id);
    let not_found = || errors::custom(StatusCode::NOT_FOUND, "No registration has this id.");
    let Some(registration) = Registration::try_lock(conn, id).await? else {
        return match Registration::find(conn, id).await.optional()? {
            Some(held) if found(&held) => Err(errors::custom(
                StatusCode::CONFLICT,
                "This registration is being decided. Please try again in a moment.",
            )),
            _ => Err(not_found()),
        };
    };
    if !found(&registration) {
        return Err(not_found());
    }
    if registration.state() != RegistrationState::Pending {
        return Err(errors::custom(
            StatusCode::CONFLICT,
            "This registration was decided already.",
        ));
    }
    Ok(registration)
}

/// A locked pending registration is always recorded; anything else is a bug, so the
/// transaction rolls back.
fn ensure_recorded(recorded: bool) -> AppResult<()> {
    if !recorded {
        return Err(errors::internal(
            "the locked registration was no longer pending",
        ));
    }
    Ok(())
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
    in_transaction(conn, async |conn| {
        if !db::try_lock_issuer_limits(conn).await? {
            return Err(errors::custom(
                StatusCode::SERVICE_UNAVAILABLE,
                "Another issuer is being registered. Please try again in a moment.",
            ));
        }
        work(conn).await
    })
    .await
}

/// Runs `work` in a transaction on `conn` itself. An error rolls the transaction back.
async fn in_transaction<T>(
    conn: &mut AsyncPgConnection,
    work: impl AsyncFnOnce(&mut AsyncPgConnection) -> AppResult<T>,
) -> AppResult<T> {
    AnsiTransactionManager::begin_transaction(conn).await?;
    let result = work(conn).await;
    match result {
        Ok(value) => {
            AnsiTransactionManager::commit_transaction(conn).await?;
            Ok(value)
        }
        Err(error) => {
            if let Err(rollback) = AnsiTransactionManager::rollback_transaction(conn).await {
                warn!(error = %rollback, "failed to roll back a sandbox transaction");
            }
            Err(error)
        }
    }
}

/// Refuses another issuer for `owner`, the GitHub account of the user `user_id`, when the
/// account registered as many as it may, or when it or the sandbox has no room for another
/// active one.
///
/// The account's registrations that wait for approval count as active issuers, and those
/// that created no issuer as registered ones, whether approval is on or not: `approving`
/// leaves out the registration being approved.
async fn ensure_may_add(
    ctx: &ServerContext,
    conn: &mut AsyncPgConnection,
    user_id: i64,
    owner: &Owner,
    approving: Option<i64>,
) -> AppResult<()> {
    let registered = registered_issuers(ctx, owner).await?;
    ensure_limits(ctx, conn, user_id, &registered, approving).await
}

/// [`ensure_may_add`], with the account's `registered` issuers read already.
async fn ensure_limits(
    ctx: &ServerContext,
    conn: &mut AsyncPgConnection,
    user_id: i64,
    registered: &[Issuer],
    approving: Option<i64>,
) -> AppResult<()> {
    // One read answers both limits on the account: when it holds fewer issuers than the
    // account may register, it holds all of them.
    let without_issuer = Registration::count_without_issuer(conn, user_id, approving).await?;
    ensure_registrations_left(&ctx.config.sandbox, registered.len() + without_issuer)?;
    let active = registered
        .iter()
        .filter(|issuer| issuer.state == IssuerState::Active)
        .count();
    let pending = Registration::count_pending(conn, user_id, approving).await?;
    ensure_room(ctx, active + pending).await
}

/// The issuer an earlier approval of a registration made at `asked_at` created, if its answer
/// never came back: an active issuer of the account with the registration's key, created
/// since. The admin API refuses a key in use, so there is at most one.
fn created_earlier<'a>(
    registered: &'a [Issuer],
    asked_at: OffsetDateTime,
    key: &PublicJwk,
) -> Option<&'a Issuer> {
    registered.iter().find(|issuer| {
        issuer.state == IssuerState::Active
            && issuer
                .created_at
                .is_some_and(|created_at| created_at >= asked_at)
            && issuer.keys.iter().any(|known| known.public_key == *key)
    })
}

/// Refuses to make an expired issuer of `owner`, the GitHub account of the user `user_id`,
/// active again when the account or the sandbox has no room for it. The account's
/// registrations that wait for approval count as active issuers.
pub(crate) async fn ensure_room_to_renew(
    ctx: &ServerContext,
    conn: &mut AsyncPgConnection,
    user_id: i64,
    owner: &Owner,
) -> AppResult<()> {
    let active = count_active(ctx, owner).await?;
    let pending = Registration::count_pending(conn, user_id, None).await?;
    ensure_room(ctx, active + pending).await
}

/// The account's sandbox issuers in every state: all of them, or at least as many as it
/// may register.
async fn registered_issuers(ctx: &ServerContext, owner: &Owner) -> AppResult<Vec<Issuer>> {
    let query = sandbox_issuers(Some(owner), None);
    let most = ctx.config.sandbox.max_registrations_per_account;
    Ok(ctx.admin_api.first_issuers(&query, most).await?)
}

/// How many active issuers the account has, as far as the limit on them needs to know.
async fn count_active(ctx: &ServerContext, owner: &Owner) -> AppResult<usize> {
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
             deleted, rejected and waiting ones included."
        )));
    }
    Ok(())
}

/// Refuses another active issuer when the account, which has `active` already, or the
/// sandbox as a whole, has as many as allowed. Only the account counts its registrations
/// that wait for approval: an approval checks the sandbox again.
async fn ensure_room(ctx: &ServerContext, active: usize) -> AppResult<()> {
    let limits = &ctx.config.sandbox;
    let per_account = limits.max_issuers_per_account;
    if active >= per_account {
        return Err(errors::custom(
            StatusCode::CONFLICT,
            format!(
                "An account can have {per_account} active sandbox issuers, registrations \
                 waiting for approval included. Delete one, or withdraw a registration, first."
            ),
        ));
    }
    let in_total = limits.max_active_issuers;
    let in_sandbox = sandbox_issuers(None, Some(IssuerState::Active));
    if ctx.admin_api.count_issuers(&in_sandbox, in_total).await? >= in_total {
        return Err(errors::custom(
            StatusCode::SERVICE_UNAVAILABLE,
            "The sandbox has no room for another issuer right now. Please try again later.",
        ));
    }
    Ok(())
}
