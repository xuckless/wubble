//! wubble's own database (Turso, SQLite-compatible schema): session, history, bookmarks, settings.
//!
//! Async runtime: none. Turso's futures are waker-based and need no executor, so every method
//! here is a plain `async fn`; the binary picks how to drive them. Write methods take
//! `&mut self` so the borrow checker serialises transactions on the single connection
//! (a second writer would get `Busy`). Times are unix milliseconds supplied by the caller.
//! Encryption is out of scope: the `logins` table holds opaque blobs the vault crate produces.
//!
//! The store applies a `wubble_core::import::Imported` profile (`import.rs`), and a
//! password vault adapts its storage trait to the plain byte-row functions in `logins.rs`;
//! both drive the futures with whatever executor they have (`pollster::block_on` is enough).

mod archive;
mod bookmarks;
mod boosts;
mod check;
mod closed;
mod downloads;
mod favicon;
mod history;
mod import;
mod logins;
mod migrate;
mod permissions;
mod session;
mod settings;

use std::path::Path;

pub use archive::ArchivedTab;
pub use bookmarks::{BookmarkId, BookmarkKind, BookmarkNode};
pub use closed::{ClosedTab, RECENTLY_CLOSED_MAX};
pub use favicon::Favicon;
pub use history::{HistoryDeleted, Transition, is_recordable, url_key};
pub use import::{ImportSummary, ItemKind, SkipReason, Skipped};
pub use logins::LoginRow;
pub use migrate::MIGRATIONS;
pub use settings::{
    ArchiveAfterSetting, BlockerCookieBanners, BlockerCustomLists, BlockerEnabled,
    BlockerExceptions, BlockerLists, BlockerListsAutoUpdate, BlockerStripParams, BoostsEnabled,
    DevtoolsRemotePort, GlassQuality, GlassScene, HintsAlphabet, HistoryRetentionDays,
    KeybindingOverrides, LiveSuggestions, MediaAskEachTime, MediaConcurrency, MediaEnabled,
    MediaKeys, MediaMinBytes, MediaQuality, MediaSaveDir, MediaYoutube, MemoryBackForwardCache,
    MemoryMaxDecodedImageMb, MiddleClickAutoscroll, OverlayScrollbars, PageThemeDefault,
    PageThemeSites, PasswordsAutofill, PasswordsOfferToSave, PipOnTabSwitch, ProcessRendererLimit,
    ProcessSiteIsolation, ProcessSpareRenderer, ReaderDetect, ReaderFontSetting, ReaderSize,
    ReaderThemeSetting, ReaderWidthSetting, RoutingRules, ScreenshotOutput, SearchEngineChoice,
    Setting, SidebarModeSetting, SidebarWidth, SiteZoom, SleepDiscardAfter, SleepFreezeAfter,
    SleepKeepPinned, SleepNeverSites, SponsorActions, SponsorAsked, SponsorEnabled, VideoCollapsed,
    VideoControls, VideoHiddenSites, VideoRates, ZoomDefault, text_of,
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
    /// A request the data model refuses (parent is not a folder, move into own subtree, ...).
    #[error("invalid request: {0}")]
    Invalid(String),
}

impl Error {
    /// A uniqueness or other constraint refused the write (for example a login id that exists).
    pub fn is_constraint(&self) -> bool {
        matches!(self, Error::Db(turso::Error::Constraint(_)))
    }
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

/// First column of every row as `i64`.
async fn ids(
    conn: &turso::Connection,
    sql: &str,
    params: impl turso::IntoParams,
) -> Result<Vec<i64>> {
    let mut rows = conn.query(sql, params).await?;
    let mut out = Vec::new();
    while let Some(r) = rows.next().await? {
        out.push(r.get(0)?);
    }
    Ok(out)
}

/// The first column of the first row, if any.
async fn scalar(
    conn: &turso::Connection,
    sql: &str,
    params: impl turso::IntoParams,
) -> Result<Option<i64>> {
    Ok(ids(conn, sql, params).await?.into_iter().next())
}

/// A blob column. `turso::Row::get` has no `Vec<u8>` reader, so go through `Value`.
fn blob(row: &turso::Row, idx: usize) -> Result<Vec<u8>> {
    match row.get_value(idx)? {
        turso::Value::Blob(b) => Ok(b),
        other => Err(Error::Corrupt(format!(
            "column {idx} is not a blob: {other:?}"
        ))),
    }
}
