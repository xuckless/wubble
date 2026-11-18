//! The host's half of the native pages beside settings: history, bookmarks, downloads and the
//! bookmark sheet. One `page` property tells the window which internal page the visible tab
//! shows ([`Host::render_pages`]); each page is drawn from its view-model
//! (`wubble_core::pages`) and every change is one store request (`persist`), whose reply
//! redraws it. The state here is the one owner of what each page shows; the store is the truth.

use std::collections::{HashMap, HashSet};

use wubble_core::model::Command;
use wubble_core::pages::{self, BookmarkEdit, BookmarkRow, ClearRange, HistoryRow, InternalPage};
use wubble_store::Favicon;
use wubble_ui::{AppWindow, BookmarkSheetView, InternalPageKind};

use super::{Host, now_ms};
use crate::bind::favicon_image;
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

/// What the native pages hold of their own.
#[derive(Debug, Default)]
pub(super) struct PagesState {
    /// The page the window showed last: a page is fetched when it is entered.
    shown: Option<InternalPage>,
    history: History,
    bookmarks: Bookmarks,
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
            self.pages.shown = page;
            match page {
                Some(InternalPage::History) => self.history_fetch(false),
                Some(InternalPage::Bookmarks) => self.bookmarks_request(None),
                _ => {}
            }
        }
        self.render_settings();
    }

    // ----- history

    fn history_fetch(&mut self, more: bool) {
        let h = &mut self.pages.history;
        h.generation += 1;
        let generation = h.generation;
        let before = h
            .rows
            .last()
            .filter(|_| more)
            .map(|r| (r.visit_ms, r.visit_id));
        self.store
            .visits(&h.query, before, HISTORY_PAGE, move |page| {
                let _ = slint::invoke_from_event_loop(move || {
                    with_host(|host| host.visits_arrived(generation, more, page));
                });
            });
    }

    fn visits_arrived(&mut self, generation: u64, more: bool, page: VisitsPage) {
        let h = &mut self.pages.history;
        if generation != h.generation {
            return;
        }
        h.exhausted = page.rows.len() < HISTORY_PAGE;
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
        let groups = pages::history_page(&h.rows, "", utc_offset_min(), now_ms());
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
        self.history_fetch(false);
    }

    pub(crate) fn history_more(&mut self) {
        self.history_fetch(true);
    }

    fn history_clear(&mut self, what: HistoryClear) {
        self.store.clear_history(what, |_| {
            let _ = slint::invoke_from_event_loop(|| {
                with_host(|host| host.history_fetch(false));
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
    pub(super) fn bookmark_page(&mut self) {
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
}
