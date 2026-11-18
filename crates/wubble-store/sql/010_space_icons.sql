-- Schema v10: each Space has its own drawn icon (wubble_core::space::SpaceIcon by id). Existing
-- Spaces keep the icon the sidebar drew for them: the three in turn by position.
ALTER TABLE spaces ADD COLUMN icon TEXT NOT NULL DEFAULT 'personal';
UPDATE spaces SET icon = CASE position % 3 WHEN 0 THEN 'personal' WHEN 1 THEN 'work' ELSE 'study' END;
