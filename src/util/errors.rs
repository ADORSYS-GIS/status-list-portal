//! Errors returned by request handlers.
//!
//! Handlers return [`AppResult`]. An error built with [`custom`] or one of its shorthands
//! ([`bad_request`], [`forbidden`], [`not_found`], ...) reaches the user with its status and
//! message. `?` converts the database's errors with the `From` impls below. Every other
//! error becomes a `500 Internal Server Error`: its details are logged and never sent to
//! the client.

mod json;

use std::any::type_name;
use std::borrow::Cow;
use std::error::Error;
use std::fmt;

use axum::response::{IntoResponse, Response};
use derive_more::Display;
use diesel_async::pooled_connection::deadpool::PoolError;
use http::StatusCode;
use tracing::error;

pub use json::{ApiErrorResponse, custom};

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

/// No database connection is available: the database is down or every connection is busy.
impl From<PoolError> for BoxedAppError {
    fn from(error: PoolError) -> Self {
        error!(error.message = %describe(&error), "no database connection");
        service_unavailable()
    }
}

/// The error and each of its causes, joined with `: `, for the logs.
pub fn describe(error: &(dyn Error + 'static)) -> String {
    std::iter::successors(Some(error), |error| (*error).source())
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(": ")
}

pub fn bad_request(detail: impl Into<Cow<'static, str>>) -> BoxedAppError {
    custom(StatusCode::BAD_REQUEST, detail)
}

pub fn unauthorized() -> BoxedAppError {
    custom(StatusCode::UNAUTHORIZED, "This action requires signing in")
}

pub fn forbidden(detail: impl Into<Cow<'static, str>>) -> BoxedAppError {
    custom(StatusCode::FORBIDDEN, detail)
}

pub fn not_found() -> BoxedAppError {
    custom(StatusCode::NOT_FOUND, "Not Found")
}

pub fn method_not_allowed() -> BoxedAppError {
    custom(StatusCode::METHOD_NOT_ALLOWED, "Method Not Allowed")
}

pub fn service_unavailable() -> BoxedAppError {
    custom(StatusCode::SERVICE_UNAVAILABLE, "Service Unavailable")
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
    use serde_json::{Value, json};

    async fn body_json(response: Response) -> Value {
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        serde_json::from_slice(&body).unwrap()
    }

    #[tokio::test]
    async fn user_errors_carry_their_status_and_detail() {
        let response = bad_request("display name is too long").response();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(
            body_json(response).await,
            json!({ "errors": [{ "detail": "display name is too long" }] })
        );
    }

    #[test]
    fn shorthands_use_their_status() {
        assert_eq!(forbidden("").response().status(), StatusCode::FORBIDDEN);
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
            json!({ "errors": [{ "detail": "Internal Server Error" }] })
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
            json!({ "errors": [{ "detail": "Service Unavailable" }] })
        );
    }

    #[tokio::test]
    async fn standard_errors_become_internal_errors() {
        let response = std::io::Error::other("disk full").response();

        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(
            body_json(response).await,
            json!({ "errors": [{ "detail": "Internal Server Error" }] })
        );
    }
}
