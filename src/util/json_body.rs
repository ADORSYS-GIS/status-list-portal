use axum::extract::FromRequest;
use axum::extract::rejection::JsonRejection;

use super::errors::{BoxedAppError, custom};

/// A JSON request body. Unlike `axum::Json`, a body it cannot read gets the API's error
/// format.
#[derive(Debug, FromRequest)]
#[from_request(via(axum::Json), rejection(BoxedAppError))]
pub struct JsonBody<T>(pub T);

impl From<JsonRejection> for BoxedAppError {
    fn from(rejection: JsonRejection) -> Self {
        custom(rejection.status(), rejection.body_text())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::Router;
    use axum::body::Body;
    use axum::extract::Request;
    use axum::response::Response;
    use axum::routing::post;
    use http::StatusCode;
    use http::header::CONTENT_TYPE;
    use serde::Deserialize;
    use serde_json::Value;
    use tower::ServiceExt;

    use crate::middleware::apply_middleware;

    #[derive(Deserialize)]
    struct Greeting {
        name: String,
    }

    async fn greet(JsonBody(greeting): JsonBody<Greeting>) -> String {
        greeting.name
    }

    async fn send(content_type: &str, body: impl Into<Body>) -> Response {
        let router = apply_middleware(Router::new().route("/greet", post(greet)));
        let request = Request::post("/greet")
            .header(CONTENT_TYPE, content_type)
            .body(body.into())
            .unwrap();
        router.oneshot(request).await.unwrap()
    }

    /// Asserts the response is a JSON error with this status, and returns its detail.
    async fn error_detail(response: Response, status: StatusCode) -> String {
        assert_eq!(response.status(), status);
        assert_eq!(response.headers()[CONTENT_TYPE], "application/json");
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let body: Value = serde_json::from_slice(&body).unwrap();
        body["errors"][0]["detail"].as_str().unwrap().to_string()
    }

    #[tokio::test]
    async fn readable_bodies_reach_the_handler() {
        let response = send("application/json", r#"{"name":"Ada"}"#).await;

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn bodies_of_the_wrong_shape_get_a_json_422() {
        let response = send("application/json", r#"{"name":1}"#).await;

        let detail = error_detail(response, StatusCode::UNPROCESSABLE_ENTITY).await;
        assert!(detail.contains("name"), "{detail}");
    }

    #[tokio::test]
    async fn malformed_bodies_get_a_json_400() {
        let response = send("application/json", "{").await;

        error_detail(response, StatusCode::BAD_REQUEST).await;
    }

    #[tokio::test]
    async fn other_content_types_get_a_json_415() {
        let response = send("text/plain", r#"{"name":"Ada"}"#).await;

        error_detail(response, StatusCode::UNSUPPORTED_MEDIA_TYPE).await;
    }

    #[tokio::test]
    async fn bodies_over_the_limit_get_a_json_413() {
        let name = "a".repeat(20 * 1024);
        let response = send("application/json", format!(r#"{{"name":"{name}"}}"#)).await;

        error_detail(response, StatusCode::PAYLOAD_TOO_LARGE).await;
    }
}
