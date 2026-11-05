use diesel::dsl::count_star;
use diesel::pg::Pg;
use diesel::prelude::*;
use diesel_async::{AsyncPgConnection, RunQueryDsl};
use serde::Serialize;
use serde_json::Value;
use time::OffsetDateTime;
use utoipa::ToSchema;

use crate::admin_api::{SigningAlgorithm, UnknownAlgorithm};
use crate::models::User;
use crate::schema::{registrations, users};
use crate::util::errors::{self, BoxedAppError};

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
    /// The key's algorithm. The portal stores only algorithms it knows, so another one
    /// is a bug.
    pub fn signing_algorithm(&self) -> Result<SigningAlgorithm, BoxedAppError> {
        self.alg
            .parse()
            .map_err(|error: UnknownAlgorithm| errors::internal(error.to_string()))
    }

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

    /// Reads the registration, of the user `user_id` when given, and locks it until the
    /// transaction ends, unless another transaction holds it: `None` then, as when there is
    /// none. Another user's registration is never locked.
    pub async fn try_lock(
        conn: &mut AsyncPgConnection,
        id: i64,
        user_id: Option<i64>,
    ) -> QueryResult<Option<Registration>> {
        let query = registrations::table
            .find(id)
            .select(Registration::as_select());
        match user_id {
            Some(user_id) => {
                query
                    .filter(registrations::user_id.eq(user_id))
                    .for_update()
                    .skip_locked()
                    .first(conn)
                    .await
            }
            None => query.for_update().skip_locked().first(conn).await,
        }
        .optional()
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

    /// How many of the user's registrations, other than `except`, wait for a decision.
    pub async fn count_pending(
        conn: &mut AsyncPgConnection,
        user_id: i64,
        except: Option<i64>,
    ) -> QueryResult<usize> {
        let count: i64 = others_of_user(user_id, except)
            .filter(registrations::decided_at.is_null())
            .select(count_star())
            .get_result(conn)
            .await?;
        Ok(usize::try_from(count).unwrap_or(usize::MAX))
    }

    /// How many of the user's registrations, other than `except`, created no issuer: pending
    /// and rejected ones.
    pub async fn count_without_issuer(
        conn: &mut AsyncPgConnection,
        user_id: i64,
        except: Option<i64>,
    ) -> QueryResult<usize> {
        let count: i64 = others_of_user(user_id, except)
            .filter(registrations::issuer_id.is_null())
            .select(count_star())
            .get_result(conn)
            .await?;
        Ok(usize::try_from(count).unwrap_or(usize::MAX))
    }

    /// Whether a registration's approval created `issuer_id`.
    pub async fn records_issuer(
        conn: &mut AsyncPgConnection,
        issuer_id: &str,
    ) -> QueryResult<bool> {
        diesel::select(diesel::dsl::exists(
            registrations::table.filter(registrations::issuer_id.eq(issuer_id)),
        ))
        .get_result(conn)
        .await
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
    pub async fn reject(conn: &mut AsyncPgConnection, id: i64, reason: &str) -> QueryResult<bool> {
        let decided = diesel::update(pending(id))
            .set((
                registrations::decided_at.eq(diesel::dsl::now),
                registrations::rejection_reason.eq(reason),
            ))
            .execute(conn)
            .await?;
        Ok(decided == 1)
    }

    /// Deletes the registration while it is pending. `false` when it was not.
    pub async fn withdraw(conn: &mut AsyncPgConnection, id: i64) -> QueryResult<bool> {
        let deleted = diesel::delete(pending(id)).execute(conn).await?;
        Ok(deleted == 1)
    }
}

/// The user's registrations, but `except`.
fn others_of_user(user_id: i64, except: Option<i64>) -> registrations::BoxedQuery<'static, Pg> {
    let query = registrations::table
        .filter(registrations::user_id.eq(user_id))
        .into_boxed();
    match except {
        Some(id) => query.filter(registrations::id.ne(id)),
        None => query,
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
