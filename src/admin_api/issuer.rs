//! Issuers as the admin API reads and writes them.
//!
//! The enums are closed, like the contract's: a value added to the contract later makes the
//! response fail to parse until the portal learns it.

use derive_more::Display;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

/// Where an issuer comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IssuerSource {
    /// Listed in the server's registry file; the admin API never changes it.
    Static,
    /// Created through the admin API, by the portal.
    Portal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IssuerTier {
    Standard,
    Sandbox,
}

/// Only `active` issuers can write status lists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IssuerState {
    Active,
    Suspended,
    Expired,
}

/// The JWS algorithms the server accepts, for issuer keys and admin keys alike.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Display)]
pub enum SigningAlgorithm {
    #[serde(rename = "ES256")]
    #[display("ES256")]
    Es256,
    #[serde(rename = "ES384")]
    #[display("ES384")]
    Es384,
    #[serde(rename = "EdDSA")]
    #[display("EdDSA")]
    EdDsa,
    #[serde(rename = "PS256")]
    #[display("PS256")]
    Ps256,
}

impl From<SigningAlgorithm> for jsonwebtoken::Algorithm {
    fn from(algorithm: SigningAlgorithm) -> Self {
        match algorithm {
            SigningAlgorithm::Es256 => Self::ES256,
            SigningAlgorithm::Es384 => Self::ES384,
            SigningAlgorithm::EdDsa => Self::EdDSA,
            SigningAlgorithm::Ps256 => Self::PS256,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum KeyType {
    #[serde(rename = "EC")]
    Ec,
    #[serde(rename = "OKP")]
    Okp,
    #[serde(rename = "RSA")]
    Rsa,
}

/// A public JSON Web Key (RFC 7517). It has no field for a private member, so none can be
/// sent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublicJwk {
    pub kty: KeyType,
    /// `P-256` or `P-384` for `EC`, `Ed25519` for `OKP`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub crv: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub y: Option<String>,
    /// RSA modulus.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub n: Option<String>,
    /// RSA exponent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub e: Option<String>,
}

/// The portal account that owns an issuer, as `<provider>|<subject>`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Owner(String);

impl Owner {
    /// A GitHub account, by its numeric id, which stays the same when the login is renamed.
    pub fn github(id: i64) -> Self {
        Self(format!("github|{id}"))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Issuer {
    /// The `iss` of the issuer's management tokens.
    pub id: String,
    /// Public id in the issuer's `aggregation_uri`.
    pub aggregation_id: String,
    #[serde(default)]
    pub aggregation_uri: Option<String>,
    #[serde(default)]
    pub display_name: Option<String>,
    pub source: IssuerSource,
    pub tier: IssuerTier,
    pub state: IssuerState,
    /// `None` for static issuers.
    pub owner: Option<Owner>,
    #[serde(with = "time::serde::timestamp::option")]
    pub created_at: Option<OffsetDateTime>,
    /// `None` when the issuer never expires.
    #[serde(with = "time::serde::timestamp::option")]
    pub expires_at: Option<OffsetDateTime>,
    /// Every key, revoked ones included, newest first.
    pub keys: Vec<IssuerKey>,
    pub usage: IssuerUsage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum KeyState {
    Active,
    Revoked,
}

/// A key accepts tokens only while it is `active` and the time is within
/// `[not_before, not_after]`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct IssuerKey {
    pub kid: String,
    pub alg: SigningAlgorithm,
    pub state: KeyState,
    #[serde(with = "time::serde::timestamp::option")]
    pub not_before: Option<OffsetDateTime>,
    #[serde(with = "time::serde::timestamp::option")]
    pub not_after: Option<OffsetDateTime>,
    #[serde(with = "time::serde::timestamp::option")]
    pub created_at: Option<OffsetDateTime>,
    pub public_key: PublicJwk,
}

/// What the issuer uses of its tier's limits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub struct IssuerUsage {
    pub lists: u32,
    pub lists_limit: u32,
    /// Keys that are not revoked.
    pub keys: u32,
    pub keys_limit: u32,
}

/// One page of issuers, ordered by `id`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct IssuerPage {
    pub issuers: Vec<Issuer>,
    /// Pass it as [`IssuerQuery::cursor`] to read the next page. `None` on the last page.
    pub next_cursor: Option<String>,
}

/// Which issuers to list. Every filter is optional.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct IssuerQuery {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<IssuerSource>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tier: Option<IssuerTier>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<IssuerState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner: Option<Owner>,
    /// Issuers per page, from 1 to 100. The server's default is 50.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<u8>,
    /// The previous page's `next_cursor`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
}

/// A sandbox issuer to create. The server assigns its `id`, `aggregation_id` and key `kid`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NewIssuer {
    pub display_name: String,
    pub public_key: PublicJwk,
    pub alg: SigningAlgorithm,
    /// Defaults to the longest lifetime the sandbox tier allows, which it may not exceed.
    #[serde(
        skip_serializing_if = "Option::is_none",
        with = "time::serde::timestamp::option"
    )]
    pub expires_at: Option<OffsetDateTime>,
}

/// The state an issuer can be put in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum RequestedState {
    /// Lifts a suspension. An expired issuer stays expired until it is renewed.
    Active,
    Suspended,
}

/// One change to an issuer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IssuerChange {
    State(RequestedState),
    /// Renews the issuer until then, at most the tier's longest lifetime from now.
    ExpiresAt(#[serde(with = "time::serde::timestamp")] OffsetDateTime),
}
