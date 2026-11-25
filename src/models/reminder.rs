use diesel::prelude::*;
use diesel::query_dsl::methods::FilterDsl;
use diesel_async::{AsyncPgConnection, RunQueryDsl};
use time::OffsetDateTime;

use crate::schema::reminders;

/// The expiry reminder of one issuer, for one window and expiry date.
#[derive(Debug, Insertable)]
#[diesel(table_name = reminders)]
pub struct NewReminder<'a> {
    pub issuer_id: &'a str,
    pub window_days: i16,
    pub expires_at: OffsetDateTime,
    /// The issuer's owner.
    pub user_id: i64,
}

/// One delivery of a reminder, counted before it starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Queryable)]
pub struct Claim {
    pub id: i64,
    /// This delivery's number, from 1.
    pub attempt: i16,
}

impl NewReminder<'_> {
    /// Counts one more delivery of the reminder and returns it, unless the reminder was sent,
    /// was given up, or had `max_attempts` deliveries already.
    pub async fn claim(
        &self,
        conn: &mut AsyncPgConnection,
        max_attempts: i16,
    ) -> QueryResult<Option<Claim>> {
        diesel::insert_into(reminders::table)
            .values((self, reminders::attempts.eq(1)))
            .on_conflict((
                reminders::issuer_id,
                reminders::window_days,
                reminders::expires_at,
            ))
            .do_update()
            .set(reminders::attempts.eq(reminders::attempts + 1))
            .filter(
                reminders::sent_at
                    .is_null()
                    .and(reminders::gave_up_at.is_null())
                    .and(reminders::attempts.lt(max_attempts)),
            )
            .returning((reminders::id, reminders::attempts))
            .get_result(conn)
            .await
            .optional()
    }
}

impl Claim {
    pub async fn sent(self, conn: &mut AsyncPgConnection) -> QueryResult<()> {
        diesel::update(reminders::table.find(self.id))
            .set((
                reminders::sent_at.eq(diesel::dsl::now),
                reminders::last_error.eq(None::<String>),
            ))
            .execute(conn)
            .await?;
        Ok(())
    }

    /// Records why the delivery failed. A reminder given up is never tried again.
    pub async fn failed(
        self,
        conn: &mut AsyncPgConnection,
        error: &str,
        give_up: bool,
    ) -> QueryResult<()> {
        let gave_up_at = give_up.then(OffsetDateTime::now_utc);
        diesel::update(reminders::table.find(self.id))
            .set((
                reminders::last_error.eq(error),
                reminders::gave_up_at.eq(gave_up_at),
            ))
            .execute(conn)
            .await?;
        Ok(())
    }
}
