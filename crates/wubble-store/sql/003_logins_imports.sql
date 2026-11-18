-- Schema v3: password-vault rows and import bookkeeping. Plain SQLite-compatible SQL.
-- Bookmarks need no change: the `bookmarks` table of v1 is used as is by `bookmarks.rs`.

-- Opaque encrypted blobs plus plaintext metadata for matching. The store never sees keys or
-- plaintext; the vault owns the crypto. Design: Chromium `login_database.cc` (`logins`),
-- Firefox `LoginManagerStorage` fields.
CREATE TABLE logins (
    id                 INTEGER PRIMARY KEY,
    origin             TEXT NOT NULL,   -- scheme://host[:port], plaintext for matching
    form_action_origin TEXT,            -- NULL: HTTP auth or no form
    username_ct        BLOB NOT NULL,
    username_nonce     BLOB NOT NULL,
    password_ct        BLOB NOT NULL,
    password_nonce     BLOB NOT NULL,
    created            INTEGER NOT NULL,
    last_used          INTEGER,         -- NULL: never
    times_used         INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX logins_by_origin ON logins(origin);

-- One row per imported Space: applying the same import again skips Spaces already imported.
-- No foreign key to `spaces`: `save_session` deletes and rewrites every space row (ids are kept).
CREATE TABLE import_marks (
    key      TEXT PRIMARY KEY NOT NULL,
    space_id INTEGER NOT NULL
);
