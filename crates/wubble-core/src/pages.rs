//! Pure view-models for the native pages (history, bookmarks, downloads, passwords, settings).
//!
//! Each takes plain input rows (the same pattern as `omnibox::HistoryEntry`: core defines the
//! row, the store or host maps into it) and returns plain structs a Slint model can show.
//! No clocks, no time zone lookups, no I/O: `now_ms` and the UTC offset are parameters.

use std::cmp::Reverse;
use std::collections::{HashMap, HashSet};

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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DayLabel {
    Today,
    Yesterday,
    Date { year: i32, month: u8, day: u8 },
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

/// (year, month 1..=12, day 1..=31) of a day count since 1970-01-01.
///
/// Source: Howard Hinnant, "chrono-Compatible Low-Level Date Algorithms" `civil_from_days`
/// (http://howardhinnant.github.io/date_algorithms.html), public domain.
fn civil_from_days(days: i64) -> (i32, u8, u8) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year as i32, month as u8, day as u8)
}

fn day_label(day: i64, today: i64) -> DayLabel {
    if day == today {
        DayLabel::Today
    } else if day == today - 1 {
        DayLabel::Yesterday
    } else {
        let (year, month, day) = civil_from_days(day);
        DayLabel::Date { year, month, day }
    }
}

fn contains_ci(haystack: &str, needle_lower: &str) -> bool {
    haystack.to_lowercase().contains(needle_lower)
}

/// The history page: visits matching `query` (case-insensitive substring of title or url; empty
/// keeps all), newest first, grouped by local day.
pub fn history_page(
    rows: &[HistoryRow],
    query: &str,
    tz_offset_min: i32,
    now_ms: i64,
) -> Vec<DayGroup> {
    let needle = query.trim().to_lowercase();
    let today = local_day(now_ms, tz_offset_min);
    let mut hits: Vec<&HistoryRow> = rows
        .iter()
        .filter(|r| {
            needle.is_empty() || contains_ci(&r.title, &needle) || contains_ci(&r.url, &needle)
        })
        .collect();
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

// -------------------------------------------------------------- downloads

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DownloadState {
    Waiting,
    Running,
    Paused,
    Failed,
    Done,
    Cancelled,
}

impl DownloadState {
    fn active(self) -> bool {
        matches!(self, Self::Waiting | Self::Running | Self::Paused)
    }
}

/// A download as the host knows it (there is no downloads table: the host fills this from the
/// engine's download state machine).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadRow {
    /// Ascending in start order.
    pub id: u64,
    pub name: String,
    pub url: String,
    pub state: DownloadState,
    pub received: u64,
    pub total: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadItem {
    pub id: u64,
    pub name: String,
    pub url: String,
    pub state: DownloadState,
    /// 0..=100; `None` while the total is unknown.
    pub percent: Option<u8>,
    /// Second line of the row: "3.2 MB of 10.0 MB", "Paused", ...
    pub label: String,
}

fn size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["KB", "MB", "GB", "TB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut v = bytes as f64 / 1024.0;
    let mut unit = 0;
    while v >= 1024.0 && unit < UNITS.len() - 1 {
        v /= 1024.0;
        unit += 1;
    }
    format!("{v:.1} {}", UNITS[unit])
}

fn download_item(r: &DownloadRow) -> DownloadItem {
    let percent = match (r.state, r.total) {
        (DownloadState::Done, _) => Some(100),
        (_, Some(t)) if t > 0 => Some((r.received.min(t) * 100 / t) as u8),
        _ => None,
    };
    let progress = match r.total {
        Some(t) => format!("{} of {}", size(r.received), size(t)),
        None => size(r.received),
    };
    let label = match r.state {
        DownloadState::Waiting => "Waiting".to_owned(),
        DownloadState::Running => progress,
        DownloadState::Paused => format!("Paused, {progress}"),
        DownloadState::Failed => "Failed".to_owned(),
        DownloadState::Cancelled => "Cancelled".to_owned(),
        DownloadState::Done => size(r.total.unwrap_or(r.received)),
    };
    DownloadItem {
        id: r.id,
        name: r.name.clone(),
        url: r.url.clone(),
        state: r.state,
        percent,
        label,
    }
}

/// Active downloads first, then finished ones; each group newest first.
pub fn downloads_list(rows: &[DownloadRow]) -> Vec<DownloadItem> {
    let mut sorted: Vec<&DownloadRow> = rows.iter().collect();
    sorted.sort_by_key(|r| (!r.state.active(), Reverse(r.id)));
    sorted.into_iter().map(download_item).collect()
}

// -------------------------------------------------------------- passwords

/// A saved login's metadata. No username and no password: both are encrypted and need the key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginEntry {
    pub id: String,
    pub origin: String,
    pub created_ms: i64,
    pub last_used_ms: Option<i64>,
    pub times_used: u32,
}

/// Logins whose origin contains `query` (case-insensitive; empty keeps all), by site then age.
pub fn passwords_list(rows: &[LoginEntry], query: &str) -> Vec<LoginEntry> {
    let needle = query.trim().to_lowercase();
    let site = |e: &LoginEntry| {
        let o = e.origin.to_lowercase();
        o.split_once("://")
            .map_or(o.clone(), |(_, rest)| rest.to_owned())
    };
    let mut out: Vec<LoginEntry> = rows
        .iter()
        .filter(|e| needle.is_empty() || contains_ci(&e.origin, &needle))
        .cloned()
        .collect();
    out.sort_by_cached_key(|e| (site(e), e.created_ms, e.id.clone()));
    out
}

// --------------------------------------------------------------- settings

#[derive(Debug, Clone, PartialEq)]
pub struct SettingRow {
    pub key: &'static str,
    pub label: &'static str,
    pub kind: SettingKind,
    /// Storage text: the stored value if valid, else the default.
    pub value: String,
    /// The value differs from the default.
    pub changed: bool,
    pub restart: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SectionView {
    pub section: Section,
    pub rows: Vec<SettingRow>,
}

/// The settings page from the stored `key -> text` rows. A missing or invalid value shows the
/// default, as the store reads it. `Hidden` settings are left out; sections keep the table order
/// and an empty section is not listed.
pub fn settings_page(stored: &HashMap<String, String>) -> Vec<SectionView> {
    const ORDER: [Section; 3] = [Section::General, Section::Privacy, Section::Appearance];
    ORDER
        .iter()
        .filter_map(|&section| {
            let rows: Vec<SettingRow> = settings::ALL
                .iter()
                .filter(|d| d.section == section)
                .map(|d| {
                    let value = stored
                        .get(d.key)
                        .filter(|v| d.kind.accepts(v))
                        .map_or(d.default, String::as_str);
                    SettingRow {
                        key: d.key,
                        label: d.label,
                        kind: d.kind,
                        value: value.to_owned(),
                        changed: value != d.default,
                        restart: d.restart,
                    }
                })
                .collect();
            (!rows.is_empty()).then_some(SectionView { section, rows })
        })
        .collect()
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
        }
    }

    #[test]
    fn civil_from_days_known_values() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(-1), (1969, 12, 31));
        assert_eq!(civil_from_days(19_782), (2024, 2, 29)); // leap day
        assert_eq!(civil_from_days(19_783), (2024, 3, 1));
        assert_eq!(civil_from_days(11_016), (2000, 2, 29));
        assert_eq!(civil_from_days(-719_468), (0, 3, 1));
        assert_eq!(civil_from_days(47_482), (2100, 1, 1));
        assert_eq!(civil_from_days(47_541), (2100, 3, 1)); // 2100 is not a leap year
        assert_eq!(civil_from_days(local_day(NOW, 0)), (2024, 3, 1));
    }

    #[test]
    fn history_groups_by_day_newest_first() {
        let rows = [
            visit("https://a.test/", "A", NOW - HOUR),
            visit("https://b.test/", "B", NOW - 30 * HOUR),
            visit("https://c.test/", "C", NOW - 2 * HOUR),
            visit("https://d.test/", "D", NOW - 24 * 3 * HOUR),
        ];
        let g = history_page(&rows, "", 0, NOW);
        let labels: Vec<_> = g.iter().map(|g| g.label).collect();
        assert_eq!(
            labels,
            [
                DayLabel::Today,
                DayLabel::Yesterday,
                DayLabel::Date {
                    year: 2024,
                    month: 2,
                    day: 27
                }
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
        assert_eq!(history_page(&rows, "", 60, now)[0].label, DayLabel::Today);
        assert_eq!(
            history_page(&rows, "", 0, now)[0].label,
            DayLabel::Yesterday
        );
        assert_eq!(
            history_page(&rows, "", -60, now)[0].label,
            DayLabel::Yesterday
        );
        // Exactly local midnight belongs to the new day; one ms before to the old one.
        let midnight = 19_783 * DAY_MS;
        let rows = [
            visit("https://a.test/", "A", midnight),
            visit("https://b.test/", "B", midnight - 1),
        ];
        let g = history_page(&rows, "", 0, NOW);
        assert_eq!(g.len(), 2);
        assert_eq!(g[0].rows[0].url, "https://a.test/");
        // Month and leap boundaries print as dates.
        let far = [visit("https://a.test/", "A", 951_782_400_000)]; // 2000-02-29
        assert_eq!(
            history_page(&far, "", 0, NOW)[0].label,
            DayLabel::Date {
                year: 2000,
                month: 2,
                day: 29
            }
        );
    }

    #[test]
    fn history_search_is_case_insensitive_over_title_and_url() {
        let rows = [
            visit("https://rust-lang.org/", "Rust Language", NOW),
            visit("https://example.com/Docs", "Example", NOW - HOUR),
            visit("https://other.test/", "Other", NOW - 2 * HOUR),
        ];
        let titles = |q: &str| -> Vec<String> {
            history_page(&rows, q, 0, NOW)
                .into_iter()
                .flat_map(|g| g.rows)
                .map(|r| r.title)
                .collect()
        };
        assert_eq!(titles("RUST"), ["Rust Language"]);
        assert_eq!(titles("docs"), ["Example"]);
        assert_eq!(titles("  "), ["Rust Language", "Example", "Other"]);
        assert!(titles("nothing").is_empty());
        assert!(history_page(&[], "", 0, NOW).is_empty());
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

    fn dl(id: u64, state: DownloadState, received: u64, total: Option<u64>) -> DownloadRow {
        DownloadRow {
            id,
            name: format!("f{id}"),
            url: format!("https://x.test/{id}"),
            state,
            received,
            total,
        }
    }

    #[test]
    fn downloads_active_first_then_newest_first() {
        use DownloadState::*;
        let rows = [
            dl(1, Done, 10, Some(10)),
            dl(2, Running, 5, Some(10)),
            dl(3, Failed, 0, None),
            dl(4, Paused, 1, Some(10)),
            dl(5, Cancelled, 0, None),
            dl(6, Waiting, 0, None),
        ];
        let ids: Vec<u64> = downloads_list(&rows).iter().map(|d| d.id).collect();
        assert_eq!(ids, [6, 4, 2, 5, 3, 1]);
    }

    #[test]
    fn download_percent_and_labels() {
        use DownloadState::*;
        let item = |r| download_item(&r);
        assert_eq!(item(dl(1, Running, 5, Some(10))).percent, Some(50));
        assert_eq!(item(dl(1, Running, 5, None)).percent, None);
        assert_eq!(item(dl(1, Running, 5, Some(0))).percent, None);
        assert_eq!(item(dl(1, Running, 50, Some(10))).percent, Some(100));
        assert_eq!(item(dl(1, Done, 7, None)).percent, Some(100));
        assert_eq!(
            item(dl(1, Running, 3 << 20, Some(10 << 20))).label,
            "3.0 MB of 10.0 MB"
        );
        assert_eq!(item(dl(1, Running, 512, None)).label, "512 B");
        assert_eq!(
            item(dl(1, Paused, 1024, Some(2048))).label,
            "Paused, 1.0 KB of 2.0 KB"
        );
        assert_eq!(item(dl(1, Done, 2048, Some(2048))).label, "2.0 KB");
        assert_eq!(item(dl(1, Failed, 0, None)).label, "Failed");
        assert_eq!(item(dl(1, Running, 1 << 40, None)).label, "1.0 TB");
    }

    fn entry(id: &str, origin: &str, created: i64) -> LoginEntry {
        LoginEntry {
            id: id.into(),
            origin: origin.into(),
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
        assert_eq!(
            sections,
            [Section::General, Section::Privacy, Section::Appearance]
        );
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
        assert!(!row("ui.compact").restart);
    }
}
