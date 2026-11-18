-- Schema v5: remembered permission decisions, one row per origin and kind
-- (wubble_core::site). `decision`: 1 allow, 0 block. No row means ask.
-- Design reference: Chromium content settings, Firefox nsIPermissionManager.
CREATE TABLE site_permissions (
    origin     TEXT NOT NULL,
    kind       TEXT NOT NULL,
    decision   INTEGER NOT NULL,
    decided_at INTEGER NOT NULL,
    PRIMARY KEY (origin, kind)
);
