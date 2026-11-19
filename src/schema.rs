// @generated automatically by Diesel CLI.

diesel::table! {
    registrations (id) {
        id -> Int8,
        user_id -> Int8,
        display_name -> Varchar,
        alg -> Varchar,
        public_key -> Jsonb,
        created_at -> Timestamptz,
        decided_at -> Nullable<Timestamptz>,
        issuer_id -> Nullable<Varchar>,
        rejection_reason -> Nullable<Varchar>,
    }
}

diesel::table! {
    terms_acceptances (user_id, terms_version) {
        user_id -> Int8,
        terms_version -> Varchar,
        email -> Varchar,
        accepted_at -> Timestamptz,
    }
}

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

diesel::joinable!(registrations -> users (user_id));
diesel::joinable!(terms_acceptances -> users (user_id));

diesel::allow_tables_to_appear_in_same_query!(registrations, terms_acceptances, users,);
