//! Tables of the portal's database, as diesel sees them.
//!
//! Every change here goes with a migration in `migrations/`. Running `diesel print-schema`
//! against a migrated database regenerates this file.

diesel::table! {
    users (id) {
        id -> Int8,
        gh_id -> Int8,
        gh_login -> Varchar,
        name -> Nullable<Varchar>,
        email -> Varchar,
        created_at -> Timestamptz,
        session_epoch -> Int4,
    }
}
