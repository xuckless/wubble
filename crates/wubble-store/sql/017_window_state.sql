-- Schema v17: a window's show state and monitor (wubble_core::window::Geometry), and Popup
-- windows. `state` is 'normal', 'maximized', 'fullscreen' or 'minimized' (WindowState::id);
-- while it is not 'normal', `width`/`height`/`x`/`y` are the window's last normal bounds, as
-- Chrome keeps restored bounds. `monitor` is the monitor's name where the platform gives one.
-- A row with `tab_id` is a Popup window showing that tab of `space_id` (Chrome restores
-- TYPE_POPUP windows); without, a Main window on `space_id`, as before v17.
ALTER TABLE windows ADD COLUMN state TEXT NOT NULL DEFAULT 'normal';
ALTER TABLE windows ADD COLUMN monitor TEXT;
ALTER TABLE windows ADD COLUMN tab_id INTEGER;
