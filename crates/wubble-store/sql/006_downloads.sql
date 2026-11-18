-- Schema v6: the downloads list (wubble_core::download), newest first by id. `state` is
-- one of running, paused, interrupted, complete, cancelled, failed; `error` explains the
-- last two. Ids are assigned by the host. No foreign keys: the tab is long gone.
CREATE TABLE downloads (
    id         INTEGER PRIMARY KEY,
    url        TEXT NOT NULL,
    name       TEXT NOT NULL,
    path       TEXT NOT NULL,
    total      INTEGER,
    received   INTEGER NOT NULL DEFAULT 0,
    state      TEXT NOT NULL,
    error      TEXT,
    started_at INTEGER NOT NULL
);
