//! Emails that remind developers their sandbox issuers expire soon.
//!
//! `portal send-reminders` runs [`Reminders::send_due`] on a schedule, such as hourly. Each
//! run reads the active sandbox issuers from the admin API and reminds the owner of each one
//! that expires within a reminder window, once per window and expiry date: a renewed issuer
//! is reminded again before its new expiry. When several windows are due at once, as after
//! the schedule stopped for a while, only the nearest is sent. A delivery that may succeed
//! later is tried again by the next runs, so the schedule spaces the tries out.

mod expiry_email;

use std::collections::HashMap;

use anyhow::Context;
use diesel_async::AsyncPgConnection;
use time::{Duration, OffsetDateTime};
use tracing::{error, info, warn};

use crate::admin_api::{AdminApi, Issuer, IssuerQuery, IssuerState, IssuerTier};
use crate::config::PublicOrigin;
use crate::db;
use crate::email::{Mailer, SendError};
use crate::models::{Claim, NewReminder, User};
use expiry_email::ExpiryEmail;

/// Days before an issuer's expiry its reminders are due, nearest first.
pub const WINDOW_DAYS: [i16; 2] = [1, 7];
/// Deliveries of one reminder, the first and up to 3 retries, before it is given up.
pub const MAX_ATTEMPTS: i16 = 4;
/// The SMTP server's last answer is kept up to this length.
const LAST_ERROR_MAX_CHARS: usize = 500;

/// What sending reminders needs.
pub struct Reminders<'a> {
    pub db: &'a db::Pool,
    pub admin_api: &'a AdminApi,
    pub mailer: &'a Mailer,
    /// Where the emails link to.
    pub origin: &'a PublicOrigin,
}

/// What became of the reminders a run tried.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Summary {
    pub sent: usize,
    /// Failed for now; a later run tries again.
    pub retrying: usize,
    /// Failed for good, or too often.
    pub given_up: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Outcome {
    Sent,
    Retrying,
    GivenUp,
}

/// An issuer whose expiry falls within a reminder window.
struct Due {
    issuer: Issuer,
    gh_id: i64,
    expires_at: OffsetDateTime,
    /// The nearest window the expiry falls within.
    window_days: i16,
}

impl Due {
    fn new(issuer: Issuer, now: OffsetDateTime) -> Option<Self> {
        let gh_id = issuer.owner.as_ref()?.github_id()?;
        let expires_at = issuer.expires_at?;
        let left = expires_at - now;
        if left <= Duration::ZERO {
            return None;
        }
        let window_days = WINDOW_DAYS
            .into_iter()
            .find(|&days| left <= Duration::days(days.into()))?;
        Some(Self {
            issuer,
            gh_id,
            expires_at,
            window_days,
        })
    }
}

impl Reminders<'_> {
    /// Sends the reminders due at `now`. One run sends at a time: while another holds the
    /// lock, this one sends nothing. Fails when the database, the admin API or the SMTP
    /// server cannot be reached.
    pub async fn send_due(&self, now: OffsetDateTime) -> anyhow::Result<Summary> {
        let mut conn = self.db.get().await.context("cannot reach the database")?;
        if !db::try_lock_reminders(&mut conn).await? {
            warn!("another run is sending reminders");
            return Ok(Summary::default());
        }
        let summary = self.send_due_locked(&mut conn, now).await;
        let unlocked = db::unlock_reminders(&mut conn).await;
        let summary = summary?;
        unlocked.context("failed to release the reminders lock")?;
        Ok(summary)
    }

    async fn send_due_locked(
        &self,
        conn: &mut AsyncPgConnection,
        now: OffsetDateTime,
    ) -> anyhow::Result<Summary> {
        let query = IssuerQuery {
            tier: Some(IssuerTier::Sandbox),
            state: Some(IssuerState::Active),
            ..IssuerQuery::default()
        };
        let due: Vec<Due> = self
            .admin_api
            .first_issuers(&query, usize::MAX)
            .await
            .context("cannot list the sandbox issuers")?
            .into_iter()
            .filter_map(|issuer| Due::new(issuer, now))
            .collect();
        if due.is_empty() {
            return Ok(Summary::default());
        }
        let gh_ids: Vec<i64> = due.iter().map(|due| due.gh_id).collect();
        let owners: HashMap<i64, User> = User::find_by_github_ids(conn, &gh_ids)
            .await?
            .into_iter()
            .map(|user| (user.gh_id, user))
            .collect();
        self.mailer.check().await?;

        let mut outcomes = Vec::with_capacity(due.len());
        for due in &due {
            let Some(owner) = owners.get(&due.gh_id) else {
                // Only the portal registers sandbox issuers, for accounts that signed in.
                warn!(issuer.id = %due.issuer.id, "no portal account owns the issuer");
                continue;
            };
            outcomes.extend(self.remind(conn, due, owner, now).await?);
        }
        let count = |wanted| {
            outcomes
                .iter()
                .filter(|&&outcome| outcome == wanted)
                .count()
        };
        Ok(Summary {
            sent: count(Outcome::Sent),
            retrying: count(Outcome::Retrying),
            given_up: count(Outcome::GivenUp),
        })
    }

    /// Sends the issuer's due reminder, unless it was sent or given up. The delivery is
    /// counted first, so a run that dies before it records the outcome counts it too.
    async fn remind(
        &self,
        conn: &mut AsyncPgConnection,
        due: &Due,
        owner: &User,
        now: OffsetDateTime,
    ) -> anyhow::Result<Option<Outcome>> {
        let reminder = NewReminder {
            issuer_id: &due.issuer.id,
            window_days: due.window_days,
            expires_at: due.expires_at,
            user_id: owner.id,
        };
        let Some(claim) = reminder.claim(conn, MAX_ATTEMPTS).await? else {
            return Ok(None);
        };
        let email = ExpiryEmail::new(&due.issuer, due.expires_at, now, self.origin)?
            .into_email(&owner.email, claim.id)?;
        let outcome = match self.mailer.send(email).await {
            Ok(()) => {
                claim.sent(conn).await?;
                info!(
                    reminder.id = claim.id,
                    reminder.window_days = due.window_days,
                    reminder.attempt = claim.attempt,
                    issuer.id = %due.issuer.id,
                    user.id = owner.id,
                    "sent an expiry reminder"
                );
                Outcome::Sent
            }
            Err(failure) => {
                self.record_failure(conn, due, owner, claim, &failure)
                    .await?
            }
        };
        Ok(Some(outcome))
    }

    async fn record_failure(
        &self,
        conn: &mut AsyncPgConnection,
        due: &Due,
        owner: &User,
        claim: Claim,
        failure: &SendError,
    ) -> anyhow::Result<Outcome> {
        let give_up = failure.is_permanent() || claim.attempt >= MAX_ATTEMPTS;
        let message: String = failure
            .to_string()
            .chars()
            .take(LAST_ERROR_MAX_CHARS)
            .collect();
        claim.failed(conn, &message, give_up).await?;
        // The SMTP server's answer can name the recipient, so the logs carry its code only;
        // the database keeps the answer.
        let smtp_code = failure.smtp_code();
        if give_up {
            error!(
                reminder.id = claim.id,
                reminder.window_days = due.window_days,
                reminder.attempt = claim.attempt,
                issuer.id = %due.issuer.id,
                user.id = owner.id,
                smtp.code = smtp_code,
                permanent = failure.is_permanent(),
                "gave up an expiry reminder"
            );
            Ok(Outcome::GivenUp)
        } else {
            warn!(
                reminder.id = claim.id,
                reminder.window_days = due.window_days,
                reminder.attempt = claim.attempt,
                issuer.id = %due.issuer.id,
                user.id = owner.id,
                smtp.code = smtp_code,
                "failed to send an expiry reminder; the next run tries again"
            );
            Ok(Outcome::Retrying)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::admin_api::Owner;

    fn issuer(owner: Option<Owner>, expires_at: Option<OffsetDateTime>) -> Issuer {
        serde_json::from_value(serde_json::json!({
            "id": "issuer-1",
            "aggregation_id": "aggregation-1",
            "source": "portal",
            "tier": "sandbox",
            "state": "active",
            "owner": owner,
            "created_at": null,
            "expires_at": expires_at.map(OffsetDateTime::unix_timestamp),
            "keys": [],
            "usage": { "lists": 0, "lists_limit": 10, "keys": 0, "keys_limit": null },
        }))
        .unwrap()
    }

    fn window_at(left: Duration) -> Option<i16> {
        let now = OffsetDateTime::from_unix_timestamp(1_800_000_000).unwrap();
        Due::new(issuer(Some(Owner::github(42)), Some(now + left)), now).map(|due| due.window_days)
    }

    #[test]
    fn the_nearest_window_the_expiry_falls_within_is_due() {
        assert_eq!(window_at(Duration::days(8)), None);
        assert_eq!(window_at(Duration::days(7)), Some(7));
        assert_eq!(window_at(Duration::days(2)), Some(7));
        assert_eq!(window_at(Duration::hours(24)), Some(1));
        assert_eq!(window_at(Duration::minutes(5)), Some(1));
        assert_eq!(window_at(Duration::ZERO), None);
        assert_eq!(window_at(-Duration::hours(1)), None);
    }

    #[test]
    fn issuers_without_a_github_owner_or_expiry_are_never_due() {
        let now = OffsetDateTime::from_unix_timestamp(1_800_000_000).unwrap();
        let soon = Some(now + Duration::days(1));

        assert!(Due::new(issuer(None, soon), now).is_none());
        assert!(Due::new(issuer(Some(Owner::github(42)), None), now).is_none());
    }
}
