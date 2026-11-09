//! Probes for the orchestrator.

use std::time::Duration;

use axum::Json;
use diesel_async::RunQueryDsl;
use serde::Serialize;
use tokio::sync::watch;
use tokio::time::Instant;
use tracing::{info, warn};
use utoipa::ToSchema;

use crate::db;
use crate::server::ServerContext;
use crate::util::errors::{self, ApiErrorResponse, AppResult};

const READINESS_TIMEOUT: Duration = Duration::from_secs(2);
const CHECK_INTERVAL: Duration = Duration::from_secs(5);
const STALE_AFTER: Duration = Duration::from_secs(15);
const FAILURE_THRESHOLD: u8 = 2;

#[derive(Clone, Copy)]
struct ProbeResult {
    ready: bool,
    checked_at: Option<Instant>,
}

/// Readiness reflects a recent database check without waiting for the request pool.
pub struct DatabaseReadiness {
    result: watch::Receiver<ProbeResult>,
    task: tokio::task::AbortHandle,
}

impl DatabaseReadiness {
    pub fn start(pool: db::Pool) -> Self {
        let (tx, rx) = watch::channel(ProbeResult {
            ready: false,
            checked_at: None,
        });
        let task = tokio::spawn(async move {
            let mut interval = tokio::time::interval(CHECK_INTERVAL);
            let mut failures: u8 = 0;
            let mut ready = false;
            loop {
                interval.tick().await;
                if ping(&pool).await {
                    failures = 0;
                    if !ready {
                        info!("database readiness restored");
                    }
                    ready = true;
                } else {
                    failures = failures.saturating_add(1);
                    if failures >= FAILURE_THRESHOLD && ready {
                        warn!("database readiness lost");
                        ready = false;
                    }
                }
                tx.send_replace(ProbeResult {
                    ready,
                    checked_at: Some(Instant::now()),
                });
            }
        })
        .abort_handle();
        Self { result: rx, task }
    }

    pub fn is_ready(&self) -> bool {
        let result = *self.result.borrow();
        result.ready
            && result
                .checked_at
                .is_some_and(|at| at.elapsed() <= STALE_AFTER)
    }
}

impl Drop for DatabaseReadiness {
    fn drop(&mut self) {
        self.task.abort();
    }
}

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

/// Readiness probe: a recent background check reached the database.
#[utoipa::path(
    get,
    path = "/health/ready",
    tag = "health",
    responses(
        (status = 200, description = "The server can handle requests.", body = inline(HealthResponse)),
        (status = 503, description = "No recent successful database check is available.", body = ApiErrorResponse),
    ),
)]
pub async fn ready(ctx: ServerContext) -> AppResult<Json<HealthResponse>> {
    if ctx.readiness.is_ready() {
        Ok(Json(OK))
    } else {
        Err(errors::service_unavailable())
    }
}

async fn ping(pool: &db::Pool) -> bool {
    let query = async {
        let Ok(mut conn) = pool.get().await else {
            return false;
        };
        diesel::sql_query("SELECT 1")
            .execute(&mut conn)
            .await
            .is_ok()
    };
    tokio::time::timeout(READINESS_TIMEOUT, query)
        .await
        .unwrap_or(false)
}
