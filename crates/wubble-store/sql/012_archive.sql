-- Schema v12: auto-archive. A tab's last visit (unix ms, 0 = unknown: the model stamps it at
-- its first tick), a Space's never-sleep flag, and the Archive list. The archive has no foreign
-- keys, like recently_closed: the Space may be gone when a tab is restored.
ALTER TABLE tabs ADD COLUMN last_active INTEGER NOT NULL DEFAULT 0;
ALTER TABLE spaces ADD COLUMN never_sleep INTEGER NOT NULL DEFAULT 0;
CREATE TABLE archive (
    id           INTEGER PRIMARY KEY,
    archived_at  INTEGER NOT NULL,
    last_active  INTEGER NOT NULL,
    space_id     INTEGER,
    container_id INTEGER NOT NULL,
    url          TEXT NOT NULL,
    title        TEXT NOT NULL DEFAULT ''
);
CREATE INDEX archive_by_time ON archive(archived_at);
