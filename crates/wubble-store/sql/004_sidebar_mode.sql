-- Schema v4: the sidebar's mode is one typed setting, `ui.sidebar_mode` (extended / rail /
-- hidden), replacing the boolean `ui.compact` (true meant hidden). Carried over once here.
INSERT INTO settings(key, value)
    SELECT 'ui.sidebar_mode', 'hidden' FROM settings WHERE key = 'ui.compact' AND value = '1';
DELETE FROM settings WHERE key = 'ui.compact';
