use axum::extract::FromRequestParts;
use axum::extract::rejection::QueryRejection;

use super::errors::{ApiErrorCode, BoxedAppError, custom};

/// Query parameters that use the API's JSON error format when decoding fails.
#[derive(Debug, FromRequestParts)]
#[from_request(via(axum::extract::Query), rejection(BoxedAppError))]
pub struct QueryParams<T>(pub T);

impl From<QueryRejection> for BoxedAppError {
    fn from(rejection: QueryRejection) -> Self {
        custom(ApiErrorCode::InvalidQuery, rejection.body_text())
    }
}

#[cfg(test)]
mod tests {
    use axum::Router;
    use axum::body::Body;
    use axum::extract::Request;
    use axum::routing::get;
    use http::StatusCode;
    use http::header::CONTENT_TYPE;
    use serde::Deserialize;
    use serde_json::Value;
    use tower::ServiceExt;

    use super::QueryParams;

    #[derive(Deserialize)]
    struct Cursor {
        cursor: Option<String>,
    }

    async fn read(QueryParams(query): QueryParams<Cursor>) -> String {
        query.cursor.unwrap_or_default()
    }

    #[tokio::test]
    async fn valid_query_reaches_the_handler() {
        let router = Router::new().route("/issuers", get(read));
        let request = Request::get("/issuers?cursor=page-2")
            .body(Body::empty())
            .unwrap();

        let response = router.oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(body.as_ref(), b"page-2");
    }

    #[tokio::test]
    async fn duplicate_query_parameter_gets_a_coded_json_400() {
        let router = Router::new().route("/issuers", get(read));
        let request = Request::get("/issuers?cursor=one&cursor=two")
            .body(Body::empty())
            .unwrap();

        let response = router.oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(response.headers()[CONTENT_TYPE], "application/json");
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let body: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(body["errors"][0]["code"], "invalid_query");
        assert!(
            body["errors"][0]["detail"]
                .as_str()
                .is_some_and(|detail| !detail.is_empty())
        );
    }
}
