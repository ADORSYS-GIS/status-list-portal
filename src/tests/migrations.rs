use crate::util::TestDatabase;
use diesel_async::AsyncMigrationHarness;
use diesel_migrations::MigrationHarness;
use status_list_portal::db;

#[tokio::test]
async fn running_migrations_again_changes_nothing() {
    let test_db = TestDatabase::new().await;

    db::run_migrations(test_db.url()).await.unwrap();
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
