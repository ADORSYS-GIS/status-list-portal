mod test_app;

pub use test_app::TestApp;

use axum::body::{Body, Bytes};
use http::header::AsHeaderName;
use http::{HeaderMap, Method, Request, StatusCode};
use tower::ServiceExt;

/// A response with its body read.
pub struct Response {
    status: StatusCode,
    headers: HeaderMap,
    body: Bytes,
}

impl Response {
    pub fn status(&self) -> StatusCode {
        self.status
    }

    pub fn header(&self, name: impl AsHeaderName) -> Option<&str> {
        self.headers.get(name).and_then(|value| value.to_str().ok())
    }

    #[track_caller]
    pub fn json(&self) -> serde_json::Value {
        serde_json::from_slice(&self.body).expect("response body is not JSON")
    }
}

/// Sends requests to the app on behalf of a user.
pub trait RequestHelper {
    fn app(&self) -> &TestApp;

    /// Starts a request carrying this user's credentials.
    fn request_builder(&self, method: Method, path: &str) -> http::request::Builder {
        Request::builder().method(method).uri(path)
    }

    async fn run(&self, request: Request<Body>) -> Response {
        let response = self
            .app()
            .router()
            .clone()
            .oneshot(request)
            .await
            .expect("the router never fails");
        let (parts, body) = response.into_parts();
        let body = axum::body::to_bytes(body, usize::MAX)
            .await
            .expect("failed to read the response body");
        Response {
            status: parts.status,
            headers: parts.headers,
            body,
        }
    }

    async fn get(&self, path: &str) -> Response {
        let request = self
            .request_builder(Method::GET, path)
            .body(Body::empty())
            .expect("invalid request");
        self.run(request).await
    }
}

/// A user who is not signed in.
pub struct MockAnonymousUser {
    app: TestApp,
}

impl RequestHelper for MockAnonymousUser {
    fn app(&self) -> &TestApp {
        &self.app
    }
}
