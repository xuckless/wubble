-- Schema v11: more than one window (wubble_core::window). One row per Main window, least
-- recently focused first by `position` (the last row is the focused window). Little and Popup
-- windows are not stored: their tab is an ordinary tab of its Space. Without rows the session
-- has one window on session.active_space_id, as before v11. `width`/`height` are the inner
-- size in logical pixels; `x`/`y` the outer position where the platform tells (not Wayland).
CREATE TABLE windows (
    id       INTEGER PRIMARY KEY,
    position INTEGER NOT NULL,
    space_id INTEGER NOT NULL REFERENCES spaces(id) ON DELETE CASCADE,
    width    INTEGER,
    height   INTEGER,
    x        INTEGER,
    y        INTEGER
);
