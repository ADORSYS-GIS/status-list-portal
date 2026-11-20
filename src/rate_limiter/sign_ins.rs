//! The sign-in limits, per client network and for all of them, kept in memory: each replica
//! counts the sign-ins it serves.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv6Addr};
use std::sync::{Mutex, PoisonError};

use time::{Duration, OffsetDateTime};

use super::bucket::Bucket;
use crate::config::RateLimit;

/// The most client networks counted at once. Networks whose buckets are full again are
/// forgotten to make room. When none are, a new network is let through without being
/// counted: a client that changes networks escapes a per-network limit anyway, and refusing
/// would lock out everyone else.
const MAX_NETWORKS: usize = 10_000;

/// The IPv6 prefix that one client controls: providers hand out /64 networks.
const IPV6_CLIENT_PREFIX: u32 = 64;

#[derive(Debug)]
pub(super) struct SignInLimiter {
    limit: RateLimit,
    total_limit: RateLimit,
    max_networks: usize,
    buckets: Mutex<Buckets>,
}

#[derive(Debug, Default)]
struct Buckets {
    by_network: HashMap<IpAddr, Bucket>,
    /// Every network's sign-ins, including those let through uncounted per network.
    total: Option<Bucket>,
    /// When full buckets were last forgotten. Looking for them takes a while, so it happens
    /// at most once per refill interval.
    swept_at: Option<OffsetDateTime>,
}

impl SignInLimiter {
    pub(super) fn new(limit: RateLimit, total_limit: RateLimit) -> Self {
        Self::with_capacity(limit, total_limit, MAX_NETWORKS)
    }

    fn with_capacity(limit: RateLimit, total_limit: RateLimit, max_networks: usize) -> Self {
        Self {
            limit,
            total_limit,
            max_networks,
            buckets: Mutex::default(),
        }
    }

    /// Takes a token for `client`'s network, then one for the replica, at `now`. Returns how
    /// long until one comes back if either has none left.
    pub(super) fn take(&self, client: IpAddr, now: OffsetDateTime) -> Result<(), Duration> {
        let network = network(client);
        let mut buckets = self.buckets.lock().unwrap_or_else(PoisonError::into_inner);
        if buckets.by_network.contains_key(&network) || self.has_room(&mut buckets, now) {
            let bucket = buckets
                .by_network
                .get(&network)
                .copied()
                .unwrap_or_else(|| Bucket::full(self.limit, now));
            buckets
                .by_network
                .insert(network, bucket.take(self.limit, now)?);
        }
        let total = buckets
            .total
            .unwrap_or_else(|| Bucket::full(self.total_limit, now));
        buckets.total = Some(total.take(self.total_limit, now)?);
        Ok(())
    }

    /// Whether another network can be counted, after forgetting full buckets if it is time.
    fn has_room(&self, buckets: &mut Buckets, now: OffsetDateTime) -> bool {
        if buckets.by_network.len() < self.max_networks {
            return true;
        }
        let due = buckets
            .swept_at
            .is_none_or(|swept_at| now - swept_at >= self.limit.refill_interval());
        if due {
            buckets
                .by_network
                .retain(|_, bucket| !bucket.is_full(self.limit, now));
            buckets.swept_at = Some(now);
        }
        buckets.by_network.len() < self.max_networks
    }
}

/// The network `address` belongs to: the address itself for IPv4, its /64 for IPv6.
fn network(address: IpAddr) -> IpAddr {
    match address {
        IpAddr::V4(_) => address,
        IpAddr::V6(address) => {
            let prefix = address.to_bits() & (u128::MAX << (128 - IPV6_CLIENT_PREFIX));
            IpAddr::V6(Ipv6Addr::from_bits(prefix))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::datetime;

    /// Two in a row, then one a minute.
    const LIMIT: RateLimit = RateLimit::new(2, 60);
    /// High enough not to matter, except where a test says otherwise.
    const NO_TOTAL: RateLimit = RateLimit::new(1_000, 1);
    const NOW: OffsetDateTime = datetime!(2026-10-25 12:00 UTC);

    fn address(input: &str) -> IpAddr {
        input.parse().unwrap()
    }

    #[test]
    fn each_network_has_its_own_limit() {
        let limiter = SignInLimiter::new(LIMIT, NO_TOTAL);
        let client = address("203.0.113.7");

        assert_eq!(limiter.take(client, NOW), Ok(()));
        assert_eq!(limiter.take(client, NOW), Ok(()));
        assert_eq!(limiter.take(client, NOW), Err(Duration::seconds(60)));
        assert_eq!(limiter.take(address("203.0.113.8"), NOW), Ok(()));
        assert_eq!(limiter.take(client, NOW + Duration::seconds(60)), Ok(()));
    }

    #[test]
    fn an_ipv6_client_is_its_64_network() {
        let limiter = SignInLimiter::new(LIMIT, NO_TOTAL);

        assert_eq!(limiter.take(address("2001:db8:1:2::1"), NOW), Ok(()));
        assert_eq!(limiter.take(address("2001:db8:1:2:ffff::9"), NOW), Ok(()));
        assert!(limiter.take(address("2001:db8:1:2::abcd"), NOW).is_err());
        assert_eq!(limiter.take(address("2001:db8:1:3::1"), NOW), Ok(()));
    }

    #[test]
    fn networks_whose_buckets_refilled_make_room() {
        let limiter = SignInLimiter::with_capacity(LIMIT, NO_TOTAL, 2);
        limiter.take(address("192.0.2.1"), NOW).unwrap();
        limiter.take(address("192.0.2.2"), NOW).unwrap();

        // Both networks still have a token out, so a third is let through uncounted.
        let third = address("192.0.2.3");
        for _ in 0..5 {
            assert_eq!(limiter.take(third, NOW), Ok(()));
        }
        // Once the others' buckets are full again, they are forgotten and it is counted.
        let later = NOW + Duration::seconds(60);
        assert_eq!(limiter.take(third, later), Ok(()));
        assert_eq!(limiter.take(third, later), Ok(()));
        assert_eq!(limiter.take(third, later), Err(Duration::seconds(60)));
    }

    #[test]
    fn full_buckets_are_looked_for_at_most_once_per_interval() {
        let limiter = SignInLimiter::with_capacity(LIMIT, NO_TOTAL, 1);
        let counted = |network| {
            limiter
                .buckets
                .lock()
                .unwrap()
                .by_network
                .contains_key(&network)
        };
        let first = address("192.0.2.1");
        limiter.take(first, NOW).unwrap();
        // The table is full, and a sweep finds nothing to forget yet.
        limiter
            .take(address("192.0.2.2"), NOW + Duration::seconds(30))
            .unwrap();

        // The first bucket is full again, but the last sweep was too recent to look again.
        let third = address("192.0.2.3");
        limiter.take(third, NOW + Duration::seconds(60)).unwrap();
        assert!(counted(first) && !counted(third));

        limiter.take(third, NOW + Duration::seconds(90)).unwrap();
        assert!(!counted(first) && counted(third));
    }

    #[test]
    fn all_networks_share_the_replica_limit() {
        let limiter = SignInLimiter::with_capacity(LIMIT, RateLimit::new(3, 60), 2);
        limiter.take(address("192.0.2.1"), NOW).unwrap();
        limiter.take(address("192.0.2.2"), NOW).unwrap();

        // The third network is past the table's capacity, so only the replica's limit counts it.
        let third = address("192.0.2.3");
        assert_eq!(limiter.take(third, NOW), Ok(()));
        assert_eq!(limiter.take(third, NOW), Err(Duration::seconds(60)));
        assert_eq!(
            limiter.take(address("192.0.2.4"), NOW),
            Err(Duration::seconds(60))
        );
    }
}
