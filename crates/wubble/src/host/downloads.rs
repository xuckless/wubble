//! The host's half of downloads: the engine's download events and the
//! popover's buttons, through `wubble_core::download` (names, the list's
//! machine), out to the engine, the store and the shell. The list here is
//! the one owner of what the popover shows; the store is its persistence.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};

use wubble_core::download::{self, DownloadEffect, DownloadEntry, DownloadEvent, DownloadState};
use wubble_core::tab::TabId;
use wubble_engine::page::{DownloadId, PageEvent};
use wubble_ui::{AppWindow, DownloadAction, DownloadItem, DownloadStatus};

use super::{Host, now_ms};

/// A download the engine still runs, from its path being given until its
/// entry ends.
#[derive(Debug)]
struct Inflight {
    tab: TabId,
    /// Reserved, so two at once never get the same name before either file
    /// exists.
    path: PathBuf,
    /// Its list entry, once started.
    entry: Option<i64>,
}

/// Downloads of this run and the last ones.
#[derive(Debug, Default)]
pub(super) struct Downloads {
    /// Where files go; `None`: no downloads directory, every download is
    /// declined.
    dir: Option<PathBuf>,
    /// Newest first.
    list: Vec<DownloadEntry>,
    next_id: i64,
    inflight: HashMap<DownloadId, Inflight>,
    /// PDFs being printed: their paths are reserved until `PdfPrinted` arrives.
    pdfs: HashSet<PathBuf>,
    /// Entries whose progress is not yet stored (written on the tick).
    unsaved: BTreeSet<i64>,
}

impl Downloads {
    pub fn new(dir: Option<PathBuf>, list: Vec<DownloadEntry>) -> Self {
        let next_id = list.iter().map(|e| e.id).max().unwrap_or(0) + 1;
        Self {
            dir,
            list,
            next_id,
            ..Self::default()
        }
    }

    /// A path in `dir` for `suggested` that no file, running download or running print has.
    fn unique_path(&self, suggested: &str) -> Option<PathBuf> {
        let dir = self.dir.as_deref()?;
        download::unique_name(dir, suggested, |p: &Path| {
            p.exists() || self.pdfs.contains(p) || self.inflight.values().any(|f| f.path == p)
        })
    }

    /// The print to `path` ended: its name is free to reuse.
    pub(super) fn finish_pdf(&mut self, path: &Path) {
        self.pdfs.remove(path);
    }

    fn entry_of(&self, download: DownloadId) -> Option<i64> {
        self.inflight.get(&download)?.entry
    }

    /// The engine's download behind entry `id`, while it runs.
    fn running(&self, id: i64) -> Option<(DownloadId, TabId)> {
        self.inflight
            .iter()
            .find_map(|(&d, f)| (f.entry == Some(id)).then_some((d, f.tab)))
    }

    /// Entries the engine still runs for `tab`.
    fn running_in(&self, tab: TabId) -> Vec<i64> {
        self.inflight
            .values()
            .filter(|f| f.tab == tab)
            .filter_map(|f| f.entry)
            .collect()
    }

    /// The tab holds a download the engine runs: it must not be discarded.
    pub fn holds(&self, tab: TabId) -> bool {
        self.inflight.values().any(|f| f.tab == tab)
    }

    fn get(&self, id: i64) -> Option<&DownloadEntry> {
        self.list.iter().find(|e| e.id == id)
    }
}

impl Host {
    pub(super) fn download_event(&mut self, window: &AppWindow, tab: TabId, event: PageEvent) {
        match event {
            PageEvent::DownloadRequested {
                id,
                download,
                suggested_name,
                ..
            } => {
                let path = self.download_path(&suggested_name);
                if path.is_none() {
                    self.toast(window, "Cannot download: no downloads folder");
                }
                if let Some(path) = &path {
                    let reserved = Inflight {
                        tab,
                        path: path.clone(),
                        entry: None,
                    };
                    self.downloads.inflight.insert(download, reserved);
                }
                if let Some(view) = self.tab_view(tab) {
                    view.answer_download_path(id, path.as_deref());
                }
            }
            PageEvent::DownloadStarted {
                download,
                url,
                path,
                total,
                ..
            } => {
                let d = &mut self.downloads;
                let entry = DownloadEntry {
                    id: d.next_id,
                    url,
                    name: path
                        .file_name()
                        .map_or_else(String::new, |n| n.to_string_lossy().into_owned()),
                    path,
                    received: 0,
                    total,
                    state: DownloadState::Running,
                    started_ms: now_ms(),
                };
                d.next_id += 1;
                d.inflight
                    .entry(download)
                    .or_insert_with(|| Inflight {
                        tab,
                        path: entry.path.clone(),
                        entry: None,
                    })
                    .entry = Some(entry.id);
                self.store.add_download(&entry);
                let text = format!("Downloading {}", entry.name);
                self.downloads.list.insert(0, entry);
                self.toast(window, &text);
                self.render_downloads(window);
                self.report_busy(tab);
            }
            PageEvent::DownloadProgress {
                download,
                received,
                total,
            } => self.engine_download(download, DownloadEvent::Progress { received, total }),
            PageEvent::DownloadPaused(download) => {
                self.engine_download(download, DownloadEvent::Paused)
            }
            PageEvent::DownloadResumed(download) => {
                self.engine_download(download, DownloadEvent::Resumed)
            }
            PageEvent::DownloadFinished { download, .. } => {
                self.engine_download(download, DownloadEvent::Finished)
            }
            PageEvent::DownloadFailed { download, error } => {
                self.engine_download(download, DownloadEvent::Interrupted(error))
            }
            PageEvent::DownloadCancelled(download) => match self.downloads.entry_of(download) {
                Some(id) => self.apply_download(id, DownloadEvent::Cancelled),
                // Cancelled before it started: only its reservation exists.
                None => {
                    self.downloads.inflight.remove(&download);
                    self.report_busy(tab);
                }
            },
            _ => {}
        }
    }

    /// Ctrl+P: saves `tab`'s page as a PDF in the downloads folder, named after its title. The
    /// result arrives as `PageEvent::PdfPrinted`, which toasts it (`Routed::Pdf`).
    pub(super) fn print(&mut self, tab: TabId) {
        let title = self
            .model
            .tabs
            .get(&tab)
            .map(crate::shell::tab_title)
            .filter(|t| !t.is_empty())
            .unwrap_or_else(|| "page".into());
        let failure = match self.download_path(&format!("{title}.pdf")) {
            Some(path) => match self.tab_view(tab).map(|view| view.print_to_pdf(&path)) {
                Some(Ok(())) => {
                    self.downloads.pdfs.insert(path);
                    return;
                }
                Some(Err(_)) => "Could not save the PDF",
                None => return,
            },
            None => "Cannot print: no downloads folder",
        };
        if let Some(window) = self.w.window.upgrade() {
            self.toast(&window, failure);
        }
    }

    /// A unique path in the downloads directory, created if needed.
    fn download_path(&self, suggested: &str) -> Option<PathBuf> {
        let dir = self.downloads.dir.as_deref()?;
        if let Err(e) = std::fs::create_dir_all(dir) {
            eprintln!("wubble: downloads folder {}: {e}", dir.display());
            return None;
        }
        self.downloads.unique_path(suggested)
    }

    fn engine_download(&mut self, download: DownloadId, event: DownloadEvent) {
        if let Some(id) = self.downloads.entry_of(download) {
            self.apply_download(id, event);
        }
    }

    /// The popover's and the downloads page's buttons.
    pub fn download_action(&mut self, id: i64, action: DownloadAction) {
        let event = match action {
            DownloadAction::Pause => DownloadEvent::Pause,
            DownloadAction::Resume => DownloadEvent::Resume,
            DownloadAction::Cancel => DownloadEvent::Cancel,
            DownloadAction::Retry => DownloadEvent::Retry,
            DownloadAction::Remove => DownloadEvent::Remove,
            DownloadAction::Reveal => {
                if let Some(e) = self.downloads.get(id) {
                    reveal(&e.path);
                }
                return;
            }
            DownloadAction::Open => {
                let path = self
                    .downloads
                    .get(id)
                    .filter(|e| e.can_open())
                    .map(|e| e.path.clone());
                match path {
                    Some(path) if path.exists() => {
                        if let Err(e) = opener::open(&path) {
                            eprintln!("wubble: cannot open {}: {e}", path.display());
                        }
                    }
                    _ => {
                        if let Some(window) = self.w.window.upgrade() {
                            self.toast(&window, "File not found");
                        }
                    }
                }
                return;
            }
        };
        self.apply_download(id, event);
    }

    /// "Clear finished": every entry that is done goes, through the one remove path.
    pub(crate) fn downloads_cleared(&mut self) {
        for id in download::finished_ids(&self.downloads.list) {
            self.apply_download(id, DownloadEvent::Remove);
        }
    }

    /// The tab's view is about to be closed by us (the tab closed, or its
    /// renderer is gone while hidden): CEF would carry on with its downloads
    /// unseen, so they are cancelled, and the user told why. A busy tab is
    /// never discarded by the sleep policy (`report_busy`).
    pub(super) fn downloads_closing(&mut self, tab: TabId) {
        let ids = self.downloads.running_in(tab);
        for &id in &ids {
            self.apply_download(id, DownloadEvent::Cancel);
        }
        if !ids.is_empty()
            && let Some(window) = self.w.window.upgrade()
        {
            self.toast(&window, "Download cancelled: its tab closed");
        }
    }

    /// The tab's view closed by itself: its downloads report nothing more.
    pub(super) fn downloads_lost(&mut self, tab: TabId) {
        for id in self.downloads.running_in(tab) {
            self.apply_download(id, DownloadEvent::Lost);
        }
        self.downloads.inflight.retain(|_, f| f.tab != tab);
    }

    fn apply_download(&mut self, id: i64, event: DownloadEvent) {
        let Some(at) = self.downloads.list.iter().position(|e| e.id == id) else {
            return;
        };
        let progress = matches!(event, DownloadEvent::Progress { .. });
        let entry = self.downloads.list[at].clone();
        let (entry, effects) = entry.apply(event);
        self.downloads.list[at] = entry.clone();
        if progress {
            self.downloads.unsaved.insert(id);
        }
        let running = self.downloads.running(id);
        for effect in effects {
            match effect {
                DownloadEffect::Save => {
                    self.downloads.unsaved.remove(&id);
                    self.store.update_download(&entry);
                }
                DownloadEffect::PauseEngine
                | DownloadEffect::ResumeEngine
                | DownloadEffect::CancelEngine => {
                    let Some((download, tab)) = running else {
                        continue;
                    };
                    let Some(view) = self.tab_view(tab) else {
                        continue;
                    };
                    match effect {
                        DownloadEffect::PauseEngine => view.pause_download(download),
                        DownloadEffect::ResumeEngine => view.resume_download(download),
                        _ => view.cancel_download(download),
                    }
                }
                DownloadEffect::StartAgain => {
                    // Any live view can start it (the downloads page's own tab has none, so
                    // the visible tab's, else another's). The new download replaces this
                    // entry; without a view the entry stays for a later retry.
                    if let Some(view) = self.any_view() {
                        view.start_download(&entry.url);
                        self.forget_download(id);
                    } else if let Some(window) = self.w.window.upgrade() {
                        self.toast(&window, "Open a page to retry the download");
                    }
                }
                DownloadEffect::Forget => self.forget_download(id),
            }
        }
        // The one place an entry's engine download ends.
        if !entry.state.is_active()
            && let Some((download, tab)) = running
        {
            self.downloads.inflight.remove(&download);
            self.report_busy(tab);
        }
        if let Some(window) = self.w.window.upgrade() {
            self.render_downloads(&window);
        }
    }

    fn forget_download(&mut self, id: i64) {
        self.downloads.list.retain(|e| e.id != id);
        self.downloads.unsaved.remove(&id);
        self.store.remove_download(id);
    }

    /// Writes progress not yet stored (the lifecycle tick and shutdown).
    pub(super) fn save_download_progress(&mut self) {
        for id in std::mem::take(&mut self.downloads.unsaved) {
            if let Some(e) = self.downloads.get(id) {
                self.store.update_download(e);
            }
        }
    }

    pub(super) fn render_downloads(&self, window: &AppWindow) {
        let items: Vec<DownloadItem> = self.downloads.list.iter().map(item).collect();
        window.set_downloads(super::surfaces::model(items));
    }
}

/// One row of the popover.
fn item(e: &DownloadEntry) -> DownloadItem {
    use DownloadState as S;
    DownloadItem {
        id: i32::try_from(e.id).unwrap_or(i32::MAX),
        name: e.name.as_str().into(),
        detail: e.detail().into(),
        progress: e.fraction().unwrap_or(-1.0),
        status: match e.state {
            S::Running => DownloadStatus::Running,
            S::Paused => DownloadStatus::Paused,
            S::Interrupted(_) => DownloadStatus::Interrupted,
            S::Complete => DownloadStatus::Complete,
            S::Cancelled => DownloadStatus::Cancelled,
            S::Failed(_) => DownloadStatus::Failed,
        },
    }
}

/// Shows the file in the file manager (its folder where it cannot select).
fn reveal(path: &Path) {
    if let Err(e) = opener::reveal(path) {
        eprintln!("wubble: cannot show {}: {e}", path.display());
        if let Some(dir) = path.parent()
            && let Err(e) = opener::open(dir)
        {
            eprintln!("wubble: cannot open {}: {e}", dir.display());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: i64, state: DownloadState) -> DownloadEntry {
        DownloadEntry {
            id,
            url: String::new(),
            name: "f.zip".into(),
            path: "/d/f.zip".into(),
            received: 1,
            total: Some(4),
            state,
            started_ms: 0,
        }
    }

    #[test]
    fn a_pdf_being_printed_keeps_its_name() {
        let mut d = Downloads::new(Some("/wubble-test-no-such-dir".into()), vec![]);
        let first = d.unique_path("Page.pdf").unwrap();
        d.pdfs.insert(first.clone());
        let second = d.unique_path("Page.pdf").unwrap();
        assert_ne!(first, second);
        d.finish_pdf(&first);
        assert_eq!(d.unique_path("Page.pdf"), Some(first));
    }

    #[test]
    fn ids_continue_after_the_loaded_ones() {
        let d = Downloads::new(
            None,
            vec![
                entry(7, DownloadState::Complete),
                entry(3, DownloadState::Complete),
            ],
        );
        assert_eq!(d.next_id, 8);
        assert_eq!(Downloads::new(None, vec![]).next_id, 1);
    }

    #[test]
    fn rows_carry_the_state_and_progress() {
        let row = item(&entry(1, DownloadState::Running));
        assert_eq!((row.status, row.progress), (DownloadStatus::Running, 0.25));
        let row = item(&entry(1, DownloadState::Interrupted("x".into())));
        assert_eq!(row.status, DownloadStatus::Interrupted);
        let row = item(&entry(1, DownloadState::Complete));
        assert_eq!((row.status, row.progress), (DownloadStatus::Complete, 1.0));
        let unknown = DownloadEntry {
            total: None,
            ..entry(1, DownloadState::Running)
        };
        assert_eq!(item(&unknown).progress, -1.0);
    }
}
