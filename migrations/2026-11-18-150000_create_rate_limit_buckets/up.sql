-- How many more times each developer may do a rate-limited action, such as registering an
-- issuer, right now: a token bucket per developer and action. `tokens` were counted at
-- `last_refill`; one comes back per refill interval, up to the action's burst.
CREATE TABLE rate_limit_buckets (
    user_id BIGINT NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    action VARCHAR NOT NULL,
    tokens INTEGER NOT NULL CHECK (tokens >= 0),
    last_refill TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (user_id, action)
);
