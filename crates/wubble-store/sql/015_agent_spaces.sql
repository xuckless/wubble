-- Schema v15: a Space an agent session made stays an agent Space (wubble_core::space::Space::agent),
-- also once the session left its tabs to the owner. Every Space before it is the user's.
ALTER TABLE spaces ADD COLUMN agent INTEGER NOT NULL DEFAULT 0;
