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
    SigningAlgorithm, UnknownAlgorithm,
};

use std::time::{Duration, Instant};

use anyhow::{Context, anyhow};
use derive_more::{Deref, Display};
use http::{HeaderName, StatusCode};
use opentelemetry::KeyValue;
use opentelemetry::metrics::{Histogram, Meter};
use opentelemetry::propagation::TextMapPropagator;
use opentelemetry_http::HeaderInjector;
use opentelemetry_sdk::propagation::TraceContextPropagator;
use reqwest::{RequestBuilder, Response};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use tracing::field::Empty;
use tracing::{Instrument, Span, info_span};
use tracing_opentelemetry::OpenTelemetrySpanExt;
use url::Url;

use crate::config;
use crate::metrics::{DURATION_BUCKETS, method_name};
use crate::util::errors;
use token::TokenSigner;

/// A request handler may make a few calls in a row, which must fit in the portal's own
/// request timeout of 30 seconds.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
/// The admin API is on the same private network: a listener that is down fails fast.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(1);
/// The contract's `maxLength` for an issuer id. A longer one cannot exist.
const ISSUER_ID_MAX_CHARS: usize = 255;
/// The most issuers the admin API returns in one page.
const MAX_PAGE_SIZE: u8 = 100;
/// The version of an issuer's writable state, on every response about one issuer.
const ISSUER_VERSION: HeaderName = HeaderName::from_static("issuer-version");
/// A change applies only while the issuer is at the version this names.
const IF_ISSUER_VERSION: HeaderName = HeaderName::from_static("if-issuer-version");
/// The admin API's routes, as telemetry names its calls.
const ISSUERS_ROUTE: &str = "/admin/v1/issuers";
const ISSUER_ROUTE: &str = "/admin/v1/issuers/{id}";

/// Client of the admin API, signed in as the portal's admin principal.
pub struct AdminApi {
    http: reqwest::Client,
    issuers_url: Url,
    tokens: TokenSigner,
    /// The admin API's host and port, as telemetry names the server.
    host: String,
    port: Option<u16>,
    request_duration: Histogram<f64>,
}

/// The issuers of one portal account. Another account's issuer looks as if it did not
/// exist.
pub struct OwnedIssuers<'a> {
    api: &'a AdminApi,
    owner: &'a Owner,
}

/// One of the account's issuers, as read at one version. A change applies to that version
/// only, and fails with [`ErrorCode::PreconditionFailed`] if the issuer changed since.
#[derive(Debug, Clone, PartialEq, Eq, Deref)]
pub struct OwnedIssuer {
    #[deref]
    issuer: Issuer,
    version: IssuerVersion,
}

impl OwnedIssuer {
    pub fn into_issuer(self) -> Issuer {
        self.issuer
    }
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
    /// The admin API limits how often the portal may call it, and the portal called too
    /// often.
    #[error("the admin API is throttling the portal")]
    Throttled,
    #[error("admin API request failed")]
    Request(#[source] reqwest::Error),
    #[error("failed to sign an admin token")]
    Token(#[from] jsonwebtoken::errors::Error),
}

/// Leaves out the request's URL, whose query can name an issuer's owner: an error is logged,
/// and logs can be exported.
impl From<reqwest::Error> for AdminApiError {
    fn from(error: reqwest::Error) -> Self {
        Self::Request(error.without_url())
    }
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

/// The version of an issuer's writable state, from its `Issuer-Version` header. A change sends
/// it back, and fails with [`ErrorCode::PreconditionFailed`] if the issuer changed since. Use
/// and the expiry the server derives do not change it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct IssuerVersion(String);

#[derive(Serialize)]
struct CreateIssuerRequest<'a> {
    #[serde(flatten)]
    issuer: &'a NewIssuer,
    owner: &'a Owner,
}

impl AdminApi {
    /// Builds the client, which records how long its calls take with `meter`. Fails when
    /// the signing key cannot be read or does not fit the configured algorithm.
    pub fn new(config: &config::AdminApi, meter: &Meter) -> anyhow::Result<Self> {
        let http = reqwest::Client::builder()
            .user_agent(env!("CARGO_PKG_NAME"))
            .timeout(REQUEST_TIMEOUT)
            .connect_timeout(CONNECT_TIMEOUT)
            // Admin tokens go to the admin API only: through no proxy, and not on to where
            // a redirect points.
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .context("failed to build the admin API client")?;
        let request_duration = meter
            .f64_histogram("http.client.request.duration")
            .with_description("How long calls to the admin API took.")
            .with_unit("s")
            .with_boundaries(DURATION_BUCKETS.to_vec())
            .build();
        Ok(Self {
            http,
            issuers_url: issuers_url(&config.url)?,
            tokens: TokenSigner::new(config)?,
            host: config.url.host_str().unwrap_or_default().to_owned(),
            port: config.url.port_or_known_default(),
            request_duration,
        })
    }

    /// The issuers `owner` owns.
    pub fn issuers_of<'a>(&'a self, owner: &'a Owner) -> OwnedIssuers<'a> {
        OwnedIssuers { api: self, owner }
    }

    /// One page of the issuers that match `query`, whoever owns them.
    async fn list_issuers(&self, query: &IssuerQuery) -> Result<IssuerPage, AdminApiError> {
        let request = self.http.get(self.issuers_url.clone()).query(query);
        json(self.send(ISSUERS_ROUTE, request).await?).await
    }

    /// How many issuers match `query`, whoever owns them. Counts no further than it must to
    /// tell whether there are at least `at_most`.
    pub async fn count_issuers(
        &self,
        query: &IssuerQuery,
        at_most: usize,
    ) -> Result<usize, AdminApiError> {
        Ok(self.first_issuers(query, at_most).await?.len())
    }

    /// The issuers that match `query`, whoever owns them: all of them, or at least the first
    /// `at_most`. Reads no more pages than that takes.
    pub async fn first_issuers(
        &self,
        query: &IssuerQuery,
        at_most: usize,
    ) -> Result<Vec<Issuer>, AdminApiError> {
        if at_most == 0 {
            return Ok(Vec::new());
        }
        let page_size =
            u8::try_from(at_most).map_or(MAX_PAGE_SIZE, |at_most| at_most.min(MAX_PAGE_SIZE));
        let mut query = IssuerQuery {
            limit: Some(page_size),
            ..query.clone()
        };
        let mut issuers = Vec::new();
        loop {
            let page = self.list_issuers(&query).await?;
            if page.issuers.is_empty() && page.next_cursor.is_some() {
                // Following its cursor could go on until the request times out.
                return Err(AdminApiError::Unexpected(
                    "an empty page that is not the last".to_string(),
                ));
            }
            issuers.extend(page.issuers);
            match page.next_cursor {
                Some(cursor) if issuers.len() < at_most => query.cursor = Some(cursor),
                _ => return Ok(issuers),
            }
        }
    }

    async fn create_issuer(
        &self,
        request: &CreateIssuerRequest<'_>,
    ) -> Result<Issuer, AdminApiError> {
        let request = self.http.post(self.issuers_url.clone()).json(request);
        json(self.send(ISSUERS_ROUTE, request).await?).await
    }

    /// `None` when the admin API knows no issuer with this id.
    async fn get_issuer(&self, id: &str) -> Result<Option<(Issuer, IssuerVersion)>, AdminApiError> {
        let Some(url) = self.issuer_url(id) else {
            return Ok(None);
        };
        self.found(self.http.get(url)).await
    }

    /// Changes the issuer, provided it is still at `version`.
    async fn update_issuer(
        &self,
        id: &str,
        version: &IssuerVersion,
        change: &IssuerChange,
    ) -> Result<Option<(Issuer, IssuerVersion)>, AdminApiError> {
        let Some(url) = self.issuer_url(id) else {
            return Ok(None);
        };
        let request = self
            .http
            .patch(url)
            .header(IF_ISSUER_VERSION, &version.0)
            .json(change);
        self.found(request).await
    }

    /// `None` for an id that one URL path segment cannot hold.
    fn issuer_url(&self, id: &str) -> Option<Url> {
        // A path segment drops `.` and `..`, which would leave the issuer's URL.
        if matches!(id, "" | "." | "..") || id.chars().count() > ISSUER_ID_MAX_CHARS {
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
    ) -> Result<Option<(Issuer, IssuerVersion)>, AdminApiError> {
        match self.send(ISSUER_ROUTE, request).await {
            Ok(response) => Ok(Some(with_version(response).await?)),
            Err(error) if error.code() == Some(ErrorCode::IssuerNotFound) => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// Sends the request with a fresh admin token, in a span that the admin API continues.
    /// Error responses become errors. `route` names the call in telemetry.
    async fn send(
        &self,
        route: &'static str,
        request: RequestBuilder,
    ) -> Result<Response, AdminApiError> {
        let mut request = request.bearer_auth(self.tokens.sign()?).build()?;
        let method = method_name(request.method());
        let span = info_span!(
            "admin_api",
            otel.name = format!("{method} {route}"),
            otel.kind = "client",
            otel.status_code = Empty,
            http.request.method = method,
            http.response.status_code = Empty,
            error.type = Empty,
            server.address = self.host.as_str(),
            server.port = self.port,
            url.template = route,
        );
        TraceContextPropagator::new()
            .inject_context(&span.context(), &mut HeaderInjector(request.headers_mut()));

        let started = Instant::now();
        let result = self.http.execute(request).instrument(span.clone()).await;
        self.record(&span, method, route, &result, started.elapsed());

        let response = result?;
        if response.status().is_success() {
            Ok(response)
        } else {
            Err(AdminApiError::from_response(response).await)
        }
    }

    /// Records how a call went, in its span and in the call durations.
    fn record(
        &self,
        span: &Span,
        method: &'static str,
        route: &'static str,
        result: &Result<Response, reqwest::Error>,
        duration: Duration,
    ) {
        let (status, error_type) = match result {
            Ok(response) => {
                let status = response.status();
                span.record("http.response.status_code", i64::from(status.as_u16()));
                // As for the server's requests, only a server error is one: the admin API
                // answers 404 and 409 in the normal course of things.
                let error_type = status.is_server_error().then(|| status.as_str().to_owned());
                (Some(i64::from(status.as_u16())), error_type)
            }
            Err(error) => (None, Some(error_type(error).to_owned())),
        };
        if let Some(error_type) = &error_type {
            span.record("otel.status_code", "ERROR");
            span.record("error.type", error_type.as_str());
        }
        let attributes: Vec<_> = [
            KeyValue::new("http.request.method", method),
            KeyValue::new("url.template", route),
            KeyValue::new("server.address", self.host.clone()),
        ]
        .into_iter()
        .chain(
            self.port
                .map(|port| KeyValue::new("server.port", i64::from(port))),
        )
        .chain(status.map(|status| KeyValue::new("http.response.status_code", status)))
        .chain(error_type.map(|error_type| KeyValue::new("error.type", error_type)))
        .collect();
        self.request_duration
            .record(duration.as_secs_f64(), &attributes);
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
        let IssuerPage {
            issuers,
            next_cursor,
        } = self.api.list_issuers(&query).await?;
        // The admin API filters by owner already; this keeps a mistake there from showing
        // other accounts' issuers.
        Ok(IssuerPage {
            issuers: issuers
                .into_iter()
                .filter(|issuer| self.owns(issuer))
                .collect(),
            next_cursor,
        })
    }

    /// Creates a sandbox issuer that the account owns.
    pub async fn create(&self, issuer: &NewIssuer) -> Result<Issuer, AdminApiError> {
        let request = CreateIssuerRequest {
            issuer,
            owner: self.owner,
        };
        let created = self.api.create_issuer(&request).await?;
        if !self.owns(&created) {
            return Err(AdminApiError::Unexpected(
                "an issuer created for another owner".to_string(),
            ));
        }
        Ok(created)
    }

    /// `None` when the account owns no issuer with this id.
    pub async fn get(&self, id: &str) -> Result<Option<OwnedIssuer>, AdminApiError> {
        let found = self.api.get_issuer(id).await?;
        Ok(found
            .filter(|(issuer, _)| self.owns(issuer))
            .map(|(issuer, version)| OwnedIssuer { issuer, version }))
    }

    /// Applies `change` to the version of the issuer that `current` holds. `None` when the
    /// account does not own it, or it is gone.
    pub async fn update(
        &self,
        current: &OwnedIssuer,
        change: &IssuerChange,
    ) -> Result<Option<Issuer>, AdminApiError> {
        if !self.owns(current) {
            return Ok(None);
        }
        let updated = self
            .api
            .update_issuer(&current.id, &current.version, change)
            .await?;
        Ok(updated.map(|(issuer, _)| issuer))
    }

    fn owns(&self, issuer: &Issuer) -> bool {
        issuer.owner.as_ref() == Some(self.owner)
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
        // The admin API's rate limiter answers before its handlers, without an error code.
        if status == StatusCode::TOO_MANY_REQUESTS {
            return Self::Throttled;
        }
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

/// Why a call got no response, as telemetry's `error.type` names it.
fn error_type(error: &reqwest::Error) -> &'static str {
    if error.is_timeout() {
        "timeout"
    } else if error.is_connect() {
        "connect"
    } else {
        "_OTHER"
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

/// The body of a response that carries the issuer's version.
async fn with_version<T: DeserializeOwned>(
    response: Response,
) -> Result<(T, IssuerVersion), AdminApiError> {
    let version = response
        .headers()
        .get(ISSUER_VERSION)
        .and_then(|value| value.to_str().ok())
        .map(|value| IssuerVersion(value.to_owned()))
        .ok_or_else(|| {
            AdminApiError::Unexpected("a response without an Issuer-Version".to_string())
        })?;
    Ok((json(response).await?, version))
}

/// The response's body. One that does not parse is contract drift, not a failed request.
async fn json<T: DeserializeOwned>(response: Response) -> Result<T, AdminApiError> {
    response.json().await.map_err(|error| {
        if error.is_decode() {
            AdminApiError::Unexpected(format!(
                "a body that does not match the contract: {}",
                errors::describe(&error.without_url())
            ))
        } else {
            error.into()
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::errors;

    #[tokio::test]
    async fn refused_connections_leave_out_the_url() {
        // Nothing listens on port 1.
        let error = reqwest::Client::new()
            .get("http://127.0.0.1:1/admin/v1/issuers?owner=github%7C42")
            .send()
            .await
            .unwrap_err();
        assert_eq!(error_type(&error), "connect");

        let error = AdminApiError::from(error);

        assert!(!errors::describe(&error).contains("owner"));
    }

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
