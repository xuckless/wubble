//! The host's half of downloads: the engine's download events and the
//! popover's buttons, through `wubble_core::download` (names, the list's
//! machine), out to the engine, the store and the shell. The list here is
//! the one owner of what the popover shows; the store is its persistence.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};

use slint::ComponentHandle;
use wubble_core::agent::log::Outcome;
use wubble_core::click_guard::{self, Start};
use wubble_core::site::Permission;
use wubble_core::download::{self, DownloadEffect, DownloadEntry, DownloadEvent, DownloadState};
use wubble_core::model::Command;
use wubble_core::space::{ContainerId, Side};
use wubble_core::tab::TabId;
use wubble_core::window::{Closing, WindowId};
use wubble_engine::page::{DownloadId, FileChooserMode, PageEvent, RequestId};
use wubble_engine::view::View;
use wubble_ui::{AppWindow, DownloadAction, DownloadItem, DownloadStatus};

use super::{Host, now_ms};
use crate::prompt::{Ask, PermissionEvent, PermissionRequest};

/// A download the engine still runs, from its path being given until its
/// entry ends.
#[derive(Debug)]
struct Inflight {
    tab: TabId,
    /// The tab's container when the download was asked for: its row's, even if the tab
    /// closes before the engine starts it.
    container: ContainerId,
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
    /// The sites (origins) that downloaded this run: their next download a click aimed at
    /// goes ahead without asking (`click_guard::download`).
    sites: BTreeSet<String>,
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
                total,
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
                // what the browser itself started (Save As) is the user's; a smoke without
                // dialogs never asks
                if save_as || self.downloads.never_ask {
                    return self.download_go(window, tab, id, download, &suggested_name, save_as);
                }
                let page = self
                    .model
                    .tabs
                    .get(&tab)
                    .map(|t| t.url.clone())
                    .unwrap_or_default();
                let origin = wubble_core::site::origin_of(&page);
                // a private window's tab reads its own choices over the stored ones
                let remembered = origin
                    .as_deref()
                    .and_then(|o| self.site_view(tab).permissions().get(o, Permission::Downloads));
                let aimed = self
                    .tab_view(tab)
                    .and_then(|v| v.last_click())
                    .is_some_and(|(click, age)| click_guard::aimed(Some(&click), age));
                let before = origin.as_ref().is_some_and(|o| self.downloads.sites.contains(o));
                match click_guard::download(remembered, aimed, before) {
                    Start::Go => {
                        self.downloads.sites.extend(origin);
                        self.download_go(window, tab, id, download, &suggested_name, false);
                    }
                    // never in silence: the user sees what was refused
                    Start::Refuse => {
                        let site = wubble_core::url::address_parts(&page).0.to_owned();
                        self.toast(window, &format!("Blocked a download from {site}"));
                        self.download_path_chosen(tab, id, download, None);
                    }
                    Start::Ask => {
                        let request = PermissionRequest {
                            origin: page,
                            ask: Ask::Download {
                                id,
                                download,
                                url,
                                name: download::sanitize_file_name(&suggested_name),
                                size: total,
                            },
                        };
                        self.apply_permission(tab, PermissionEvent::Requested(request));
                    }
                }
            }
            PageEvent::DownloadStarted {
                download,
                url,
                path,
                total,
                ..
            } => {
                // the container at request time, else the tab's; with neither it is cancelled
                let container = self
                    .downloads
                    .inflight
                    .get(&download)
                    .map(|f| f.container)
                    .or_else(|| self.model.container_of(tab));
                let Some(container) = container else {
                    if let Some(view) = self.tab_view(tab) {
                        view.cancel_download(download);
                    }
                    return;
                };
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
                    speed: None,
                };
                d.next_id += 1;
                d.inflight
                    .entry(download)
                    .or_insert_with(|| Inflight {
                        tab,
                        container,
                        path: entry.path.clone(),
                        entry: None,
                    })
                    .entry = Some(entry.id);
                let text = format!("Downloading {}", entry.name);
                self.push_entry(entry, self.model.side_of_container(container));
                self.toast(window, &text);
                self.render_downloads(window);
                self.report_busy(tab);
            }
            PageEvent::DownloadProgress {
                download,
                received,
                total,
                speed,
            } => self.engine_download(
                download,
                DownloadEvent::Progress {
                    received,
                    total,
                    speed,
                },
            ),
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

    /// A download that may go: to the downloads folder, or asked where (the settings, Save As).
    fn download_go(
        &mut self,
        window: &AppWindow,
        tab: TabId,
        id: RequestId,
        download: DownloadId,
        suggested_name: &str,
        save_as: bool,
    ) {
        if download::asks_where(self.downloads.ask_where, save_as) {
            return self.ask_download_path(window, tab, id, download, suggested_name);
        }
        let path = self.download_path(suggested_name);
        if path.is_none() {
            self.toast(window, "Cannot download: no downloads folder");
        }
        self.download_path_chosen(tab, id, download, path);
    }

    /// The prompt's answer to "Allow <site> to download files?": the download goes ahead (and
    /// the site's next aimed one needs no question this run), or is declined.
    pub(super) fn download_answered(
        &mut self,
        tab: TabId,
        id: RequestId,
        download: DownloadId,
        name: &str,
        allow: bool,
    ) {
        let window = self.w.window.upgrade();
        match (allow, window) {
            (true, Some(window)) => {
                let origin = self
                    .model
                    .tabs
                    .get(&tab)
                    .and_then(|t| wubble_core::site::origin_of(&t.url));
                self.downloads.sites.extend(origin);
                self.download_go(&window, tab, id, download, name, false);
            }
            _ => self.download_path_chosen(tab, id, download, None),
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
        // an unknown tab's download is cancelled (no Default stand-in)
        let container = self.model.container_of(tab);
        let path = path.filter(|_| container.is_some());
        let Some(view) = self.tab_view(tab) else {
            return;
        };
        let ok = view.answer_download_path(id, path.as_deref());
        if ok && let (Some(path), Some(container)) = (path, container) {
            let reserved = Inflight {
                tab,
                container,
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

    /// "Clear finished": every entry that is done goes, through the one remove path (none while
    /// the page's search hides rows: `download::clearable_ids`).
    pub(crate) fn downloads_cleared(&mut self) {
        for id in download::clearable_ids(&self.listed_downloads(), &self.downloads.query) {
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

    /// The downloads still running that `closing` cancels: the engine's from the private tabs
    /// it ends, and when it ends every private tab, the media jobs of the private windows'
    /// container too (they outlive their tab, not the container: `forget_private_downloads`).
    fn private_downloads_ending(&self, closing: Closing) -> Vec<i64> {
        let ended = self.model.private_tabs_ended(closing);
        let every = self
            .model
            .tabs
            .keys()
            .all(|&t| self.model.side_of_tab(t) == Side::Normal || ended.contains(&t));
        let media = self.downloads.list.iter().filter(|e| {
            every
                && self.media.owns(e.id)
                && self.model.side_of_record(e.container) == Side::Private
        });
        ended
            .iter()
            .flat_map(|&tab| self.downloads.running_in(tab))
            .chain(media.map(|e| e.id))
            .filter(|&id| self.downloads.get(id).is_some_and(|e| e.state.is_active()))
            .collect()
    }

    /// Whether `closing` waits for an answer: while it would cancel private downloads, the
    /// question (`menu::close_entries`) opens over a page and nothing closes yet. It asks in
    /// the closing window when it shows a page, else in a private window showing a tab the
    /// close ends (raised), else in this window; with no page there it closes. A normal
    /// window's page never carries it unless the close is that window's or the app's.
    pub(super) fn close_asks(&mut self, closing: Closing) -> bool {
        let count = self.private_downloads_ending(closing).len();
        if count == 0 {
            return false;
        }
        let shows = |w: WindowId| !self.model.window_tabs(w).is_empty();
        let closing_window = match closing {
            Closing::Window(w) => Some(w).filter(|&w| shows(w)),
            Closing::App | Closing::Restart => None,
        };
        let holding = self
            .model
            .private_tabs_ended(closing)
            .iter()
            .filter_map(|&t| self.model.location(t))
            .find_map(|(space, _)| self.model.space_window(space))
            .filter(|&w| shows(w));
        let here = matches!(closing, Closing::App | Closing::Restart).then_some(self.w.id);
        let Some(window) = closing_window.or(holding).or(here) else {
            return false;
        };
        if window != self.w.id {
            self.raise_window(window);
            self.enter(window);
        }
        match self.shown() {
            Some(tab) => self.ask_in_menu(tab, crate::menu::close_entries(closing, count)),
            None => false,
        }
    }

    /// The question's "Cancel Downloads and Close": the private downloads `closing` ends are
    /// cancelled, then it closes (and asks nothing again: none is running).
    pub(super) fn close_cancelling_downloads(&mut self, closing: Closing) {
        for id in self.private_downloads_ending(closing) {
            self.apply_download(id, DownloadEvent::Cancel);
        }
        match closing {
            Closing::Window(w) => self.dispatch(Command::CloseWindow(w)),
            Closing::App => crate::quit(),
            Closing::Restart => self.restart_now(),
        }
    }

    /// The in-memory container `container` (the private windows') is gone with the last private
    /// window: its downloads leave the list, as Chrome's off-the-record download manager goes
    /// with its profile. A media job still running is cancelled first (the close asked:
    /// `close_asks`). Their files stay; none of them was stored.
    pub(super) fn forget_private_downloads(&mut self, container: ContainerId) {
        let gone: Vec<i64> = self
            .downloads
            .list
            .iter()
            .filter(|e| e.container == container)
            .map(|e| e.id)
            .collect();
        if gone.is_empty() {
            return;
        }
        for &id in &gone {
            if self.media.owns(id) && self.downloads.get(id).is_some_and(|e| e.state.is_active()) {
                self.apply_download(id, DownloadEvent::Cancel);
            }
        }
        self.downloads.list.retain(|e| e.container != container);
        for id in &gone {
            self.downloads.unsaved.remove(id);
        }
        if let Some(window) = self.w.window.upgrade() {
            self.render_downloads(&window);
        }
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
            speed: None,
        };
        d.next_id += 1;
        let (id, text) = (entry.id, format!("Downloading {}", entry.name));
        self.push_entry(entry, self.model.side_of_container(container));
        if let Some(window) = self.w.window.upgrade() {
            self.toast(&window, &text);
            self.render_downloads(&window);
        }
        id
    }

    /// A file wubble wrote itself (a print saved as PDF), listed as a finished download of `url`
    /// in `tab`'s container (stored only for a normal tab: `push_entry`).
    pub(super) fn add_finished_download(&mut self, url: &str, path: &Path, tab: Option<TabId>) {
        let size = std::fs::metadata(path).map_or(0, |m| m.len());
        // the printed tab closed meanwhile: its file stays, unlisted (no Default stand-in)
        let Some(container) = tab.and_then(|t| self.model.container_of(t)) else {
            return;
        };
        let side = self.model.side_of_container(container);
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
            container,
            speed: None,
        };
        d.next_id += 1;
        self.push_entry(entry, side);
        if let Some(window) = self.w.window.upgrade() {
            self.render_downloads(&window);
        }
    }

    /// `entry` goes to the top of the list, and to the store only from the normal `side`: a
    /// private window's downloads are listed for this run only (their files stay). An unknown
    /// tab's side is private, so its entry is never stored (fails closed).
    fn push_entry(&mut self, entry: DownloadEntry, side: Side) {
        if side == Side::Normal {
            self.store.add_download(&entry);
        }
        self.downloads.list.insert(0, entry);
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

    pub(super) fn render_downloads(&mut self, window: &AppWindow) {
        let listed = self.listed_downloads();
        let items: Vec<DownloadItem> = listed.iter().map(item).collect();
        crate::bind::sync(window.get_downloads(), items, |m| window.set_downloads(m));
        // nothing to list: the popover closes
        if listed.is_empty() {
            self.overlay_closed(wubble_core::overlay::Kind::Downloads);
        }
        let filtered: Vec<DownloadItem> = download::filter_entries(&listed, &self.downloads.query)
            .iter()
            .map(item)
            .collect();
        crate::bind::sync(window.get_downloads_page_items(), filtered, |m| {
            window.set_downloads_page_items(m)
        });
        window.set_downloads_searching(!self.downloads.query.trim().is_empty());
        window.set_downloads_has_finished(
            !download::clearable_ids(&listed, &self.downloads.query).is_empty(),
        );
    }

    /// The entries this window lists: a private window's downloads in private windows only,
    /// every other one in normal windows only (Chrome's off-the-record profile has a download
    /// manager of its own).
    fn listed_downloads(&self) -> Vec<DownloadEntry> {
        let side = self.model.side_of_window(self.w.id);
        self.downloads
            .list
            .iter()
            .filter(|e| self.model.side_of_record(e.container) == side)
            .cloned()
            .collect()
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
            speed: None,
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
