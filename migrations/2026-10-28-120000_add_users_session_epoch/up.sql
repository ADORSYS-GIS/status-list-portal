-- Copied into every session cookie. Bumping it ends all of the account's sessions at once.
ALTER TABLE users ADD COLUMN session_epoch INTEGER NOT NULL DEFAULT 0;
