//! The command bar's rows and what choosing one does, as pure functions over
//! `wubble_core::omnibox` (classify and rank) and the keymap.
//!
//! Slint's bar fires `result-chosen` only for an existing row; `omnibox::rank`
//! always puts the classified intent ("open" or "search") first, so Enter on
//! free text chooses it.

use wubble_core::action::Action;
use wubble_core::answer;
use wubble_core::closed::Listed;
use wubble_core::keys::Keymap;
use wubble_core::model::{Command, Model};
use wubble_core::omnibox::{
    self, Bookmark, Candidates, Disposition, HistoryEntry, Kind, OmniResult, OpenTab, SearchClosed,
    SearchTab, Target,
};
use wubble_core::search::{EngineRef, Engines};
use wubble_core::space::SpaceKind;
use wubble_core::tab::TabId;
use wubble_ui::CommandKind;

use crate::shell::tab_title;

/// Rows the bar shows at most.
pub const MAX_ROWS: usize = 12;
/// History hits asked of the store per query.
pub const HISTORY_HITS: usize = 20;
/// Rows tab search lists at most (its list scrolls): every open tab of a heavy session and the
/// whole recently-closed list.
pub const TAB_SEARCH_ROWS: usize = 500;

/// What Enter on the query does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BarMode {
    /// Ctrl+T: open a new tab.
    NewTab,
    /// Ctrl+L: load in this tab.
    Navigate(TabId),
    /// Split view: the page chosen goes beside the focused one.
    Beside,
    /// Mod+Shift+A, tab search: the rows are every open and recently closed tab.
    Tabs,
}

/// What a row stands for: an omnibox result, or (first run only) a search
/// engine to choose, by its stored id. The engine choice is the bar's own row, not a global
/// `Action`: it exists only until a choice is stored.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RowTarget {
    Omni(Target),
    PickEngine(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct BarRow {
    pub kind: CommandKind,
    pub title: String,
    pub subtitle: String,
    /// The action's shortcut, from the keymap; empty for other rows.
    pub shortcut: String,
    pub target: RowTarget,
}

impl BarRow {
    /// The page whose stored favicon the row shows: a history or bookmark row's URL.
    pub fn icon_url(&self) -> Option<&str> {
        match (&self.target, self.kind) {
            (RowTarget::Omni(Target::Url(u)), CommandKind::History | CommandKind::Bookmark) => {
                Some(u)
            }
            (RowTarget::Omni(Target::ClosedTab { url, .. }), _) if !url.is_empty() => Some(url),
            _ => None,
        }
    }

    /// The open tab whose favicon the row shows (a switch-to-tab row).
    pub fn icon_tab(&self) -> Option<TabId> {
        match self.target {
            RowTarget::Omni(Target::OpenTab(t)) => Some(t),
            _ => None,
        }
    }
}

/// Everything the rows are ranked over.
pub struct BarInputs<'a> {
    pub tabs: &'a [OpenTab],
    pub history: &'a [HistoryEntry],
    /// Bookmarked links.
    pub bookmarks: &'a [Bookmark],
    /// Every engine; its default is `None` until the first-run choice.
    pub engines: &'a Engines,
    pub keymap: &'a Keymap,
    /// Live engine suggestions, already fetched (`suggest_fx`).
    pub suggestions: &'a [String],
    pub now_ms: i64,
    /// No inline completion: the edit only deleted text, a completion was just accepted, or the
    /// field cannot show one (`omnibox::Candidates::prevent_inline`).
    pub prevent_inline: bool,
}

/// What choosing a row does.
#[derive(Clone, Debug, PartialEq)]
pub enum Choice {
    Model(Command),
    Perform(Action),
    /// Store the search engine (first run), by its id.
    SetEngine(String),
    /// Reopen this recently-closed entry (by its store id) and take it off the list.
    ReopenClosed(i64),
    /// Shift+Enter: a new window, then this URL in a new tab there.
    NewWindow(String),
    /// Put the text on the clipboard (an answer row).
    Copy(String),
}

/// The open tabs, active Space first, each Space in sidebar order. A private window lists its
/// own tabs only, and no other window lists a private one (Chrome's tab switcher stays within
/// one profile).
pub fn open_tabs(model: &Model) -> Vec<OpenTab> {
    let mut spaces: Vec<_> = model
        .listed_spaces(model.active_space())
        .map(|s| s.id)
        .collect();
    spaces.sort_by_key(|&s| s != model.active_space());
    spaces
        .into_iter()
        .flat_map(|s| model.order(s))
        .filter_map(|id| model.tabs.get(&id))
        .map(|t| OpenTab {
            id: t.id,
            title: tab_title(t),
            url: t.url.clone(),
        })
        .collect()
}

/// The open tabs tab search lists, in sidebar order (`omnibox::tab_search` orders them): every
/// listed Space's (a private window its own only, as `open_tabs`) but an agent Space's, without
/// a peek's and without the tabs `left_out` names (the host's: those an agent session drives).
/// A tab on screen counts as active now.
pub fn search_tabs(model: &Model, left_out: impl Fn(TabId) -> bool) -> Vec<SearchTab> {
    let active = model.active_space();
    model
        .listed_spaces(active)
        .filter(|s| s.kind != SpaceKind::Agent)
        .flat_map(|s| model.order(s.id).into_iter().map(move |id| (s, id)))
        .filter(|&(_, id)| !model.is_glance(id) && !left_out(id))
        .filter_map(|(s, id)| model.tabs.get(&id).map(|t| (s, t)))
        .map(|(s, t)| SearchTab {
            id: t.id,
            title: tab_title(t),
            url: t.url.clone(),
            space: s.name.clone(),
            active_space: s.id == active,
            last_active_ms: if model.window_of(t.id).is_some() {
                i64::MAX
            } else {
                t.last_active_ms
            },
        })
        .collect()
}

/// Tab search's rows for `query` (`omnibox::tab_search`).
pub fn tab_search_rows(tabs: &[SearchTab], closed: &[SearchClosed], query: &str) -> Vec<BarRow> {
    omnibox::tab_search(query, tabs, closed, TAB_SEARCH_ROWS)
        .into_iter()
        .map(|r| BarRow {
            kind: kind(r.kind),
            title: r.title,
            subtitle: r.subtitle,
            shortcut: String::new(),
            target: RowTarget::Omni(r.target),
        })
        .collect()
}

fn kind(k: Kind) -> CommandKind {
    match k {
        Kind::OpenTab => CommandKind::OpenTab,
        Kind::History => CommandKind::History,
        Kind::Bookmark => CommandKind::Bookmark,
        Kind::Search => CommandKind::Search,
        Kind::Action => CommandKind::Action,
        Kind::ClosedTab => CommandKind::ClosedTab,
        Kind::Answer => CommandKind::Answer,
    }
}

fn omni_row(r: OmniResult, keymap: &Keymap) -> BarRow {
    let shortcut = match r.target {
        Target::Action(a) => keymap
            .shortcut_for(a)
            .map(|c| c.to_string())
            .unwrap_or_default(),
        _ => String::new(),
    };
    BarRow {
        kind: kind(r.kind),
        title: r.title,
        subtitle: r.subtitle,
        shortcut,
        target: RowTarget::Omni(r.target),
    }
}

/// The rows for `query`. Once an engine is chosen: `omnibox::rank` (nothing
/// for a blank query). Before: the engine choice instead of every search
/// row, after the "open" row if the query is an address.
pub fn bar_rows(inputs: &BarInputs, query: &str) -> Vec<BarRow> {
    let cands = Candidates {
        tabs: inputs.tabs,
        history: inputs.history,
        bookmarks: inputs.bookmarks,
        suggestions: inputs.suggestions,
        prevent_inline: inputs.prevent_inline,
    };
    // Before the choice the default search rows are replaced below; a keyword's rows (its
    // engine's results pages) stay.
    let ranked = omnibox::rank(query, inputs.engines, &cands, inputs.now_ms, MAX_ROWS);
    let rows = ranked.into_iter().map(|r| omni_row(r, inputs.keymap));
    if inputs.engines.default.is_some() {
        return rows.collect();
    }
    let mut rows: Vec<BarRow> = rows
        .filter(|r| !matches!(r.target, RowTarget::Omni(Target::Search(_))))
        .collect();
    // The "open" row, a keyword search and a calculation's answer stay above the choice, as
    // above a search.
    let leads = inputs.engines.keyword_search(query).is_some()
        || match omnibox::classify(query) {
            omnibox::Intent::Url(_) => true,
            omnibox::Intent::Search(q) => {
                answer::answer(&q).is_some_and(|a| a.kind == answer::AnswerKind::Math)
            }
        };
    let at = usize::from(leads);
    let at = at.min(rows.len());
    // the first-run rows: the region's engines (`Engines::first_run`), each a search row (the
    // search glyph): choosing one searches the query with it
    let choice = inputs.engines.first_run().map(|e| BarRow {
        kind: CommandKind::Search,
        title: format!("Search with {}", e.name),
        subtitle: "Choose the search engine".into(),
        shortcut: String::new(),
        target: RowTarget::PickEngine(e.id.clone()),
    });
    rows.splice(at..at, choice);
    rows.truncate(MAX_ROWS);
    rows
}

/// The inline completion the field shows for `query`, the same one `bar_rows` puts first.
pub fn bar_inline(inputs: &BarInputs, query: &str) -> Option<omnibox::InlineMatch> {
    if inputs.prevent_inline {
        return None;
    }
    omnibox::inline_match(query, inputs.history, inputs.bookmarks)
}

/// The URL Shift+Delete forgets on `row`: a history row's (Chrome deletes only what history can
/// forget), `None` for any other row.
pub fn removable_url(row: &BarRow) -> Option<&str> {
    match (&row.target, row.kind) {
        (RowTarget::Omni(Target::Url(u)), CommandKind::History) => Some(u),
        _ => None,
    }
}

/// Recently-closed rows a typed query adds at most.
const CLOSED_HITS: usize = 3;

/// `rows` with the recently-closed list's rows (Chrome's tab search and History menu list
/// "Recently closed" with titles): on a blank query every entry, newest first; on a typed one
/// the entries it finds, after the ranked rows. Not for a bar that puts a page beside another.
pub fn with_closed(
    mut rows: Vec<BarRow>,
    closed: &[Listed],
    query: &str,
    mode: BarMode,
) -> Vec<BarRow> {
    if mode == BarMode::Beside {
        return rows;
    }
    let query = query.trim().to_lowercase();
    // a blank query lists every entry the rows above leave room for (the first-run choice
    // stays); a typed one adds its few hits
    let room = if query.is_empty() {
        MAX_ROWS.saturating_sub(rows.len())
    } else {
        CLOSED_HITS
    };
    let found: Vec<BarRow> = closed
        .iter()
        .filter(|l| l.entry.matches(&query))
        .take(room)
        .map(|l| BarRow {
            kind: CommandKind::ClosedTab,
            title: l.entry.title(),
            subtitle: format!("{} \u{b7} {}", omnibox::RECENTLY_CLOSED, l.entry.note()),
            shortcut: String::new(),
            target: RowTarget::Omni(Target::ClosedTab {
                id: l.id,
                url: l.entry.icon_url().unwrap_or_default().to_owned(),
            }),
        })
        .collect();
    rows.truncate(MAX_ROWS.saturating_sub(found.len()).max(1));
    rows.extend(found);
    rows.truncate(MAX_ROWS);
    rows
}

/// What choosing `target` in a bar opened in `mode` does, with the keys Enter was pressed with
/// (`omnibox::disposition`): Alt+Enter loads in a new tab, Shift+Enter in a new window, whatever
/// the bar was opened for. A bar choosing a page beside the focused one keeps doing that; a switch
/// to an open tab and an action are what they are. A search needs an engine: without one it is
/// `None` (the first-run rows replace it).
pub fn target_command_as(
    target: &RowTarget,
    mode: BarMode,
    engine: Option<&EngineRef>,
    disposition: Disposition,
) -> Option<Choice> {
    let load = |url: String| match (mode, disposition) {
        (BarMode::Beside, _) => Choice::Model(Command::OpenBeside { url }),
        (_, Disposition::NewWindow) => Choice::NewWindow(url),
        (BarMode::Navigate(tab), Disposition::NewTab) => Choice::Model(Command::NewTab {
            url,
            container: None,
            opener: Some(tab),
        }),
        (_, _) => Choice::Model(load_as_opened(mode, url)),
    };
    Some(match target {
        RowTarget::Omni(Target::Url(url)) => load(url.clone()),
        RowTarget::Omni(Target::Search(q)) => load(engine?.search_url(q)),
        RowTarget::Omni(Target::OpenTab(tab)) => Choice::Model(match mode {
            BarMode::Beside => Command::SplitWith(*tab),
            _ => Command::ActivateTab(*tab),
        }),
        RowTarget::Omni(Target::Action(a)) => Choice::Perform(*a),
        RowTarget::Omni(Target::ClosedTab { id, .. }) => Choice::ReopenClosed(*id),
        RowTarget::Omni(Target::Copy(text)) => Choice::Copy(text.clone()),
        RowTarget::PickEngine(id) => Choice::SetEngine(id.clone()),
    })
}

/// A load where the bar was opened for it.
fn load_as_opened(mode: BarMode, url: String) -> Command {
    match mode {
        BarMode::NewTab | BarMode::Tabs => Command::NewTab {
            url,
            container: None,
            opener: None,
        },
        BarMode::Navigate(tab) => Command::Navigate { tab, url },
        BarMode::Beside => Command::OpenBeside { url },
    }
}

/// Whether choosing `target` loads something the user typed or picked from
/// the bar: such a load is a typed visit in history.
pub fn is_typed(target: &RowTarget) -> bool {
    matches!(target, RowTarget::Omni(Target::Url(_) | Target::Search(_)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use wubble_core::keys::Platform;
    use wubble_core::model::apply;
    use wubble_core::space::{ContainerId, Rgb, SpaceId};

    fn tab(m: Model, url: &str, title: &str) -> (Model, TabId) {
        let (m, _) = apply(
            m,
            Command::NewTab {
                url: url.into(),
                container: None,
                opener: None,
            },
        );
        let t = m.visible_tab().unwrap();
        let (m, _) = apply(
            m,
            Command::TitleChanged {
                tab: t,
                title: title.into(),
            },
        );
        (m, t)
    }

    #[test]
    fn history_and_bookmark_rows_name_an_icon_by_url_and_tab_rows_by_tab() {
        let row = |kind, target| BarRow {
            kind,
            title: String::new(),
            subtitle: String::new(),
            shortcut: String::new(),
            target,
        };
        let url = RowTarget::Omni(Target::Url("https://a.test/".into()));
        assert_eq!(
            row(CommandKind::History, url.clone()).icon_url(),
            Some("https://a.test/")
        );
        assert_eq!(
            row(CommandKind::Bookmark, url.clone()).icon_url(),
            Some("https://a.test/")
        );
        assert_eq!(row(CommandKind::Action, url).icon_url(), None);
        let act = RowTarget::Omni(Target::Action(Action::Reload));
        assert_eq!(row(CommandKind::History, act).icon_url(), None);
        let tab = RowTarget::Omni(Target::OpenTab(TabId(3)));
        assert_eq!(
            row(CommandKind::OpenTab, tab.clone()).icon_tab(),
            Some(TabId(3))
        );
        assert_eq!(row(CommandKind::OpenTab, tab).icon_url(), None);
    }

    /// The rows with `default` the chosen engine's id (`""`: none chosen yet).
    fn rows(default: &str, tabs: &[OpenTab], query: &str) -> Vec<BarRow> {
        let keymap = Keymap::defaults(Platform::Linux);
        bar_rows(
            &BarInputs {
                tabs,
                history: &[],
                bookmarks: &[],
                engines: &Engines::resolve(default, &[], &[]),
                keymap: &keymap,
                suggestions: &[],
                now_ms: 0,
                prevent_inline: false,
            },
            query,
        )
    }

    fn targets(rows: &[BarRow]) -> Vec<RowTarget> {
        rows.iter().map(|r| r.target.clone()).collect()
    }

    const DDG: &str = "duckduckgo";

    #[test]
    fn open_tabs_list_the_active_space_first() {
        let (m, a) = tab(
            Model::new(std::time::Instant::now()),
            "https://mail.test/",
            "Inbox",
        );
        let (m, _) = apply(
            m,
            Command::NewSpace {
                name: "W".into(),
                tint: Rgb(0, 0, 0),
                container: ContainerId::DEFAULT,
            },
        );
        let (m, _) = apply(m, Command::SwitchSpace(SpaceId(1)));
        let (m, w) = tab(m, "https://w.test/", "");
        let tabs = open_tabs(&m);
        assert_eq!(tabs.iter().map(|t| t.id).collect::<Vec<_>>(), [w, a]);
        assert_eq!(tabs[0].title, "w.test", "an untitled tab shows its host");
    }

    #[test]
    fn the_first_row_is_the_intent() {
        let rows_for = |q| rows(DDG, &[], q);
        let r = rows_for("example.com");
        assert_eq!(
            r[0].target,
            RowTarget::Omni(Target::Url("https://example.com/".into()))
        );
        let r = rows_for("cats");
        assert_eq!(r[0].kind, CommandKind::Search);
        assert_eq!(r[0].target, RowTarget::Omni(Target::Search("cats".into())));
        assert!(rows_for("").is_empty());
    }

    #[test]
    fn tabs_and_actions_are_ranked_with_shortcuts() {
        let (m, t) = tab(
            Model::new(std::time::Instant::now()),
            "https://reload.test/",
            "Reloaded news",
        );
        let r = rows(DDG, &open_tabs(&m), "reload");
        let found = targets(&r);
        assert!(
            found.contains(&RowTarget::Omni(Target::OpenTab(t))),
            "{r:?}"
        );
        let action = r
            .iter()
            .find(|r| r.target == RowTarget::Omni(Target::Action(Action::Reload)))
            .expect("the Reload action");
        assert_eq!(action.kind, CommandKind::Action);
        assert_eq!(action.shortcut, "Ctrl+R");
        let tab_row = r
            .iter()
            .find(|r| r.target == RowTarget::Omni(Target::OpenTab(t)))
            .unwrap();
        assert_eq!(tab_row.shortcut, "");
    }

    #[test]
    fn the_first_run_offers_the_region_s_engines() {
        let picks = |r: &[BarRow]| -> Vec<RowTarget> {
            r.iter()
                .filter(|r| matches!(r.target, RowTarget::PickEngine(_)))
                .map(|r| r.target.clone())
                .collect()
        };
        // no region stored: Chromium's default list
        let all: Vec<RowTarget> = ["google", "bing", "yahoo"]
            .map(|id| RowTarget::PickEngine(id.into()))
            .to_vec();
        // A blank query and a search: the choice comes first, no search row.
        assert_eq!(targets(&rows("", &[], "")), all);
        let r = rows("", &[], "cats");
        assert_eq!(targets(&r[..all.len()]), all);
        assert!(
            !targets(&r).contains(&RowTarget::Omni(Target::Search("cats".into()))),
            "{r:?}"
        );
        assert!(
            r.iter()
                .all(|r| r.kind != CommandKind::Search
                    || matches!(r.target, RowTarget::PickEngine(_))),
            "only the engines are search rows"
        );
        // the engine rows wear the search glyph
        assert!(
            r.iter()
                .filter(|r| matches!(r.target, RowTarget::PickEngine(_)))
                .all(|r| r.kind == CommandKind::Search)
        );
        // An address still opens first.
        let r = rows("", &[], "example.com");
        assert_eq!(
            r[0].target,
            RowTarget::Omni(Target::Url("https://example.com/".into()))
        );
        assert_eq!(targets(&r[1..1 + all.len()]), all);
        // A keyword searches its engine before any choice.
        let r = rows("", &[], "wikipedia.org cats");
        assert_eq!(
            r[0].target,
            RowTarget::Omni(Target::Url(
                "https://en.wikipedia.org/wiki/Special:Search?search=cats".into()
            ))
        );
        assert_eq!(targets(&r[1..1 + all.len()]), all);
        // Once chosen, never again.
        for q in ["", "cats", "example.com"] {
            assert_eq!(picks(&rows(DDG, &[], q)), vec![], "{q}");
            assert_eq!(picks(&rows("google", &[], q)), vec![]);
        }
    }

    #[test]
    fn live_suggestions_follow_the_local_rows() {
        let keymap = Keymap::defaults(Platform::Linux);
        let on = bar_rows(
            &BarInputs {
                tabs: &[],
                history: &[],
                bookmarks: &[],
                engines: &Engines::resolve(DDG, &[], &[]),
                keymap: &keymap,
                suggestions: &["cats and dogs".into()],
                now_ms: 0,
                prevent_inline: false,
            },
            "cats",
        );
        assert_eq!(on[0].target, RowTarget::Omni(Target::Search("cats".into())));
        assert!(
            on.iter()
                .any(|r| r.target == RowTarget::Omni(Target::Search("cats and dogs".into())))
        );
    }

    #[test]
    fn bookmarks_are_candidates() {
        let keymap = Keymap::defaults(Platform::Linux);
        let bookmarks = [Bookmark {
            url: "https://docs.rs/".into(),
            title: "Docs".into(),
        }];
        let r = bar_rows(
            &BarInputs {
                tabs: &[],
                history: &[],
                bookmarks: &bookmarks,
                engines: &Engines::resolve(DDG, &[], &[]),
                keymap: &keymap,
                suggestions: &[],
                now_ms: 0,
                prevent_inline: false,
            },
            "docs",
        );
        let row = r.iter().find(|r| r.kind == CommandKind::Bookmark).unwrap();
        assert_eq!(row.icon_url(), Some("https://docs.rs/"));
    }

    #[test]
    fn history_rows_open_their_url() {
        let keymap = Keymap::defaults(Platform::Linux);
        let history = [HistoryEntry {
            url: "https://rust-lang.org/".into(),
            title: "Rust".into(),
            visit_count: 3,
            typed_count: 1,
            last_visit_ms: 0,
        }];
        let r = bar_rows(
            &BarInputs {
                tabs: &[],
                history: &history,
                bookmarks: &[],
                engines: &Engines::resolve(DDG, &[], &[]),
                keymap: &keymap,
                suggestions: &[],
                now_ms: 0,
                // the page would complete inline and lead as the completion's row
                prevent_inline: true,
            },
            "rust",
        );
        let row = r.iter().find(|r| r.kind == CommandKind::History).unwrap();
        assert_eq!(
            row.target,
            RowTarget::Omni(Target::Url("https://rust-lang.org/".into()))
        );
    }

    #[test]
    fn targets_map_to_choices() {
        let t = TabId(4);
        let new_tab = |url: &str| {
            Choice::Model(Command::NewTab {
                url: url.into(),
                container: None,
                opener: None,
            })
        };
        let navigate = |url: &str| {
            Choice::Model(Command::Navigate {
                tab: t,
                url: url.into(),
            })
        };
        let url = RowTarget::Omni(Target::Url("https://a.test/".into()));
        let search = RowTarget::Omni(Target::Search("a b".into()));
        let (google, ddg) = (EngineRef::builtin("google"), EngineRef::builtin(DDG));
        let (google, ddg) = (google.as_ref(), ddg.as_ref());
        let pick = RowTarget::PickEngine("google".into());
        let cases = [
            (&url, BarMode::NewTab, ddg, Some(new_tab("https://a.test/"))),
            (
                &url,
                BarMode::Navigate(t),
                None,
                Some(navigate("https://a.test/")),
            ),
            (
                &search,
                BarMode::NewTab,
                google,
                Some(new_tab("https://www.google.com/search?q=a%20b")),
            ),
            (
                &search,
                BarMode::Navigate(t),
                ddg,
                Some(navigate("https://duckduckgo.com/?q=a%20b")),
            ),
            (&search, BarMode::NewTab, None, None),
            (
                &RowTarget::Omni(Target::OpenTab(TabId(9))),
                BarMode::Navigate(t),
                ddg,
                Some(Choice::Model(Command::ActivateTab(TabId(9)))),
            ),
            (
                &RowTarget::Omni(Target::Action(Action::ReopenClosedTab)),
                BarMode::NewTab,
                ddg,
                Some(Choice::Perform(Action::ReopenClosedTab)),
            ),
            (
                &url,
                BarMode::Beside,
                ddg,
                Some(Choice::Model(Command::OpenBeside {
                    url: "https://a.test/".into(),
                })),
            ),
            (
                &RowTarget::Omni(Target::OpenTab(TabId(9))),
                BarMode::Beside,
                ddg,
                Some(Choice::Model(Command::SplitWith(TabId(9)))),
            ),
            (
                &pick,
                BarMode::NewTab,
                None,
                Some(Choice::SetEngine("google".into())),
            ),
        ];
        for (target, mode, engine, want) in cases {
            assert_eq!(
                target_command_as(target, mode, engine, Disposition::AsOpened),
                want,
                "{target:?} {mode:?} {engine:?}"
            );
        }
    }

    #[test]
    fn only_loads_from_the_bar_are_typed() {
        assert!(is_typed(&RowTarget::Omni(Target::Url("u".into()))));
        assert!(is_typed(&RowTarget::Omni(Target::Search("q".into()))));
        assert!(!is_typed(&RowTarget::Omni(Target::OpenTab(TabId(1)))));
        assert!(!is_typed(&RowTarget::Omni(Target::Action(Action::Reload))));
        assert!(!is_typed(&RowTarget::PickEngine("google".into())));
    }
}
