use axum::Router;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::controllers::health;
use crate::openapi::BaseOpenApi;
use crate::server::ServerContext;
use crate::util::errors::{self, BoxedAppError};

/// Every route the backend serves, with its `OpenAPI` description.
pub(crate) fn openapi_router() -> OpenApiRouter<ServerContext> {
    BaseOpenApi::router()
        .routes(routes!(health::live))
        .routes(routes!(health::ready))
}

pub(crate) fn build_router(ctx: ServerContext) -> Router {
    let (router, _) = openapi_router().split_for_parts();
    router.fallback(fallback).with_state(ctx)
}

async fn fallback() -> BoxedAppError {
    errors::not_found()
}
