//! Probes for the orchestrator.

use std::time::Duration;

use anyhow::Context;
use axum::Json;
use diesel_async::RunQueryDsl;
use serde::Serialize;
use tokio::sync::watch;
use tokio::time::Instant;
use tracing::{debug, info, warn};
use utoipa::ToSchema;

use crate::db;
use crate::server::ServerContext;
use crate::util::errors::{self, ApiErrorResponse, AppResult};

/// A database check that takes longer than this counts as a failure.
const READINESS_TIMEOUT: Duration = Duration::from_secs(2);
/// Time between background database checks.
const CHECK_INTERVAL: Duration = Duration::from_secs(5);
/// A result older than this cannot establish readiness.
const STALE_AFTER: Duration = Duration::from_secs(15);
/// Consecutive failed checks required to mark a ready server unready.
const FAILURE_THRESHOLD: u8 = 2;

#[derive(Clone, Copy)]
struct ProbeResult {
    ready: bool,
    checked_at: Option<Instant>,
    failures: u8,
}

impl ProbeResult {
    fn after_check(self, succeeded: bool, checked_at: Instant) -> Self {
        let failures = if succeeded {
            0
        } else {
            self.failures.saturating_add(1)
        };
        Self {
            ready: succeeded || (self.ready && failures < FAILURE_THRESHOLD),
            checked_at: Some(checked_at),
            failures,
        }
    }

    fn is_ready_at(self, now: Instant) -> bool {
        self.ready
            && self
                .checked_at
                .is_some_and(|at| now.duration_since(at) <= STALE_AFTER)
    }
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
            failures: 0,
        });
        let task = tokio::spawn(async move {
            let mut interval = tokio::time::interval(CHECK_INTERVAL);
            let mut result = *tx.borrow();
            loop {
                interval.tick().await;
                let check = ping(&pool).await;
                let next = result.after_check(check.is_ok(), Instant::now());
                match check {
                    Ok(()) if !result.ready => {
                        info!("database readiness restored");
                    }
                    Err(error) if next.failures == FAILURE_THRESHOLD => {
                        warn!(error = %format_args!("{error:#}"), "database readiness lost");
                    }
                    Err(error) => {
                        debug!(error = %format_args!("{error:#}"), "database readiness check failed");
                    }
                    Ok(()) => {}
                }
                result = next;
                tx.send_replace(result);
            }
        })
        .abort_handle();
        Self { result: rx, task }
    }

    pub fn is_ready(&self) -> bool {
        self.result.borrow().is_ready_at(Instant::now())
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
    if ctx
        .readiness
        .as_ref()
        .is_some_and(DatabaseReadiness::is_ready)
    {
        Ok(Json(OK))
    } else {
        Err(errors::service_unavailable())
    }
}

async fn ping(pool: &db::Pool) -> anyhow::Result<()> {
    let query = async {
        let mut conn = pool
            .get()
            .await
            .context("failed to get a readiness connection")?;
        diesel::sql_query("SELECT 1")
            .execute(&mut conn)
            .await
            .context("readiness query failed")?;
        anyhow::Ok(())
    };
    tokio::time::timeout(READINESS_TIMEOUT, query)
        .await
        .context("the database did not answer in time")?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readiness_tolerates_one_failure_then_becomes_unready() {
        let now = Instant::now();
        let initial = ProbeResult {
            ready: false,
            checked_at: None,
            failures: 0,
        };
        assert!(!initial.is_ready_at(now));

        let ready = initial.after_check(true, now);
        assert!(ready.is_ready_at(now));
        let tolerated = ready.after_check(false, now + CHECK_INTERVAL);
        assert!(tolerated.is_ready_at(now + CHECK_INTERVAL));
        assert_eq!(tolerated.failures, 1);

        let unavailable = tolerated.after_check(false, now + CHECK_INTERVAL * 2);
        assert!(!unavailable.is_ready_at(now + CHECK_INTERVAL * 2));
        assert_eq!(unavailable.failures, FAILURE_THRESHOLD);

        let restored = unavailable.after_check(true, now + CHECK_INTERVAL * 3);
        assert!(restored.is_ready_at(now + CHECK_INTERVAL * 3));
        assert_eq!(restored.failures, 0);
    }

    #[test]
    fn readiness_expires_when_checks_stop() {
        let now = Instant::now();
        let result = ProbeResult {
            ready: false,
            checked_at: None,
            failures: 0,
        }
        .after_check(true, now);
        assert!(result.is_ready_at(now + STALE_AFTER));
        assert!(!result.is_ready_at(now + STALE_AFTER + Duration::from_millis(1)));
    }
}
