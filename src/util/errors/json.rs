use std::borrow::Cow;

use axum::Json;
use axum::response::{IntoResponse, Response};
use derive_more::Display;
use http::StatusCode;
use http::header::RETRY_AFTER;
use serde::Serialize;
use utoipa::ToSchema;

use super::{AppError, BoxedAppError};

/// The body of every error response.
#[derive(Debug, Serialize, ToSchema)]
pub struct ApiErrorResponse {
    /// What prevented the request from succeeding. Always exactly one entry.
    errors: Vec<ApiErrorDetail>,
}

#[derive(Debug, Serialize, ToSchema)]
struct ApiErrorDetail {
    /// Stable machine-readable reason. Clients may branch on this value.
    code: ApiErrorCode,
    /// Human-readable description, safe to show to the user.
    detail: String,
}

/// Stable reasons an API request can fail. The detail may change without changing the code.
/// New codes may be added. Clients must handle an unknown code as they would its HTTP status.
#[derive(Clone, Copy, Debug, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ApiErrorCode {
    /// The request requires a signed-in user.
    Unauthorized,
    /// The requested resource does not exist or is not visible to the user.
    NotFound,
    /// The HTTP method is not supported for this resource.
    MethodNotAllowed,
    /// The request came from a disallowed origin.
    CrossOriginForbidden,
    /// The current terms of use have not been accepted.
    TermsNotAccepted,
    /// The terms changed since the user read them.
    TermsChanged,
    /// The account has reached its lifetime registration limit.
    RegistrationLimitReached,
    /// The account has reached its active issuer limit.
    ActiveIssuerLimitReached,
    /// The sandbox has reached its active issuer limit.
    SandboxIssuerLimitReached,
    /// Another issuer registration currently holds the limits lock.
    IssuerRegistrationInProgress,
    /// The public key is already registered.
    KeyMaterialInUse,
    /// The issuer has reached its tier's key limit.
    KeyLimitReached,
    /// The public key is malformed or incompatible with the chosen algorithm.
    InvalidPublicKey,
    /// The submitted key contains private key material.
    PrivateKeySubmitted,
    /// The issuer display name is invalid.
    InvalidDisplayName,
    /// The issuer changed during the request.
    IssuerChanged,
    /// The issuer cannot be renewed in its current state.
    IssuerNotRenewable,
    /// Another decision is in progress for the registration.
    RegistrationBeingDecided,
    /// The registration has already been decided.
    RegistrationAlreadyDecided,
    /// An operator's rejection reason is invalid.
    InvalidRejectionReason,
    /// The JSON request body has invalid syntax.
    InvalidJson,
    /// The JSON body does not match the expected shape.
    InvalidRequestBody,
    /// The query string cannot be decoded into the expected parameters.
    InvalidQuery,
    /// The request body could not be read.
    BodyReadFailed,
    /// The request has an unsupported content type.
    UnsupportedMediaType,
    /// The request body exceeds the size limit.
    PayloadTooLarge,
    /// Too many requests of this kind in a short time. `Retry-After` says when to try again.
    RateLimited,
    /// The Status List Server did not complete the request.
    BadGateway,
    /// A dependency is temporarily unavailable.
    ServiceUnavailable,
    /// The portal stopped waiting for the request. Check whether a mutation succeeded before retrying.
    RequestTimedOut,
    /// An unexpected server error occurred.
    InternalError,
}

impl ApiErrorCode {
    pub fn status(self) -> StatusCode {
        use ApiErrorCode::*;
        match self {
            Unauthorized => StatusCode::UNAUTHORIZED,
            NotFound => StatusCode::NOT_FOUND,
            MethodNotAllowed => StatusCode::METHOD_NOT_ALLOWED,
            CrossOriginForbidden | TermsNotAccepted | RegistrationLimitReached => {
                StatusCode::FORBIDDEN
            }
            TermsChanged
            | ActiveIssuerLimitReached
            | KeyMaterialInUse
            | KeyLimitReached
            | IssuerChanged
            | IssuerNotRenewable
            | RegistrationBeingDecided
            | RegistrationAlreadyDecided => StatusCode::CONFLICT,
            InvalidPublicKey
            | PrivateKeySubmitted
            | InvalidDisplayName
            | InvalidRejectionReason
            | InvalidRequestBody => StatusCode::UNPROCESSABLE_ENTITY,
            InvalidJson | InvalidQuery | BodyReadFailed => StatusCode::BAD_REQUEST,
            UnsupportedMediaType => StatusCode::UNSUPPORTED_MEDIA_TYPE,
            PayloadTooLarge => StatusCode::PAYLOAD_TOO_LARGE,
            RateLimited => StatusCode::TOO_MANY_REQUESTS,
            BadGateway => StatusCode::BAD_GATEWAY,
            SandboxIssuerLimitReached
            | IssuerRegistrationInProgress
            | ServiceUnavailable
            | RequestTimedOut => StatusCode::SERVICE_UNAVAILABLE,
            InternalError => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl ApiErrorResponse {
    fn new(code: ApiErrorCode, detail: impl Into<String>) -> Self {
        Self {
            errors: vec![ApiErrorDetail {
                code,
                detail: detail.into(),
            }],
        }
    }
}

fn json_error(code: ApiErrorCode, detail: &str) -> Response {
    (code.status(), Json(ApiErrorResponse::new(code, detail))).into_response()
}

pub(super) fn internal_server_error() -> Response {
    json_error(ApiErrorCode::InternalError, "Internal Server Error")
}

/// An error with a stable code and a human-readable detail.
pub fn custom(code: ApiErrorCode, detail: impl Into<Cow<'static, str>>) -> BoxedAppError {
    Box::new(CustomApiError {
        code,
        detail: detail.into(),
    })
}

#[derive(Debug, Display)]
#[display("{detail}")]
struct CustomApiError {
    code: ApiErrorCode,
    detail: Cow<'static, str>,
}

impl AppError for CustomApiError {
    fn response(&self) -> Response {
        json_error(self.code, &self.detail)
    }
}

/// Too many requests in a short time: `429 Too Many Requests`, with `Retry-After` saying in how
/// many seconds to try again.
pub fn too_many_requests(detail: &'static str, retry_after: time::Duration) -> BoxedAppError {
    Box::new(TooManyRequests {
        detail,
        retry_after,
    })
}

#[derive(Debug, Display)]
#[display("{detail}")]
struct TooManyRequests {
    detail: &'static str,
    retry_after: time::Duration,
}

impl AppError for TooManyRequests {
    fn response(&self) -> Response {
        // Rounded up, so that a client that waits as long finds a token.
        let whole = self.retry_after.whole_seconds();
        let seconds = if self.retry_after > time::Duration::seconds(whole) {
            whole + 1
        } else {
            whole
        };
        let detail = format!("{} Please try again later.", self.detail);
        let code = ApiErrorCode::RateLimited;
        let body = Json(ApiErrorResponse::new(code, detail));
        let retry_after = [(RETRY_AFTER, seconds.max(1).to_string())];
        (code.status(), retry_after, body).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn published_codes_keep_their_wire_names_and_http_statuses() {
        use ApiErrorCode::*;

        for (code, wire_name, status) in [
            (Unauthorized, "unauthorized", StatusCode::UNAUTHORIZED),
            (NotFound, "not_found", StatusCode::NOT_FOUND),
            (
                MethodNotAllowed,
                "method_not_allowed",
                StatusCode::METHOD_NOT_ALLOWED,
            ),
            (
                CrossOriginForbidden,
                "cross_origin_forbidden",
                StatusCode::FORBIDDEN,
            ),
            (
                TermsNotAccepted,
                "terms_not_accepted",
                StatusCode::FORBIDDEN,
            ),
            (TermsChanged, "terms_changed", StatusCode::CONFLICT),
            (
                RegistrationLimitReached,
                "registration_limit_reached",
                StatusCode::FORBIDDEN,
            ),
            (
                ActiveIssuerLimitReached,
                "active_issuer_limit_reached",
                StatusCode::CONFLICT,
            ),
            (
                SandboxIssuerLimitReached,
                "sandbox_issuer_limit_reached",
                StatusCode::SERVICE_UNAVAILABLE,
            ),
            (
                IssuerRegistrationInProgress,
                "issuer_registration_in_progress",
                StatusCode::SERVICE_UNAVAILABLE,
            ),
            (
                KeyMaterialInUse,
                "key_material_in_use",
                StatusCode::CONFLICT,
            ),
            (KeyLimitReached, "key_limit_reached", StatusCode::CONFLICT),
            (
                InvalidPublicKey,
                "invalid_public_key",
                StatusCode::UNPROCESSABLE_ENTITY,
            ),
            (
                PrivateKeySubmitted,
                "private_key_submitted",
                StatusCode::UNPROCESSABLE_ENTITY,
            ),
            (
                InvalidDisplayName,
                "invalid_display_name",
                StatusCode::UNPROCESSABLE_ENTITY,
            ),
            (IssuerChanged, "issuer_changed", StatusCode::CONFLICT),
            (
                IssuerNotRenewable,
                "issuer_not_renewable",
                StatusCode::CONFLICT,
            ),
            (
                RegistrationBeingDecided,
                "registration_being_decided",
                StatusCode::CONFLICT,
            ),
            (
                RegistrationAlreadyDecided,
                "registration_already_decided",
                StatusCode::CONFLICT,
            ),
            (
                InvalidRejectionReason,
                "invalid_rejection_reason",
                StatusCode::UNPROCESSABLE_ENTITY,
            ),
            (InvalidJson, "invalid_json", StatusCode::BAD_REQUEST),
            (
                InvalidRequestBody,
                "invalid_request_body",
                StatusCode::UNPROCESSABLE_ENTITY,
            ),
            (InvalidQuery, "invalid_query", StatusCode::BAD_REQUEST),
            (BodyReadFailed, "body_read_failed", StatusCode::BAD_REQUEST),
            (
                UnsupportedMediaType,
                "unsupported_media_type",
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
            ),
            (
                PayloadTooLarge,
                "payload_too_large",
                StatusCode::PAYLOAD_TOO_LARGE,
            ),
            (RateLimited, "rate_limited", StatusCode::TOO_MANY_REQUESTS),
            (BadGateway, "bad_gateway", StatusCode::BAD_GATEWAY),
            (
                ServiceUnavailable,
                "service_unavailable",
                StatusCode::SERVICE_UNAVAILABLE,
            ),
            (
                RequestTimedOut,
                "request_timed_out",
                StatusCode::SERVICE_UNAVAILABLE,
            ),
            (
                InternalError,
                "internal_error",
                StatusCode::INTERNAL_SERVER_ERROR,
            ),
        ] {
            assert_eq!(serde_json::to_value(code).unwrap(), wire_name);
            assert_eq!(code.status(), status, "{wire_name}");
        }
    }
}
