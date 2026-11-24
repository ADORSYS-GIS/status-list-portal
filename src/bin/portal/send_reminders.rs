//! Emails developers whose sandbox issuers expire soon, as a scheduled job.

use anyhow::bail;
use opentelemetry::metrics::Meter;
use status_list_portal::admin_api::AdminApi;
use status_list_portal::config::ReminderConfig;
use status_list_portal::db;
use status_list_portal::email::Mailer;
use status_list_portal::reminders::Reminders;
use time::OffsetDateTime;
use tracing::info;

/// Sends the reminders due now. Fails when it gives one up, so the job shows the failure.
pub async fn run(config: ReminderConfig, meter: &Meter) -> anyhow::Result<()> {
    super::run_operator_command("send-reminders", async {
        let db = db::pool(&config.database, config.env)?;
        let admin_api = AdminApi::new(&config.admin_api, meter)?;
        let mailer = Mailer::new(&config.email, config.env)?;
        let reminders = Reminders {
            db: &db,
            admin_api: &admin_api,
            mailer: &mailer,
            origin: &config.server.origin,
        };
        let summary = reminders.send_due(OffsetDateTime::now_utc()).await?;
        info!(
            reminders.sent = summary.sent,
            reminders.retrying = summary.retrying,
            reminders.given_up = summary.given_up,
            "sent the due expiry reminders"
        );
        if summary.given_up > 0 {
            bail!("gave up {} expiry reminders", summary.given_up);
        }
        Ok(())
    })
    .await
}
