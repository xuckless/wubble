//! wubble's own database (Turso, SQLite-compatible schema): session, history, bookmarks, settings.
//!
//! Async runtime: none. Turso's futures are waker-based and need no executor, so every method
//! here is a plain `async fn`; the binary picks how to drive them. Write methods take
//! `&mut self` so the borrow checker serialises transactions on the single connection
//! (a second writer would get `Busy`). Times are unix milliseconds supplied by the caller.
//! Passwords are out of scope.

mod closed;
mod history;
mod migrate;
mod session;
mod settings;

use std::path::Path;

pub use closed::{ClosedTab, RECENTLY_CLOSED_MAX};
pub use history::{HistoryHit, Transition, is_recordable, url_key};
pub use migrate::MIGRATIONS;
pub use settings::{
    BlockerEnabled, BlockerExceptions, Codec, CompactMode, KeybindingOverrides, LiveSuggestions,
    SearchEngine, SearchEngineChoice, Setting, SidebarWidth,
};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("database error: {0}")]
    Db(#[from] turso::Error),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("database schema v{found} is newer than this build supports (v{supported})")]
    DatabaseTooNew { found: i64, supported: i64 },
    #[error("corrupt stored value: {0}")]
    Corrupt(String),
}

pub type Result<T> = std::result::Result<T, Error>;

pub struct Store {
    // Keeps the database alive for the connection's lifetime.
    _db: turso::Database,
    conn: turso::Connection,
}

impl Store {
    /// Open (creating if needed) the database at `path`, then migrate it.
    pub async fn open(path: &Path) -> Result<Store> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let mut store = Self::connect(&path.to_string_lossy()).await?;
        migrate::migrate(&mut store.conn, MIGRATIONS).await?;
        Ok(store)
    }

    pub async fn open_in_memory() -> Result<Store> {
        let mut store = Self::connect(":memory:").await?;
        migrate::migrate(&mut store.conn, MIGRATIONS).await?;
        Ok(store)
    }

    async fn connect(path: &str) -> Result<Store> {
        let db = turso::Builder::new_local(path).build().await?;
        let conn = db.connect()?;
        // Per-connection setting, not stored in the file.
        conn.execute_batch("PRAGMA foreign_keys = ON").await?;
        Ok(Store { _db: db, conn })
    }
}

/// Read a column as `i64` and convert to a narrower id type.
fn narrow<T: TryFrom<i64>>(v: i64, what: &str) -> Result<T> {
    T::try_from(v).map_err(|_| Error::Corrupt(format!("{what} out of range: {v}")))
}
