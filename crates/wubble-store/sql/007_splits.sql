-- Schema v7: split view (wubble_core::space::SplitGroup). One row per pane of a Space's split,
-- left to right by `position`. `stop` is where the seam after the pane rests, as a fraction of
-- the width; NULL on the last pane. A group that does not load as a valid split is dropped.
CREATE TABLE split_panes (
    space_id INTEGER NOT NULL REFERENCES spaces(id) ON DELETE CASCADE,
    position INTEGER NOT NULL,
    tab_id   INTEGER NOT NULL REFERENCES tabs(id) ON DELETE CASCADE,
    stop     REAL,
    PRIMARY KEY (space_id, position)
);
