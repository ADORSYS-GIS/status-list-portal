//! A token bucket: a client may act as long as a token is left, and tokens come back one per
//! refill interval, up to the burst.

use time::{Duration, OffsetDateTime};

use crate::config::RateLimit;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Bucket {
    pub(super) tokens: u32,
    /// When `tokens` was last counted.
    pub(super) refilled_at: OffsetDateTime,
}

impl Bucket {
    pub(super) fn full(limit: RateLimit, now: OffsetDateTime) -> Self {
        Self {
            tokens: limit.burst.get(),
            refilled_at: now,
        }
    }

    /// Takes a token at `now`. Returns the bucket that is left, or how long until a token
    /// comes back.
    pub(super) fn take(self, limit: RateLimit, now: OffsetDateTime) -> Result<Self, Duration> {
        let refilled = self.refill(limit, now);
        match refilled.tokens.checked_sub(1) {
            Some(tokens) => Ok(Self { tokens, ..refilled }),
            None => Err(refilled.refilled_at + limit.refill_interval() - now),
        }
    }

    /// Adds the tokens that came back since the last count. A full bucket starts counting
    /// again from `now`, so that tokens do not pile up beyond the burst.
    fn refill(self, limit: RateLimit, now: OffsetDateTime) -> Self {
        let interval = limit.refill_interval();
        // A count from the future, written by a clock that is ahead or went back since, starts
        // again from `now`: the wait never exceeds one interval.
        let refilled_at = self.refilled_at.min(now);
        let elapsed = now - refilled_at;
        let intervals =
            u32::try_from(elapsed.whole_seconds() / interval.whole_seconds()).unwrap_or(u32::MAX);
        let tokens = self.tokens.saturating_add(intervals);
        if tokens >= limit.burst.get() {
            Self::full(limit, now)
        } else {
            Self {
                tokens,
                refilled_at: refilled_at + interval * intervals,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::datetime;

    /// Three in a row, then one a minute.
    const LIMIT: RateLimit = RateLimit::new(3, 60);
    const START: OffsetDateTime = datetime!(2026-10-25 12:00 UTC);

    #[test]
    fn a_full_bucket_allows_a_burst() {
        let mut bucket = Bucket::full(LIMIT, START);
        for left in [2, 1, 0] {
            bucket = bucket.take(LIMIT, START).unwrap();
            assert_eq!(bucket.tokens, left);
        }

        assert_eq!(bucket.take(LIMIT, START), Err(Duration::seconds(60)));
        let later = START + Duration::seconds(45);
        assert_eq!(bucket.take(LIMIT, later), Err(Duration::seconds(15)));
    }

    #[test]
    fn tokens_come_back_one_interval_at_a_time() {
        let empty = Bucket {
            tokens: 0,
            refilled_at: START,
        };

        let after = empty.take(LIMIT, START + Duration::seconds(150)).unwrap();

        // Two intervals passed: one token is taken, one is left, and the half interval
        // still counts towards the next.
        assert_eq!(
            after,
            Bucket {
                tokens: 1,
                refilled_at: START + Duration::seconds(120),
            }
        );
    }

    #[test]
    fn tokens_do_not_pile_up_beyond_the_burst() {
        let bucket = Bucket {
            tokens: 1,
            refilled_at: START,
        };
        let now = START + Duration::days(1);

        assert_eq!(bucket.take(LIMIT, now).unwrap().tokens, 2);
    }

    #[test]
    fn a_lower_burst_applies_at_once() {
        let bucket = Bucket::full(RateLimit::new(10, 60), START);
        assert_eq!(bucket.take(LIMIT, START).unwrap().tokens, 2);
    }

    #[test]
    fn a_clock_that_goes_back_waits_one_interval_at_most() {
        let bucket = Bucket {
            tokens: 0,
            refilled_at: START,
        };
        let earlier = START - Duration::hours(1);
        assert_eq!(bucket.take(LIMIT, earlier), Err(Duration::seconds(60)));
    }
}
