use std::num::NonZeroUsize;

use crate::util::TestDatabase;
use diesel::dsl::sql;
use diesel::sql_types::Text;
use diesel_async::RunQueryDsl;
use secrecy::SecretString;
use status_list_portal::{Env, config, db};

#[tokio::test]
async fn pool_connections_time_out_long_queries_and_open_transactions() {
    let test_db = TestDatabase::new().await;
    let pool = db::pool(
        &config::Database {
            url: SecretString::from(test_db.url()),
            tls_ca_file: None,
            pool_size: NonZeroUsize::MIN,
        },
        Env::Test,
    )
    .unwrap();
    let mut conn = pool.get().await.unwrap();

    let timeouts = diesel::select(sql::<(Text, Text)>(
        "current_setting('statement_timeout'), \
         current_setting('idle_in_transaction_session_timeout')",
    ))
    .get_result::<(String, String)>(&mut conn)
    .await
    .unwrap();

    assert_eq!(timeouts, ("10s".to_string(), "1min".to_string()));
}
