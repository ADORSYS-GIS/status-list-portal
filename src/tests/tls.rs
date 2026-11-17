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
fn production_rejects_remote_plaintext_and_unsupported_modes() {
    for url in [
        "postgres://db.example/portal?sslmode=disable",
        "host=localhost hostaddr=10.0.0.1 sslmode=disable",
    ] {
        let error = db::pool(&database(url, None), Env::Production)
            .err()
            .unwrap();
        assert!(
            error
                .to_string()
                .contains("only for Unix sockets or loopback"),
            "{error}"
        );
    }
    for mode in ["allow", "verify-ca", "verify-full"] {
        let url = format!("postgres://db.example/portal?sslmode={mode}");
        let error = db::pool(&database(&url, None), Env::Production)
            .err()
            .unwrap();
        assert!(
            error.to_string().contains("invalid APP_DATABASE__URL"),
            "{error}"
        );
    }
}

#[tokio::test]
async fn development_can_use_plaintext() {
    let test_db = TestDatabase::new().await;
    let mut url = url::Url::parse(test_db.url()).unwrap();
    url.set_query(Some("sslmode=disable"));
    let pool = db::pool(&database(url.as_str(), None), Env::Development).unwrap();
    let _conn = pool.get().await.unwrap();

    let _conn = db::pool(&database(url.as_str(), None), Env::Production)
        .unwrap()
        .get()
        .await
        .unwrap();

    url.set_query(None);
    let error = db::pool(&database(url.as_str(), None), Env::Production)
        .unwrap()
        .get()
        .await
        .err()
        .expect("plaintext Postgres unexpectedly accepted production TLS");
    let detail = format!("{error:?}");
    assert!(detail.contains("TLS") || detail.contains("ssl"), "{detail}");
}

#[tokio::test]
async fn production_verifies_tls_ca_and_hostname() {
    let (Ok(url), Ok(ca), Ok(wrong_ca)) = (
        std::env::var("TEST_TLS_DATABASE_URL"),
        std::env::var("TEST_TLS_CA_FILE"),
        std::env::var("TEST_TLS_WRONG_CA_FILE"),
    ) else {
        if std::env::var_os("CI").is_some() {
            panic!(
                "CI must provide TEST_TLS_DATABASE_URL, TEST_TLS_CA_FILE and TEST_TLS_WRONG_CA_FILE"
            );
        }
        eprintln!(
            "skipping TLS integration test: set TEST_TLS_DATABASE_URL, TEST_TLS_CA_FILE and TEST_TLS_WRONG_CA_FILE"
        );
        return;
    };
    let good = database(&url, Some(ca.into()));
    let _conn = db::pool(&good, Env::Production)
        .unwrap()
        .get()
        .await
        .unwrap();
    let prefer = database(&format!("{url}?sslmode=prefer"), good.tls_ca_file.clone());
    let _conn = db::pool(&prefer, Env::Production)
        .unwrap()
        .get()
        .await
        .unwrap();

    let wrong_ca = database(&url, Some(wrong_ca.into()));
    let error = db::pool(&wrong_ca, Env::Production)
        .unwrap()
        .get()
        .await
        .err()
        .expect("wrong CA unexpectedly worked");
    assert!(format!("{error:?}").contains("UnknownIssuer"), "{error:?}");

    let require = database(
        &format!("{url}?sslmode=require"),
        wrong_ca.tls_ca_file.clone(),
    );
    let error = db::pool(&require, Env::Development)
        .unwrap()
        .get()
        .await
        .err()
        .expect("wrong CA unexpectedly worked with require");
    assert!(format!("{error:?}").contains("UnknownIssuer"), "{error:?}");

    let wrong_host = url.replace("@localhost:", "@127.0.0.1:");
    let wrong_host = database(&wrong_host, good.tls_ca_file.clone());
    let error = db::pool(&wrong_host, Env::Production)
        .unwrap()
        .get()
        .await
        .err()
        .expect("wrong host unexpectedly worked");
    assert!(
        format!("{error:?}").contains("certificate not valid for name"),
        "{error:?}"
    );
}
