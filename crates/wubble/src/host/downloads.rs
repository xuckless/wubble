//! The host's half of downloads: the engine's download events and the
//! popover's buttons, through `wubble_core::download` (names, the list's
//! machine), out to the engine, the store and the shell. The list here is
//! the one owner of what the popover shows; the store is its persistence.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};

use slint::ComponentHandle;
use wubble_core::agent::log::Outcome;
use wubble_core::download::{self, DownloadEffect, DownloadEntry, DownloadEvent, DownloadState};
use wubble_core::space::ContainerId;
use wubble_core::tab::TabId;
use wubble_engine::page::{DownloadId, FileChooserMode, PageEvent, RequestId};
use wubble_engine::view::View;
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
    /// Files wubble itself is writing (a print's PDF until `PdfPrinted`, a screenshot until
    /// it is saved): their paths are reserved meanwhile.
    reserved: HashSet<PathBuf>,
    /// Entries whose progress is not yet stored (written on the tick).
    unsaved: BTreeSet<i64>,
    /// "Always ask where to save" (`settings::DOWNLOADS_ASK_WHERE`).
    pub(super) ask_where: bool,
    /// No save dialog: a download that would ask goes to the downloads folder (`--smoke
    /// parity`, where an OS dialog would wait for a person).
    pub(super) never_ask: bool,
    /// Save ... As downloads started from a menu and not yet offered: (tab, URL). Each asks
    /// where to save it.
    save_as: Vec<(TabId, String)>,
    /// The downloads page's search filter (the popover stays unfiltered).
    query: String,
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
            p.exists() || self.reserved.contains(p) || self.inflight.values().any(|f| f.path == p)
        })
    }

    /// The download of `url` (or, after redirects, `original`) in `tab` is a Save ... As: taken
    /// off the list.
    fn take_save_as(&mut self, tab: TabId, url: &str, original: &str) -> bool {
        let at = self
            .save_as
            .iter()
            .position(|(t, u)| *t == tab && (u == url || u == original));
        at.map(|i| self.save_as.remove(i)).is_some()
    }

    /// Holds `path` for a file wubble is about to write.
    pub(super) fn reserve(&mut self, path: &Path) {
        self.reserved.insert(path.to_path_buf());
    }

    /// The file at `path` is written (or not): its name is free to reuse.
    pub(super) fn release(&mut self, path: &Path) {
        self.reserved.remove(path);
    }

    /// Downloads listed (this run's and earlier ones, prints saved as PDF among them).
    pub(super) fn started(&self) -> usize {
        self.list.len()
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

    /// The list, newest first (`--smoke data`).
    pub(super) fn entries(&self) -> &[DownloadEntry] {
        &self.list
    }

    pub(super) fn get(&self, id: i64) -> Option<&DownloadEntry> {
        self.list.iter().find(|e| e.id == id)
    }

    /// The downloads folder, when there is one.
    pub(super) fn dir(&self) -> Option<&Path> {
        self.dir.as_deref()
    }

    /// A running download or print holds `path`.
    pub(super) fn reserved(&self, path: &Path) -> bool {
        self.reserved.contains(path) || self.inflight.values().any(|f| f.path == path)
    }
}

impl Host {
    pub(super) fn download_event(&mut self, window: &AppWindow, tab: TabId, event: PageEvent) {
        match event {
            PageEvent::DownloadRequested {
                id,
                download,
                url,
                original_url,
                suggested_name,
                ..
            } => {
                // An agent-driven tab's download is refused (no airlock yet: TODO.md, MCP server).
                if self.agent_drives(tab) {
                    self.download_path_chosen(tab, id, download, None);
                    match self.agent_grant_of(tab) {
                        Some((session, _)) => {
                            self.agent_log(&session, "download", Some(url), Outcome::Refused)
                        }
                        None => eprintln!("wubble: agent: a download in a driven tab was refused"),
                    }
                    return;
                }
                let save_as = self.downloads.take_save_as(tab, &url, &original_url);
                if download::asks_where(self.downloads.ask_where, save_as) {
                    return self.ask_download_path(window, tab, id, download, &suggested_name);
                }
                let path = self.download_path(&suggested_name);
                if path.is_none() {
                    self.toast(window, "Cannot download: no downloads folder");
                }
                self.download_path_chosen(tab, id, download, path);
            }
            PageEvent::DownloadStarted {
                download,
                url,
                path,
                total,
                ..
            } => {
                let container = self
                    .model
                    .tabs
                    .get(&tab)
                    .map_or(ContainerId::DEFAULT, |t| t.container);
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
                    container,
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

    /// Save ... As from a menu: downloads `url` with the page's cookies, asking where.
    pub(super) fn save_as(&mut self, tab: TabId, url: String) {
        if let Some(view) = self.tab_view(tab) {
            view.start_download(&url);
            self.downloads.save_as.push((tab, url));
        }
    }

    /// Opens the save dialog for a download (the file chooser's), proposing the downloads
    /// folder and the page's name; the download waits for the answer, and a cancelled dialog
    /// cancels it.
    fn ask_download_path(
        &mut self,
        window: &AppWindow,
        tab: TabId,
        id: RequestId,
        download: DownloadId,
        suggested: &str,
    ) {
        if self.downloads.never_ask {
            let path = self.download_path(suggested);
            return self.download_path_chosen(tab, id, download, path);
        }
        let name = download::sanitize_file_name(suggested);
        let proposed = self
            .downloads
            .dir
            .as_deref()
            .map_or_else(|| PathBuf::from(&name), |d| d.join(&name));
        let spec = crate::chooser::dialog_spec(FileChooserMode::Save, "Save as", &proposed, &[]);
        let dialog = crate::chooser::dialog(&spec, Some(window.window()));
        let task = slint::spawn_local(async move {
            let picked = crate::chooser::pick(spec, dialog).await;
            let path = picked.and_then(|p| p.into_iter().next());
            crate::with_host(|host| host.download_path_chosen(tab, id, download, path));
        });
        if let Err(e) = task {
            eprintln!("wubble: cannot show the save dialog: {e}");
            self.download_path_chosen(tab, id, download, None);
        }
    }

    /// The save dialog's answer: the download goes there, or is cancelled.
    fn download_path_chosen(
        &mut self,
        tab: TabId,
        id: RequestId,
        download: DownloadId,
        path: Option<PathBuf>,
    ) {
        let Some(view) = self.tab_view(tab) else {
            return;
        };
        view.answer_download_path(id, path.as_deref());
        if let Some(path) = path {
            let reserved = Inflight {
                tab,
                path,
                entry: None,
            };
            self.downloads.inflight.insert(download, reserved);
        }
    }

    /// A unique path in the downloads directory, created if needed.
    pub(super) fn download_path(&self, suggested: &str) -> Option<PathBuf> {
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

    pub(crate) fn downloads_query(&mut self, query: &str) {
        query.clone_into(&mut self.downloads.query);
        if let Some(window) = self.w.window.upgrade() {
            self.render_downloads(&window);
        }
    }

    /// The tab's view is about to be closed by us (the tab closed, or its
    /// renderer is gone while hidden): CEF would carry on with its downloads
    /// unseen, so they are cancelled, and the user told why. A busy tab is
    /// never discarded by the sleep policy (`report_busy`).
    pub(super) fn downloads_closing(&mut self, tab: TabId) {
        self.downloads.save_as.retain(|(t, _)| *t != tab);
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
        self.downloads.save_as.retain(|(t, _)| *t != tab);
        for id in self.downloads.running_in(tab) {
            self.apply_download(id, DownloadEvent::Lost);
        }
        self.downloads.inflight.retain(|_, f| f.tab != tab);
    }

    /// A new running entry of the list (a media download from a tab of `container`), stored and
    /// shown; its id.
    pub(super) fn add_download_entry(
        &mut self,
        url: &str,
        path: &Path,
        total: Option<u64>,
        container: ContainerId,
    ) -> i64 {
        let d = &mut self.downloads;
        let entry = DownloadEntry {
            id: d.next_id,
            url: url.to_owned(),
            name: file_name(path),
            path: path.to_path_buf(),
            received: 0,
            total,
            state: DownloadState::Running,
            started_ms: now_ms(),
            container,
        };
        d.next_id += 1;
        self.store.add_download(&entry);
        let (id, text) = (entry.id, format!("Downloading {}", entry.name));
        self.downloads.list.insert(0, entry);
        if let Some(window) = self.w.window.upgrade() {
            self.toast(&window, &text);
            self.render_downloads(&window);
        }
        id
    }

    /// A file wubble wrote itself (a print saved as PDF), listed as a finished download of `url`.
    pub(super) fn add_finished_download(&mut self, url: &str, path: &Path) {
        let size = std::fs::metadata(path).map_or(0, |m| m.len());
        let d = &mut self.downloads;
        let entry = DownloadEntry {
            id: d.next_id,
            url: url.to_owned(),
            name: file_name(path),
            path: path.to_path_buf(),
            received: size,
            total: Some(size),
            state: DownloadState::Complete,
            started_ms: now_ms(),
            container: ContainerId::DEFAULT,
        };
        d.next_id += 1;
        self.store.add_download(&entry);
        self.downloads.list.insert(0, entry);
        if let Some(window) = self.w.window.upgrade() {
            self.render_downloads(&window);
        }
    }

    /// The entry's file moved to `path` (a media download's final name).
    pub(super) fn set_download_path(&mut self, id: i64, path: &Path) {
        if let Some(e) = self.downloads.list.iter_mut().find(|e| e.id == id) {
            e.path = path.to_path_buf();
            e.name = file_name(path);
            self.store.update_download(e);
        }
    }

    pub(super) fn download_url(&self, id: i64) -> Option<String> {
        self.downloads.get(id).map(|e| e.url.clone())
    }

    pub(super) fn apply_download(&mut self, id: i64, event: DownloadEvent) {
        let Some(at) = self.downloads.list.iter().position(|e| e.id == id) else {
            return;
        };
        let progress = matches!(event, DownloadEvent::Progress { .. });
        let save = self.video_save_now();
        let entry = self.downloads.list[at].clone();
        let (entry, effects) = entry.apply(event);
        self.downloads.list[at] = entry.clone();
        if progress {
            self.downloads.unsaved.insert(id);
        }
        let running = self.downloads.running(id);
        // A media download's buttons steer its job, not a CEF download.
        if self.media.owns(id) {
            for effect in effects {
                match effect {
                    DownloadEffect::Save => {
                        self.downloads.unsaved.remove(&id);
                        self.store.update_download(&entry);
                    }
                    DownloadEffect::Forget => {
                        self.media_list_effect(id, DownloadEffect::Forget);
                        self.forget_download(id);
                    }
                    effect => self.media_list_effect(id, effect),
                }
            }
            if let Some(window) = self.w.window.upgrade() {
                self.render_downloads(&window);
            }
            // the video strip's save button shows the download too
            self.video_save_redraw(save);
            return;
        }
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
                DownloadEffect::StartAgain if super::media::is_stream_entry(&entry.url) => {
                    if let Some(window) = self.w.window.upgrade() {
                        self.toast(&window, "Open the page again to save this video");
                    }
                }
                DownloadEffect::StartAgain => {
                    // Retry in a non-agent live view belonging to the download's container so
                    // cookies never cross containers and agent sessions are never used.
                    if let Some(view) = self.view_for_download(entry.container) {
                        view.start_download(&entry.url);
                        self.forget_download(id);
                    } else if let Some(window) = self.w.window.upgrade() {
                        let msg = if entry.container == ContainerId::DEFAULT {
                            "Open a page to retry the download"
                        } else {
                            "Open a page in this container to retry the download"
                        };
                        self.toast(&window, msg);
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

    /// A non-agent live view in `container`, preferring the visible tab.
    fn view_for_download(&mut self, container: ContainerId) -> Option<&mut View> {
        let eligible = |t: TabId| {
            self.views.contains_key(&t)
                && !self.agent_drives(t)
                && self
                    .model
                    .tabs
                    .get(&t)
                    .is_some_and(|tab| tab.container == container)
        };
        let tab = self
            .shown()
            .filter(|&t| eligible(t))
            .or_else(|| self.views.keys().copied().find(|&t| eligible(t)))?;
        self.tab_view(tab)
    }

    pub(super) fn forget_download(&mut self, id: i64) {
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
        crate::bind::sync(window.get_downloads(), items, |m| window.set_downloads(m));
        if self.downloads.list.is_empty() {
            window.set_downloads_open(false);
        }
        let filtered: Vec<DownloadItem> =
            download::filter_entries(&self.downloads.list, &self.downloads.query)
                .iter()
                .map(item)
                .collect();
        crate::bind::sync(window.get_downloads_page_items(), filtered, |m| {
            window.set_downloads_page_items(m)
        });
        window.set_downloads_searching(!self.downloads.query.trim().is_empty());
        window.set_downloads_has_finished(!download::finished_ids(&self.downloads.list).is_empty());
    }
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned())
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
            container: ContainerId::DEFAULT,
        }
    }

    #[test]
    fn a_pdf_being_printed_keeps_its_name() {
        let mut d = Downloads::new(Some("/wubble-test-no-such-dir".into()), vec![]);
        let first = d.unique_path("Page.pdf").unwrap();
        d.reserve(&first);
        let second = d.unique_path("Page.pdf").unwrap();
        assert_ne!(first, second);
        d.release(&first);
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
