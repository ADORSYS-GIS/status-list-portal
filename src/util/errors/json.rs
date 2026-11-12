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
    code: &'static str,
    /// Human-readable description, safe to show to the user.
    detail: String,
}

impl ApiErrorResponse {
    fn new(code: &'static str, detail: impl Into<String>) -> Self {
        Self {
            errors: vec![ApiErrorDetail {
                code,
                detail: detail.into(),
            }],
        }
    }
}

fn json_error(status: StatusCode, code: &'static str, detail: &str) -> Response {
    (status, Json(ApiErrorResponse::new(code, detail))).into_response()
}

pub(super) fn internal_server_error() -> Response {
    json_error(
        StatusCode::INTERNAL_SERVER_ERROR,
        "internal_error",
        "Internal Server Error",
    )
}

/// An error with a stable code and a human-readable detail.
pub fn custom(
    status: StatusCode,
    code: &'static str,
    detail: impl Into<Cow<'static, str>>,
) -> BoxedAppError {
    Box::new(CustomApiError {
        status,
        code,
        detail: detail.into(),
    })
}

#[derive(Debug, Display)]
#[display("{detail}")]
struct CustomApiError {
    status: StatusCode,
    code: &'static str,
    detail: Cow<'static, str>,
}

impl AppError for CustomApiError {
    fn response(&self) -> Response {
        json_error(self.status, self.code, &self.detail)
    }
}
