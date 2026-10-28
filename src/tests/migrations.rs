use crate::util::TestDatabase;
use diesel::migration::MigrationSource;
use diesel::pg::Pg;
use diesel_async::AsyncMigrationHarness;
use diesel_migrations::MigrationHarness;
use status_list_portal::db;
use tokio::task::JoinSet;

/// Instances that start at once in `concurrent_runs_apply_each_migration_once`.
const CONCURRENT_RUNS: usize = 4;

#[tokio::test]
async fn running_migrations_again_changes_nothing() {
    let test_db = TestDatabase::new().await;

    let applied = db::run_migrations(test_db.url()).await.unwrap();

    assert_eq!(applied, Vec::<String>::new());
}

#[tokio::test(flavor = "multi_thread")]
async fn concurrent_runs_apply_each_migration_once() {
    let test_db = TestDatabase::unmigrated().await;
    let mut runs = JoinSet::new();
    for _ in 0..CONCURRENT_RUNS {
        let url = test_db.url().to_string();
        runs.spawn(async move { db::run_migrations(&url).await.unwrap() });
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
    let mut harness = AsyncMigrationHarness::new(test_db.connect().await);

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
