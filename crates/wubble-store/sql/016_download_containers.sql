-- Schema v16: the container a download came from (wubble_core::space::ContainerId), so a retry
-- after a restart runs in that container's cookies. Every download before it was the default's.
ALTER TABLE downloads ADD COLUMN container_id INTEGER NOT NULL DEFAULT 0;
