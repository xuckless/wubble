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

/// Isolated cookie/cache jar. A Space has a default container; Spaces may share one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ContainerId(pub u32);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn container_ids_compare_by_value() {
        assert_eq!(ContainerId(3), ContainerId(3));
        assert_ne!(ContainerId(3), ContainerId(4));
    }
}
