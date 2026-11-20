//! Pure view-models for the native pages (history, bookmarks, downloads, passwords, settings).
//!
//! Each takes plain input rows (the same pattern as `omnibox::HistoryEntry`: core defines the
//! row, the store or host maps into it) and returns plain structs a Slint model can show.
//! No clocks, no time zone lookups, no I/O: `now_ms` and the UTC offset are parameters.

use std::cmp::Reverse;
use std::collections::{HashMap, HashSet};

use chrono::NaiveDate;

use crate::settings::{self, Section, SettingKind};

const MIN_MS: i64 = 60_000;
const DAY_MS: i64 = 86_400_000;

// ---------------------------------------------------------------- history

/// One visit of a page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryRow {
    pub url: String,
    pub title: String,
    pub visit_ms: i64,
    /// Store row id of the visit: with `visit_ms` it is the paging cursor.
    pub visit_id: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DayLabel {
    Today,
    Yesterday,
    Date(NaiveDate),
}

/// The visits of one local day, newest first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DayGroup {
    /// Local days since 1970-01-01.
    pub day: i64,
    pub label: DayLabel,
    pub rows: Vec<HistoryRow>,
}

/// Local day number of `ms` at a UTC offset in minutes (east positive). One offset for all
/// rows: around a DST change an older visit can land on the neighbouring day.
fn local_day(ms: i64, tz_offset_min: i32) -> i64 {
    (ms + i64::from(tz_offset_min) * MIN_MS).div_euclid(DAY_MS)
}

fn day_label(day: i64, today: i64) -> DayLabel {
    if day == today {
        DayLabel::Today
    } else if day == today - 1 {
        DayLabel::Yesterday
    } else {
        // Days from 0001-01-01 (CE day 1) are the epoch day count plus 719_163.
        i32::try_from(day + 719_163)
            .ok()
            .and_then(NaiveDate::from_num_days_from_ce_opt)
            .map_or(DayLabel::Today, DayLabel::Date)
    }
}

fn contains_ci(haystack: &str, needle_lower: &str) -> bool {
    haystack.to_lowercase().contains(needle_lower)
}

/// The history page: visits newest first, grouped by local day. Searching is the store's job
/// (`list_visits`), so a search reaches rows older than any loaded page.
pub fn history_page(rows: &[HistoryRow], tz_offset_min: i32, now_ms: i64) -> Vec<DayGroup> {
    let today = local_day(now_ms, tz_offset_min);
    let mut hits: Vec<&HistoryRow> = rows.iter().collect();
    hits.sort_by_key(|r| Reverse(r.visit_ms));
    let mut groups: Vec<DayGroup> = Vec::new();
    for r in hits {
        let day = local_day(r.visit_ms, tz_offset_min);
        match groups.last_mut() {
            Some(g) if g.day == day => g.rows.push(r.clone()),
            _ => groups.push(DayGroup {
                day,
                label: day_label(day, today),
                rows: vec![r.clone()],
            }),
        }
    }
    groups
}

impl DayLabel {
    /// A group header: "Today", "Yesterday", "Mon, 3 Nov 2026".
    pub fn text(self) -> String {
        match self {
            DayLabel::Today => "Today".to_owned(),
            DayLabel::Yesterday => "Yesterday".to_owned(),
            DayLabel::Date(date) => date.format("%a, %-d %b %Y").to_string(),
        }
    }
}

/// One line of the history page's flat list: a day header or a visit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HistoryLine {
    Day { label: String },
    Visit(HistoryRow),
}

/// The day groups as the flat list a Slint model shows (headers are rows, as in [`flatten`]).
pub fn history_lines(groups: &[DayGroup]) -> Vec<HistoryLine> {
    let mut out = Vec::new();
    for g in groups {
        out.push(HistoryLine::Day {
            label: g.label.text(),
        });
        out.extend(g.rows.iter().cloned().map(HistoryLine::Visit));
    }
    out
}

/// What "Clear browsing data" removes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClearRange {
    LastHour,
    LastDay,
    All,
}

impl ClearRange {
    pub const ALL: [ClearRange; 3] = [ClearRange::LastHour, ClearRange::LastDay, ClearRange::All];

    pub fn label(self) -> &'static str {
        match self {
            ClearRange::LastHour => "Last hour",
            ClearRange::LastDay => "Last 24 hours",
            ClearRange::All => "All time",
        }
    }

    /// The `[from, to)` span for the store's range delete.
    pub fn bounds(self, now_ms: i64) -> (i64, i64) {
        match self {
            ClearRange::LastHour => (now_ms - 60 * MIN_MS, i64::MAX),
            ClearRange::LastDay => (now_ms - DAY_MS, i64::MAX),
            ClearRange::All => (i64::MIN, i64::MAX),
        }
    }
}

// -------------------------------------------------------------- bookmarks

/// A bookmark or folder, flat as the store keeps it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BookmarkRow {
    pub id: i64,
    pub parent: Option<i64>,
    pub is_folder: bool,
    pub position: u32,
    pub title: String,
    /// `Some` for links.
    pub url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BookmarkItem {
    pub id: i64,
    pub title: String,
    pub url: Option<String>,
    pub is_folder: bool,
    pub children: Vec<BookmarkItem>,
}

/// The bookmark tree, siblings by position. A row whose parent is missing is shown at the top
/// level rather than dropped. Rows in a parent cycle are unreachable from any root and left out.
pub fn bookmark_tree(rows: &[BookmarkRow]) -> Vec<BookmarkItem> {
    let ids: HashSet<i64> = rows.iter().map(|r| r.id).collect();
    let mut by_parent: HashMap<Option<i64>, Vec<&BookmarkRow>> = HashMap::new();
    for r in rows {
        let parent = r.parent.filter(|p| ids.contains(p) && *p != r.id);
        by_parent.entry(parent).or_default().push(r);
    }
    for siblings in by_parent.values_mut() {
        siblings.sort_by_key(|r| (r.position, r.id));
    }
    fn build(
        by_parent: &HashMap<Option<i64>, Vec<&BookmarkRow>>,
        parent: Option<i64>,
    ) -> Vec<BookmarkItem> {
        by_parent.get(&parent).map_or_else(Vec::new, |siblings| {
            siblings
                .iter()
                .map(|r| BookmarkItem {
                    id: r.id,
                    title: r.title.clone(),
                    url: r.url.clone(),
                    is_folder: r.is_folder,
                    children: build(by_parent, Some(r.id)),
                })
                .collect()
        })
    }
    build(&by_parent, None)
}

/// One visible line of the tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FlatBookmark<'a> {
    pub depth: usize,
    pub item: &'a BookmarkItem,
}

/// The tree as the list a Slint model shows: a folder's children follow it only when its id is
/// in `expanded`.
pub fn flatten<'a>(tree: &'a [BookmarkItem], expanded: &HashSet<i64>) -> Vec<FlatBookmark<'a>> {
    fn walk<'a>(
        items: &'a [BookmarkItem],
        depth: usize,
        expanded: &HashSet<i64>,
        out: &mut Vec<FlatBookmark<'a>>,
    ) {
        for item in items {
            out.push(FlatBookmark { depth, item });
            if item.is_folder && expanded.contains(&item.id) {
                walk(&item.children, depth + 1, expanded, out);
            }
        }
    }
    let mut out = Vec::new();
    walk(tree, 0, expanded, &mut out);
    out
}

/// A bookmark change as a value: the page and the sheet build it, the store thread applies it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BookmarkEdit {
    Add {
        parent: Option<i64>,
        title: String,
        url: String,
    },
    AddFolder {
        parent: Option<i64>,
        title: String,
    },
    Rename {
        id: i64,
        title: String,
    },
    Move {
        id: i64,
        to: Option<i64>,
        index: u32,
    },
    Delete {
        id: i64,
    },
}

/// The bookmark of `url`, if any (compared by [`crate::url::url_key`], so `www.` and the
/// scheme do not matter): the address pill's star.
pub fn bookmark_for_url(rows: &[BookmarkRow], url: &str) -> Option<i64> {
    let key = |u: &str| crate::url::url_key(u).trim_end_matches('/').to_owned();
    let want = key(url);
    rows.iter()
        .find(|r| r.url.as_deref().is_some_and(|u| key(u) == want))
        .map(|r| r.id)
}

/// The title a new bookmark starts with: the page's, else its address.
pub fn default_title(tab_title: &str, url: &str) -> String {
    let t = tab_title.trim();
    if t.is_empty() {
        url.to_owned()
    } else {
        t.to_owned()
    }
}

/// Adding the visible page under `folder`; `None` for an address that is not a web page or that
/// is already bookmarked (the sheet then offers removing it).
pub fn add_current(
    rows: &[BookmarkRow],
    tab_title: &str,
    url: &str,
    folder: Option<i64>,
) -> Option<BookmarkEdit> {
    (crate::url::is_recordable(url) && bookmark_for_url(rows, url).is_none()).then(|| {
        BookmarkEdit::Add {
            parent: folder,
            title: default_title(tab_title, url),
            url: url.to_owned(),
        }
    })
}

/// Moving `id` to the end of `folder`.
pub fn move_to_end(rows: &[BookmarkRow], id: i64, folder: Option<i64>) -> BookmarkEdit {
    let siblings = rows
        .iter()
        .filter(|r| r.parent == folder && r.id != id)
        .count();
    BookmarkEdit::Move {
        id,
        to: folder,
        index: u32::try_from(siblings).unwrap_or(u32::MAX),
    }
}

/// One entry of the folder picker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderChoice {
    /// `None` is the top level.
    pub id: Option<i64>,
    pub label: String,
    pub depth: usize,
}

/// The folders a bookmark can go in, depth first, the top level first. With `moving` set, that
/// folder and everything below it is left out (a folder cannot move into itself).
pub fn folder_choices(rows: &[BookmarkRow], moving: Option<i64>) -> Vec<FolderChoice> {
    fn walk(
        items: &[BookmarkItem],
        depth: usize,
        moving: Option<i64>,
        out: &mut Vec<FolderChoice>,
    ) {
        for item in items.iter().filter(|i| i.is_folder && Some(i.id) != moving) {
            out.push(FolderChoice {
                id: Some(item.id),
                label: item.title.clone(),
                depth,
            });
            walk(&item.children, depth + 1, moving, out);
        }
    }
    let mut out = vec![FolderChoice {
        id: None,
        label: "Bookmarks".to_owned(),
        depth: 0,
    }];
    walk(&bookmark_tree(rows), 1, moving, &mut out);
    out
}

/// The links matching `query` (case-insensitive title or url), each with its folder path as the
/// subtitle; the tree is not shown while searching. An empty query gives nothing.
pub fn filter_tree(rows: &[BookmarkRow], query: &str) -> Vec<(BookmarkRow, String)> {
    let needle = query.trim().to_lowercase();
    if needle.is_empty() {
        return Vec::new();
    }
    let by_id: HashMap<i64, &BookmarkRow> = rows.iter().map(|r| (r.id, r)).collect();
    let path = |r: &BookmarkRow| {
        let mut parts = Vec::new();
        let mut at = r.parent;
        while let Some(p) = at.and_then(|id| by_id.get(&id)) {
            parts.push(p.title.as_str());
            at = p.parent;
            if parts.len() > rows.len() {
                break;
            }
        }
        parts.reverse();
        parts.join(" / ")
    };
    rows.iter()
        .filter(|r| {
            r.url
                .as_deref()
                .is_some_and(|u| contains_ci(&r.title, &needle) || contains_ci(u, &needle))
        })
        .map(|r| (r.clone(), path(r)))
        .collect()
}

// -------------------------------------------------------------- passwords

/// A saved login as the page lists it: where, and as whom. The password is never here; it is
/// asked for on an explicit click.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginView {
    pub id: String,
    pub origin: String,
    /// `None` where the row does not open with the current key.
    pub username: Option<String>,
    pub created_ms: i64,
    pub last_used_ms: Option<i64>,
    pub times_used: u32,
}

impl LoginView {
    /// The origin without its scheme: what the row shows as the site.
    pub fn site(&self) -> &str {
        self.origin
            .split_once("://")
            .map_or(self.origin.as_str(), |(_, rest)| rest)
    }
}

/// Logins whose site or username contains `query` (case-insensitive; empty keeps all), by site
/// then age.
pub fn passwords_list(rows: &[LoginView], query: &str) -> Vec<LoginView> {
    let needle = query.trim().to_lowercase();
    let mut out: Vec<LoginView> = rows
        .iter()
        .filter(|e| {
            needle.is_empty()
                || contains_ci(&e.origin, &needle)
                || e.username
                    .as_deref()
                    .is_some_and(|u| contains_ci(u, &needle))
        })
        .cloned()
        .collect();
    out.sort_by_cached_key(|e| (e.site().to_lowercase(), e.created_ms, e.id.clone()));
    out
}

/// Whether the saved passwords can be shown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VaultStatus {
    /// Not unlocked: nothing is listed.
    Locked,
    Unlocked,
    /// The OS keyring cannot be used; the reason is its own text.
    Unavailable(String),
    /// The key is new or lost and `rows` saved logins no longer open with it.
    Unreadable {
        rows: usize,
    },
}

/// The status card's title and body.
pub fn status_text(status: &VaultStatus) -> (&'static str, String) {
    match status {
        VaultStatus::Locked => (
            "Passwords are locked",
            "Unlock them with your system keyring to see saved logins.".to_owned(),
        ),
        VaultStatus::Unlocked => ("Saved passwords", String::new()),
        VaultStatus::Unavailable(why) => (
            "Passwords are unavailable",
            format!("The system keyring cannot be used: {why}"),
        ),
        VaultStatus::Unreadable { rows } => (
            "Saved passwords cannot be read",
            format!(
                "{rows} saved login{} no longer open with the key in your system keyring. \
                 They stay on disk, and new logins can still be saved.",
                if *rows == 1 { "" } else { "s" }
            ),
        ),
    }
}

// ---------------------------------------------------------------- blocker

/// The address pill's blocker badge for the visible page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockerBadge {
    pub count: u32,
    /// The site is on the exceptions list: blocking is off there.
    pub site_allowed: bool,
    /// The badge shows: something was blocked, or blocking is off for the site.
    pub visible: bool,
}

/// How long ago `then` was, as a short phrase ("3 days ago"); times in the future read as "just
/// now". Unix seconds both.
pub fn relative_age(now: u64, then: u64) -> String {
    let secs = now.saturating_sub(then);
    let (n, unit) = match secs {
        0..60 => return "just now".into(),
        60..3_600 => (secs / 60, "minute"),
        3_600..86_400 => (secs / 3_600, "hour"),
        86_400..2_592_000 => (secs / 86_400, "day"),
        2_592_000..31_536_000 => (secs / 2_592_000, "month"),
        _ => (secs / 31_536_000, "year"),
    };
    format!("{n} {unit}{} ago", if n == 1 { "" } else { "s" })
}

pub fn blocker_state(count: u32, page_url: &str, exceptions: &[String]) -> BlockerBadge {
    let site_allowed = crate::zoom::host_of(page_url).is_some_and(|h| exceptions.contains(&h));
    BlockerBadge {
        count,
        site_allowed,
        visible: count > 0 || site_allowed,
    }
}

// ---------------------------------------------------------- internal pages

/// A page the shell draws itself (native Slint, never a web page), shown in a tab whose URL is
/// [`InternalPage::url`]. The tab's URL is the one owner: the session restores it as any tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InternalPage {
    Settings,
    History,
    Bookmarks,
    Downloads,
    Passwords,
    Archive,
    /// The Task manager: tabs and processes with memory, CPU and state.
    Tasks,
    /// The start page, the new-tab page: the design's living bubble (`start`), a search capsule
    /// and the shell's favourites (TODO.md Decided, "Start page").
    Start,
}

impl InternalPage {
    pub const ALL: [InternalPage; 8] = [
        InternalPage::Settings,
        InternalPage::History,
        InternalPage::Bookmarks,
        InternalPage::Downloads,
        InternalPage::Passwords,
        InternalPage::Archive,
        InternalPage::Tasks,
        InternalPage::Start,
    ];

    pub fn url(self) -> &'static str {
        match self {
            InternalPage::Settings => "wubble://settings",
            InternalPage::History => "wubble://history",
            InternalPage::Bookmarks => "wubble://bookmarks",
            InternalPage::Downloads => "wubble://downloads",
            InternalPage::Passwords => "wubble://passwords",
            InternalPage::Archive => "wubble://archive",
            InternalPage::Tasks => "wubble://tasks",
            InternalPage::Start => "wubble://start",
        }
    }

    /// The tab row's title.
    pub fn title(self) -> &'static str {
        match self {
            InternalPage::Settings => "Settings",
            InternalPage::History => "History",
            InternalPage::Bookmarks => "Bookmarks",
            InternalPage::Downloads => "Downloads",
            InternalPage::Passwords => "Passwords",
            InternalPage::Archive => "Archive",
            InternalPage::Tasks => "Task manager",
            InternalPage::Start => "New tab",
        }
    }

    /// The page a URL names; a trailing slash and the scheme's case do not matter.
    pub fn from_url(url: &str) -> Option<InternalPage> {
        let url = url.trim().trim_end_matches('/');
        Self::ALL
            .into_iter()
            .find(|p| p.url().eq_ignore_ascii_case(url))
    }
}

// --------------------------------------------------------------- settings

/// The control a settings row shows, picked by the setting's kind ([`control_for`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Control {
    /// On / off.
    Toggle,
    /// One of these stored words (a search engine's id, a choice's word).
    Segmented(&'static [&'static str]),
    /// A whole number, stepped within the range.
    Stepper { min: u32, max: u32 },
    /// A number dragged within the range.
    Slider { min: f32, max: f32 },
    /// A list of strings: rows with remove, and a field to add.
    ListEditor,
    /// A map of key to number: rows with remove.
    MapEditor,
    /// One line of text, applied on Enter.
    TextField,
}

/// The search engines' stored ids, the words of the engine's segmented control.
const ENGINE_IDS: [&str; crate::omnibox::ENGINES.len()] = {
    let mut ids = [""; crate::omnibox::ENGINES.len()];
    let mut i = 0;
    while i < ids.len() {
        ids[i] = crate::omnibox::ENGINES[i].id;
        i += 1;
    }
    ids
};

/// The control for a kind. Exhaustive, with no catch-all: a new kind does not compile until it
/// has a control, and a new row of an existing kind needs no UI code at all.
pub fn control_for(kind: SettingKind) -> Control {
    match kind {
        SettingKind::Bool => Control::Toggle,
        SettingKind::Count { min, max } => Control::Stepper { min, max },
        SettingKind::Float { min, max } => Control::Slider { min, max },
        SettingKind::SearchEngine => Control::Segmented(&ENGINE_IDS),
        SettingKind::Choice(words) => Control::Segmented(words),
        SettingKind::List => Control::ListEditor,
        SettingKind::Map => Control::MapEditor,
        SettingKind::Text => Control::TextField,
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SettingRow {
    pub key: &'static str,
    pub label: &'static str,
    pub kind: SettingKind,
    pub control: Control,
    /// Storage text: the stored value if valid, else the default.
    pub value: String,
    /// The value differs from the default.
    pub changed: bool,
    pub restart: bool,
    /// A list or map row's entries, decoded once here; empty for other kinds.
    pub entries: Vec<Entry>,
    /// A list or map row's column titles in its list editor ([`columns_of`]).
    pub columns: &'static [&'static str],
}

/// One removable entry of a list: what it shows (one cell per column of its list editor, and
/// the whole as one line for a screen reader), and what removing it names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub label: String,
    pub key: String,
    pub cells: Vec<String>,
}

/// The column titles of a list or map setting's list editor; the first names the entry.
/// Empty for the other kinds.
pub fn columns_of(def: &settings::SettingDef) -> &'static [&'static str] {
    let sites = [
        settings::BLOCKER_EXCEPTIONS.key,
        settings::SLEEP_NEVER_SITES.key,
        settings::VIDEO_HIDDEN_SITES.key,
    ];
    match def.kind {
        SettingKind::List if def.key == settings::ROUTING_RULES.key => {
            &["Site", "Space", "Container"]
        }
        SettingKind::List if def.key == settings::BLOCKER_CUSTOM_LISTS.key => &["Address"],
        SettingKind::List if sites.contains(&def.key) => &["Site"],
        SettingKind::List => &["Entry"],
        SettingKind::Map if def.key == settings::BLOCKER_LISTS.key => &["List", "Switched"],
        SettingKind::Map => &["Entry", "Value"],
        _ => &[],
    }
}

/// A list item's cells: a routing rule split into its site, Space and container; any other
/// item is one cell.
fn list_cells(def: &settings::SettingDef, item: &str) -> Vec<String> {
    if def.key != settings::ROUTING_RULES.key {
        return vec![item.to_owned()];
    }
    match crate::routing::Rule::parse(item) {
        Some(rule) => vec![
            match rule.pattern {
                crate::routing::Pattern::Host(h) => h,
                crate::routing::Pattern::Subdomains(base) => format!("*.{base}"),
            },
            rule.space.unwrap_or_default(),
            rule.container.unwrap_or_default(),
        ],
        None => vec![item.to_owned(), String::new(), String::new()],
    }
}

/// A list or map row's entries: a list item is its own key, a map entry is named by its key.
fn entries_of(def: &settings::SettingDef, stored: &HashMap<String, String>) -> Vec<Entry> {
    match def.kind {
        SettingKind::List => crate::apply::list(def, stored)
            .into_iter()
            .map(|item| Entry {
                label: item.clone(),
                cells: list_cells(def, &item),
                key: item,
            })
            .collect(),
        SettingKind::Map => crate::apply::map(def, stored)
            .into_iter()
            .map(|(key, v)| Entry {
                label: format!("{key}: {v}"),
                cells: vec![
                    key.clone(),
                    if def.key == settings::BLOCKER_LISTS.key {
                        if v == 0 { "Off" } else { "On" }.to_owned()
                    } else {
                        v.to_string()
                    },
                ],
                key,
            })
            .collect(),
        _ => vec![],
    }
}

/// The list editor's submitted draft (`kit/controls/list-editor.slint`): its cells joined by
/// [`CELL_SEP`], after the key of the row it replaces and [`REPLACE_SEP`] when it edits one.
pub const CELL_SEP: char = '\u{1f}';
pub const REPLACE_SEP: char = '\u{1e}';

/// A draft the list editor submitted, decoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Submission<'a> {
    /// The key of the row it replaces (an edit in place); `None` for a new row.
    pub replacing: Option<&'a str>,
    pub cells: Vec<&'a str>,
}

pub fn submission(text: &str) -> Submission<'_> {
    let (replacing, cells) = match text.split_once(REPLACE_SEP) {
        Some((key, cells)) => (Some(key), cells),
        None => (None, text),
    };
    Submission {
        replacing,
        cells: cells.split(CELL_SEP).collect(),
    }
}

/// The stored entry a list editor's cells make: a routing rule `site -> Space, Container`
/// (`routing::Rule`'s text), any other list's first cell.
pub fn entry_text(key: &str, cells: &[&str]) -> String {
    let cell = |i: usize| cells.get(i).map_or("", |c| c.trim());
    if key != settings::ROUTING_RULES.key {
        return cell(0).to_owned();
    }
    match (cell(1), cell(2)) {
        (space, "") => format!("{} -> {space}", cell(0)),
        (space, container) => format!("{} -> {space}, {container}", cell(0)),
    }
}

/// The Spaces section's list editor speaks through the settings entry callbacks under this
/// key, which no setting has: added, a new Space; removed, the Space by its id.
pub const SPACES_LIST: &str = "spaces";

#[derive(Debug, Clone, PartialEq)]
pub struct SectionView {
    pub section: Section,
    pub rows: Vec<SettingRow>,
}

/// The order the page lists its sections in; `Hidden` is never listed.
pub const SECTION_ORDER: [Section; 4] = [
    Section::General,
    Section::Privacy,
    Section::Appearance,
    Section::Performance,
];

/// The settings page from the stored `key -> text` rows. A missing or invalid value shows the
/// default, as the store reads it. `Hidden` settings are left out; sections keep the table order
/// and an empty section is not listed.
pub fn settings_page(stored: &HashMap<String, String>) -> Vec<SectionView> {
    settings_page_from(settings::ALL, stored)
}

/// [`settings_page`] over any table: the page is generated from the definitions alone.
pub fn settings_page_from(
    defs: &[&settings::SettingDef],
    stored: &HashMap<String, String>,
) -> Vec<SectionView> {
    SECTION_ORDER
        .iter()
        .filter_map(|&section| {
            let rows: Vec<SettingRow> = defs
                .iter()
                .filter(|d| d.section == section)
                .map(|d| {
                    let value = crate::apply::value_of(d, stored);
                    SettingRow {
                        key: d.key,
                        label: d.label,
                        kind: d.kind,
                        control: control_for(d.kind),
                        value: value.to_owned(),
                        changed: value != d.default,
                        restart: d.restart,
                        entries: entries_of(d, stored),
                        columns: columns_of(d),
                    }
                })
                .collect();
            (!rows.is_empty()).then_some(SectionView { section, rows })
        })
        .collect()
}

/// The rows whose label or key contains `query` (case-insensitive; blank keeps all). A section
/// left with no row is dropped.
pub fn filter(sections: &[SectionView], query: &str) -> Vec<SectionView> {
    let needle = query.trim().to_lowercase();
    sections
        .iter()
        .filter_map(|s| {
            let rows: Vec<SettingRow> = s
                .rows
                .iter()
                .filter(|r| {
                    needle.is_empty() || contains_ci(r.label, &needle) || r.key.contains(&needle)
                })
                .cloned()
                .collect();
            (!rows.is_empty()).then_some(SectionView {
                section: s.section,
                rows,
            })
        })
        .collect()
}

// ------------------------------------------------------------------ sites

/// Per-site data the user can delete: saved zoom levels and permission decisions.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SitesView {
    /// By host; each named by its host.
    pub zoom: Vec<Entry>,
    /// By origin then permission; each named by [`permission_key`].
    pub permissions: Vec<Entry>,
}

/// What names a remembered decision on the page: stable while the list changes under it.
pub fn permission_key(origin: &str, kind: crate::site::Permission) -> String {
    format!("{origin}|{}", kind.as_str())
}

/// The origin and permission a [`permission_key`] names.
pub fn parse_permission_key(key: &str) -> Option<(&str, crate::site::Permission)> {
    let (origin, kind) = key.rsplit_once('|')?;
    Some((origin, crate::site::Permission::parse(kind)?))
}

/// The Sites section from the zoom table and the remembered decisions.
pub fn sites_view(
    zoom: &crate::zoom::ZoomTable,
    permissions: &crate::site::SitePermissions,
) -> SitesView {
    SitesView {
        zoom: zoom
            .iter()
            .map(|(h, p)| Entry {
                label: format!("{h}: {p}%"),
                key: h.clone(),
                cells: vec![h.clone(), format!("{p}%")],
            })
            .collect(),
        permissions: permissions
            .iter()
            .map(|(o, k, d)| {
                // links to other apps are kept per site and scheme
                let (site, what) = match crate::external::parse_key(o) {
                    Some((scheme, site)) if k == crate::site::Permission::ExternalLinks => {
                        (site, format!("Open {scheme} links in another app"))
                    }
                    _ => (o, k.ask_text().to_owned()),
                };
                Entry {
                    label: format!("{site}: {what} ({d:?})"),
                    key: permission_key(o, k),
                    cells: vec![site.to_owned(), what, format!("{d:?}")],
                }
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOUR: i64 = 3_600_000;
    // 2024-03-01T12:00:00Z
    const NOW: i64 = 1_709_294_400_000;

    fn visit(url: &str, title: &str, ms: i64) -> HistoryRow {
        HistoryRow {
            url: url.into(),
            title: title.into(),
            visit_ms: ms,
            visit_id: 0,
        }
    }

    #[test]
    fn history_groups_by_day_newest_first() {
        let rows = [
            visit("https://a.test/", "A", NOW - HOUR),
            visit("https://b.test/", "B", NOW - 30 * HOUR),
            visit("https://c.test/", "C", NOW - 2 * HOUR),
            visit("https://d.test/", "D", NOW - 24 * 3 * HOUR),
        ];
        let g = history_page(&rows, 0, NOW);
        let labels: Vec<_> = g.iter().map(|g| g.label).collect();
        assert_eq!(
            labels,
            [
                DayLabel::Today,
                DayLabel::Yesterday,
                DayLabel::Date(NaiveDate::from_ymd_opt(2024, 2, 27).unwrap())
            ]
        );
        let urls: Vec<_> = g[0].rows.iter().map(|r| r.url.as_str()).collect();
        assert_eq!(urls, ["https://a.test/", "https://c.test/"]);
        assert_eq!(g[1].rows.len(), 1);
    }

    #[test]
    fn history_day_boundary_follows_the_offset() {
        // 23:30 UTC on 2024-02-29 is already Mar 1 at +60 and still Feb 29 at -60.
        let late = NOW - 12 * HOUR - HOUR / 2;
        let rows = [visit("https://a.test/", "A", late)];
        let now = NOW; // Mar 1 12:00 UTC
        assert_eq!(history_page(&rows, 60, now)[0].label, DayLabel::Today);
        assert_eq!(history_page(&rows, 0, now)[0].label, DayLabel::Yesterday);
        assert_eq!(history_page(&rows, -60, now)[0].label, DayLabel::Yesterday);
        // Exactly local midnight belongs to the new day; one ms before to the old one.
        let midnight = 19_783 * DAY_MS;
        let rows = [
            visit("https://a.test/", "A", midnight),
            visit("https://b.test/", "B", midnight - 1),
        ];
        let g = history_page(&rows, 0, NOW);
        assert_eq!(g.len(), 2);
        assert_eq!(g[0].rows[0].url, "https://a.test/");
        // Month and leap boundaries print as dates.
        let far = [visit("https://a.test/", "A", 951_782_400_000)]; // 2000-02-29
        assert_eq!(
            history_page(&far, 0, NOW)[0].label,
            DayLabel::Date(NaiveDate::from_ymd_opt(2000, 2, 29).unwrap())
        );
    }

    fn bm(id: i64, parent: Option<i64>, folder: bool, pos: u32, title: &str) -> BookmarkRow {
        BookmarkRow {
            id,
            parent,
            is_folder: folder,
            position: pos,
            title: title.into(),
            url: (!folder).then(|| format!("https://{title}.test")),
        }
    }

    #[test]
    fn bookmark_tree_orders_by_position_and_nests() {
        let rows = [
            bm(3, Some(1), false, 1, "b"),
            bm(2, Some(1), false, 0, "a"),
            bm(1, None, true, 1, "F"),
            bm(4, None, false, 0, "top"),
            bm(5, Some(1), true, 2, "G"),
            bm(6, Some(5), false, 0, "deep"),
        ];
        let tree = bookmark_tree(&rows);
        let names: Vec<_> = tree.iter().map(|i| i.title.as_str()).collect();
        assert_eq!(names, ["top", "F"]);
        let kids: Vec<_> = tree[1].children.iter().map(|i| i.title.as_str()).collect();
        assert_eq!(kids, ["a", "b", "G"]);
        assert_eq!(tree[1].children[2].children[0].title, "deep");
        assert_eq!(tree[0].url.as_deref(), Some("https://top.test"));
    }

    #[test]
    fn bookmark_orphans_go_to_the_top_and_cycles_are_dropped() {
        let rows = [
            bm(1, Some(99), false, 0, "orphan"),
            bm(2, None, true, 1, "root"),
            bm(10, Some(11), true, 0, "x"),
            bm(11, Some(10), true, 0, "y"),
            bm(12, Some(12), false, 5, "self"),
        ];
        let tree = bookmark_tree(&rows);
        let names: Vec<_> = tree.iter().map(|i| i.title.as_str()).collect();
        assert_eq!(names, ["orphan", "root", "self"]);
        assert!(bookmark_tree(&[]).is_empty());
    }

    #[test]
    fn flatten_shows_children_of_expanded_folders_only() {
        let rows = [
            bm(1, None, true, 0, "F"),
            bm(2, Some(1), true, 0, "G"),
            bm(3, Some(2), false, 0, "deep"),
            bm(4, Some(1), false, 1, "b"),
            bm(5, None, false, 1, "top"),
        ];
        let tree = bookmark_tree(&rows);
        let line = |e: &HashSet<i64>| -> Vec<(usize, String)> {
            flatten(&tree, e)
                .iter()
                .map(|f| (f.depth, f.item.title.clone()))
                .collect()
        };
        assert_eq!(line(&HashSet::new()), [(0, "F".into()), (0, "top".into())]);
        assert_eq!(
            line(&HashSet::from([1])),
            [
                (0, "F".into()),
                (1, "G".into()),
                (1, "b".into()),
                (0, "top".into())
            ]
        );
        assert_eq!(line(&HashSet::from([1, 2])).len(), 5);
        // Expanding a nested folder whose parent is collapsed shows nothing extra.
        assert_eq!(line(&HashSet::from([2])).len(), 2);
    }

    fn entry(id: &str, origin: &str, created: i64) -> LoginView {
        LoginView {
            id: id.into(),
            origin: origin.into(),
            username: Some(format!("user{id}")),
            created_ms: created,
            last_used_ms: None,
            times_used: 0,
        }
    }

    #[test]
    fn passwords_sort_by_site_then_age_and_filter() {
        let rows = [
            entry("1", "https://b.test", 5),
            entry("2", "http://a.test", 9),
            entry("3", "https://a.test", 1),
            entry("4", "https://A.test:8443", 3),
        ];
        let ids = |q: &str| -> Vec<String> {
            passwords_list(&rows, q).into_iter().map(|e| e.id).collect()
        };
        assert_eq!(ids(""), ["3", "2", "4", "1"]);
        assert_eq!(ids("A.TEST"), ["3", "2", "4"]);
        assert_eq!(ids(":8443"), ["4"]);
        assert!(ids("zzz").is_empty());
        assert_eq!(ids("USER2"), ["2"]);
    }

    #[test]
    fn status_text_names_each_state() {
        let t = |s| status_text(&s);
        assert_eq!(t(VaultStatus::Locked).0, "Passwords are locked");
        assert_eq!(t(VaultStatus::Unlocked).0, "Saved passwords");
        let (title, body) = t(VaultStatus::Unavailable("no daemon".into()));
        assert_eq!(title, "Passwords are unavailable");
        assert!(body.contains("no daemon"));
        let (title, body) = t(VaultStatus::Unreadable { rows: 1 });
        assert_eq!(title, "Saved passwords cannot be read");
        assert!(body.starts_with("1 saved login no longer"));
        assert!(
            t(VaultStatus::Unreadable { rows: 3 })
                .1
                .starts_with("3 saved logins")
        );
    }

    fn stored(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn settings_page_groups_sections_and_skips_hidden() {
        let page = settings_page(&HashMap::new());
        let sections: Vec<_> = page.iter().map(|s| s.section).collect();
        assert_eq!(sections, SECTION_ORDER);
        let keys: Vec<_> = page.iter().flat_map(|s| &s.rows).map(|r| r.key).collect();
        assert!(!keys.contains(&"ui.sidebar_width") && !keys.contains(&"keys.overrides"));
        // Every non-hidden setting appears exactly once.
        let shown = settings::ALL
            .iter()
            .filter(|d| d.section != Section::Hidden)
            .count();
        assert_eq!(keys.len(), shown);
        assert!(page.iter().flat_map(|s| &s.rows).all(|r| !r.changed));
    }

    #[test]
    fn settings_page_values_changed_and_fallback() {
        let page = settings_page(&stored(&[
            ("suggest.live", "1"),
            ("blocker.enabled", "maybe"),
            ("history.retention_days", "-3"),
            ("search.engine", "duckduckgo"),
        ]));
        let row = |key: &str| {
            page.iter()
                .flat_map(|s| &s.rows)
                .find(|r| r.key == key)
                .unwrap()
        };
        assert_eq!(
            (
                row("suggest.live").value.as_str(),
                row("suggest.live").changed
            ),
            ("1", true)
        );
        assert_eq!(
            (
                row("blocker.enabled").value.as_str(),
                row("blocker.enabled").changed
            ),
            ("1", false)
        );
        assert_eq!(row("history.retention_days").value, "0");
        assert!(row("search.engine").changed);
        assert!(!row("ui.sidebar_mode").restart);
    }

    #[test]
    fn every_kind_has_a_control() {
        let kinds = [
            (SettingKind::Bool, Control::Toggle),
            (
                SettingKind::Count { min: 1, max: 9 },
                Control::Stepper { min: 1, max: 9 },
            ),
            (
                SettingKind::Float { min: 0.5, max: 2.0 },
                Control::Slider { min: 0.5, max: 2.0 },
            ),
            (SettingKind::SearchEngine, Control::Segmented(&ENGINE_IDS)),
            (
                SettingKind::Choice(&["a", "b"]),
                Control::Segmented(&["a", "b"]),
            ),
            (SettingKind::List, Control::ListEditor),
            (SettingKind::Map, Control::MapEditor),
            (SettingKind::Text, Control::TextField),
        ];
        for (kind, want) in kinds {
            assert_eq!(control_for(kind), want, "{kind:?}");
        }
    }

    /// A table the app does not ship: every kind becomes a row with its control, with no UI
    /// code for any of them.
    #[test]
    fn a_page_is_generated_from_any_table() {
        use settings::SettingDef;
        const fn def(key: &'static str, kind: SettingKind, default: &'static str) -> SettingDef {
            SettingDef {
                key,
                kind,
                default,
                label: key,
                section: Section::General,
                restart: false,
            }
        }
        const DEFS: [SettingDef; 8] = [
            def("t.bool", SettingKind::Bool, "1"),
            def("t.count", SettingKind::Count { min: 0, max: 9 }, "3"),
            def("t.float", SettingKind::Float { min: 0.0, max: 1.0 }, "0.5"),
            def("t.engine", SettingKind::SearchEngine, ""),
            def("t.choice", SettingKind::Choice(&["x", "y"]), "x"),
            def("t.list", SettingKind::List, "[]"),
            def("t.map", SettingKind::Map, "{}"),
            SettingDef {
                section: Section::Hidden,
                ..def("t.hidden", SettingKind::Bool, "0")
            },
        ];
        let defs: Vec<&SettingDef> = DEFS.iter().collect();
        let page = settings_page_from(
            &defs,
            &stored(&[
                ("t.count", "7"),
                ("t.list", r#"["a.test"]"#),
                ("t.map", r#"{"b.test":90}"#),
            ]),
        );
        assert_eq!(page.len(), 1);
        let rows = &page[0].rows;
        assert_eq!(rows.len(), 7);
        for (row, d) in rows.iter().zip(&DEFS) {
            assert_eq!(row.control, control_for(d.kind), "{}", d.key);
        }
        assert_eq!((rows[1].value.as_str(), rows[1].changed), ("7", true));
        // List and map rows come decoded: what each entry shows and what removing it names.
        let entry = |label: &str, key: &str, cells: &[&str]| Entry {
            label: label.into(),
            key: key.into(),
            cells: cells.iter().map(|c| c.to_string()).collect(),
        };
        assert_eq!(rows[5].entries, [entry("a.test", "a.test", &["a.test"])]);
        assert_eq!(
            rows[6].entries,
            [entry("b.test: 90", "b.test", &["b.test", "90"])]
        );
        assert!(rows[0].entries.is_empty());
        assert_eq!(
            (rows[5].columns, rows[6].columns),
            (&["Entry"][..], &["Entry", "Value"][..])
        );
        assert!(rows[0].columns.is_empty());
    }

    #[test]
    fn filter_matches_label_or_key_and_drops_empty_sections() {
        let page = settings_page(&HashMap::new());
        let keys = |q: &str| -> Vec<&str> {
            filter(&page, q)
                .iter()
                .flat_map(|s| s.rows.iter().map(|r| r.key))
                .collect()
        };
        assert_eq!(keys("BLOCK ADS"), ["blocker.enabled"]);
        assert_eq!(keys("glass.quality"), ["glass.quality"]);
        assert_eq!(filter(&page, "zoom").len(), 1);
        assert!(filter(&page, "no such setting").is_empty());
        assert_eq!(filter(&page, "  "), page);
    }

    #[test]
    fn day_labels_read_as_dates() {
        assert_eq!(DayLabel::Today.text(), "Today");
        assert_eq!(DayLabel::Yesterday.text(), "Yesterday");
        let d = |y, m, day| DayLabel::Date(NaiveDate::from_ymd_opt(y, m, day).unwrap()).text();
        assert_eq!(d(1970, 1, 1), "Thu, 1 Jan 1970");
        assert_eq!(d(2026, 11, 3), "Tue, 3 Nov 2026");
        assert_eq!(d(2024, 2, 29), "Thu, 29 Feb 2024");
        assert_eq!(d(1969, 12, 31), "Wed, 31 Dec 1969");
    }

    #[test]
    fn history_lines_put_a_header_before_each_day() {
        let row = |ms| HistoryRow {
            url: format!("https://x.test/{ms}"),
            title: String::new(),
            visit_ms: ms,
            visit_id: ms,
        };
        let now = 3 * DAY_MS + 5;
        let groups = history_page(&[row(now), row(now - 1), row(DAY_MS)], 0, now);
        let lines = history_lines(&groups);
        assert_eq!(lines.len(), 5);
        assert_eq!(
            lines[0],
            HistoryLine::Day {
                label: "Today".into()
            }
        );
        assert!(matches!(&lines[1], HistoryLine::Visit(r) if r.visit_ms == now));
        assert!(matches!(&lines[3], HistoryLine::Day { label } if label == "Fri, 2 Jan 1970"));
        assert!(history_lines(&[]).is_empty());
    }

    #[test]
    fn clear_ranges_reach_back_from_now() {
        assert_eq!(
            ClearRange::LastHour.bounds(10 * MIN_MS * 60),
            (9 * 60 * MIN_MS, i64::MAX)
        );
        assert_eq!(ClearRange::LastDay.bounds(2 * DAY_MS), (DAY_MS, i64::MAX));
        assert_eq!(ClearRange::All.bounds(5), (i64::MIN, i64::MAX));
        assert_eq!(ClearRange::ALL.len(), 3);
    }

    fn link(id: i64, parent: Option<i64>, pos: u32, title: &str, url: &str) -> BookmarkRow {
        BookmarkRow {
            url: Some(url.into()),
            ..bm(id, parent, false, pos, title)
        }
    }

    fn sample() -> Vec<BookmarkRow> {
        vec![
            bm(1, None, true, 0, "News"),
            bm(2, Some(1), true, 0, "World"),
            link(3, Some(2), 0, "Wire", "https://www.wire.test/a"),
            link(4, None, 1, "Home", "https://home.test/"),
        ]
    }

    #[test]
    fn bookmark_for_url_ignores_scheme_and_www() {
        let rows = sample();
        assert_eq!(bookmark_for_url(&rows, "http://wire.test/a"), Some(3));
        assert_eq!(bookmark_for_url(&rows, "https://wire.test/b"), None);
    }

    #[test]
    fn add_current_refuses_duplicates_and_non_pages() {
        let rows = sample();
        assert_eq!(add_current(&rows, "T", "https://home.test", None), None);
        assert_eq!(add_current(&rows, "T", "about:blank", None), None);
        assert_eq!(add_current(&rows, "T", "wubble://history", None), None);
        assert_eq!(
            add_current(&rows, " ", "https://new.test/", Some(1)),
            Some(BookmarkEdit::Add {
                parent: Some(1),
                title: "https://new.test/".into(),
                url: "https://new.test/".into()
            })
        );
    }

    #[test]
    fn move_to_end_counts_the_new_siblings() {
        let rows = sample();
        assert_eq!(
            move_to_end(&rows, 4, Some(1)),
            BookmarkEdit::Move {
                id: 4,
                to: Some(1),
                index: 1
            }
        );
        // Moving within the same folder does not count itself.
        assert_eq!(
            move_to_end(&rows, 4, None),
            BookmarkEdit::Move {
                id: 4,
                to: None,
                index: 1
            }
        );
    }

    #[test]
    fn folder_choices_are_depth_first_without_the_moved_subtree() {
        let rows = sample();
        let labels = |m| -> Vec<(String, usize)> {
            folder_choices(&rows, m)
                .into_iter()
                .map(|c| (c.label, c.depth))
                .collect()
        };
        assert_eq!(
            labels(None),
            [
                ("Bookmarks".into(), 0),
                ("News".into(), 1),
                ("World".into(), 2)
            ]
        );
        assert_eq!(labels(Some(2)).len(), 2);
        assert_eq!(labels(Some(1)).len(), 1);
    }

    #[test]
    fn filter_tree_lists_matching_links_with_their_path() {
        let rows = sample();
        assert!(filter_tree(&rows, "  ").is_empty());
        let hits = filter_tree(&rows, "WIRE");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].0.id, 3);
        assert_eq!(hits[0].1, "News / World");
        assert_eq!(filter_tree(&rows, "home")[0].1, "");
        // Folders are not matches.
        assert!(filter_tree(&rows, "news").is_empty());
    }

    #[test]
    fn blocker_badge_shows_for_a_count_or_an_allowed_site() {
        let ex = vec!["a.test".to_owned()];
        assert!(!blocker_state(0, "https://b.test/", &ex).visible);
        assert!(blocker_state(2, "https://b.test/", &ex).visible);
        let s = blocker_state(0, "https://A.test/x", &ex);
        assert!(s.site_allowed && s.visible);
        assert!(!blocker_state(0, "about:blank", &ex).visible);
    }

    #[test]
    fn internal_pages_round_trip_their_url() {
        for p in InternalPage::ALL {
            assert_eq!(InternalPage::from_url(p.url()), Some(p));
            assert_eq!(InternalPage::from_url(&format!("{}/", p.url())), Some(p));
            assert!(!p.title().is_empty());
        }
        assert_eq!(
            InternalPage::from_url("WUBBLE://settings"),
            Some(InternalPage::Settings)
        );
        assert_eq!(
            InternalPage::from_url("wubble://History/"),
            Some(InternalPage::History)
        );
        assert_eq!(InternalPage::from_url("https://settings"), None);
        assert_eq!(InternalPage::from_url("wubble://nope"), None);
        // the new-tab page
        assert_eq!(InternalPage::ALL.len(), 8);
        assert_eq!(
            InternalPage::from_url("Wubble://Start/"),
            Some(InternalPage::Start)
        );
        assert_eq!(InternalPage::Start.url(), "wubble://start");
        assert_eq!(InternalPage::Start.title(), "New tab");
    }

    #[test]
    fn a_rule_shows_in_three_columns_and_its_draft_makes_it_again() {
        let stored = stored(&[(
            settings::ROUTING_RULES.key,
            r#"["*.corp.test -> Work, Bank","a.test -> , Bank","b.test -> Home"]"#,
        )]);
        let row = settings_page(&stored)
            .into_iter()
            .flat_map(|s| s.rows)
            .find(|r| r.key == settings::ROUTING_RULES.key)
            .unwrap();
        assert_eq!(row.columns, ["Site", "Space", "Container"]);
        let cells: Vec<Vec<String>> = row.entries.iter().map(|e| e.cells.clone()).collect();
        assert_eq!(
            cells,
            [
                ["*.corp.test", "Work", "Bank"],
                ["a.test", "", "Bank"],
                ["b.test", "Home", ""]
            ]
        );
        for e in &row.entries {
            let cells: Vec<&str> = e.cells.iter().map(String::as_str).collect();
            let text = entry_text(settings::ROUTING_RULES.key, &cells);
            assert_eq!(
                crate::routing::Rule::parse(&text),
                crate::routing::Rule::parse(&e.key),
                "{text}"
            );
        }
        let draft = format!("a.test{REPLACE_SEP}c.test{CELL_SEP}Work{CELL_SEP}");
        assert_eq!(
            submission(&draft),
            Submission {
                replacing: Some("a.test"),
                cells: vec!["c.test", "Work", ""]
            }
        );
        assert_eq!(
            submission("x.test"),
            Submission {
                replacing: None,
                cells: vec!["x.test"]
            }
        );
        assert_eq!(entry_text("blocker.exceptions", &[" x.test "]), "x.test");
        assert!(
            settings::ALL.iter().all(|d| d.key != SPACES_LIST),
            "the Spaces list's key is no setting's"
        );
    }

    #[test]
    fn sites_view_lists_zoom_and_decisions_in_order() {
        use crate::site::{Decision, Permission, SitePermissions};
        let zoom = crate::zoom::ZoomTable::from([("b.test".into(), 125), ("a.test".into(), 90)]);
        let mut perms = SitePermissions::default();
        perms.set("https://b.test", Permission::Camera, Decision::Allow);
        perms.set("https://a.test", Permission::Microphone, Decision::Block);
        perms.set("https://a.test", Permission::Camera, Decision::Allow);
        let v = sites_view(&zoom, &perms);
        let zoom: Vec<(&str, &str)> = v
            .zoom
            .iter()
            .map(|e| (e.key.as_str(), e.label.as_str()))
            .collect();
        assert_eq!(
            zoom,
            [("a.test", "a.test: 90%"), ("b.test", "b.test: 125%")]
        );
        assert_eq!(v.zoom[0].cells, ["a.test", "90%"]);
        assert_eq!(
            v.permissions[0].cells,
            ["https://a.test", "Use your camera", "Allow"]
        );
        // Each decision is named by its origin and kind, not its place in the list.
        let origins: Vec<(&str, Permission)> = v
            .permissions
            .iter()
            .map(|e| parse_permission_key(&e.key).unwrap())
            .collect();
        assert_eq!(
            v.permissions[0].label,
            "https://a.test: Use your camera (Allow)"
        );
        assert_eq!(parse_permission_key("https://a.test|nope"), None);
        assert_eq!(parse_permission_key("https://a.test"), None);
        assert_eq!(
            origins,
            [
                ("https://a.test", Permission::Camera),
                ("https://a.test", Permission::Microphone),
                ("https://b.test", Permission::Camera)
            ]
        );
        // A link to another app is named by its site and scheme.
        perms.set(
            "mailto:https://a.test",
            Permission::ExternalLinks,
            Decision::Allow,
        );
        let v = sites_view(&Default::default(), &perms);
        let row = v.permissions.last().unwrap();
        assert_eq!(
            row.label,
            "https://a.test: Open mailto links in another app (Allow)"
        );
        assert_eq!(
            parse_permission_key(&row.key),
            Some(("mailto:https://a.test", Permission::ExternalLinks))
        );
    }
}

#[cfg(test)]
mod relative_age_tests {
    use super::relative_age;

    #[test]
    fn ages_read_in_the_largest_unit() {
        const NOW: u64 = 1_000_000_000;
        for (ago, text) in [
            (0, "just now"),
            (59, "just now"),
            (60, "1 minute ago"),
            (7_200, "2 hours ago"),
            (86_400, "1 day ago"),
            (3 * 86_400, "3 days ago"),
            (40 * 86_400, "1 month ago"),
            (800 * 86_400, "2 years ago"),
        ] {
            assert_eq!(relative_age(NOW, NOW - ago), text, "{ago}");
        }
        assert_eq!(relative_age(NOW, NOW + 500), "just now");
    }
}
