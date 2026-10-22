//! Probes for the orchestrator.

use std::time::Duration;

use anyhow::Context;
use axum::Json;
use diesel_async::RunQueryDsl;
use serde::Serialize;
use tracing::warn;
use utoipa::ToSchema;

use crate::db;
use crate::server::ServerContext;
use crate::util::errors::{self, ApiErrorResponse, AppResult};

/// A database that takes longer to answer counts as unreachable. Orchestrators give up on a
/// probe after a second or two.
const READINESS_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Debug, Serialize, ToSchema)]
pub struct HealthResponse {
    #[schema(example = "ok")]
    status: &'static str,
}

const OK: HealthResponse = HealthResponse { status: "ok" };

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
    Json(OK)
}

/// Readiness probe: the server can reach its database.
#[utoipa::path(
    get,
    path = "/health/ready",
    tag = "health",
    responses(
        (status = 200, description = "The server can handle requests.", body = inline(HealthResponse)),
        (status = 503, description = "The database cannot be reached.", body = ApiErrorResponse),
    ),
)]
pub async fn ready(ctx: ServerContext) -> AppResult<Json<HealthResponse>> {
    match ping(&ctx.db).await {
        Ok(()) => Ok(Json(OK)),
        Err(error) => {
            warn!(error = %format_args!("{error:#}"), "database is unreachable");
            Err(errors::service_unavailable())
        }
    }
}

async fn ping(pool: &db::Pool) -> anyhow::Result<()> {
    let query = async {
        let mut conn = pool.get().await?;
        diesel::sql_query("SELECT 1").execute(&mut conn).await?;
        anyhow::Ok(())
    };
    tokio::time::timeout(READINESS_TIMEOUT, query)
        .await
        .context("the database did not answer in time")?
}
