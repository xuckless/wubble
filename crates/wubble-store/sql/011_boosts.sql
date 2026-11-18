-- Schema v11: Boosts, the user's own CSS, scripts, tint and font per site
-- (wubble_core::boost::Boost). `tint` is 0xRRGGBB, NULL for none; `js_at` 0 document start,
-- 1 DOMContentLoaded. Renumber after the windows lane's 011_windows.sql when the lanes merge.
CREATE TABLE boosts (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL DEFAULT '',
    site TEXT NOT NULL,
    css TEXT NOT NULL DEFAULT '',
    js TEXT NOT NULL DEFAULT '',
    js_at INTEGER NOT NULL DEFAULT 0,
    tint INTEGER,
    tint_strength INTEGER NOT NULL DEFAULT 0,
    font TEXT,
    enabled INTEGER NOT NULL DEFAULT 1,
    position INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
