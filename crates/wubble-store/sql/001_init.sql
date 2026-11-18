-- Schema v1. Plain SQLite-compatible SQL (no STRICT, WITHOUT ROWID, triggers, FTS) so rusqlite
-- can replace turso. Design references: Chromium history (urls/visits) and Firefox Places
-- (moz_places/moz_historyvisits/moz_bookmarks); see THIRD_PARTY_NOTICES.md.
-- All times are unix milliseconds supplied by the caller.

CREATE TABLE containers (
    id   INTEGER PRIMARY KEY,
    name TEXT NOT NULL
);

CREATE TABLE spaces (
    id           INTEGER PRIMARY KEY,
    name         TEXT NOT NULL,
    position     INTEGER NOT NULL,
    tint         TEXT,
    container_id INTEGER NOT NULL REFERENCES containers(id)
);

CREATE TABLE tab_folders (
    id        INTEGER PRIMARY KEY,
    space_id  INTEGER NOT NULL REFERENCES spaces(id) ON DELETE CASCADE,
    name      TEXT NOT NULL,
    position  INTEGER NOT NULL,
    collapsed INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE tabs (
    id           INTEGER PRIMARY KEY,
    space_id     INTEGER NOT NULL REFERENCES spaces(id) ON DELETE CASCADE,
    container_id INTEGER NOT NULL REFERENCES containers(id),
    folder_id    INTEGER REFERENCES tab_folders(id) ON DELETE SET NULL,
    pinned       INTEGER NOT NULL DEFAULT 0,
    position     INTEGER NOT NULL,
    url          TEXT NOT NULL,
    title        TEXT NOT NULL DEFAULT '',
    liveness     INTEGER NOT NULL,
    busy         INTEGER NOT NULL DEFAULT 0,
    scroll_x     REAL NOT NULL DEFAULT 0,
    scroll_y     REAL NOT NULL DEFAULT 0,
    last_visible INTEGER
);
CREATE INDEX tabs_by_space ON tabs(space_id, pinned, position);

-- Single row (id = 1): what was active when the session was saved.
CREATE TABLE session (
    id              INTEGER PRIMARY KEY CHECK (id = 1),
    active_space_id INTEGER,
    active_tab_id   INTEGER,
    saved_at        INTEGER NOT NULL
);

CREATE TABLE urls (
    id              INTEGER PRIMARY KEY,
    url             TEXT NOT NULL UNIQUE,
    url_key         TEXT NOT NULL,
    title           TEXT NOT NULL DEFAULT '',
    visit_count     INTEGER NOT NULL DEFAULT 0,
    typed_count     INTEGER NOT NULL DEFAULT 0,
    last_visit_time INTEGER NOT NULL,
    hidden          INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX urls_by_key ON urls(url_key);

CREATE TABLE visits (
    id         INTEGER PRIMARY KEY,
    url_id     INTEGER NOT NULL REFERENCES urls(id) ON DELETE CASCADE,
    visit_time INTEGER NOT NULL,
    transition INTEGER NOT NULL
);
CREATE INDEX visits_by_url ON visits(url_id, visit_time);
CREATE INDEX visits_by_time ON visits(visit_time);

CREATE TABLE bookmarks (
    id            INTEGER PRIMARY KEY,
    parent_id     INTEGER REFERENCES bookmarks(id) ON DELETE CASCADE,
    kind          INTEGER NOT NULL, -- 0 url, 1 folder
    position      INTEGER NOT NULL,
    title         TEXT NOT NULL DEFAULT '',
    url           TEXT,
    date_added    INTEGER NOT NULL,
    date_modified INTEGER NOT NULL
);
CREATE INDEX bookmarks_by_parent ON bookmarks(parent_id, position);
CREATE INDEX bookmarks_by_url ON bookmarks(url);

CREATE TABLE settings (
    key   TEXT PRIMARY KEY NOT NULL,
    value TEXT NOT NULL
);
