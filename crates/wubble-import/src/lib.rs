//! Read Zen/Firefox and Arc profiles into plain data.
//!
//! Every parser is a pure function of file contents (`&[u8] -> Result<_, ImportError>`).
//! Only [`discover`] touches the file system. Nothing here writes to the store: the app
//! maps [`Imported`] onto the model (ids are allocated there, so output refers to spaces
//! and folders by index into the vectors of the same [`Imported`]).
//! Passwords and cookies are out of scope. The data types live in `wubble_core::import`
//! (re-exported here) so the store can apply them without depending on the parsers.

pub mod arc;
pub mod containers;
pub mod discover;
pub mod mozlz4;
pub mod places;
pub mod session;

pub use wubble_core::import::{
    BookmarkNode, Imported, ImportedFolder, ImportedSpace, ImportedTab, ImportedVisit, Layout,
    Placement, Source,
};

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

/// Only pages worth restoring: web and local files. Drops `about:`, `javascript:`,
/// `arc:`, `place:`, `moz-extension:` and the like.
pub(crate) fn is_web_url(url: &str) -> bool {
    ["http://", "https://", "file://"]
        .iter()
        .any(|p| url.starts_with(p))
}
