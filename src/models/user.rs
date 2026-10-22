use diesel::pg::Pg;
use diesel::prelude::*;
use diesel_async::{AsyncPgConnection, RunQueryDsl};
use time::OffsetDateTime;

use crate::schema::users;

/// A developer who signed in with GitHub.
#[derive(Debug, Clone, PartialEq, Eq, Queryable, Selectable)]
#[diesel(table_name = users, check_for_backend(Pg))]
pub struct User {
    pub id: i64,
    /// GitHub's numeric account id. Unlike the login, it never changes.
    pub gh_id: i64,
    pub gh_login: String,
    pub name: Option<String>,
    /// Verified primary email. Signing in needs one.
    pub email: String,
    pub created_at: OffsetDateTime,
    /// Copied into every session cookie; a cookie with another value is void.
    pub session_epoch: i32,
}

impl User {
    pub async fn find(conn: &mut AsyncPgConnection, id: i64) -> QueryResult<User> {
        users::table
            .find(id)
            .select(User::as_select())
            .first(conn)
            .await
    }

    /// Ends every session of the account, if `epoch` is still current. A stale epoch
    /// changes nothing, so an old cookie cannot sign anyone out.
    pub async fn end_sessions(
        conn: &mut AsyncPgConnection,
        id: i64,
        epoch: i32,
    ) -> QueryResult<usize> {
        diesel::update(users::table.find(id).filter(users::session_epoch.eq(epoch)))
            .set(users::session_epoch.eq(users::session_epoch + 1))
            .execute(conn)
            .await
    }
}

/// Account details GitHub reports at sign-in.
#[derive(Debug, Insertable, AsChangeset)]
#[diesel(table_name = users, check_for_backend(Pg), treat_none_as_null = true)]
pub struct NewUser<'a> {
    pub gh_id: i64,
    pub gh_login: &'a str,
    pub name: Option<&'a str>,
    pub email: &'a str,
}

impl NewUser<'_> {
    /// Creates the account, or refreshes the details of an account that signed in before.
    ///
    /// Details GitHub no longer reports, such as a removed display name, are cleared.
    pub async fn insert_or_update(&self, conn: &mut AsyncPgConnection) -> QueryResult<User> {
        diesel::insert_into(users::table)
            .values(self)
            .on_conflict(users::gh_id)
            .do_update()
            .set(self)
            .returning(User::as_returning())
            .get_result(conn)
            .await
    }
}
