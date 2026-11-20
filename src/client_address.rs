//! The address of the client that sent a request.
//!
//! Behind a proxy, every request comes from the proxy, which names the client in
//! `X-Forwarded-For`. Anyone can send that header, so the portal reads it only from the
//! proxies it is told to trust (`APP_SERVER__TRUSTED_PROXIES`), and only as far back as
//! trusted proxies wrote it.

use std::net::{IpAddr, SocketAddr};

use axum::extract::{ConnectInfo, FromRequestParts};
use http::HeaderMap;
use http::request::Parts;
use ipnet::IpNet;
use serde::{Deserialize, Deserializer, de};

use crate::server::ServerContext;
use crate::util::errors::{self, BoxedAppError};

const X_FORWARDED_FOR: &str = "x-forwarded-for";

/// The networks of the proxies in front of the portal: addresses or CIDR blocks, separated
/// by commas, such as `10.0.0.0/8, 192.168.1.10`. None by default.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TrustedProxies(Vec<IpNet>);

impl TrustedProxies {
    /// Refuses a network that holds every address: then any client could name itself.
    pub fn parse(input: &str) -> Result<Self, String> {
        input
            .split(',')
            .map(str::trim)
            .filter(|entry| !entry.is_empty())
            .map(|entry| {
                let network = entry
                    .parse::<IpNet>()
                    .or_else(|_| entry.parse::<IpAddr>().map(IpNet::from))
                    .map_err(|_| format!("{entry:?} is not an address or a CIDR block"))?;
                if network.prefix_len() == 0 {
                    return Err(format!(
                        "{entry:?} holds every address, so any client could name itself"
                    ));
                }
                Ok(network)
            })
            .collect::<Result<_, _>>()
            .map(Self)
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    fn contains(&self, address: IpAddr) -> bool {
        self.0.iter().any(|network| network.contains(&address))
    }
}

impl<'de> Deserialize<'de> for TrustedProxies {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let input = String::deserialize(deserializer)?;
        Self::parse(&input).map_err(de::Error::custom)
    }
}

/// The address of the client that sent the request: the peer, unless the peer is a trusted
/// proxy, in which case the address it forwarded the request for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClientAddress(pub IpAddr);

impl FromRequestParts<ServerContext> for ClientAddress {
    type Rejection = BoxedAppError;

    async fn from_request_parts(
        parts: &mut Parts,
        ctx: &ServerContext,
    ) -> Result<Self, Self::Rejection> {
        let ConnectInfo(peer) = ConnectInfo::<SocketAddr>::from_request_parts(parts, ctx)
            .await
            .map_err(|_| errors::internal("the server does not record peer addresses"))?;
        let trusted = &ctx.config.server.trusted_proxies;
        Ok(Self(client_address(peer.ip(), &parts.headers, trusted)))
    }
}

/// Walks `X-Forwarded-For` from its end, where the nearest proxy wrote, and stops at the
/// first address that is not a trusted proxy's. Anything before it was written by the
/// client and could be made up.
fn client_address(peer: IpAddr, headers: &HeaderMap, trusted: &TrustedProxies) -> IpAddr {
    let peer = peer.to_canonical();
    if !trusted.contains(peer) {
        return peer;
    }
    // A header line that is not text cannot be read, so the walk ends there.
    let hops: Vec<Option<&str>> = headers
        .get_all(X_FORWARDED_FOR)
        .iter()
        .flat_map(|value| match value.to_str() {
            Ok(value) => value.split(',').map(|hop| Some(hop.trim())).collect(),
            Err(_) => vec![None],
        })
        .collect();
    let mut client = peer;
    for hop in hops.into_iter().rev() {
        let Some(address) = hop.and_then(parse_hop) else {
            break;
        };
        client = address;
        if !trusted.contains(address) {
            break;
        }
    }
    client
}

/// An `X-Forwarded-For` entry: an address, perhaps with a port.
fn parse_hop(hop: &str) -> Option<IpAddr> {
    hop.parse::<IpAddr>()
        .or_else(|_| hop.parse::<SocketAddr>().map(|address| address.ip()))
        .ok()
        .map(|address| address.to_canonical())
}

#[cfg(test)]
mod tests {
    use super::*;
    use http::HeaderValue;

    fn address(input: &str) -> IpAddr {
        input.parse().unwrap()
    }

    fn forwarded_for(values: &[&str]) -> HeaderMap {
        values
            .iter()
            .map(|value| {
                (
                    http::HeaderName::from_static(X_FORWARDED_FOR),
                    HeaderValue::from_str(value).unwrap(),
                )
            })
            .collect()
    }

    #[test]
    fn proxies_are_addresses_or_networks() {
        let trusted = TrustedProxies::parse(" 10.0.0.0/8, 192.168.1.10 ,fd00::/8,").unwrap();

        assert!(trusted.contains(address("10.20.30.40")));
        assert!(trusted.contains(address("192.168.1.10")));
        assert!(!trusted.contains(address("192.168.1.11")));
        assert!(trusted.contains(address("fd12::1")));
        assert_eq!(
            TrustedProxies::parse("").unwrap(),
            TrustedProxies::default()
        );
        assert_eq!(
            TrustedProxies::parse("10.0.0.0/8, proxy").unwrap_err(),
            r#""proxy" is not an address or a CIDR block"#
        );
        for everything in ["0.0.0.0/0", "::/0"] {
            let error = TrustedProxies::parse(everything).unwrap_err();
            assert!(
                error.ends_with("so any client could name itself"),
                "{error}"
            );
        }
    }

    #[test]
    fn unreadable_header_lines_end_the_walk() {
        let trusted = TrustedProxies::parse("10.0.0.0/8").unwrap();
        let mut headers = forwarded_for(&["203.0.113.7"]);
        let unreadable = HeaderValue::from_bytes(b"198.51.100.4\xff").unwrap();
        headers.append(http::HeaderName::from_static(X_FORWARDED_FOR), unreadable);
        headers.append(
            http::HeaderName::from_static(X_FORWARDED_FOR),
            HeaderValue::from_static("10.1.1.1"),
        );

        let client = client_address(address("10.2.2.2"), &headers, &trusted);

        assert_eq!(client, address("10.1.1.1"));
    }

    #[test]
    fn untrusted_peers_are_the_client() {
        let headers = forwarded_for(&["203.0.113.7"]);
        let client = client_address(
            address("198.51.100.1"),
            &headers,
            &TrustedProxies::default(),
        );
        assert_eq!(client, address("198.51.100.1"));
    }

    #[test]
    fn trusted_proxies_name_the_client() {
        let trusted = TrustedProxies::parse("10.0.0.0/8").unwrap();
        // The client made up the first entry; the proxies appended the rest.
        let headers = forwarded_for(&["192.0.2.66, 203.0.113.7", "10.1.1.1"]);

        let client = client_address(address("10.2.2.2"), &headers, &trusted);

        assert_eq!(client, address("203.0.113.7"));
    }

    #[test]
    fn unreadable_entries_end_the_walk() {
        let trusted = TrustedProxies::parse("10.0.0.0/8").unwrap();
        let headers = forwarded_for(&["203.0.113.7, unknown, 10.1.1.1"]);

        let client = client_address(address("10.2.2.2"), &headers, &trusted);

        assert_eq!(client, address("10.1.1.1"));
    }

    #[test]
    fn ports_and_mapped_addresses_are_read() {
        let trusted = TrustedProxies::parse("10.0.0.0/8").unwrap();
        let headers = forwarded_for(&["203.0.113.7:51234"]);

        let client = client_address(address("::ffff:10.2.2.2"), &headers, &trusted);

        assert_eq!(client, address("203.0.113.7"));
    }

    #[test]
    fn a_trusted_proxy_without_the_header_is_the_client() {
        let trusted = TrustedProxies::parse("10.0.0.0/8").unwrap();
        let client = client_address(address("10.2.2.2"), &HeaderMap::new(), &trusted);
        assert_eq!(client, address("10.2.2.2"));
    }
}
