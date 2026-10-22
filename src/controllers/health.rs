//! Probes for the orchestrator.

use axum::Json;
use serde::Serialize;
use utoipa::ToSchema;

#[derive(Debug, Serialize, ToSchema)]
pub struct HealthResponse {
    #[schema(example = "ok")]
    status: &'static str,
}

/// Liveness probe: the process is up and answers requests.
#[utoipa::path(
    get,
    path = "/health/live",
    tag = "health",
    responses(
        (status = 200, description = "The server is running.", body = inline(HealthResponse)),
    ),
)]
pub async fn live() -> Json<HealthResponse> {
    Json(HealthResponse { status: "ok" })
}
