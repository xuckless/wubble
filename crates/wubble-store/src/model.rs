//! Plain row structs for the stored session. The edge converts to and from the live model
//! in `wubble-core` (which holds `Instant`s and has no folders or scroll offsets).
//!
//! Vector order is display order: positions are written from the index on save and read back
//! ordered by position on load.

use wubble_core::space::{ContainerId, SpaceId};
use wubble_core::tab::{Liveness, TabId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FolderId(pub u32);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredContainer {
    pub id: ContainerId,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredSpace {
    pub id: SpaceId,
    pub name: String,
    pub tint: Option<String>,
    pub container: ContainerId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredFolder {
    pub id: FolderId,
    pub space: SpaceId,
    pub name: String,
    pub collapsed: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StoredTab {
    pub id: TabId,
    pub space: SpaceId,
    pub container: ContainerId,
    pub folder: Option<FolderId>,
    pub pinned: bool,
    pub url: String,
    pub title: String,
    pub liveness: Liveness,
    pub busy: bool,
    pub scroll: (f64, f64),
    /// Unix ms; `None` if the tab was never visible.
    pub last_visible_ms: Option<i64>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct StoredSession {
    pub containers: Vec<StoredContainer>,
    pub spaces: Vec<StoredSpace>,
    pub folders: Vec<StoredFolder>,
    pub tabs: Vec<StoredTab>,
    pub active_space: Option<SpaceId>,
    pub active_tab: Option<TabId>,
    pub saved_at_ms: i64,
}

/// Database codes for [`Liveness`]; one table, both directions derive from it.
const LIVENESS_CODES: [(Liveness, i64); 4] = [
    (Liveness::Active, 0),
    (Liveness::Background, 1),
    (Liveness::Frozen, 2),
    (Liveness::Discarded, 3),
];

pub fn liveness_code(l: Liveness) -> i64 {
    LIVENESS_CODES
        .iter()
        .find(|(v, _)| *v == l)
        .map_or(0, |(_, c)| *c)
}

pub fn liveness_from_code(code: i64) -> Option<Liveness> {
    LIVENESS_CODES
        .iter()
        .find(|(_, c)| *c == code)
        .map(|(v, _)| *v)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn liveness_codes_round_trip() {
        for (l, _) in LIVENESS_CODES {
            assert_eq!(liveness_from_code(liveness_code(l)), Some(l));
        }
        assert_eq!(liveness_from_code(99), None);
    }
}
