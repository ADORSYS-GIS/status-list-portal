use std::num::NonZeroUsize;

use crate::util::TestDatabase;
use diesel::dsl::sql;
use diesel::sql_types::Text;
use diesel_async::RunQueryDsl;
use secrecy::SecretString;
use status_list_portal::{config, db};

#[tokio::test]
async fn pool_connections_cancel_long_queries() {
    let test_db = TestDatabase::new().await;
    let pool = db::pool(&config::Database {
        url: SecretString::from(test_db.url()),
        pool_size: NonZeroUsize::MIN,
    })
    .unwrap();
    let mut conn = pool.get().await.unwrap();

    let timeout = diesel::select(sql::<Text>("current_setting('statement_timeout')"))
        .get_result::<String>(&mut conn)
        .await
        .unwrap();

    assert_eq!(timeout, "10s");
}
