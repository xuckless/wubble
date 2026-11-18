//! The host's half of downloads: the engine's download events and the
//! popover's buttons, through `wubble_core::download` (names, the list's
//! machine), out to the engine, the store and the shell. The list here is
//! the one owner of what the popover shows; the store is its persistence.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use wubble_core::download::{self, DownloadEffect, DownloadEntry, DownloadEvent, DownloadState};
use wubble_core::tab::TabId;
use wubble_engine::page::{DownloadId, PageEvent};
use wubble_ui::{AppWindow, DownloadAction, DownloadItem};

use super::{Host, now_ms};

/// Downloads of this run and the last ones.
#[derive(Debug, Default)]
pub(super) struct Downloads {
    /// Where files go; `None`: no downloads directory, every download is
    /// declined.
    dir: Option<PathBuf>,
    /// Newest first.
    list: Vec<DownloadEntry>,
    next_id: i64,
    /// The engine's downloads still running, by entry id.
    live: BTreeMap<i64, (TabId, DownloadId)>,
    /// Paths given to downloads that have not ended, so two at once never
    /// get the same name before either file exists.
    reserved: HashMap<DownloadId, PathBuf>,
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

    fn entry_of(&self, download: DownloadId) -> Option<i64> {
        self.live
            .iter()
            .find_map(|(&id, &(_, d))| (d == download).then_some(id))
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
                    self.downloads.reserved.insert(download, path.clone());
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
                d.live.insert(entry.id, (tab, download));
                self.store.add_download(&entry);
                let text = format!("Downloading {}", entry.name);
                self.downloads.list.insert(0, entry);
                self.toast(window, &text);
                self.render_downloads(window);
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
            PageEvent::DownloadCancelled(download) => {
                // Cancelled before it started: only its reservation exists.
                self.downloads.reserved.remove(&download);
                self.engine_download(download, DownloadEvent::Cancelled)
            }
            _ => {}
        }
    }

    /// A unique path in the downloads directory, created if needed.
    fn download_path(&self, suggested: &str) -> Option<PathBuf> {
        let dir = self.downloads.dir.as_deref()?;
        if let Err(e) = std::fs::create_dir_all(dir) {
            eprintln!("wubble: downloads folder {}: {e}", dir.display());
            return None;
        }
        let reserved = &self.downloads.reserved;
        download::unique_name(dir, suggested, |p: &Path| {
            p.exists() || reserved.values().any(|r| r == p)
        })
    }

    fn engine_download(&mut self, download: DownloadId, event: DownloadEvent) {
        if let Some(id) = self.downloads.entry_of(download) {
            self.apply_download(id, event);
        }
    }

    /// The popover's buttons.
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
        };
        self.apply_download(id, event);
    }

    /// The tab's view closed: its downloads report nothing more.
    pub(super) fn downloads_lost(&mut self, tab: TabId) {
        let ids: Vec<i64> = self
            .downloads
            .live
            .iter()
            .filter(|(_, (t, _))| *t == tab)
            .map(|(&id, _)| id)
            .collect();
        for id in ids {
            self.apply_download(id, DownloadEvent::Lost);
        }
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
        let live = self.downloads.live.get(&id).copied();
        for effect in effects {
            match effect {
                DownloadEffect::Save => {
                    self.downloads.unsaved.remove(&id);
                    self.store.update_download(&entry);
                }
                DownloadEffect::PauseEngine
                | DownloadEffect::ResumeEngine
                | DownloadEffect::CancelEngine => {
                    let Some((tab, download)) = live else {
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
                    // Any live view can start it; the visible tab's is at hand.
                    if let Some(view) = self.view() {
                        view.start_download(&entry.url);
                    } else if let Some(window) = self.window.upgrade() {
                        self.toast(&window, "Open a page to retry the download");
                    }
                }
                DownloadEffect::Forget => {
                    self.downloads.list.retain(|e| e.id != id);
                    self.downloads.unsaved.remove(&id);
                    self.store.remove_download(id);
                }
            }
        }
        if !entry.state.is_active()
            && let Some((_, download)) = self.downloads.live.remove(&id)
        {
            self.downloads.reserved.remove(&download);
        }
        if let Some(window) = self.window.upgrade() {
            self.render_downloads(&window);
        }
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
        active: e.state.is_active(),
        can_pause: e.state == S::Running,
        can_resume: matches!(e.state, S::Paused | S::Interrupted(_)),
        can_retry: matches!(e.state, S::Cancelled | S::Failed(_)),
        done: e.state == S::Complete,
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
    fn rows_offer_what_the_state_allows() {
        let row = item(&entry(1, DownloadState::Running));
        assert!(row.can_pause && !row.can_resume && !row.can_retry && row.active && !row.done);
        assert_eq!(row.progress, 0.25);
        let row = item(&entry(1, DownloadState::Interrupted("x".into())));
        assert!(row.can_resume && !row.can_pause);
        let row = item(&entry(1, DownloadState::Failed("x".into())));
        assert!(row.can_retry && !row.active);
        let row = item(&entry(1, DownloadState::Complete));
        assert!(row.done && !row.can_retry && row.progress == 1.0);
        let unknown = DownloadEntry {
            total: None,
            ..entry(1, DownloadState::Running)
        };
        assert_eq!(item(&unknown).progress, -1.0);
    }
}
