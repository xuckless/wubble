//! Tab identity and lifecycle.

use crate::busy::Busy;
use crate::lifecycle::Lifecycle;
use crate::space::ContainerId;

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

#[derive(Debug, Clone, PartialEq)]
pub struct Tab {
    pub id: TabId,
    pub url: String,
    pub title: String,
    pub lifecycle: Lifecycle,
    /// The context the page runs in, fixed at creation from the override or the Space default.
    pub container: ContainerId,
    /// The tab that opened this one; consulted when this tab closes, forgotten on a user tab switch.
    pub opener: Option<TabId>,
    /// Unix milliseconds of the last time the tab was on screen; 0 until known. Stamped by the
    /// model when the tab is hidden, kept in the session, read by the auto-archive rule.
    pub last_active_ms: i64,
    /// Why the tab must stay awake (the one owner; `lifecycle.activity` follows it). Not stored.
    pub busy: Busy,
}
