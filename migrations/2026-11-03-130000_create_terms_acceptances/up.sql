-- Which versions of the terms of use each developer accepted. Deleting an account
-- deletes its acceptances with it.
CREATE TABLE terms_acceptances (
    user_id BIGINT NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    terms_version VARCHAR NOT NULL,
    -- The verified email the account had when it accepted.
    email VARCHAR NOT NULL,
    accepted_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (user_id, terms_version)
);
