use axum::Router;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::controllers::{health, issuer, session, terms, user};
use crate::openapi::BaseOpenApi;
use crate::server::ServerContext;
use crate::util::errors::{self, BoxedAppError};

/// Every route the backend serves, with its `OpenAPI` description.
pub(crate) fn openapi_router() -> OpenApiRouter<ServerContext> {
    BaseOpenApi::router()
        .routes(routes!(health::live))
        .routes(routes!(health::ready))
        .routes(routes!(session::begin))
        .routes(routes!(session::callback))
        .routes(routes!(session::sign_out))
        .routes(routes!(user::me))
        .routes(routes!(terms::get_terms))
        .routes(routes!(terms::accept_terms))
        .routes(routes!(issuer::list_issuers, issuer::create_issuer))
        .routes(routes!(issuer::get_issuer, issuer::delete_issuer))
        .routes(routes!(issuer::renew_issuer))
}

pub(crate) fn build_router(ctx: ServerContext) -> Router {
    let (router, _) = openapi_router().split_for_parts();
    router
        .fallback(fallback)
        .method_not_allowed_fallback(method_not_allowed_fallback)
        .with_state(ctx)
}

async fn fallback() -> BoxedAppError {
    errors::not_found()
}

/// axum still sets `Allow` on the response.
async fn method_not_allowed_fallback() -> BoxedAppError {
    errors::method_not_allowed()
}
