//! Tab identity and lifecycle.

use std::time::Instant;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TabId(pub u64);

/// How much of a tab is currently alive, cheapest last.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Liveness {
    /// Renderer alive, producing frames.
    Active,
    /// Renderer alive, hidden: no frames, timers throttled.
    Background,
    /// Renderer alive but frozen (no JS, no timers).
    Frozen,
    /// Renderer gone. Only URL, title, favicon, scroll and a snapshot remain.
    Discarded,
}

#[derive(Debug, Clone)]
pub struct Tab {
    pub id: TabId,
    pub url: String,
    pub title: String,
    pub liveness: Liveness,
    pub pinned: bool,
    /// Playing audio, capturing media, holding a connection the user cares about, etc.
    pub busy: bool,
    pub last_visible: Instant,
}
