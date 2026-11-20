-- Schema v14: the agent activity log (wubble_core::agent::log::Entry), newest id last.
-- `outcome` 0 done, 1 refused, 2 failed. Bounded by the store on append.
CREATE TABLE agent_log (
    id INTEGER PRIMARY KEY,
    session TEXT NOT NULL,
    at_ms INTEGER NOT NULL,
    tool TEXT NOT NULL,
    target TEXT,
    outcome INTEGER NOT NULL
);
