use diesel::dsl::exists;
use diesel::prelude::*;
use diesel_async::{AsyncPgConnection, RunQueryDsl};

use crate::schema::terms_acceptances;

/// A developer's acceptance of one version of the terms of use.
#[derive(Insertable)]
#[diesel(table_name = terms_acceptances)]
pub struct TermsAcceptance<'a> {
    pub user_id: i64,
    pub terms_version: &'a str,
    /// The verified email the account has when it accepts.
    pub email: &'a str,
}

impl TermsAcceptance<'_> {
    /// Records the acceptance. Accepting the same version again keeps the first record.
    pub async fn record(&self, conn: &mut AsyncPgConnection) -> QueryResult<()> {
        diesel::insert_into(terms_acceptances::table)
            .values(self)
            .on_conflict_do_nothing()
            .execute(conn)
            .await?;
        Ok(())
    }

    pub async fn exists(
        conn: &mut AsyncPgConnection,
        user_id: i64,
        terms_version: &str,
    ) -> QueryResult<bool> {
        diesel::select(exists(
            terms_acceptances::table.find((user_id, terms_version)),
        ))
        .get_result(conn)
        .await
    }
}
