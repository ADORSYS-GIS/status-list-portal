//! Client of the Status List Server's admin API, where the portal creates and manages
//! sandbox issuers.
//!
//! The admin API is served on a private network only. Every call carries a fresh admin token
//! signed with the portal's key. Developers reach their issuers through [`OwnedIssuers`],
//! which never shows or changes another account's issuers.

mod issuer;
mod token;

pub use issuer::{
    Issuer, IssuerChange, IssuerKey, IssuerPage, IssuerQuery, IssuerSource, IssuerState,
    IssuerTier, IssuerUsage, KeyState, KeyType, NewIssuer, Owner, PublicJwk, RequestedState,
    SigningAlgorithm,
};

use std::time::Duration;

use anyhow::{Context, anyhow};
use derive_more::Display;
use http::StatusCode;
use http::header::{ETAG, IF_MATCH};
use reqwest::{RequestBuilder, Response};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use url::Url;

use crate::config;
use token::TokenSigner;

/// A request handler may make a few calls in a row, which must fit in the server's request
/// timeout.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);

/// Client of the admin API, signed in as the portal's admin principal.
pub struct AdminApi {
    http: reqwest::Client,
    issuers_url: Url,
    tokens: TokenSigner,
}

/// The issuers of one portal account. Another account's issuer looks as if it did not
/// exist.
pub struct OwnedIssuers<'a> {
    api: &'a AdminApi,
    owner: &'a Owner,
}

#[derive(Debug, thiserror::Error)]
pub enum AdminApiError {
    /// The admin API refused the call and said why.
    #[error(
        "the admin API answered {status} {code}: {}",
        description.as_deref().unwrap_or("no description")
    )]
    Refused {
        status: StatusCode,
        code: ErrorCode,
        /// Human-readable, for the logs; its wording is not part of the contract.
        description: Option<String>,
    },
    /// The admin API answered in a way its contract does not allow.
    #[error("the admin API sent {0}")]
    Unexpected(String),
    #[error("admin API request failed")]
    Request(#[from] reqwest::Error),
    #[error("failed to sign an admin token")]
    Token(#[from] jsonwebtoken::errors::Error),
}

/// Why the admin API refused a call. Branch on it, never on the description.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Display)]
#[serde(rename_all = "snake_case")]
#[display(rename_all = "snake_case")]
pub enum ErrorCode {
    InvalidQuery,
    InvalidInput,
    MissingToken,
    InvalidToken,
    InsufficientScope,
    IssuerNotFound,
    KeyNotFound,
    IssuerManagedByConfiguration,
    DynamicIssuersDisabled,
    KeyLimitReached,
    KeyMaterialInUse,
    PreconditionFailed,
    PayloadTooLarge,
    InvalidPublicJwk,
    UnsupportedAlgorithm,
    InvalidExpiry,
    PreconditionRequired,
    InternalError,
    ServiceUnavailable,
    /// A code added to the contract after this client was written.
    #[serde(other)]
    Unknown,
}

#[derive(Deserialize)]
struct ErrorResponse {
    error: ErrorCode,
    #[serde(default)]
    error_description: Option<String>,
}

/// An issuer's version, from its `ETag` header (RFC 9110 §8.8.3). A change sends it back, and
/// fails with [`ErrorCode::PreconditionFailed`] if the issuer changed since.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ETag(String);

#[derive(Serialize)]
struct CreateIssuerRequest<'a> {
    #[serde(flatten)]
    issuer: &'a NewIssuer,
    owner: &'a Owner,
}

impl AdminApi {
    /// Builds the client. Fails when the signing key cannot be read or does not fit the
    /// configured algorithm.
    pub fn new(config: &config::AdminApi) -> anyhow::Result<Self> {
        let http = reqwest::Client::builder()
            .user_agent(env!("CARGO_PKG_NAME"))
            .timeout(REQUEST_TIMEOUT)
            // Admin tokens go to the admin API only: through no proxy, and not on to where
            // a redirect points.
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .context("failed to build the admin API client")?;
        Ok(Self {
            http,
            issuers_url: issuers_url(&config.url)?,
            tokens: TokenSigner::new(config)?,
        })
    }

    /// The issuers `owner` owns.
    pub fn issuers_of<'a>(&'a self, owner: &'a Owner) -> OwnedIssuers<'a> {
        OwnedIssuers { api: self, owner }
    }

    /// One page of the issuers that match `query`, whoever owns them.
    pub async fn list_issuers(&self, query: &IssuerQuery) -> Result<IssuerPage, AdminApiError> {
        let request = self.http.get(self.issuers_url.clone()).query(query);
        Ok(self.send(request).await?.json().await?)
    }

    async fn create_issuer(
        &self,
        request: &CreateIssuerRequest<'_>,
    ) -> Result<Issuer, AdminApiError> {
        let request = self.http.post(self.issuers_url.clone()).json(request);
        Ok(self.send(request).await?.json().await?)
    }

    /// `None` when the admin API knows no issuer with this id.
    async fn get_issuer(&self, id: &str) -> Result<Option<(Issuer, ETag)>, AdminApiError> {
        let Some(url) = self.issuer_url(id) else {
            return Ok(None);
        };
        self.found(self.http.get(url)).await
    }

    /// Changes the issuer, provided it is still at version `etag`.
    async fn update_issuer(
        &self,
        id: &str,
        etag: &ETag,
        change: &IssuerChange,
    ) -> Result<Option<(Issuer, ETag)>, AdminApiError> {
        let Some(url) = self.issuer_url(id) else {
            return Ok(None);
        };
        let request = self.http.patch(url).header(IF_MATCH, &etag.0).json(change);
        self.found(request).await
    }

    /// `None` for an id that one URL path segment cannot hold.
    fn issuer_url(&self, id: &str) -> Option<Url> {
        // A path segment drops `.` and `..`, which would leave the issuer's URL.
        if matches!(id, "" | "." | "..") {
            return None;
        }
        let mut url = self.issuers_url.clone();
        url.path_segments_mut()
            .expect("`issuers_url` added segments to this URL")
            .push(id);
        Some(url)
    }

    /// Sends a request about one issuer. `None` when the admin API knows no issuer with its
    /// id.
    async fn found(
        &self,
        request: RequestBuilder,
    ) -> Result<Option<(Issuer, ETag)>, AdminApiError> {
        match self.send(request).await {
            Ok(response) => Ok(Some(with_etag(response).await?)),
            Err(error) if error.code() == Some(ErrorCode::IssuerNotFound) => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// Sends the request with a fresh admin token. Error responses become errors.
    async fn send(&self, request: RequestBuilder) -> Result<Response, AdminApiError> {
        let response = request.bearer_auth(self.tokens.sign()?).send().await?;
        if response.status().is_success() {
            Ok(response)
        } else {
            Err(AdminApiError::from_response(response).await)
        }
    }
}

impl OwnedIssuers<'_> {
    /// One page of the account's issuers. `cursor` is the previous page's `next_cursor`.
    pub async fn list(&self, cursor: Option<&str>) -> Result<IssuerPage, AdminApiError> {
        let query = IssuerQuery {
            owner: Some(self.owner.clone()),
            cursor: cursor.map(str::to_owned),
            ..IssuerQuery::default()
        };
        self.api.list_issuers(&query).await
    }

    /// Creates a sandbox issuer that the account owns.
    pub async fn create(&self, issuer: &NewIssuer) -> Result<Issuer, AdminApiError> {
        let request = CreateIssuerRequest {
            issuer,
            owner: self.owner,
        };
        self.api.create_issuer(&request).await
    }

    /// `None` when the account owns no issuer with this id.
    pub async fn get(&self, id: &str) -> Result<Option<Issuer>, AdminApiError> {
        let issuer = self.get_with_etag(id).await?;
        Ok(issuer.map(|(issuer, _)| issuer))
    }

    /// Changes the issuer. `None` when the account owns no issuer with this id.
    pub async fn update(
        &self,
        id: &str,
        change: &IssuerChange,
    ) -> Result<Option<Issuer>, AdminApiError> {
        let Some((_, etag)) = self.get_with_etag(id).await? else {
            return Ok(None);
        };
        // The ETag pins the change to the version whose owner was just checked.
        let issuer = self.api.update_issuer(id, &etag, change).await?;
        Ok(issuer.map(|(issuer, _)| issuer))
    }

    async fn get_with_etag(&self, id: &str) -> Result<Option<(Issuer, ETag)>, AdminApiError> {
        let issuer = self.api.get_issuer(id).await?;
        Ok(issuer.filter(|(issuer, _)| issuer.owner.as_ref() == Some(self.owner)))
    }
}

impl AdminApiError {
    /// The code the admin API refused the call with, if it did.
    pub fn code(&self) -> Option<ErrorCode> {
        match self {
            Self::Refused { code, .. } => Some(*code),
            _ => None,
        }
    }

    async fn from_response(response: Response) -> Self {
        let status = response.status();
        match response.json::<ErrorResponse>().await {
            Ok(body) => Self::Refused {
                status,
                code: body.error,
                description: body.error_description,
            },
            Err(_) => Self::Unexpected(format!("{status} without an error code")),
        }
    }
}

/// The admin API's issuers collection under `base`, which may have a path of its own.
fn issuers_url(base: &Url) -> anyhow::Result<Url> {
    let mut url = base.clone();
    url.path_segments_mut()
        .map_err(|()| anyhow!("the admin API URL {base} has no path"))?
        .pop_if_empty()
        .extend(["admin", "v1", "issuers"]);
    Ok(url)
}

/// The body of a response that carries the issuer's `ETag`.
async fn with_etag<T: DeserializeOwned>(response: Response) -> Result<(T, ETag), AdminApiError> {
    let etag = response
        .headers()
        .get(ETAG)
        .and_then(|value| value.to_str().ok())
        .map(|value| ETag(value.to_owned()))
        .ok_or_else(|| AdminApiError::Unexpected("a response without an ETag".to_string()))?;
    Ok((response.json().await?, etag))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_issuers_live_under_the_admin_api_url() {
        for (base, expected) in [
            ("http://admin:8002", "http://admin:8002/admin/v1/issuers"),
            ("http://admin:8002/", "http://admin:8002/admin/v1/issuers"),
            (
                "http://admin/status",
                "http://admin/status/admin/v1/issuers",
            ),
            (
                "http://admin/status/",
                "http://admin/status/admin/v1/issuers",
            ),
        ] {
            let url = issuers_url(&Url::parse(base).unwrap()).unwrap();

            assert_eq!(url.as_str(), expected);
        }
    }
}
