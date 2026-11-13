use std::borrow::Cow;

use axum::Json;
use axum::response::{IntoResponse, Response};
use derive_more::Display;
use http::StatusCode;
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
    /// No matching registration exists.
    RegistrationNotFound,
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
    /// The request body could not be read.
    BodyReadFailed,
    /// The request has an unsupported content type.
    UnsupportedMediaType,
    /// The request body exceeds the size limit.
    PayloadTooLarge,
    /// The Status List Server did not complete the request.
    BadGateway,
    /// A dependency is temporarily unavailable.
    ServiceUnavailable,
    /// An unexpected server error occurred.
    InternalError,
}

impl ApiErrorCode {
    pub fn status(self) -> StatusCode {
        use ApiErrorCode::*;
        match self {
            Unauthorized => StatusCode::UNAUTHORIZED,
            NotFound | RegistrationNotFound => StatusCode::NOT_FOUND,
            MethodNotAllowed => StatusCode::METHOD_NOT_ALLOWED,
            CrossOriginForbidden | TermsNotAccepted | RegistrationLimitReached => {
                StatusCode::FORBIDDEN
            }
            TermsChanged
            | ActiveIssuerLimitReached
            | KeyMaterialInUse
            | IssuerChanged
            | IssuerNotRenewable
            | RegistrationBeingDecided
            | RegistrationAlreadyDecided => StatusCode::CONFLICT,
            InvalidPublicKey
            | PrivateKeySubmitted
            | InvalidDisplayName
            | InvalidRejectionReason
            | InvalidRequestBody => StatusCode::UNPROCESSABLE_ENTITY,
            InvalidJson | BodyReadFailed => StatusCode::BAD_REQUEST,
            UnsupportedMediaType => StatusCode::UNSUPPORTED_MEDIA_TYPE,
            PayloadTooLarge => StatusCode::PAYLOAD_TOO_LARGE,
            BadGateway => StatusCode::BAD_GATEWAY,
            SandboxIssuerLimitReached | IssuerRegistrationInProgress | ServiceUnavailable => {
                StatusCode::SERVICE_UNAVAILABLE
            }
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
