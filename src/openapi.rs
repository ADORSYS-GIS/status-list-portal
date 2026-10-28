//! `OpenAPI` description of the backend's API.
//!
//! The document is generated from the `#[utoipa::path]` attributes of the handlers. A
//! snapshot test keeps the committed copy in `src/snapshots/` up to date.

use utoipa::OpenApi;
use utoipa_axum::router::OpenApiRouter;

#[derive(OpenApi)]
#[openapi(info(
    title = "Status List Portal API",
    description = "API between the portal's frontend and its backend. It is not a public \
                   contract and can change in any release.",
    version = "0.0.0",
))]
pub struct BaseOpenApi;

impl BaseOpenApi {
    pub fn router<S: Send + Sync + Clone + 'static>() -> OpenApiRouter<S> {
        OpenApiRouter::with_openapi(Self::openapi())
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn openapi_snapshot() {
        let openapi = crate::router::openapi_router().into_openapi();
        insta::assert_json_snapshot!(openapi);
    }
}
