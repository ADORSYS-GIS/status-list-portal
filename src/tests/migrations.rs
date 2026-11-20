use crate::util::TestDatabase;
use diesel::migration::MigrationSource;
use diesel::pg::Pg;
use diesel_async::AsyncMigrationHarness;
use diesel_async::RunQueryDsl;
use diesel_migrations::MigrationHarness;
use status_list_portal::Env;
use status_list_portal::db;
use tokio::task::JoinSet;

const CONCURRENT_RUNS: usize = 4;

#[tokio::test]
async fn running_migrations_again_changes_nothing() {
    let test_db = TestDatabase::new().await;
    let applied = db::run_migrations(&test_db.migration_config(), Env::Test)
        .await
        .unwrap();
    assert!(applied.is_empty());
}

#[tokio::test]
async fn schema_check_rejects_missing_migrations() {
    let test_db = TestDatabase::unmigrated().await;
    let mut conn = test_db.connect().await;
    let error = db::ensure_current_schema(&mut conn).await.unwrap_err();
    assert!(
        error.to_string().contains("run portal migrate"),
        "{error:#}"
    );
    db::run_migrations(&test_db.config(), Env::Test)
        .await
        .unwrap();
    db::ensure_current_schema(&mut conn).await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn concurrent_runs_apply_each_migration_once() {
    let test_db = TestDatabase::unmigrated().await;
    let mut runs = JoinSet::new();
    for _ in 0..CONCURRENT_RUNS {
        let config = test_db.config();
        runs.spawn(async move { db::run_migrations(&config, Env::Test).await.unwrap() });
    }
    let mut applied = runs.join_all().await.concat();
    let mut expected = MigrationSource::<Pg>::migrations(&db::MIGRATIONS)
        .unwrap()
        .iter()
        .map(|migration| migration.name().version().to_string())
        .collect::<Vec<_>>();
    applied.sort();
    expected.sort();
    assert_eq!(applied, expected);
}

#[tokio::test]
async fn every_migration_can_be_reverted_and_applied_again() {
    let test_db = TestDatabase::new().await;
    let mut harness = AsyncMigrationHarness::new(test_db.admin_connect().await);
    let (reverted, applied) = tokio::task::spawn_blocking(move || {
        let reverted = harness.revert_all_migrations(db::MIGRATIONS).unwrap().len();
        let applied = harness
            .run_pending_migrations(db::MIGRATIONS)
            .unwrap()
            .len();
        (reverted, applied)
    })
    .await
    .unwrap();
    assert!(reverted > 0);
    assert_eq!(applied, reverted);
}

#[tokio::test]
async fn application_role_can_use_tables_but_cannot_change_schema() {
    let test_db = TestDatabase::new().await;
    let mut app = test_db.connect().await;
    diesel::sql_query(
        "INSERT INTO users (gh_id, gh_login, email) VALUES (1, 'test', 'test@example.com')",
    )
    .execute(&mut app)
    .await
    .unwrap();
    for (sql, expected) in [
        (
            "CREATE TABLE forbidden (id bigint)",
            "permission denied for schema public",
        ),
        (
            "ALTER TABLE users ADD COLUMN forbidden bigint",
            "must be owner of table users",
        ),
        (
            "DELETE FROM __diesel_schema_migrations",
            "permission denied for table __diesel_schema_migrations",
        ),
    ] {
        let error = diesel::sql_query(sql).execute(&mut app).await.unwrap_err();
        assert!(error.to_string().contains(expected), "{sql}: {error}");
    }
}
