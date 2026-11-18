-- Saved logins. Times are unix milliseconds supplied by the caller.
-- `id` is generated client side (128-bit random hex) because it is part of the
-- AEAD associated data. `origin` is `scheme://host[:port]` (non-default port only).
-- username_enc / password_enc are `nonce(12) || ciphertext || tag(16)`.
-- Renumber when wubble-store adopts it.
CREATE TABLE logins (
    id                 TEXT PRIMARY KEY,
    origin             TEXT NOT NULL,
    form_action_origin TEXT,
    username_enc       BLOB NOT NULL,
    password_enc       BLOB NOT NULL,
    created_ms         INTEGER NOT NULL,
    last_used_ms       INTEGER,
    times_used         INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX logins_origin ON logins(origin);
