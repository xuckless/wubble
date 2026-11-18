//! Plain data a profile importer produces and the store applies. Spaces and folders are
//! referred to by index into the vectors of the same [`Imported`]; ids are allocated on apply.

use crate::space::{Container, ContainerId, Rgb};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Zen,
    Firefox,
    Arc,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportedSpace {
    pub name: String,
    /// Emoji or icon name as the source stores it.
    pub icon: Option<String>,
    pub tint: Option<Rgb>,
    /// Zen: the space's default container. Arc: always `DEFAULT`.
    pub container: ContainerId,
    /// Arc profile (a Chromium profile directory name); `None` for Zen/Firefox.
    pub profile: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportedFolder {
    /// Index into `Imported::spaces`.
    pub space: usize,
    /// Index into `Imported::folders`.
    pub parent: Option<usize>,
    pub name: String,
    pub collapsed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placement {
    Essential,
    Pinned,
    Normal,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportedTab {
    pub space: usize,
    pub folder: Option<usize>,
    pub placement: Placement,
    pub container: ContainerId,
    pub url: String,
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BookmarkNode {
    Link {
        title: String,
        url: String,
        added_ms: i64,
    },
    Folder {
        title: String,
        children: Vec<BookmarkNode>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportedVisit {
    pub url: String,
    pub title: String,
    pub at_ms: i64,
    pub typed: bool,
}

/// What a session (or Arc sidebar) file yields.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Layout {
    pub spaces: Vec<ImportedSpace>,
    pub folders: Vec<ImportedFolder>,
    pub tabs: Vec<ImportedTab>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Imported {
    pub source: Source,
    pub containers: Vec<Container>,
    pub spaces: Vec<ImportedSpace>,
    pub folders: Vec<ImportedFolder>,
    pub tabs: Vec<ImportedTab>,
    pub bookmarks: Vec<BookmarkNode>,
    /// Newest first.
    pub history: Vec<ImportedVisit>,
    /// Files that were present but unusable; the rest of the import is still returned.
    pub warnings: Vec<String>,
}

impl Imported {
    pub fn new(
        source: Source,
        containers: Vec<Container>,
        layout: Layout,
        bookmarks: Vec<BookmarkNode>,
        history: Vec<ImportedVisit>,
    ) -> Self {
        Imported {
            source,
            containers,
            spaces: layout.spaces,
            folders: layout.folders,
            tabs: layout.tabs,
            bookmarks,
            history,
            warnings: Vec::new(),
        }
    }
}
