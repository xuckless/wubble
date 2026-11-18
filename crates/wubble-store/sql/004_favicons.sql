-- Page icons as raw RGBA (<= 64x64), shared by every page of an origin.
-- No foreign keys: rows are deleted explicitly (see favicon.rs).
CREATE TABLE favicons (
    icon_url  TEXT PRIMARY KEY NOT NULL,
    width     INTEGER NOT NULL,
    height    INTEGER NOT NULL,
    rgba      BLOB NOT NULL,        -- width * height * 4, straight alpha
    last_used INTEGER NOT NULL      -- unix ms, supplied by the caller
);

CREATE TABLE favicon_origins (
    origin    TEXT PRIMARY KEY NOT NULL,   -- scheme://host[:port], wubble_core::url::origin_key
    icon_url  TEXT NOT NULL,
    last_used INTEGER NOT NULL
);

CREATE INDEX favicon_origins_by_icon ON favicon_origins(icon_url);
