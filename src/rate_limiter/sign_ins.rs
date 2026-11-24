//! The sign-in limit, per client network, kept in memory: each replica counts the sign-ins it
//! serves.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv6Addr};
use std::sync::{Mutex, PoisonError};

use time::{Duration, OffsetDateTime};

use super::bucket::Bucket;
use crate::config::RateLimit;

/// The most client networks counted at once. To count another, the network whose tokens were
/// counted longest ago is forgotten: it has had the longest to fill up again, and forgetting a
/// network only gives it its full burst back.
const MAX_NETWORKS: usize = 10_000;

/// The IPv6 prefix that one client controls: providers hand out /64 networks.
const IPV6_CLIENT_PREFIX: u32 = 64;

#[derive(Debug)]
pub(super) struct SignInLimiter {
    limit: RateLimit,
    max_networks: usize,
    by_network: Mutex<HashMap<IpAddr, Bucket>>,
}

impl SignInLimiter {
    pub(super) fn new(limit: RateLimit) -> Self {
        Self::with_capacity(limit, MAX_NETWORKS)
    }

    fn with_capacity(limit: RateLimit, max_networks: usize) -> Self {
        Self {
            limit,
            max_networks,
            by_network: Mutex::default(),
        }
    }

    /// Takes a token for `client` at `now`. Returns how long until one comes back if none is
    /// left.
    pub(super) fn take(&self, client: IpAddr, now: OffsetDateTime) -> Result<(), Duration> {
        let network = network(client);
        let mut by_network = self
            .by_network
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let bucket = match by_network.get(&network) {
            Some(bucket) => *bucket,
            None => {
                if by_network.len() >= self.max_networks {
                    forget_oldest(&mut by_network);
                }
                Bucket::full(self.limit, now)
            }
        };
        by_network.insert(network, bucket.take(self.limit, now)?);
        Ok(())
    }
}

fn forget_oldest(by_network: &mut HashMap<IpAddr, Bucket>) {
    let oldest = by_network
        .iter()
        .min_by_key(|(_, bucket)| bucket.refilled_at)
        .map(|(network, _)| *network);
    if let Some(network) = oldest {
        by_network.remove(&network);
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
    const NOW: OffsetDateTime = datetime!(2026-10-25 12:00 UTC);

    fn address(input: &str) -> IpAddr {
        input.parse().unwrap()
    }

    #[test]
    fn each_network_has_its_own_limit() {
        let limiter = SignInLimiter::new(LIMIT);
        let client = address("203.0.113.7");

        assert_eq!(limiter.take(client, NOW), Ok(()));
        assert_eq!(limiter.take(client, NOW), Ok(()));
        assert_eq!(limiter.take(client, NOW), Err(Duration::seconds(60)));
        assert_eq!(limiter.take(address("203.0.113.8"), NOW), Ok(()));
        assert_eq!(limiter.take(client, NOW + Duration::seconds(60)), Ok(()));
    }

    #[test]
    fn an_ipv6_client_is_its_64_network() {
        let limiter = SignInLimiter::new(LIMIT);

        assert_eq!(limiter.take(address("2001:db8:1:2::1"), NOW), Ok(()));
        assert_eq!(limiter.take(address("2001:db8:1:2:ffff::9"), NOW), Ok(()));
        assert!(limiter.take(address("2001:db8:1:2::abcd"), NOW).is_err());
        assert_eq!(limiter.take(address("2001:db8:1:3::1"), NOW), Ok(()));
    }

    #[test]
    fn a_network_past_capacity_is_counted_too() {
        let limiter = SignInLimiter::with_capacity(LIMIT, 1);
        limiter.take(address("192.0.2.1"), NOW).unwrap();
        let second = address("192.0.2.2");

        assert_eq!(limiter.take(second, NOW), Ok(()));
        assert_eq!(limiter.take(second, NOW), Ok(()));
        assert_eq!(limiter.take(second, NOW), Err(Duration::seconds(60)));
    }

    #[test]
    fn the_network_counted_longest_ago_makes_room() {
        let limiter = SignInLimiter::with_capacity(LIMIT, 2);
        let (first, second) = (address("192.0.2.1"), address("192.0.2.2"));
        limiter.take(first, NOW).unwrap();
        limiter.take(second, NOW + Duration::seconds(10)).unwrap();

        limiter
            .take(address("192.0.2.3"), NOW + Duration::seconds(20))
            .unwrap();

        let counted = limiter.by_network.lock().unwrap();
        assert!(!counted.contains_key(&first));
        assert!(counted.contains_key(&second));
    }
}
