//! Spaces (workspaces): an ordered set of tabs with its own pinned section, tint and
//! default container. Placement of tabs lives only here; see `model` for mutation.

use crate::tab::TabId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SpaceId(pub u32);

/// A cookie/cache context. 0 is the built-in Default and is never deletable
/// (Firefox `userContextId` / Zen `containerTabId` convention).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ContainerId(pub u32);

impl ContainerId {
    pub const DEFAULT: ContainerId = ContainerId(0);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FolderId(pub u32);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Container {
    pub id: ContainerId,
    pub name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb(pub u8, pub u8, pub u8);

/// One level of grouping. Holds unpinned tabs only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Folder {
    pub id: FolderId,
    pub name: String,
    pub tabs: Vec<TabId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Item {
    Tab(TabId),
    Folder(Folder),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Space {
    pub id: SpaceId,
    pub name: String,
    pub tint: Rgb,
    /// Default container of tabs without an override. Spaces may share one.
    pub container: ContainerId,
    pub pinned: Vec<TabId>,
    pub items: Vec<Item>,
    /// `Some` exactly when the Space has a tab; remembered while another Space is shown.
    pub active: Option<TabId>,
}

impl Space {
    pub fn folder_mut(&mut self, id: FolderId) -> Option<&mut Folder> {
        self.items.iter_mut().find_map(|i| match i {
            Item::Folder(f) if f.id == id => Some(f),
            _ => None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn container_ids_compare_by_value() {
        assert_eq!(ContainerId(3), ContainerId(3));
        assert_ne!(ContainerId(3), ContainerId(4));
    }
}
