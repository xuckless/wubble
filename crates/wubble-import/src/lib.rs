//! Read Zen/Firefox and Arc profiles into plain data.
//!
//! Every parser is a pure function of file contents (`&[u8] -> Result<_, ImportError>`).
//! Only [`discover`] touches the file system. Nothing here writes to the store: the app
//! maps [`Imported`] onto the model (ids are allocated there, so output refers to spaces
//! and folders by index into the vectors of the same [`Imported`]).
//! Passwords and cookies are out of scope.

pub mod arc;
pub mod containers;
pub mod discover;
pub mod mozlz4;
pub mod places;
pub mod session;

use wubble_core::space::{Container, ContainerId, Rgb};

#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    /// Bytes that do not match the expected format (bad framing, JSON, SQLite).
    #[error("unreadable: {0}")]
    Unreadable(&'static str),
    /// A valid format we deliberately do not handle (encrypted session files).
    #[error("unsupported: {0}")]
    Unsupported(&'static str),
    #[error("i/o: {0}")]
    Io(#[from] std::io::Error),
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
}

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
}

impl Imported {
    pub fn new(
        source: Source,
        containers: Vec<Container>,
        layout: Layout,
        places: places::Places,
    ) -> Self {
        Imported {
            source,
            containers,
            spaces: layout.spaces,
            folders: layout.folders,
            tabs: layout.tabs,
            bookmarks: places.bookmarks,
            history: places.history,
        }
    }
}

/// Only pages worth restoring: web and local files. Drops `about:`, `javascript:`,
/// `arc:`, `place:`, `moz-extension:` and the like.
pub(crate) fn is_web_url(url: &str) -> bool {
    ["http://", "https://", "file://"]
        .iter()
        .any(|p| url.starts_with(p))
}
