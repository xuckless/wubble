//! The host's half of the native pages beside settings: history, bookmarks, downloads and the
//! bookmark sheet. One `page` property tells the window which internal page the visible tab
//! shows ([`Host::render_pages`]); each page is drawn from its view-model
//! (`wubble_core::pages`) and every change is one store request (`persist`), whose reply
//! redraws it. The state here is the one owner of what each page shows; the store is the truth.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::time::{Duration, Instant};

use slint::ComponentHandle;
use wubble_core::apply::{self, Applied};
use wubble_core::model::Command;
use wubble_core::pages::{
    self, BookmarkEdit, BookmarkRow, ClearRange, HistoryRow, InternalPage, LoginView, VaultStatus,
};
use wubble_core::settings as defs;
use wubble_store::Favicon;
use wubble_ui::{AppWindow, BookmarkSheetView, InternalPageKind};
use wubble_vault::{RecordId, VaultAnswer, VaultCommand};
use zeroize::Zeroizing;

use super::{Host, now_ms};
use crate::bind::favicon_image;
use crate::menu::MenuEvent;
use crate::persist::{BookmarksPage, HistoryClear, VisitsPage};
use crate::with_host;

/// Visits asked for at once; "Show more" asks for the next page.
pub(super) const HISTORY_PAGE: usize = 200;

/// The page's window kind.
pub(super) fn kind_of(page: Option<InternalPage>) -> InternalPageKind {
    match page {
        None => InternalPageKind::None,
        Some(InternalPage::Settings) => InternalPageKind::Settings,
        Some(InternalPage::History) => InternalPageKind::History,
        Some(InternalPage::Bookmarks) => InternalPageKind::Bookmarks,
        Some(InternalPage::Downloads) => InternalPageKind::Downloads,
        Some(InternalPage::Passwords) => InternalPageKind::Passwords,
    }
}

#[derive(Debug, Default)]
struct History {
    query: String,
    rows: Vec<HistoryRow>,
    /// The last page came back short: nothing older is left.
    exhausted: bool,
    /// Bumped per fetch; a reply for an older one is dropped.
    generation: u64,
    icons: HashMap<String, Favicon>,
}

/// What the bookmark sheet is for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum SheetKind {
    /// The visible page: `existing` is its bookmark, when it has one.
    Page {
        url: String,
        title: String,
        existing: Option<i64>,
    },
    Rename {
        id: i64,
    },
    Move {
        id: i64,
    },
    NewFolder,
}

#[derive(Debug, Default)]
struct Bookmarks {
    /// Every row as the store last said (also the address pill's star).
    rows: Vec<BookmarkRow>,
    icons: HashMap<String, Favicon>,
    expanded: HashSet<i64>,
    query: String,
    sheet: Option<SheetKind>,
    /// Bumped per opening, so the sheet restarts from its own fields.
    serial: i32,
    loaded: bool,
}

/// How long a revealed password stays on the page.
const REVEAL_FOR: Duration = Duration::from_secs(30);

/// A password the user asked to see: shown until `until`, or until the page is left.
struct Revealed {
    id: String,
    password: Zeroizing<String>,
    until: Instant,
}

impl fmt::Debug for Revealed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Revealed")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

#[derive(Debug)]
struct Passwords {
    status: VaultStatus,
    /// Empty while locked: nothing is shown without the key.
    rows: Vec<LoginView>,
    revealed: Option<Revealed>,
    query: String,
    /// The last import's report or the last refusal, under the status card.
    notice: String,
}

impl Default for Passwords {
    fn default() -> Self {
        Self {
            status: VaultStatus::Locked,
            rows: vec![],
            revealed: None,
            query: String::new(),
            notice: String::new(),
        }
    }
}

/// What a vault answer is for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Then {
    /// Take it into the page's state.
    Apply,
    /// Show the revealed password of this login.
    Show(String),
    /// Put the revealed password on the clipboard and forget it.
    Copy,
}

/// What the native pages hold of their own.
#[derive(Debug, Default)]
pub(super) struct PagesState {
    /// The page the window showed last: a page is fetched when it is entered.
    shown: Option<InternalPage>,
    history: History,
    bookmarks: Bookmarks,
    passwords: Passwords,
}

impl Passwords {
    /// Takes `answer` in; the command to send next, if any (a list after an unlock, say).
    /// A password answer is returned for the host to copy, never stored on a `Copy`.
    fn apply(
        &mut self,
        then: Then,
        answer: VaultAnswer,
        now: Instant,
    ) -> (Option<VaultCommand>, Option<Zeroizing<String>>) {
        match answer {
            VaultAnswer::Status(status) => {
                let open = matches!(
                    status,
                    VaultStatus::Unlocked | VaultStatus::Unreadable { .. }
                );
                if !open {
                    self.rows.clear();
                    self.revealed = None;
                }
                self.status = status;
                self.notice.clear();
                (open.then_some(VaultCommand::List), None)
            }
            VaultAnswer::Entries(rows) => {
                self.rows = rows;
                (None, None)
            }
            VaultAnswer::Revealed(password) => match then {
                Then::Show(id) => {
                    self.revealed = Some(Revealed {
                        id,
                        password,
                        until: now + REVEAL_FOR,
                    });
                    (None, None)
                }
                Then::Copy => (None, Some(password)),
                Then::Apply => (None, None),
            },
            VaultAnswer::Done => (Some(VaultCommand::List), None),
            VaultAnswer::Imported(line) => {
                self.notice = line;
                (Some(VaultCommand::List), None)
            }
            VaultAnswer::Failed(why) => {
                self.notice = why;
                (None, None)
            }
        }
    }
}

/// The sheet's fields for `kind` (pure).
fn sheet_view(kind: &SheetKind, rows: &[BookmarkRow], serial: i32) -> BookmarkSheetView {
    let (title, accept, show_name, name, show_folders, folder) = match kind {
        SheetKind::Page {
            title, existing, ..
        } => (
            if existing.is_some() {
                "Edit bookmark"
            } else {
                "Add bookmark"
            },
            "Save",
            true,
            title.clone(),
            true,
            existing
                .and_then(|id| rows.iter().find(|r| r.id == id))
                .and_then(|r| r.parent),
        ),
        SheetKind::Rename { id } => (
            "Rename",
            "Save",
            true,
            rows.iter()
                .find(|r| r.id == *id)
                .map(|r| r.title.clone())
                .unwrap_or_default(),
            false,
            None,
        ),
        SheetKind::Move { id } => (
            "Move to folder",
            "Move",
            false,
            String::new(),
            true,
            rows.iter().find(|r| r.id == *id).and_then(|r| r.parent),
        ),
        SheetKind::NewFolder => ("New folder", "Create", true, String::new(), false, None),
    };
    BookmarkSheetView {
        open: true,
        serial,
        title: title.into(),
        accept: accept.into(),
        show_name,
        name: name.as_str().into(),
        show_folders,
        folder: folder.map_or(-1, |f| i32::try_from(f).unwrap_or(-1)),
        can_remove: matches!(
            kind,
            SheetKind::Page {
                existing: Some(_),
                ..
            }
        ),
    }
}

/// The store commands an answered sheet means; none when it was cancelled or refused (pure).
pub(super) fn sheet_edits(
    kind: &SheetKind,
    rows: &[BookmarkRow],
    accept: bool,
    remove: bool,
    name: &str,
    folder: Option<i64>,
) -> Vec<BookmarkEdit> {
    let name = name.trim();
    match kind {
        SheetKind::Page {
            existing: Some(id), ..
        } if remove => vec![BookmarkEdit::Delete { id: *id }],
        _ if !accept => vec![],
        SheetKind::Page {
            url,
            title,
            existing: None,
        } => pages::add_current(
            rows,
            if name.is_empty() { title } else { name },
            url,
            folder,
        )
        .into_iter()
        .collect(),
        SheetKind::Page {
            existing: Some(id), ..
        } => {
            let current = rows.iter().find(|r| r.id == *id);
            let mut edits = vec![];
            if !name.is_empty() && current.is_some_and(|r| r.title != name) {
                edits.push(BookmarkEdit::Rename {
                    id: *id,
                    title: name.to_owned(),
                });
            }
            if current.is_some_and(|r| r.parent != folder) {
                edits.push(pages::move_to_end(rows, *id, folder));
            }
            edits
        }
        SheetKind::Rename { id } if !name.is_empty() => vec![BookmarkEdit::Rename {
            id: *id,
            title: name.to_owned(),
        }],
        SheetKind::Move { id } => vec![pages::move_to_end(rows, *id, folder)],
        SheetKind::NewFolder if !name.is_empty() => vec![BookmarkEdit::AddFolder {
            parent: None,
            title: name.to_owned(),
        }],
        _ => vec![],
    }
}

/// The local UTC offset in minutes (east positive): one for every row of the history page.
fn utc_offset_min() -> i32 {
    use chrono::Offset;
    chrono::Local::now().offset().fix().local_minus_utc() / 60
}

impl Host {
    /// The internal page the visible tab shows.
    pub(super) fn shown_page(&self) -> Option<InternalPage> {
        self.model
            .visible_tab()
            .and_then(|t| self.model.tabs.get(&t))
            .and_then(|t| InternalPage::from_url(&t.url))
    }

    /// Tells the window which page shows, fetching a page's data when it is entered, and draws
    /// the page. The one place the page layer changes.
    pub(super) fn render_pages(&mut self) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let page = self.shown_page();
        window.set_page(kind_of(page));
        if !self.pages.bookmarks.loaded {
            self.pages.bookmarks.loaded = true;
            self.bookmarks_request(None);
        }
        if self.pages.shown != page {
            if self.pages.shown == Some(InternalPage::Passwords) {
                self.passwords_left();
            }
            self.pages.shown = page;
            match page {
                Some(InternalPage::History) => self.history_fetch(false, HISTORY_PAGE),
                Some(InternalPage::Bookmarks) => self.bookmarks_request(None),
                _ => {}
            }
        }
        self.render_settings();
        self.render_passwords(&window);
        self.render_pill(&window);
    }

    // ----- the address pill: the bookmark star and the blocker badge

    /// The hosts the blocker skips, from the stored setting.
    fn blocker_exceptions(&self) -> Vec<String> {
        match apply::effect_of(defs::BLOCKER_EXCEPTIONS.key, &self.settings.stored) {
            Some(Applied::BlockerExceptions(hosts)) => hosts,
            _ => vec![],
        }
    }

    fn render_pill(&mut self, window: &AppWindow) {
        let tab = self.model.visible_tab();
        let url = tab
            .and_then(|t| self.model.tabs.get(&t))
            .map(|t| t.url.clone())
            .unwrap_or_default();
        let web = wubble_core::url::is_recordable(&url);
        let star = web && pages::bookmark_for_url(&self.pages.bookmarks.rows, &url).is_some();
        let count = tab.and_then(|t| self.blocked_of(t)).unwrap_or(0);
        let badge = pages::blocker_state(count, &url, &self.blocker_exceptions());
        window.set_address_bookmarkable(web);
        window.set_address_bookmarked(star);
        window.set_address_blocker_visible(web && badge.visible);
        window.set_address_blocker_allowed(badge.site_allowed);
        window.set_address_blocked(i32::try_from(badge.count).unwrap_or(i32::MAX));
    }

    /// The badge was clicked: the blocker menu opens under it (`x`, `y` in window pixels).
    pub(crate) fn blocker_menu_requested(&mut self, x: f32, y: f32) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let Some(tab) = self.model.visible_tab() else {
            return;
        };
        let url = self
            .model
            .tabs
            .get(&tab)
            .map(|t| t.url.clone())
            .unwrap_or_default();
        let Some(host) = wubble_core::zoom::host_of(&url) else {
            return;
        };
        let allowed = self.blocker_exceptions().contains(&host);
        let (count, hosts) = self.blocked_breakdown(tab).unwrap_or_default();
        let scale = window.window().scale_factor();
        let at = ((x * scale) as i32, (y * scale) as i32);
        let entries = crate::menu::blocker_entries(count, &hosts, &host, allowed);
        self.apply_menu(tab, MenuEvent::ShellRequested { at, entries });
        self.render_surfaces(&window);
    }

    /// The menu's switch: the site joins or leaves the blocker's exceptions (the stored setting
    /// the settings page shows).
    pub(super) fn toggle_blocker(&mut self, host: &str) {
        let key = defs::BLOCKER_EXCEPTIONS.key;
        if self.blocker_exceptions().iter().any(|h| h == host) {
            self.setting_entry_removed(key, host);
        } else {
            self.setting_entry_added(key, host);
        }
        self.render_pages();
    }

    // ----- history

    /// Fetch `limit` rows: the next page after the loaded ones (`more`) or a fresh list.
    fn history_fetch(&mut self, more: bool, limit: usize) {
        let h = &mut self.pages.history;
        h.generation += 1;
        let generation = h.generation;
        let before = h
            .rows
            .last()
            .filter(|_| more)
            .map(|r| (r.visit_ms, r.visit_id));
        self.store.visits(&h.query, before, limit, move |page| {
            let _ = slint::invoke_from_event_loop(move || {
                with_host(|host| host.visits_arrived(generation, more, limit, page));
            });
        });
    }

    fn visits_arrived(&mut self, generation: u64, more: bool, limit: usize, page: VisitsPage) {
        let h = &mut self.pages.history;
        if generation != h.generation {
            return;
        }
        h.exhausted = page.rows.len() < limit;
        if more {
            h.rows.extend(page.rows);
            h.icons.extend(page.icons);
        } else {
            h.rows = page.rows;
            h.icons = page.icons;
        }
        self.render_history();
    }

    pub(super) fn render_history(&self) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let h = &self.pages.history;
        let groups = pages::history_page(&h.rows, utc_offset_min(), now_ms());
        let lines = pages::history_lines(&groups);
        let items = wubble_ui::pages::history_items(&lines, |url| {
            h.icons
                .get(url)
                .map(|f| favicon_image(f.width, f.height, &f.rgba))
                .unwrap_or_default()
        });
        crate::bind::sync(window.get_history_lines(), items, |m| {
            window.set_history_lines(m)
        });
        window.set_history_searching(!h.query.trim().is_empty());
        window.set_history_has_more(!h.exhausted);
    }

    pub(crate) fn history_query(&mut self, query: &str) {
        query.clone_into(&mut self.pages.history.query);
        self.history_fetch(false, HISTORY_PAGE);
    }

    pub(crate) fn history_more(&mut self) {
        self.history_fetch(true, HISTORY_PAGE);
    }

    fn history_clear(&mut self, what: HistoryClear) {
        // Reload as many rows as are shown, so "Show more" pages survive a delete.
        let shown = self.pages.history.rows.len().max(HISTORY_PAGE);
        self.store.clear_history(what, move |_| {
            let _ = slint::invoke_from_event_loop(move || {
                with_host(|host| host.history_fetch(false, shown));
            });
        });
    }

    pub(crate) fn history_deleted(&mut self, url: &str) {
        self.history_clear(HistoryClear::Url(url.to_owned()));
    }

    pub(crate) fn history_cleared(&mut self, range: i32) {
        let range = usize::try_from(range)
            .ok()
            .and_then(|i| ClearRange::ALL.get(i).copied());
        if let Some(range) = range {
            let (from, to) = range.bounds(now_ms());
            self.history_clear(HistoryClear::Range(from, to));
        }
    }

    // ----- rows that open an address

    /// A history or bookmark row clicked: the page's own tab goes there.
    pub(crate) fn page_opened(&mut self, url: &str) {
        if let Some(tab) = self.model.visible_tab() {
            self.dispatch(Command::Navigate {
                tab,
                url: url.to_owned(),
            });
        }
    }

    /// Middle-click: a new tab beside it.
    pub(crate) fn page_opened_new_tab(&mut self, url: &str) {
        let opener = self.model.visible_tab();
        self.dispatch(Command::NewTab {
            url: url.to_owned(),
            container: None,
            opener,
        });
    }

    // ----- bookmarks

    /// Applies `edit` (or only reads), then draws from the store's rows.
    fn bookmarks_request(&mut self, edit: Option<BookmarkEdit>) {
        self.store.bookmarks(edit.map(|e| (e, now_ms())), |page| {
            let _ = slint::invoke_from_event_loop(move || {
                with_host(|host| host.bookmarks_arrived(page));
            });
        });
    }

    fn bookmarks_arrived(&mut self, page: Option<BookmarksPage>) {
        // A refused edit replies nothing: keep what is shown.
        if let Some(page) = page {
            let b = &mut self.pages.bookmarks;
            b.rows = page.rows;
            b.icons = page.icons;
        }
        self.render_bookmarks();
    }

    pub(super) fn render_bookmarks(&mut self) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let b = &self.pages.bookmarks;
        let icon = |url: &str| {
            b.icons
                .get(url)
                .map(|f| favicon_image(f.width, f.height, &f.rgba))
                .unwrap_or_default()
        };
        let items = if b.query.trim().is_empty() {
            let tree = pages::bookmark_tree(&b.rows);
            wubble_ui::pages::bookmark_items(&pages::flatten(&tree, &b.expanded), &b.expanded, icon)
        } else {
            wubble_ui::pages::bookmark_hits(&pages::filter_tree(&b.rows, &b.query), icon)
        };
        crate::bind::sync(window.get_bookmark_lines(), items, |m| {
            window.set_bookmark_lines(m)
        });
        window.set_bookmarks_searching(!b.query.trim().is_empty());
        self.render_bookmark_sheet(&window);
    }

    fn render_bookmark_sheet(&self, window: &AppWindow) {
        let b = &self.pages.bookmarks;
        match &b.sheet {
            Some(kind) => {
                window.set_bookmark_sheet_view(sheet_view(kind, &b.rows, b.serial));
                let moving = match kind {
                    SheetKind::Move { id } => Some(*id),
                    _ => None,
                };
                crate::bind::sync(
                    window.get_bookmark_folders(),
                    wubble_ui::pages::folder_items(&pages::folder_choices(&b.rows, moving)),
                    |m| window.set_bookmark_folders(m),
                );
            }
            None => {
                let mut closed = window.get_bookmark_sheet_view();
                closed.open = false;
                window.set_bookmark_sheet_view(closed);
            }
        }
    }

    pub(crate) fn bookmarks_query(&mut self, query: &str) {
        query.clone_into(&mut self.pages.bookmarks.query);
        self.render_bookmarks();
    }

    pub(crate) fn bookmark_toggled(&mut self, id: i32) {
        let expanded = &mut self.pages.bookmarks.expanded;
        let id = i64::from(id);
        if !expanded.remove(&id) {
            expanded.insert(id);
        }
        self.render_bookmarks();
    }

    pub(crate) fn bookmark_deleted(&mut self, id: i32) {
        self.bookmarks_request(Some(BookmarkEdit::Delete { id: i64::from(id) }));
    }

    fn open_sheet(&mut self, kind: SheetKind) {
        let b = &mut self.pages.bookmarks;
        b.sheet = Some(kind);
        b.serial = b.serial.wrapping_add(1);
        self.render_bookmarks();
    }

    pub(crate) fn bookmark_rename_requested(&mut self, id: i32) {
        self.open_sheet(SheetKind::Rename { id: i64::from(id) });
    }

    pub(crate) fn bookmark_move_requested(&mut self, id: i32) {
        self.open_sheet(SheetKind::Move { id: i64::from(id) });
    }

    pub(crate) fn bookmark_folder_requested(&mut self) {
        self.open_sheet(SheetKind::NewFolder);
    }

    /// Mod+D: the sheet for the visible web page. Not while an engine dialog waits for its
    /// answer (one sheet at a time).
    pub(crate) fn bookmark_page(&mut self) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let Some(tab) = self
            .model
            .visible_tab()
            .and_then(|t| self.model.tabs.get(&t))
        else {
            return;
        };
        if window.get_dialog().open || !wubble_core::url::is_recordable(&tab.url) {
            return;
        }
        let existing = pages::bookmark_for_url(&self.pages.bookmarks.rows, &tab.url);
        let title = pages::default_title(&tab.title, &tab.url);
        let url = tab.url.clone();
        self.open_sheet(SheetKind::Page {
            url,
            title,
            existing,
        });
    }

    pub(crate) fn bookmark_sheet_answered(
        &mut self,
        accept: bool,
        remove: bool,
        name: &str,
        folder: i32,
    ) {
        let b = &mut self.pages.bookmarks;
        let Some(kind) = b.sheet.take() else {
            return;
        };
        let folder = (folder >= 0).then(|| i64::from(folder));
        for edit in sheet_edits(&kind, &b.rows, accept, remove, name, folder) {
            self.bookmarks_request(Some(edit));
        }
        self.render_bookmarks();
    }
}

impl Host {
    // ----- passwords

    fn vault_request(&mut self, command: VaultCommand, then: Then) {
        self.store.vault_command(command, move |answer| {
            let _ = slint::invoke_from_event_loop(move || {
                with_host(|host| host.vault_answered(then, answer));
            });
        });
    }

    fn vault_answered(&mut self, then: Then, answer: VaultAnswer) {
        let (next, copy) = self.pages.passwords.apply(then, answer, Instant::now());
        if let Some(password) = copy {
            self.copy(&password);
            if let Some(window) = self.window.upgrade() {
                self.toast(&window, "Password copied");
            }
        }
        if let Some(command) = next {
            self.vault_request(command, Then::Apply);
        }
        if let Some(window) = self.window.upgrade() {
            self.render_passwords(&window);
        }
    }

    /// The page was left: lock the vault and forget everything it showed.
    fn passwords_left(&mut self) {
        self.pages.passwords = Passwords::default();
        self.vault_request(VaultCommand::Lock, Then::Apply);
        if let Some(window) = self.window.upgrade() {
            self.render_passwords(&window);
        }
    }

    /// Hides a revealed password once its time is up (the lifecycle tick).
    pub(super) fn passwords_tick(&mut self, now: Instant) {
        let p = &mut self.pages.passwords;
        if p.revealed.as_ref().is_some_and(|r| r.until <= now) {
            p.revealed = None;
            if let Some(window) = self.window.upgrade() {
                self.render_passwords(&window);
            }
        }
    }

    pub(super) fn render_passwords(&self, window: &AppWindow) {
        let p = &self.pages.passwords;
        window.set_passwords_view(wubble_ui::pages::passwords_view(&p.status, &p.notice));
        let shown = pages::passwords_list(&p.rows, &p.query);
        let revealed = p
            .revealed
            .as_ref()
            .map(|r| (r.id.as_str(), r.password.as_str()));
        crate::bind::sync(
            window.get_logins(),
            wubble_ui::pages::login_items(&shown, revealed),
            |m| window.set_logins(m),
        );
    }

    pub(crate) fn passwords_query(&mut self, query: &str) {
        query.clone_into(&mut self.pages.passwords.query);
        if let Some(window) = self.window.upgrade() {
            self.render_passwords(&window);
        }
    }

    pub(crate) fn passwords_unlock(&mut self) {
        self.vault_request(VaultCommand::Unlock, Then::Apply);
    }

    pub(crate) fn login_reveal(&mut self, id: &str) {
        self.vault_request(
            VaultCommand::Reveal(RecordId(id.to_owned())),
            Then::Show(id.to_owned()),
        );
    }

    pub(crate) fn login_hide(&mut self) {
        self.pages.passwords.revealed = None;
        if let Some(window) = self.window.upgrade() {
            self.render_passwords(&window);
        }
    }

    pub(crate) fn login_copy(&mut self, id: &str) {
        self.vault_request(VaultCommand::Reveal(RecordId(id.to_owned())), Then::Copy);
    }

    pub(crate) fn login_deleted(&mut self, id: &str) {
        self.vault_request(VaultCommand::Delete(RecordId(id.to_owned())), Then::Apply);
    }

    pub(crate) fn login_added(&mut self, site: &str, username: &str, password: &str) {
        let command = VaultCommand::Add {
            site: site.to_owned(),
            username: username.to_owned(),
            password: Zeroizing::new(password.to_owned()),
        };
        self.vault_request(command, Then::Apply);
    }

    /// "Import CSV": the OS file chooser, then the vault reads the file on the store thread.
    pub(crate) fn passwords_import_requested(&mut self) {
        use crate::chooser::{self, DialogSpec, Pick};
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let spec = DialogSpec {
            pick: Pick::File,
            title: "Import passwords".into(),
            directory: None,
            file_name: None,
            filters: vec![("CSV".into(), vec!["csv".into()])],
        };
        let dialog = chooser::dialog(&spec, Some(window.window()));
        let task = slint::spawn_local(async move {
            if let Some(path) = chooser::pick(spec, dialog)
                .await
                .and_then(|p| p.into_iter().next())
            {
                with_host(|host| host.vault_request(VaultCommand::ImportCsv(path), Then::Apply));
            }
        });
        if let Err(e) = task {
            eprintln!("wubble: cannot show the file dialog: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: i64, parent: Option<i64>, title: &str, url: Option<&str>) -> BookmarkRow {
        BookmarkRow {
            id,
            parent,
            is_folder: url.is_none(),
            position: 0,
            title: title.into(),
            url: url.map(Into::into),
        }
    }

    fn page(existing: Option<i64>) -> SheetKind {
        SheetKind::Page {
            url: "https://a.test/".into(),
            title: "A".into(),
            existing,
        }
    }

    #[test]
    fn each_page_has_a_window_kind() {
        assert_eq!(kind_of(None), InternalPageKind::None);
        assert_eq!(
            kind_of(Some(InternalPage::History)),
            InternalPageKind::History
        );
        assert_eq!(
            kind_of(Some(InternalPage::Passwords)),
            InternalPageKind::Passwords
        );
    }

    #[test]
    fn the_page_sheet_adds_a_bookmark_in_the_chosen_folder() {
        let rows = [row(1, None, "F", None)];
        assert_eq!(
            sheet_edits(&page(None), &rows, true, false, " Mine ", Some(1)),
            [BookmarkEdit::Add {
                parent: Some(1),
                title: "Mine".into(),
                url: "https://a.test/".into()
            }]
        );
        // A blank name keeps the page's title; cancel does nothing.
        let edits = sheet_edits(&page(None), &rows, true, false, "", None);
        assert!(matches!(&edits[..], [BookmarkEdit::Add { title, .. }] if title == "A"));
        assert!(sheet_edits(&page(None), &rows, false, false, "x", None).is_empty());
    }

    #[test]
    fn an_existing_bookmark_is_renamed_moved_or_removed() {
        let rows = [
            row(1, None, "F", None),
            row(2, None, "Old", Some("https://a.test/")),
        ];
        let kind = page(Some(2));
        assert_eq!(
            sheet_edits(&kind, &rows, false, true, "", None),
            [BookmarkEdit::Delete { id: 2 }]
        );
        let edits = sheet_edits(&kind, &rows, true, false, "New", Some(1));
        assert_eq!(edits.len(), 2);
        assert!(matches!(&edits[0], BookmarkEdit::Rename { id: 2, title } if title == "New"));
        assert!(matches!(
            edits[1],
            BookmarkEdit::Move {
                id: 2,
                to: Some(1),
                ..
            }
        ));
        // Nothing changed: nothing to store.
        assert!(sheet_edits(&kind, &rows, true, false, "Old", None).is_empty());
    }

    #[test]
    fn rename_move_and_new_folder_each_need_their_field() {
        let rows = [
            row(1, None, "F", None),
            row(2, None, "L", Some("https://l.test")),
        ];
        assert!(
            sheet_edits(&SheetKind::Rename { id: 2 }, &rows, true, false, " ", None).is_empty()
        );
        assert_eq!(
            sheet_edits(&SheetKind::Rename { id: 2 }, &rows, true, false, "N", None),
            [BookmarkEdit::Rename {
                id: 2,
                title: "N".into()
            }]
        );
        assert!(matches!(
            sheet_edits(&SheetKind::Move { id: 2 }, &rows, true, false, "", Some(1))[..],
            [BookmarkEdit::Move {
                id: 2,
                to: Some(1),
                index: 0
            }]
        ));
        assert!(sheet_edits(&SheetKind::NewFolder, &rows, true, false, "", None).is_empty());
        assert!(matches!(
            &sheet_edits(&SheetKind::NewFolder, &rows, true, false, "G", None)[..],
            [BookmarkEdit::AddFolder { parent: None, title }] if title == "G"
        ));
    }

    #[test]
    fn the_sheet_view_shows_the_fields_of_its_kind() {
        let rows = [
            row(1, None, "F", None),
            row(2, Some(1), "L", Some("https://l.test")),
        ];
        let v = sheet_view(&page(Some(2)), &rows, 3);
        assert!(v.show_name && v.show_folders && v.can_remove);
        assert_eq!((v.folder, v.serial), (1, 3));
        let v = sheet_view(&SheetKind::Move { id: 2 }, &rows, 4);
        assert!(!v.show_name && v.show_folders && !v.can_remove);
        let v = sheet_view(&SheetKind::Rename { id: 2 }, &rows, 5);
        assert!(v.show_name && !v.show_folders && v.name == "L");
        assert!(!sheet_view(&page(None), &rows, 6).can_remove);
    }

    fn login(id: &str) -> LoginView {
        LoginView {
            id: id.into(),
            origin: "https://a.test".into(),
            username: Some("me".into()),
            created_ms: 0,
            last_used_ms: None,
            times_used: 0,
        }
    }

    #[test]
    fn an_unlock_asks_for_the_list_and_a_lock_forgets_it() {
        let now = Instant::now();
        let mut p = Passwords::default();
        let (next, _) = p.apply(Then::Apply, VaultAnswer::Status(VaultStatus::Unlocked), now);
        assert!(matches!(next, Some(VaultCommand::List)));
        p.apply(Then::Apply, VaultAnswer::Entries(vec![login("1")]), now);
        p.apply(
            Then::Show("1".into()),
            VaultAnswer::Revealed(Zeroizing::new("pw".into())),
            now,
        );
        assert_eq!(p.rows.len(), 1);
        assert!(p.revealed.is_some());
        let (next, _) = p.apply(Then::Apply, VaultAnswer::Status(VaultStatus::Locked), now);
        assert!(next.is_none() && p.rows.is_empty() && p.revealed.is_none());
        // The unreadable and unavailable cases: unreadable still lists (new logins save).
        let (next, _) = p.apply(
            Then::Apply,
            VaultAnswer::Status(VaultStatus::Unreadable { rows: 2 }),
            now,
        );
        assert!(matches!(next, Some(VaultCommand::List)));
        let (next, _) = p.apply(
            Then::Apply,
            VaultAnswer::Status(VaultStatus::Unavailable("x".into())),
            now,
        );
        assert!(next.is_none() && p.rows.is_empty());
    }

    #[test]
    fn a_password_to_copy_is_handed_out_and_not_kept() {
        let now = Instant::now();
        let mut p = Passwords::default();
        let (_, copy) = p.apply(
            Then::Copy,
            VaultAnswer::Revealed(Zeroizing::new("pw".into())),
            now,
        );
        assert_eq!(copy.as_deref().map(String::as_str), Some("pw"));
        assert!(p.revealed.is_none());
    }

    #[test]
    fn changes_and_refusals_are_followed_by_a_list_or_a_notice() {
        let now = Instant::now();
        let mut p = Passwords::default();
        for answer in [
            VaultAnswer::Done,
            VaultAnswer::Imported("Imported 2".into()),
        ] {
            let (next, _) = p.apply(Then::Apply, answer, now);
            assert!(matches!(next, Some(VaultCommand::List)));
        }
        assert_eq!(p.notice, "Imported 2");
        let (next, _) = p.apply(Then::Apply, VaultAnswer::Failed("not a site".into()), now);
        assert!(next.is_none());
        assert_eq!(p.notice, "not a site");
    }

    #[test]
    fn the_page_state_never_prints_a_revealed_password() {
        let mut state = PagesState::default();
        state.passwords.apply(
            Then::Show("1".into()),
            VaultAnswer::Revealed(Zeroizing::new("hunter2-secret".into())),
            Instant::now(),
        );
        assert!(!format!("{state:?}").contains("hunter2-secret"));
    }
}
