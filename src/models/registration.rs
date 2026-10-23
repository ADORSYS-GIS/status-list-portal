use diesel::dsl::count_star;
use diesel::pg::Pg;
use diesel::prelude::*;
use diesel_async::{AsyncPgConnection, RunQueryDsl};
use serde::Serialize;
use serde_json::Value;
use time::OffsetDateTime;
use utoipa::ToSchema;

use crate::models::User;
use crate::schema::{registrations, users};

/// An issuer a developer asked for while registrations need an operator's approval.
#[derive(Debug, Clone, PartialEq, Queryable, Selectable)]
#[diesel(table_name = registrations, check_for_backend(Pg))]
pub struct Registration {
    pub id: i64,
    pub user_id: i64,
    pub display_name: String,
    /// JWS algorithm of the key, such as `ES256`.
    pub alg: String,
    /// The public JWK, as checked when the developer asked.
    pub public_key: Value,
    pub created_at: OffsetDateTime,
    pub decided_at: Option<OffsetDateTime>,
    /// The issuer the approval created.
    pub issuer_id: Option<String>,
    pub rejection_reason: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum RegistrationState {
    Pending,
    Approved,
    Rejected,
}

impl Registration {
    pub fn state(&self) -> RegistrationState {
        match (self.decided_at, &self.issuer_id) {
            (None, _) => RegistrationState::Pending,
            (Some(_), Some(_)) => RegistrationState::Approved,
            (Some(_), None) => RegistrationState::Rejected,
        }
    }

    pub async fn find(conn: &mut AsyncPgConnection, id: i64) -> QueryResult<Registration> {
        registrations::table
            .find(id)
            .select(Registration::as_select())
            .first(conn)
            .await
    }

    /// Reads the registration and locks it until the transaction ends: an approval holds it
    /// while it creates the issuer, so a withdrawal or a rejection waits for the outcome.
    pub async fn find_for_update(
        conn: &mut AsyncPgConnection,
        id: i64,
    ) -> QueryResult<Registration> {
        registrations::table
            .find(id)
            .select(Registration::as_select())
            .for_update()
            .first(conn)
            .await
    }

    /// The user's registrations, newest first.
    pub async fn of_user(
        conn: &mut AsyncPgConnection,
        user_id: i64,
    ) -> QueryResult<Vec<Registration>> {
        registrations::table
            .filter(registrations::user_id.eq(user_id))
            .order((registrations::created_at.desc(), registrations::id.desc()))
            .select(Registration::as_select())
            .load(conn)
            .await
    }

    /// Pending registrations, oldest first, with who asked for them.
    pub async fn pending(conn: &mut AsyncPgConnection) -> QueryResult<Vec<(Registration, User)>> {
        registrations::table
            .inner_join(users::table)
            .filter(registrations::decided_at.is_null())
            .order((registrations::created_at.asc(), registrations::id.asc()))
            .select((Registration::as_select(), User::as_select()))
            .load(conn)
            .await
    }

    /// How many registrations of the user, or of everyone, wait for a decision.
    pub async fn count_pending(
        conn: &mut AsyncPgConnection,
        user_id: Option<i64>,
    ) -> QueryResult<usize> {
        let mut query = registrations::table
            .filter(registrations::decided_at.is_null())
            .into_boxed();
        if let Some(user_id) = user_id {
            query = query.filter(registrations::user_id.eq(user_id));
        }
        let count: i64 = query.select(count_star()).get_result(conn).await?;
        Ok(usize::try_from(count).unwrap_or(usize::MAX))
    }

    /// How many of the user's registrations created no issuer: pending and rejected ones.
    pub async fn count_without_issuer(
        conn: &mut AsyncPgConnection,
        user_id: i64,
    ) -> QueryResult<usize> {
        let count: i64 = registrations::table
            .filter(registrations::user_id.eq(user_id))
            .filter(registrations::issuer_id.is_null())
            .select(count_star())
            .get_result(conn)
            .await?;
        Ok(usize::try_from(count).unwrap_or(usize::MAX))
    }

    /// Records that the registration created `issuer_id`. `false` when it was no longer
    /// pending.
    pub async fn approve(
        conn: &mut AsyncPgConnection,
        id: i64,
        issuer_id: &str,
    ) -> QueryResult<bool> {
        let decided = diesel::update(pending(id))
            .set((
                registrations::decided_at.eq(diesel::dsl::now),
                registrations::issuer_id.eq(issuer_id),
            ))
            .execute(conn)
            .await?;
        Ok(decided == 1)
    }

    /// Records the rejection. `false` when the registration was no longer pending.
    pub async fn reject(
        conn: &mut AsyncPgConnection,
        id: i64,
        reason: Option<&str>,
    ) -> QueryResult<bool> {
        let decided = diesel::update(pending(id))
            .set((
                registrations::decided_at.eq(diesel::dsl::now),
                registrations::rejection_reason.eq(reason),
            ))
            .execute(conn)
            .await?;
        Ok(decided == 1)
    }

    /// Deletes the user's registration while it is pending. `false` when there is none.
    pub async fn withdraw(
        conn: &mut AsyncPgConnection,
        id: i64,
        user_id: i64,
    ) -> QueryResult<bool> {
        let deleted = diesel::delete(pending(id).filter(registrations::user_id.eq(user_id)))
            .execute(conn)
            .await?;
        Ok(deleted == 1)
    }
}

/// The registration with this id, while it is pending.
#[diesel::dsl::auto_type]
fn pending(id: i64) -> _ {
    registrations::table
        .filter(registrations::id.eq(id))
        .filter(registrations::decided_at.is_null())
}

/// What a developer asks for.
#[derive(Debug, Insertable)]
#[diesel(table_name = registrations)]
pub struct NewRegistration<'a> {
    pub user_id: i64,
    pub display_name: &'a str,
    pub alg: &'a str,
    pub public_key: &'a Value,
}

impl NewRegistration<'_> {
    pub async fn insert(&self, conn: &mut AsyncPgConnection) -> QueryResult<Registration> {
        diesel::insert_into(registrations::table)
            .values(self)
            .returning(Registration::as_returning())
            .get_result(conn)
            .await
    }
}
