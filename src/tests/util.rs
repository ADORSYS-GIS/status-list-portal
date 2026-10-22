mod test_app;
mod test_db;

pub use test_app::{ORIGIN, TestApp};
pub use test_db::TestDatabase;

use axum::body::{Body, Bytes};
use axum_extra::extract::cookie::Cookie;
use http::header::{AsHeaderName, COOKIE, SET_COOKIE};
use http::{HeaderMap, Method, Request, StatusCode};
use status_list_portal::models::User;
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

    /// The cookie the response sets under `name`, if any.
    pub fn cookie(&self, name: &str) -> Option<Cookie<'static>> {
        self.headers
            .get_all(SET_COOKIE)
            .iter()
            .filter_map(|value| value.to_str().ok())
            .filter_map(|value| Cookie::parse(value.to_owned()).ok())
            .find(|cookie| cookie.name() == name)
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

    async fn delete(&self, path: &str) -> Response {
        let request = self
            .request_builder(Method::DELETE, path)
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

/// A signed-in user.
pub struct MockCookieUser {
    app: TestApp,
    user: User,
    /// `Cookie` header that carries the user's session.
    cookie: String,
}

impl MockCookieUser {
    pub fn user(&self) -> &User {
        &self.user
    }
}

impl RequestHelper for MockCookieUser {
    fn app(&self) -> &TestApp {
        &self.app
    }

    fn request_builder(&self, method: Method, path: &str) -> http::request::Builder {
        Request::builder()
            .method(method)
            .uri(path)
            .header(COOKIE, &self.cookie)
    }
}
