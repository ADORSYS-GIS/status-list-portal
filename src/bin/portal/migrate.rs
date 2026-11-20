//! Runs the database migrations as a standalone operator job.

use status_list_portal::config::MigrationConfig;
use status_list_portal::db;

pub async fn run(config: MigrationConfig) -> anyhow::Result<()> {
    super::run_operator_command("migrate", async {
        db::run_migrations(&config.database, config.env)
            .await
            .map(|_| ())
    })
    .await
}
