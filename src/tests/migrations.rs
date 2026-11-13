use crate::util::TestDatabase;
use diesel::migration::MigrationSource;
use diesel::pg::Pg;
use diesel_async::AsyncMigrationHarness;
use diesel_async::{AsyncConnection, RunQueryDsl};
use diesel_migrations::MigrationHarness;
use status_list_portal::Env;
use status_list_portal::db;
use tokio::task::JoinSet;

/// Instances that start at once in `concurrent_runs_apply_each_migration_once`.
const CONCURRENT_RUNS: usize = 4;

#[tokio::test]
async fn running_migrations_again_changes_nothing() {
    let test_db = TestDatabase::new().await;

    let applied = db::run_migrations(&test_db.config(), Env::Test)
        .await
        .unwrap();

    assert_eq!(applied, Vec::<String>::new());
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

#[tokio::test]
async fn application_role_can_use_tables_but_cannot_change_schema() {
    let test_db = TestDatabase::unmigrated().await;
    let app_role = format!("portal_app_{}", std::process::id());
    let migration_role = format!("portal_migrator_{}", std::process::id());
    let mut admin = test_db.connect().await;
    for role in [&app_role, &migration_role] {
        diesel::sql_query(format!("CREATE ROLE {role} LOGIN PASSWORD 'test-password'"))
            .execute(&mut admin)
            .await
            .unwrap();
    }
    for grant in [
        format!(
            "GRANT CONNECT ON DATABASE \"{}\" TO {app_role}, {migration_role}",
            test_db.name()
        ),
        format!("GRANT USAGE ON SCHEMA public TO {app_role}, {migration_role}"),
        format!("GRANT CREATE ON SCHEMA public TO {migration_role}"),
        format!(
            "ALTER DEFAULT PRIVILEGES FOR ROLE {migration_role} IN SCHEMA public GRANT SELECT, INSERT, UPDATE, DELETE ON TABLES TO {app_role}"
        ),
        format!(
            "ALTER DEFAULT PRIVILEGES FOR ROLE {migration_role} IN SCHEMA public GRANT USAGE, SELECT ON SEQUENCES TO {app_role}"
        ),
    ] {
        diesel::sql_query(grant).execute(&mut admin).await.unwrap();
    }

    let mut maintenance_url = url::Url::parse(test_db.url()).unwrap();
    maintenance_url.set_path("postgres");
    let mut migration_url = url::Url::parse(test_db.url()).unwrap();
    migration_url.set_username(&migration_role).unwrap();
    migration_url.set_password(Some("test-password")).unwrap();
    let migration_config = status_list_portal::config::Database {
        url: secrecy::SecretString::from(migration_url.as_str()),
        tls_ca_file: None,
        pool_size: std::num::NonZeroUsize::MIN,
    };
    assert!(
        !db::run_migrations(&migration_config, Env::Test)
            .await
            .unwrap()
            .is_empty()
    );

    let mut app_url = url::Url::parse(test_db.url()).unwrap();
    app_url.set_username(&app_role).unwrap();
    app_url.set_password(Some("test-password")).unwrap();
    let mut app = diesel_async::AsyncPgConnection::establish(app_url.as_str())
        .await
        .unwrap();
    diesel::sql_query(
        "INSERT INTO users (gh_id, gh_login, email) VALUES (1, 'test', 'test@example.com')",
    )
    .execute(&mut app)
    .await
    .unwrap();
    assert!(
        diesel::sql_query("CREATE TABLE forbidden (id bigint)")
            .execute(&mut app)
            .await
            .is_err()
    );
    assert!(
        diesel::sql_query("ALTER TABLE users ADD COLUMN forbidden bigint")
            .execute(&mut app)
            .await
            .is_err()
    );

    drop(app);
    drop(admin);
    drop(test_db);
    let mut maintenance = diesel_async::AsyncPgConnection::establish(maintenance_url.as_str())
        .await
        .unwrap();
    for role in [&app_role, &migration_role] {
        diesel::sql_query(format!("DROP ROLE {role}"))
            .execute(&mut maintenance)
            .await
            .unwrap();
    }
}
