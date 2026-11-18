-- Schema v2: the store persists `wubble_core::model::Model` directly.
-- Placement order: tabs.position is the index inside its list (pinned list, top-level items, or
-- folder); top-level tabs and folders share one position sequence per Space.
-- Plain SQLite (ADD/DROP COLUMN only, no table rebuilds).

-- Each Space remembers its own active tab.
ALTER TABLE spaces ADD COLUMN active_tab_id INTEGER;
-- Container order (containers[0] is Default). v1 ordered by id.
ALTER TABLE containers ADD COLUMN position INTEGER NOT NULL DEFAULT 0;
UPDATE containers SET position = id;

-- v1 numbered folders and tabs in two separate sequences; v2 has one per Space for top-level
-- items. v1 never recorded the interleaving, so folders go first (old order), then top-level tabs.
UPDATE tabs SET position = position + (SELECT COUNT(*) FROM tab_folders f WHERE f.space_id = tabs.space_id)
WHERE pinned = 0 AND folder_id IS NULL;
UPDATE tab_folders SET position = (
    SELECT COUNT(*) FROM tab_folders f
    WHERE f.space_id = tab_folders.space_id
      AND (f.position < tab_folders.position OR (f.position = tab_folders.position AND f.id < tab_folders.id))
);

-- Backfill: the first tab of each Space (pinned first), then the saved active tab for the
-- active Space. Core requires an active tab exactly when a Space has tabs.
UPDATE spaces SET active_tab_id = (
    SELECT id FROM tabs WHERE tabs.space_id = spaces.id ORDER BY pinned DESC, position LIMIT 1
);
UPDATE spaces SET active_tab_id = (SELECT active_tab_id FROM session WHERE id = 1)
WHERE id = (SELECT active_space_id FROM session WHERE id = 1)
  AND (SELECT active_tab_id FROM session WHERE id = 1) IN (SELECT id FROM tabs WHERE space_id = spaces.id);

-- Columns the core model has no counterpart for (tabs restore as Discarded and idle).
ALTER TABLE tab_folders DROP COLUMN collapsed;
ALTER TABLE tabs DROP COLUMN liveness;
ALTER TABLE tabs DROP COLUMN busy;
ALTER TABLE tabs DROP COLUMN scroll_x;
ALTER TABLE tabs DROP COLUMN scroll_y;
ALTER TABLE tabs DROP COLUMN last_visible;
ALTER TABLE session DROP COLUMN active_tab_id;
ALTER TABLE session DROP COLUMN saved_at;

-- Reopen-closed-tab list, bounded in code. No foreign keys: the Space may be gone by then.
CREATE TABLE recently_closed (
    id           INTEGER PRIMARY KEY,
    closed_at    INTEGER NOT NULL,
    space_id     INTEGER,
    container_id INTEGER NOT NULL,
    url          TEXT NOT NULL,
    title        TEXT NOT NULL
);
