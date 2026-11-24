-- Emails that remind developers their sandbox issuers expire soon: one per issuer, reminder
-- window and expiry date, so a renewed issuer is reminded again before its new expiry. A
-- reminder is pending until it is sent or given up. Each delivery is counted in `attempts`
-- before it starts, so no two runs send the same reminder, and a run that dies mid-send
-- counts that try.
CREATE TABLE reminders (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    issuer_id VARCHAR NOT NULL,
    -- Days before the expiry the reminder is due, such as 7.
    window_days SMALLINT NOT NULL CHECK (window_days > 0),
    expires_at TIMESTAMPTZ NOT NULL,
    -- The issuer's owner, whose verified email it goes to.
    user_id BIGINT NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    attempts SMALLINT NOT NULL CHECK (attempts > 0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    sent_at TIMESTAMPTZ,
    gave_up_at TIMESTAMPTZ,
    -- Why the last delivery failed, as the SMTP server said: it can name the recipient, so
    -- it stays out of the logs.
    last_error VARCHAR,
    CHECK (sent_at IS NULL OR gave_up_at IS NULL),
    UNIQUE (issuer_id, window_days, expires_at)
);

CREATE INDEX reminders_user_id ON reminders (user_id);
