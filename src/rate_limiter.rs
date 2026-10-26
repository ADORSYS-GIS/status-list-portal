//! Rate limits: how often a developer may change their issuers, and how often a client may
//! finish a sign-in. A refused change is answered `429 Too Many Requests`, with
//! `Retry-After` saying when to try again; a refused sign-in goes to the home page, which
//! says to try again later.
//!
//! Developers' limits are token buckets in the database, so that they hold across replicas
//! and restarts. Sign-ins come before there is an account to count them against, so each
//! replica counts them per client network, in memory.

mod bucket;
mod sign_ins;

use std::net::IpAddr;

use diesel::prelude::*;
use diesel_async::{AsyncConnection, AsyncPgConnection, RunQueryDsl};
use time::{Duration, OffsetDateTime};

use crate::config::{RateLimit, RateLimits};
use crate::metrics::Metrics;
use crate::schema::rate_limit_buckets;
use crate::util::errors::{self, AppResult, BoxedAppError};
use bucket::Bucket;
use sign_ins::SignInLimiter;

/// What a developer does that is rate-limited.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LimitedAction {
    /// Registering an issuer, at once or for approval.
    Register,
    Renew,
    Delete,
}

impl LimitedAction {
    /// The action's name in the database and in metrics.
    fn as_str(self) -> &'static str {
        match self {
            Self::Register => "register",
            Self::Renew => "renew",
            Self::Delete => "delete",
        }
    }

    fn limit(self, limits: &RateLimits) -> RateLimit {
        match self {
            Self::Register => limits.register,
            Self::Renew => limits.renew,
            Self::Delete => limits.delete,
        }
    }

    fn refusal(self) -> &'static str {
        match self {
            Self::Register => "You registered too many issuers in a short time.",
            Self::Renew => "You renewed issuers too often in a short time.",
            Self::Delete => "You deleted too many issuers in a short time.",
        }
    }
}

pub struct RateLimiter {
    limits: RateLimits,
    sign_ins: SignInLimiter,
    metrics: Metrics,
}

impl RateLimiter {
    /// Counts refusals in `metrics`.
    pub fn new(limits: RateLimits, metrics: Metrics) -> Self {
        Self {
            limits,
            sign_ins: SignInLimiter::new(limits.sign_in),
            metrics,
        }
    }

    /// Takes one of the tokens of user `user_id` for `action`, or refuses the request.
    pub async fn check(
        &self,
        conn: &mut AsyncPgConnection,
        user_id: i64,
        action: LimitedAction,
    ) -> AppResult<()> {
        let limit = action.limit(&self.limits);
        let now = OffsetDateTime::now_utc();
        let taken = conn
            .transaction(async |conn| take_token(conn, user_id, action, limit, now).await)
            .await?;
        taken.map_err(|retry_after| self.refuse(action.as_str(), action.refusal(), retry_after))
    }

    /// Takes one of the sign-in tokens of the network `client` belongs to. Without one, counts
    /// the refusal and returns how long until a token comes back.
    pub fn check_sign_in(&self, client: IpAddr) -> Result<(), Duration> {
        self.sign_ins
            .take(client, OffsetDateTime::now_utc())
            .inspect_err(|_| self.metrics.rate_limited("sign_in"))
    }

    fn refuse(
        &self,
        action: &'static str,
        refusal: &'static str,
        retry_after: Duration,
    ) -> BoxedAppError {
        self.metrics.rate_limited(action);
        errors::too_many_requests(refusal, retry_after)
    }
}

/// Takes a token from the bucket of `user_id` for `action`, which starts full. Returns how
/// long until a token comes back if none is left. Run it in a transaction: the bucket stays
/// locked until the transaction ends, so concurrent requests take turns.
async fn take_token(
    conn: &mut AsyncPgConnection,
    user_id: i64,
    action: LimitedAction,
    limit: RateLimit,
    now: OffsetDateTime,
) -> QueryResult<Result<(), Duration>> {
    use rate_limit_buckets::dsl;

    let full = Bucket::full(limit, now);
    diesel::insert_into(dsl::rate_limit_buckets)
        .values((
            dsl::user_id.eq(user_id),
            dsl::action.eq(action.as_str()),
            dsl::tokens.eq(stored(full.tokens)),
            dsl::last_refill.eq(full.refilled_at),
        ))
        .on_conflict_do_nothing()
        .execute(conn)
        .await?;
    let key = (user_id, action.as_str());
    let (tokens, refilled_at): (i32, OffsetDateTime) = dsl::rate_limit_buckets
        .find(key)
        .select((dsl::tokens, dsl::last_refill))
        .for_update()
        .first(conn)
        .await?;
    let bucket = Bucket {
        tokens: u32::try_from(tokens).unwrap_or(0),
        refilled_at,
    };

    match bucket.take(limit, now) {
        Ok(left) => {
            diesel::update(dsl::rate_limit_buckets.find(key))
                .set((
                    dsl::tokens.eq(stored(left.tokens)),
                    dsl::last_refill.eq(left.refilled_at),
                ))
                .execute(conn)
                .await?;
            Ok(Ok(()))
        }
        Err(retry_after) => Ok(Err(retry_after)),
    }
}

/// `tokens` as the database stores them.
fn stored(tokens: u32) -> i32 {
    i32::try_from(tokens).unwrap_or(i32::MAX)
}
