-- Schema v9: one `logins` shape, the vault's. Replaces the v3 table (integer id, split nonce
-- columns), which nothing ever wrote to: the vault binds a client-generated text id into the
-- AEAD associated data, which an autoincrement id cannot provide before the row exists.
-- Old rows are not convertible and there are none.
DROP TABLE logins;

-- Times are unix milliseconds. `id` is a 128-bit random hex string made by the vault.
-- `origin` is `scheme://host[:port]` (non-default port only), plaintext for matching.
-- username_enc / password_enc are `nonce(12) || ciphertext || tag(16)`, opaque to the store.
CREATE TABLE logins (
    id                 TEXT PRIMARY KEY,
    origin             TEXT NOT NULL,
    form_action_origin TEXT,            -- NULL: HTTP auth or no form
    username_enc       BLOB NOT NULL,
    password_enc       BLOB NOT NULL,
    created_ms         INTEGER NOT NULL,
    last_used_ms       INTEGER,         -- NULL: never
    times_used         INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX logins_by_origin ON logins(origin);
