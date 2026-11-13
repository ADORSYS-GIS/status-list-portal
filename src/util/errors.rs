//! Errors returned by request handlers.
//!
//! Handlers return [`AppResult`]. An error built with [`custom`] or one of its shorthands
//! ([`unauthorized`], [`not_found`], ...) reaches the user with its code and message.
//! `?` converts the errors of the database and of the admin API with the `From`
//! impls below. Every other error becomes a `500 Internal Server Error`: its details are
//! logged and never sent to the client.

mod json;

use std::any::type_name;
use std::borrow::Cow;
use std::error::Error;
use std::fmt;

use axum::response::{IntoResponse, Response};
use derive_more::Display;
use diesel_async::pooled_connection::deadpool::PoolError;
use tracing::{error, warn};

use crate::admin_api::{AdminApiError, ErrorCode};

pub use json::{ApiErrorCode, ApiErrorResponse, custom};

pub type BoxedAppError = Box<dyn AppError>;
pub type AppResult<T> = Result<T, BoxedAppError>;

/// An error that renders itself as an HTTP response.
pub trait AppError: Send + fmt::Display + fmt::Debug + 'static {
    fn response(&self) -> Response;
}

impl IntoResponse for BoxedAppError {
    fn into_response(self) -> Response {
        self.response()
    }
}

/// Any standard error is an internal error.
impl<E: Error + Send + 'static> AppError for E {
    fn response(&self) -> Response {
        error!(error.kind = type_name::<E>(), error.message = %describe(self), "request failed");
        json::internal_server_error()
    }
}

impl From<diesel::result::Error> for BoxedAppError {
    fn from(error: diesel::result::Error) -> Self {
        match error {
            diesel::result::Error::NotFound => not_found(),
            error => Box::new(error),
        }
    }
}

impl From<serde_json::Error> for BoxedAppError {
    fn from(error: serde_json::Error) -> Self {
        Box::new(error)
    }
}

/// No database connection is available: the database is down or every connection is busy.
impl From<PoolError> for BoxedAppError {
    fn from(error: PoolError) -> Self {
        error!(error.message = %describe(&error), "no database connection");
        service_unavailable()
    }
}

impl From<AdminApiError> for BoxedAppError {
    fn from(error: AdminApiError) -> Self {
        if matches!(error, AdminApiError::Throttled) {
            warn!(error.message = %describe(&error), "the admin API is unavailable");
            return service_unavailable();
        }
        match error.code() {
            Some(ErrorCode::IssuerNotFound) => not_found(),
            Some(ErrorCode::KeyMaterialInUse) => custom(
                ApiErrorCode::KeyMaterialInUse,
                "This public key is already registered, maybe for an issuer you deleted, since \
                 a deleted issuer keeps its key. Make a new key pair, or, if you just \
                 registered this one, check your issuers.",
            ),
            Some(ErrorCode::InvalidPublicJwk | ErrorCode::UnsupportedAlgorithm) => custom(
                ApiErrorCode::InvalidPublicKey,
                "The Status List Server does not accept this public key.",
            ),
            Some(ErrorCode::PreconditionFailed) => custom(
                ApiErrorCode::IssuerChanged,
                "The issuer changed while this request ran. Please try again.",
            ),
            Some(ErrorCode::ServiceUnavailable) => {
                warn!(error.message = %describe(&error), "the admin API is unavailable");
                service_unavailable()
            }
            _ => {
                error!(error.message = %describe(&error), "admin API call failed");
                bad_gateway(
                    "The Status List Server did not complete the request. Please try again later.",
                )
            }
        }
    }
}

/// The error and each of its causes, joined with `: `, for the logs.
pub fn describe(error: &(dyn Error + 'static)) -> String {
    std::iter::successors(Some(error), |error| (*error).source())
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(": ")
}

pub fn unauthorized() -> BoxedAppError {
    custom(
        ApiErrorCode::Unauthorized,
        "This action requires signing in",
    )
}

pub fn bad_gateway(detail: impl Into<Cow<'static, str>>) -> BoxedAppError {
    custom(ApiErrorCode::BadGateway, detail)
}

pub fn not_found() -> BoxedAppError {
    custom(ApiErrorCode::NotFound, "Not Found")
}

pub fn method_not_allowed() -> BoxedAppError {
    custom(ApiErrorCode::MethodNotAllowed, "Method Not Allowed")
}

pub fn service_unavailable() -> BoxedAppError {
    custom(ApiErrorCode::ServiceUnavailable, "Service Unavailable")
}

/// An internal error described by a message that is logged and never shown to the user.
pub fn internal(description: impl Into<String>) -> BoxedAppError {
    Box::new(InternalAppError {
        description: description.into(),
    })
}

#[derive(Debug, Display)]
#[display("{description}")]
struct InternalAppError {
    description: String,
}

impl AppError for InternalAppError {
    fn response(&self) -> Response {
        error!(error.message = %self.description, "request failed");
        json::internal_server_error()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use http::StatusCode;
    use serde_json::{Value, json};

    async fn body_json(response: Response) -> Value {
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        serde_json::from_slice(&body).unwrap()
    }

    #[tokio::test]
    async fn user_errors_carry_their_code_and_detail() {
        let response =
            custom(ApiErrorCode::InvalidDisplayName, "display name is too long").response();

        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(
            body_json(response).await,
            json!({ "errors": [{ "code": "invalid_display_name", "detail": "display name is too long" }] })
        );
    }

    #[test]
    fn shorthands_use_their_status() {
        assert_eq!(not_found().response().status(), StatusCode::NOT_FOUND);
        assert_eq!(
            method_not_allowed().response().status(),
            StatusCode::METHOD_NOT_ALLOWED
        );
        assert_eq!(
            service_unavailable().response().status(),
            StatusCode::SERVICE_UNAVAILABLE
        );
    }

    #[tokio::test]
    async fn internal_errors_hide_their_description() {
        let response = internal("database password rejected").response();

        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(
            body_json(response).await,
            json!({ "errors": [{ "code": "internal_error", "detail": "Internal Server Error" }] })
        );
    }

    #[test]
    fn missing_rows_are_not_found() {
        let error = BoxedAppError::from(diesel::result::Error::NotFound);

        assert_eq!(error.response().status(), StatusCode::NOT_FOUND);
    }

    #[test]
    fn other_database_errors_are_internal_errors() {
        let error = BoxedAppError::from(diesel::result::Error::RollbackTransaction);

        assert_eq!(error.response().status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[tokio::test]
    async fn missing_connections_are_service_unavailable() {
        let error = PoolError::Timeout(deadpool::managed::TimeoutType::Wait);

        let response = BoxedAppError::from(error).response();

        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(
            body_json(response).await,
            json!({ "errors": [{ "code": "service_unavailable", "detail": "Service Unavailable" }] })
        );
    }

    /// The mapping reads only the code, so every refusal here shares one status.
    fn refused(code: ErrorCode) -> AdminApiError {
        AdminApiError::Refused {
            status: StatusCode::CONFLICT,
            code,
            description: Some("the server's own wording".to_string()),
        }
    }

    #[tokio::test]
    async fn admin_api_codes_map_to_portal_errors() {
        for (code, portal_code, serialized_code) in [
            (
                ErrorCode::IssuerNotFound,
                ApiErrorCode::NotFound,
                "not_found",
            ),
            (
                ErrorCode::KeyMaterialInUse,
                ApiErrorCode::KeyMaterialInUse,
                "key_material_in_use",
            ),
            (
                ErrorCode::InvalidPublicJwk,
                ApiErrorCode::InvalidPublicKey,
                "invalid_public_key",
            ),
            (
                ErrorCode::UnsupportedAlgorithm,
                ApiErrorCode::InvalidPublicKey,
                "invalid_public_key",
            ),
            (
                ErrorCode::PreconditionFailed,
                ApiErrorCode::IssuerChanged,
                "issuer_changed",
            ),
            (
                ErrorCode::ServiceUnavailable,
                ApiErrorCode::ServiceUnavailable,
                "service_unavailable",
            ),
            (
                ErrorCode::InvalidToken,
                ApiErrorCode::BadGateway,
                "bad_gateway",
            ),
            (ErrorCode::Unknown, ApiErrorCode::BadGateway, "bad_gateway"),
        ] {
            let response = BoxedAppError::from(refused(code)).response();

            assert_eq!(response.status(), portal_code.status(), "{code}");
            assert_eq!(
                body_json(response).await["errors"][0]["code"],
                serialized_code,
                "{code}"
            );
        }
    }

    #[tokio::test]
    async fn admin_api_failures_keep_their_details_out_of_responses() {
        for code in [ErrorCode::KeyMaterialInUse, ErrorCode::InvalidToken] {
            let response = BoxedAppError::from(refused(code)).response();

            let body = body_json(response).await.to_string();
            assert!(!body.contains("the server's own wording"), "{code}: {body}");
        }
    }

    #[tokio::test]
    async fn standard_errors_become_internal_errors() {
        let response = std::io::Error::other("disk full").response();

        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(
            body_json(response).await,
            json!({ "errors": [{ "code": "internal_error", "detail": "Internal Server Error" }] })
        );
    }
}
