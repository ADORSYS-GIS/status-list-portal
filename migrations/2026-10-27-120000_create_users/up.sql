-- Developers who signed in with GitHub.
CREATE TABLE users (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    -- GitHub's numeric account id. Logins can be renamed; this id never changes.
    gh_id BIGINT NOT NULL UNIQUE,
    -- GitHub login, refreshed at every sign-in.
    gh_login VARCHAR NOT NULL,
    -- Display name from the GitHub profile, if set.
    name VARCHAR,
    -- Verified primary email, if the account has one. Used for terms acceptance and
    -- expiry reminders.
    email VARCHAR,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
