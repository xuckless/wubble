-- Schema v17: the recently-closed list holds windows, folders and Spaces as well as tabs
-- (wubble_core::closed::Entry; Chromium TabRestoreService's TAB, WINDOW and GROUP entries).
-- `kind`: 0 a tab (url, title, container_id, space_id as before), 1 a Main window (space_id,
-- the Space as it was in name/tint/icon/container_id, the window's width/height/x/y), 2 a folder
-- (name, space_id), 3 a Space (name/tint/icon/container_id). Existing rows are tabs. A grouped
-- entry's tabs are rows of recently_closed_tabs, in `position` order.
ALTER TABLE recently_closed ADD COLUMN kind INTEGER NOT NULL DEFAULT 0;
ALTER TABLE recently_closed ADD COLUMN name TEXT;
ALTER TABLE recently_closed ADD COLUMN tint TEXT;
ALTER TABLE recently_closed ADD COLUMN icon TEXT;
ALTER TABLE recently_closed ADD COLUMN width INTEGER;
ALTER TABLE recently_closed ADD COLUMN height INTEGER;
ALTER TABLE recently_closed ADD COLUMN x INTEGER;
ALTER TABLE recently_closed ADD COLUMN y INTEGER;

-- `pinned`: 1 for a pinned tab. `folder`: the ordinal of the folder it was in (in its Space's
-- order), with that folder's `folder_name`; NULL for a top-level tab.
CREATE TABLE recently_closed_tabs (
    id           INTEGER PRIMARY KEY,
    entry_id     INTEGER NOT NULL REFERENCES recently_closed(id) ON DELETE CASCADE,
    position     INTEGER NOT NULL,
    url          TEXT NOT NULL,
    title        TEXT NOT NULL,
    container_id INTEGER NOT NULL,
    pinned       INTEGER NOT NULL DEFAULT 0,
    folder       INTEGER,
    folder_name  TEXT
);
CREATE INDEX recently_closed_tabs_entry ON recently_closed_tabs(entry_id, position);
