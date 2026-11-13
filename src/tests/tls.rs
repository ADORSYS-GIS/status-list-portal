use std::num::NonZeroUsize;
use std::path::PathBuf;

use secrecy::SecretString;
use status_list_portal::{Env, config, db};

use crate::util::TestDatabase;

fn database(url: &str, ca: Option<PathBuf>) -> config::Database {
    config::Database {
        url: SecretString::from(url),
        tls_ca_file: ca,
        pool_size: NonZeroUsize::MIN,
    }
}

#[test]
fn production_rejects_weak_tls_modes_before_connecting() {
    for mode in ["disable", "allow", "prefer", "verify-ca"] {
        let url = format!("postgres://localhost/portal?sslmode={mode}");
        let error = db::pool(&database(&url, None), Env::Production)
            .err()
            .unwrap();
        assert!(error.to_string().contains("must not weaken"), "{error}");
    }
}

#[tokio::test]
async fn development_can_use_plaintext() {
    let test_db = TestDatabase::new().await;
    let mut url = url::Url::parse(test_db.url()).unwrap();
    url.set_query(Some("sslmode=disable"));
    let pool = db::pool(&database(url.as_str(), None), Env::Development).unwrap();
    let _conn = pool.get().await.unwrap();
}

#[tokio::test]
async fn production_verifies_tls_ca_and_hostname() {
    let (Ok(url), Ok(ca), Ok(wrong_ca)) = (
        std::env::var("TEST_TLS_DATABASE_URL"),
        std::env::var("TEST_TLS_CA_FILE"),
        std::env::var("TEST_TLS_WRONG_CA_FILE"),
    ) else {
        // The CI job starts a separate TLS Postgres. Local tests can run without it.
        return;
    };
    let good = database(&url, Some(ca.into()));
    let _conn = db::pool(&good, Env::Production)
        .unwrap()
        .get()
        .await
        .unwrap();
    let verify_full = database(
        &format!("{url}?sslmode=verify-full"),
        good.tls_ca_file.clone(),
    );
    let _conn = db::pool(&verify_full, Env::Production)
        .unwrap()
        .get()
        .await
        .unwrap();

    let wrong_ca = database(&url, Some(wrong_ca.into()));
    assert!(
        db::pool(&wrong_ca, Env::Production)
            .unwrap()
            .get()
            .await
            .is_err()
    );

    let wrong_host = url.replace("@localhost:", "@127.0.0.1:");
    let wrong_host = database(&wrong_host, good.tls_ca_file.clone());
    assert!(
        db::pool(&wrong_host, Env::Production)
            .unwrap()
            .get()
            .await
            .is_err()
    );
}
