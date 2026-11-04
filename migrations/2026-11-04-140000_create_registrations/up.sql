-- Issuers developers asked for while registrations wait for an operator's approval.
-- A registration is pending until `decided_at` is set; it was approved if it then names
-- the issuer the approval created, and rejected otherwise.
CREATE TABLE registrations (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    user_id BIGINT NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    display_name VARCHAR NOT NULL,
    -- JWS algorithm of the key, such as ES256.
    alg VARCHAR NOT NULL,
    -- The public JWK, checked like any registration's. Never a private key.
    public_key JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    decided_at TIMESTAMPTZ,
    issuer_id VARCHAR,
    -- Why an operator rejected the registration. The developer sees it.
    rejection_reason VARCHAR,
    CHECK (issuer_id IS NULL OR decided_at IS NOT NULL),
    CHECK (rejection_reason IS NULL OR (decided_at IS NOT NULL AND issuer_id IS NULL)),
    -- A rejection always says why.
    CHECK (decided_at IS NULL OR issuer_id IS NOT NULL OR btrim(rejection_reason) <> '')
);

CREATE INDEX registrations_user_id ON registrations (user_id);
CREATE UNIQUE INDEX registrations_issuer_id ON registrations (issuer_id);
CREATE INDEX registrations_pending ON registrations (created_at) WHERE decided_at IS NULL;
