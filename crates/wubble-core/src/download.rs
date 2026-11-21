//! Downloads as the user sees them: where a file goes (a safe, unique name
//! in the downloads directory) and the list's entries, one enum of states
//! with a pure transition. The engine steers CEF's own download item
//! (`wubble_engine::download`); this is the part that is shown, persisted
//! and survives a restart.
//!
//! Ported rules (BSD-3-Clause, Chromium):
//! - [`sanitize_file_name`]: `net/base/filename_util.cc` `GenerateFileName`
//!   and `net/base/filename_util_internal.cc` `SanitizeGeneratedFileName`:
//!   directory parts dropped, characters no file system takes replaced,
//!   leading and trailing dots and spaces trimmed, Windows device names
//!   prefixed, an empty name becomes "download".
//! - [`unique_name`]: `base/files/file_util.cc` `GetUniquePathNumber` (" (1)"
//!   up to " (100)" before the extension) with the in-flight reservations of
//!   `chrome/browser/download/download_path_reservation_tracker.cc` folded
//!   into the injected `taken` predicate.

use std::path::{Path, PathBuf};

use crate::space::ContainerId;

/// A name used when the page suggests nothing usable.
pub const FALLBACK_NAME: &str = "download";

/// Most numbered variants tried before giving up (Chromium's limit).
pub const MAX_UNIQUE: u32 = 100;

/// Longest file name in bytes most file systems take.
const MAX_NAME_BYTES: usize = 255;

/// Characters no common file system takes in a name.
const ILLEGAL: &[char] = &['/', '\\', ':', '*', '?', '"', '<', '>', '|'];

/// Windows device names, which no file may be called (with any extension).
const RESERVED: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// Whether a download asks where to save it: always with the "Always ask where to save"
/// setting, and for a Save ... As the user chose from a menu. Otherwise it goes to the downloads
/// folder at once.
pub fn asks_where(always_ask: bool, save_as: bool) -> bool {
    always_ask || save_as
}

/// A file name safe on every platform from what the page suggested.
pub fn sanitize_file_name(suggested: &str) -> String {
    let last = suggested
        .rsplit(['/', '\\'])
        .find(|part| !part.trim().is_empty())
        .unwrap_or("");
    let replaced: String = last
        .chars()
        .map(|c| {
            if c.is_control() || ILLEGAL.contains(&c) {
                '_'
            } else {
                c
            }
        })
        .collect();
    let trimmed = replaced.trim_matches(|c: char| c == '.' || c.is_whitespace());
    if trimmed.is_empty() {
        return FALLBACK_NAME.into();
    }
    let stem = trimmed.split('.').next().unwrap_or("");
    let name = if RESERVED.iter().any(|r| r.eq_ignore_ascii_case(stem)) {
        format!("_{trimmed}")
    } else {
        trimmed.to_string()
    };
    fit(&name, MAX_NAME_BYTES)
}

/// `name` cut to `max` bytes at a character boundary, keeping a short
/// extension.
fn fit(name: &str, max: usize) -> String {
    if name.len() <= max {
        return name.to_string();
    }
    let (stem, ext) = split_ext(name);
    let ext = if ext.len() < max / 2 { ext } else { "" };
    let mut keep = max - ext.len();
    while !stem.is_char_boundary(keep.min(stem.len())) {
        keep -= 1;
    }
    format!("{}{ext}", &stem[..keep.min(stem.len())])
}

/// `("archive.tar", ".gz")`; a name without a dot (or only a leading one)
/// has no extension.
fn split_ext(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        Some(i) if i > 0 => name.split_at(i),
        _ => (name, ""),
    }
}

/// Where a download suggested as `suggested` goes in `dir`: its sanitised
/// name, or "name (n).ext" for the first n up to [`MAX_UNIQUE`] that is not
/// `taken` (a file on disk or a path another running download reserved).
/// `None` when all are taken.
pub fn unique_name(dir: &Path, suggested: &str, taken: impl Fn(&Path) -> bool) -> Option<PathBuf> {
    let name = sanitize_file_name(suggested);
    let first = dir.join(&name);
    if !taken(&first) {
        return Some(first);
    }
    let (stem, ext) = split_ext(&name);
    (1..=MAX_UNIQUE)
        .map(|n| dir.join(format!("{stem} ({n}){ext}")))
        .find(|p| !taken(p))
}

// ------------------------------------------------------------ the list

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DownloadState {
    Running,
    /// Paused by the user.
    Paused,
    /// The engine lost the connection; it can still be resumed.
    Interrupted(String),
    Complete,
    Cancelled,
    /// Ended without a file; retrying starts over.
    Failed(String),
}

impl DownloadState {
    /// Still has (or may get) bytes from the engine.
    pub fn is_active(&self) -> bool {
        matches!(self, Self::Running | Self::Paused | Self::Interrupted(_))
    }

    /// The stored name. Stable: never rename one.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Paused => "paused",
            Self::Interrupted(_) => "interrupted",
            Self::Complete => "complete",
            Self::Cancelled => "cancelled",
            Self::Failed(_) => "failed",
        }
    }

    /// The state stored as `name`, with its error text (for interrupted and
    /// failed); `None` for an unknown name.
    pub fn parse(name: &str, error: Option<String>) -> Option<DownloadState> {
        let error = || error.clone().unwrap_or_default();
        Some(match name {
            "running" => Self::Running,
            "paused" => Self::Paused,
            "interrupted" => Self::Interrupted(error()),
            "complete" => Self::Complete,
            "cancelled" => Self::Cancelled,
            "failed" => Self::Failed(error()),
            _ => return None,
        })
    }

    pub fn error(&self) -> Option<&str> {
        match self {
            Self::Interrupted(e) | Self::Failed(e) => Some(e),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DownloadEvent {
    // From the engine.
    Progress {
        received: u64,
        total: Option<u64>,
    },
    Paused,
    Resumed,
    Finished,
    Interrupted(String),
    Cancelled,
    /// Ended without a file, for this reason (a media download that cannot be saved).
    Failed(String),
    /// The engine no longer has this download: the process restarted, or
    /// the view it belonged to closed.
    Lost,
    // From the user.
    Pause,
    Resume,
    Cancel,
    Retry,
    Remove,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DownloadEffect {
    /// Write the entry to the store.
    Save,
    PauseEngine,
    ResumeEngine,
    CancelEngine,
    /// Start the same URL again as a new download, which replaces this
    /// entry: the host forgets this one only once the new one has started,
    /// so a retry that cannot start keeps it.
    StartAgain,
    /// Drop the entry from the list and the store.
    Forget,
}

/// Why a download the engine lost failed.
pub const LOST: &str = "Interrupted";

pub fn transition(
    state: DownloadState,
    event: DownloadEvent,
) -> (DownloadState, Vec<DownloadEffect>) {
    use DownloadEffect as Fx;
    use DownloadEvent as E;
    use DownloadState as S;
    let active = state.is_active();
    match (state, event) {
        (s, E::Progress { .. }) => (s, vec![]),
        (S::Running, E::Paused) => (S::Paused, vec![Fx::Save]),
        (S::Paused | S::Interrupted(_), E::Resumed) => (S::Running, vec![Fx::Save]),
        (_, E::Finished) if active => (S::Complete, vec![Fx::Save]),
        (_, E::Interrupted(e)) if active => (S::Interrupted(e), vec![Fx::Save]),
        (_, E::Cancelled) if active => (S::Cancelled, vec![Fx::Save]),
        (_, E::Lost) if active => (S::Failed(LOST.into()), vec![Fx::Save]),
        (_, E::Failed(e)) if active => (S::Failed(e), vec![Fx::Save]),
        (S::Running, E::Pause) => (S::Paused, vec![Fx::PauseEngine, Fx::Save]),
        (S::Paused | S::Interrupted(_), E::Resume | E::Retry) => {
            (S::Running, vec![Fx::ResumeEngine, Fx::Save])
        }
        (_, E::Cancel) if active => (S::Cancelled, vec![Fx::CancelEngine, Fx::Save]),
        (s @ (S::Cancelled | S::Failed(_)), E::Retry) => (s, vec![Fx::StartAgain]),
        // Removing a running download ends it first: nothing of it stays.
        (_, E::Remove) if active => (S::Cancelled, vec![Fx::CancelEngine, Fx::Forget]),
        (s, E::Remove) => (s, vec![Fx::Forget]),
        (s, _) => (s, vec![]),
    }
}

/// One row of the downloads list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DownloadEntry {
    /// The store's row id, assigned by the host.
    pub id: i64,
    pub url: String,
    /// The file's name as saved.
    pub name: String,
    pub path: PathBuf,
    pub received: u64,
    pub total: Option<u64>,
    pub state: DownloadState,
    /// Unix ms.
    pub started_ms: i64,
    /// The container whose cookies started this download.
    pub container: ContainerId,
}

impl DownloadEntry {
    /// Runs `event` through [`transition`]; progress counts only while the
    /// download is active, and a finished one counts every byte.
    pub fn apply(self, event: DownloadEvent) -> (DownloadEntry, Vec<DownloadEffect>) {
        let progress = match (&event, self.state.is_active()) {
            (DownloadEvent::Progress { received, total }, true) => Some((*received, *total)),
            _ => None,
        };
        let (state, effects) = transition(self.state.clone(), event);
        let mut entry = DownloadEntry { state, ..self };
        if let Some((received, total)) = progress {
            entry.received = received;
            entry.total = total.or(entry.total);
        }
        if entry.state == DownloadState::Complete {
            entry.total = Some(entry.total.unwrap_or(entry.received).max(entry.received));
            entry.received = entry.total.unwrap_or(entry.received);
        }
        (entry, effects)
    }

    /// The bar's fill, 0 to 1; `None` while the size is unknown.
    pub fn fraction(&self) -> Option<f32> {
        match self.state {
            DownloadState::Complete => Some(1.0),
            _ => self
                .total
                .filter(|&t| t > 0)
                .map(|t| (self.received as f64 / t as f64).clamp(0.0, 1.0) as f32),
        }
    }

    /// The row's second line.
    pub fn detail(&self) -> String {
        let amount = match self.total {
            Some(total) => format!("{} of {}", format_bytes(self.received), format_bytes(total)),
            None => format_bytes(self.received),
        };
        match &self.state {
            DownloadState::Running => amount,
            DownloadState::Paused => format!("Paused, {amount}"),
            DownloadState::Interrupted(e) => format!("Interrupted: {e}"),
            DownloadState::Complete => format_bytes(self.received),
            DownloadState::Cancelled => "Cancelled".into(),
            DownloadState::Failed(e) => format!("Failed: {e}. Retry starts over"),
        }
    }
}

/// The ids of the entries that are done with (not active): what "Clear finished" removes.
pub fn finished_ids(entries: &[DownloadEntry]) -> Vec<i64> {
    entries
        .iter()
        .filter(|e| !e.state.is_active())
        .map(|e| e.id)
        .collect()
}

/// What the downloads page's "Clear finished" removes while its search shows `query`: the
/// finished entries, or none while a search hides some rows (the button is off then, so it
/// never clears rows the user cannot see).
pub fn clearable_ids(entries: &[DownloadEntry], query: &str) -> Vec<i64> {
    if query.trim().is_empty() {
        finished_ids(entries)
    } else {
        vec![]
    }
}

/// Entries whose file name or URL matches `query` (case-insensitive; blank returns all).
pub fn filter_entries(entries: &[DownloadEntry], query: &str) -> Vec<DownloadEntry> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return entries.to_vec();
    }
    entries
        .iter()
        .filter(|e| e.name.to_lowercase().contains(&q) || e.url.to_lowercase().contains(&q))
        .cloned()
        .collect()
}

impl DownloadEntry {
    /// The file can be opened: the download completed.
    pub fn can_open(&self) -> bool {
        self.state == DownloadState::Complete
    }
}

/// Every entry the engine still had is failed: run at start, when no
/// download of the last run is alive.
pub fn lost_all(entries: Vec<DownloadEntry>) -> Vec<DownloadEntry> {
    entries
        .into_iter()
        .map(|e| e.apply(DownloadEvent::Lost).0)
        .collect()
}

/// "999 B", "1.5 KB", "12.3 MB", "4.0 GB" (decimal units, one decimal).
pub fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["KB", "MB", "GB", "TB"];
    if bytes < 1000 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64 / 1000.0;
    let mut unit = 0;
    while value >= 999.95 && unit < UNITS.len() - 1 {
        value /= 1000.0;
        unit += 1;
    }
    format!("{value:.1} {}", UNITS[unit])
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn a_download_asks_with_the_setting_or_for_save_as() {
        assert!(!asks_where(false, false));
        assert!(asks_where(false, true));
        assert!(asks_where(true, false));
        assert!(asks_where(true, true));
    }

    #[test]
    fn sanitizing() {
        let cases = [
            ("report.pdf", "report.pdf"),
            ("../../etc/passwd", "passwd"),
            ("C:\\Users\\x\\evil.exe", "evil.exe"),
            ("dir/", "dir"),
            ("a:b*c?d\"e<f>g|h.txt", "a_b_c_d_e_f_g_h.txt"),
            ("tab\there\n.txt", "tab_here_.txt"),
            ("name. . .", "name"),
            ("  spaced  ", "spaced"),
            (".bashrc", "bashrc"),
            ("CON", "_CON"),
            ("nul.txt", "_nul.txt"),
            ("com1.tar.gz", "_com1.tar.gz"),
            ("console.txt", "console.txt"),
            ("", FALLBACK_NAME),
            ("...", FALLBACK_NAME),
            ("///", FALLBACK_NAME),
        ];
        for (input, want) in cases {
            assert_eq!(sanitize_file_name(input), want, "{input:?}");
        }
    }

    #[test]
    fn long_names_are_cut_at_a_character_keeping_the_extension() {
        let long = format!("{}.txt", "é".repeat(300));
        let name = sanitize_file_name(&long);
        assert!(name.len() <= 255, "{}", name.len());
        assert!(name.ends_with(".txt"));
        assert!(name.starts_with('é'));
        let plain = "x".repeat(400);
        assert_eq!(sanitize_file_name(&plain).len(), 255);
    }

    fn taken_set(names: &[&str]) -> impl Fn(&Path) -> bool {
        let set: BTreeSet<PathBuf> = names.iter().map(|n| Path::new("/d").join(n)).collect();
        move |p: &Path| set.contains(p)
    }

    #[test]
    fn unique_names() {
        let dir = Path::new("/d");
        let at = |n: &str| Some(dir.join(n));
        assert_eq!(unique_name(dir, "a.txt", taken_set(&[])), at("a.txt"));
        assert_eq!(
            unique_name(dir, "a.txt", taken_set(&["a.txt"])),
            at("a (1).txt")
        );
        assert_eq!(
            unique_name(dir, "a.txt", taken_set(&["a.txt", "a (1).txt"])),
            at("a (2).txt")
        );
        assert_eq!(unique_name(dir, "a", taken_set(&["a"])), at("a (1)"));
        assert_eq!(
            unique_name(dir, "x.tar.gz", taken_set(&["x.tar.gz"])),
            at("x.tar (1).gz")
        );
        assert_eq!(
            unique_name(dir, "../a.txt", taken_set(&["a.txt"])),
            at("a (1).txt")
        );
    }

    #[test]
    fn unique_name_gives_up_after_the_limit() {
        let dir = Path::new("/d");
        assert_eq!(unique_name(dir, "a.txt", |_| true), None);
        let mut names = vec!["a.txt".to_string()];
        names.extend((1..MAX_UNIQUE).map(|n| format!("a ({n}).txt")));
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        assert_eq!(
            unique_name(dir, "a.txt", taken_set(&refs)),
            Some(dir.join(format!("a ({MAX_UNIQUE}).txt")))
        );
    }

    fn states() -> Vec<DownloadState> {
        use DownloadState::*;
        vec![
            Running,
            Paused,
            Interrupted("net".into()),
            Complete,
            Cancelled,
            Failed("x".into()),
        ]
    }

    fn events() -> Vec<DownloadEvent> {
        use DownloadEvent::*;
        vec![
            Progress {
                received: 5,
                total: Some(10),
            },
            Paused,
            Resumed,
            Finished,
            Interrupted("net".into()),
            Cancelled,
            Failed("bad".into()),
            Lost,
            Pause,
            Resume,
            Cancel,
            Retry,
            Remove,
        ]
    }

    /// The table written out a second way: every (state, event) pair.
    fn expected(s: &DownloadState, e: &DownloadEvent) -> (DownloadState, Vec<DownloadEffect>) {
        use DownloadEffect as Fx;
        use DownloadEvent as E;
        use DownloadState as S;
        let same = (s.clone(), vec![]);
        let active = matches!(s, S::Running | S::Paused | S::Interrupted(_));
        match e {
            E::Progress { .. } => same,
            E::Paused if *s == S::Running => (S::Paused, vec![Fx::Save]),
            E::Resumed if matches!(s, S::Paused | S::Interrupted(_)) => {
                (S::Running, vec![Fx::Save])
            }
            E::Finished if active => (S::Complete, vec![Fx::Save]),
            E::Interrupted(err) if active => (S::Interrupted(err.clone()), vec![Fx::Save]),
            E::Cancelled if active => (S::Cancelled, vec![Fx::Save]),
            E::Lost if active => (S::Failed(LOST.into()), vec![Fx::Save]),
            E::Failed(err) if active => (S::Failed(err.clone()), vec![Fx::Save]),
            E::Pause if *s == S::Running => (S::Paused, vec![Fx::PauseEngine, Fx::Save]),
            E::Resume | E::Retry if matches!(s, S::Paused | S::Interrupted(_)) => {
                (S::Running, vec![Fx::ResumeEngine, Fx::Save])
            }
            E::Cancel if active => (S::Cancelled, vec![Fx::CancelEngine, Fx::Save]),
            E::Retry if matches!(s, S::Cancelled | S::Failed(_)) => {
                (s.clone(), vec![Fx::StartAgain])
            }
            E::Remove if active => (S::Cancelled, vec![Fx::CancelEngine, Fx::Forget]),
            E::Remove => (s.clone(), vec![Fx::Forget]),
            _ => same,
        }
    }

    #[test]
    fn transition_covers_every_pair() {
        for s in states() {
            for e in events() {
                assert_eq!(
                    transition(s.clone(), e.clone()),
                    expected(&s, &e),
                    "{s:?} + {e:?}"
                );
            }
        }
    }

    #[test]
    fn removing_a_running_download_ends_it() {
        let (s, fx) = transition(DownloadState::Running, DownloadEvent::Remove);
        assert!(!s.is_active(), "its in-flight state must be released");
        assert_eq!(
            fx,
            vec![DownloadEffect::CancelEngine, DownloadEffect::Forget]
        );
    }

    #[test]
    fn a_retry_does_not_forget_the_entry_by_itself() {
        let failed = DownloadState::Failed("x".into());
        let (s, fx) = transition(failed.clone(), DownloadEvent::Retry);
        assert_eq!((s, fx), (failed, vec![DownloadEffect::StartAgain]));
    }

    #[test]
    fn ended_downloads_ignore_the_engine() {
        for s in [DownloadState::Complete, DownloadState::Cancelled] {
            for e in events() {
                if matches!(e, DownloadEvent::Retry | DownloadEvent::Remove) {
                    continue;
                }
                assert_eq!(
                    transition(s.clone(), e.clone()),
                    (s.clone(), vec![]),
                    "{e:?}"
                );
            }
        }
    }

    #[test]
    fn state_names_round_trip() {
        for s in states() {
            let error = s.error().map(String::from);
            assert_eq!(DownloadState::parse(s.as_str(), error), Some(s.clone()));
        }
        assert_eq!(DownloadState::parse("done", None), None);
    }

    fn entry(state: DownloadState) -> DownloadEntry {
        DownloadEntry {
            id: 1,
            url: "https://a.test/f.zip".into(),
            name: "f.zip".into(),
            path: "/d/f.zip".into(),
            received: 0,
            total: None,
            state,
            started_ms: 0,
            container: ContainerId::DEFAULT,
        }
    }

    #[test]
    fn finished_ids_skip_active_entries() {
        use DownloadState::*;
        let with = |id, state| DownloadEntry { id, ..entry(state) };
        let list = [
            with(1, Running),
            with(2, Complete),
            with(3, Paused),
            with(4, Failed("x".into())),
            with(5, Cancelled),
            with(6, Interrupted("x".into())),
        ];
        assert_eq!(finished_ids(&list), [2, 4, 5]);
        assert_eq!(clearable_ids(&list, " "), [2, 4, 5]);
        assert!(
            clearable_ids(&list, "report").is_empty(),
            "a search hides rows: nothing is cleared"
        );
        assert!(entry(Complete).can_open());
        assert!(!entry(Cancelled).can_open());
        assert!(!entry(Running).can_open());
    }

    #[test]
    fn progress_counts_while_active() {
        let (e, fx) = entry(DownloadState::Running).apply(DownloadEvent::Progress {
            received: 1_500_000,
            total: Some(3_000_000),
        });
        assert!(fx.is_empty());
        assert_eq!((e.received, e.total), (1_500_000, Some(3_000_000)));
        assert_eq!(e.fraction(), Some(0.5));
        assert_eq!(e.detail(), "1.5 MB of 3.0 MB");
        let (e, _) = e.apply(DownloadEvent::Pause);
        assert_eq!(e.detail(), "Paused, 1.5 MB of 3.0 MB");
        let (e, _) = e.apply(DownloadEvent::Finished);
        assert_eq!((e.received, e.fraction()), (3_000_000, Some(1.0)));
        assert_eq!(e.detail(), "3.0 MB");
        // A late progress report does not move an ended download.
        let (e, _) = e.apply(DownloadEvent::Progress {
            received: 1,
            total: None,
        });
        assert_eq!(e.received, 3_000_000);
    }

    #[test]
    fn unknown_size_and_errors() {
        let (e, _) = entry(DownloadState::Running).apply(DownloadEvent::Progress {
            received: 42,
            total: None,
        });
        assert_eq!((e.fraction(), e.detail()), (None, "42 B".to_string()));
        let (e, _) = e.apply(DownloadEvent::Interrupted("Network error".into()));
        assert_eq!(e.detail(), "Interrupted: Network error");
        let failed = lost_all(vec![e, entry(DownloadState::Complete)]);
        assert_eq!(failed[0].state, DownloadState::Failed(LOST.into()));
        assert_eq!(failed[0].detail(), "Failed: Interrupted. Retry starts over");
        assert_eq!(failed[1].state, DownloadState::Complete);
        assert_eq!(entry(DownloadState::Cancelled).detail(), "Cancelled");
    }

    #[test]
    fn bytes() {
        let cases = [
            (0, "0 B"),
            (999, "999 B"),
            (1000, "1.0 KB"),
            (1_536, "1.5 KB"),
            (999_999, "1.0 MB"),
            (12_345_678, "12.3 MB"),
            (4_000_000_000, "4.0 GB"),
            (u64::MAX, "18446744.1 TB"),
        ];
        for (n, want) in cases {
            assert_eq!(format_bytes(n), want, "{n}");
        }
    }

    #[test]
    fn filter_entries_matches_name_and_url_case_insensitively() {
        let a = DownloadEntry {
            id: 1,
            name: "Report-2026.PDF".into(),
            url: "https://docs.example.com/files/report.pdf".into(),
            ..entry(DownloadState::Complete)
        };
        let b = DownloadEntry {
            id: 2,
            name: "archive.tar.gz".into(),
            url: "https://cdn.wubble.test/builds/nightly.tar.gz".into(),
            ..entry(DownloadState::Running)
        };
        let list = [a.clone(), b.clone()];
        assert_eq!(filter_entries(&list, ""), list);
        assert_eq!(filter_entries(&list, "   "), list);
        assert_eq!(filter_entries(&list, "report"), std::slice::from_ref(&a));
        assert_eq!(filter_entries(&list, "PDF"), std::slice::from_ref(&a));
        assert_eq!(
            filter_entries(&list, "WUBBLE.TEST"),
            std::slice::from_ref(&b)
        );
        assert_eq!(filter_entries(&list, "nightly"), [b]);
        assert!(filter_entries(&list, "missing").is_empty());
    }
}
