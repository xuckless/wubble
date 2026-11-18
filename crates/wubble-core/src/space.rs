//! Spaces (workspaces): an ordered set of tabs with its own pinned section and theme.

use crate::tab::TabId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SpaceId(pub u32);

#[derive(Debug, Clone)]
pub struct Space {
    pub id: SpaceId,
    pub name: String,
    pub pinned: Vec<TabId>,
    pub tabs: Vec<TabId>,
}
