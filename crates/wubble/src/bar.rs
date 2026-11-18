//! The command bar's rows and what choosing one does, as pure functions over
//! `wubble_core::omnibox` (classify and rank) and the keymap.
//!
//! Slint's bar fires `result-chosen` only for an existing row; `omnibox::rank`
//! always puts the classified intent ("open" or "search") first, so Enter on
//! free text chooses it.

use wubble_core::action::Action;
use wubble_core::keys::Keymap;
use wubble_core::model::{Command, Model};
use wubble_core::omnibox::{
    self, Bookmark, Candidates, HistoryEntry, Kind, OmniResult, OpenTab, SearchEngine, Target,
};
use wubble_core::tab::TabId;
use wubble_ui::CommandKind;

use crate::shell::tab_title;

/// Rows the bar shows at most.
pub const MAX_ROWS: usize = 12;
/// History hits asked of the store per query.
pub const HISTORY_HITS: usize = 20;

/// What Enter on the query does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BarMode {
    /// Ctrl+T: open a new tab.
    NewTab,
    /// Ctrl+L: load in this tab.
    Navigate(TabId),
    /// Split view: the page chosen goes beside the focused one.
    Beside,
}

/// What a row stands for: an omnibox result, or (first run only) a search
/// engine to choose. The engine choice is the bar's own row, not a global
/// `Action`: it exists only until a choice is stored.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RowTarget {
    Omni(Target),
    PickEngine(SearchEngine),
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
    /// `None` until the first-run choice.
    pub engine: Option<SearchEngine>,
    pub keymap: &'a Keymap,
    /// Live engine suggestions, already fetched (`suggest_fx`).
    pub suggestions: &'a [String],
    pub now_ms: i64,
}

/// What choosing a row does.
#[derive(Clone, Debug, PartialEq)]
pub enum Choice {
    Model(Command),
    Perform(Action),
    /// Store the search engine (first run).
    SetEngine(SearchEngine),
}

/// The open tabs, active Space first, each Space in sidebar order.
pub fn open_tabs(model: &Model) -> Vec<OpenTab> {
    let mut spaces: Vec<_> = model.spaces.iter().map(|s| s.id).collect();
    spaces.sort_by_key(|&s| s != model.active_space);
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

fn kind(k: Kind) -> CommandKind {
    match k {
        Kind::OpenTab => CommandKind::OpenTab,
        Kind::History => CommandKind::History,
        Kind::Bookmark => CommandKind::Bookmark,
        Kind::Search => CommandKind::Search,
        Kind::Action => CommandKind::Action,
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

/// The first-run rows: one per engine in `omnibox::ENGINES`.
fn engine_rows() -> impl Iterator<Item = BarRow> {
    omnibox::ENGINES.iter().map(|e| BarRow {
        kind: CommandKind::Action,
        title: format!("Search with {}", e.name),
        subtitle: "Choose the search engine".into(),
        shortcut: String::new(),
        target: RowTarget::PickEngine(e.engine),
    })
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
    };
    // Before the choice the engine only names the search rows, which are
    // replaced below.
    let ranking_engine = inputs.engine.unwrap_or(SearchEngine::DuckDuckGo);
    let ranked = omnibox::rank(query, ranking_engine, &cands, inputs.now_ms, MAX_ROWS);
    let rows = ranked.into_iter().map(|r| omni_row(r, inputs.keymap));
    if inputs.engine.is_some() {
        return rows.collect();
    }
    let mut rows: Vec<BarRow> = rows
        .filter(|r| !matches!(r.target, RowTarget::Omni(Target::Search(_))))
        .collect();
    let at = usize::from(matches!(omnibox::classify(query), omnibox::Intent::Url(_)));
    let at = at.min(rows.len());
    rows.splice(at..at, engine_rows());
    rows.truncate(MAX_ROWS);
    rows
}

/// What choosing `target` in a bar opened in `mode` does. A search needs an
/// engine: without one it is `None` (the first-run rows replace it).
pub fn target_command(
    target: &RowTarget,
    mode: BarMode,
    engine: Option<SearchEngine>,
) -> Option<Choice> {
    let load = |url: String| match mode {
        BarMode::NewTab => Command::NewTab {
            url,
            container: None,
            opener: None,
        },
        BarMode::Navigate(tab) => Command::Navigate { tab, url },
        BarMode::Beside => Command::OpenBeside { url },
    };
    Some(match target {
        RowTarget::Omni(Target::Url(url)) => Choice::Model(load(url.clone())),
        RowTarget::Omni(Target::Search(q)) => {
            Choice::Model(load(omnibox::fill(engine?.info().search, q)))
        }
        RowTarget::Omni(Target::OpenTab(tab)) => Choice::Model(match mode {
            BarMode::Beside => Command::SplitWith(*tab),
            _ => Command::ActivateTab(*tab),
        }),
        RowTarget::Omni(Target::Action(a)) => Choice::Perform(*a),
        RowTarget::PickEngine(e) => Choice::SetEngine(*e),
    })
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

    fn rows(engine: Option<SearchEngine>, tabs: &[OpenTab], query: &str) -> Vec<BarRow> {
        let keymap = Keymap::defaults(Platform::Linux);
        bar_rows(
            &BarInputs {
                tabs,
                history: &[],
                bookmarks: &[],
                engine,
                keymap: &keymap,
                suggestions: &[],
                now_ms: 0,
            },
            query,
        )
    }

    fn targets(rows: &[BarRow]) -> Vec<RowTarget> {
        rows.iter().map(|r| r.target.clone()).collect()
    }

    const DDG: Option<SearchEngine> = Some(SearchEngine::DuckDuckGo);

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
    fn the_first_run_offers_the_two_engines() {
        let picks = |r: &[BarRow]| -> Vec<RowTarget> {
            r.iter()
                .filter(|r| matches!(r.target, RowTarget::PickEngine(_)))
                .map(|r| r.target.clone())
                .collect()
        };
        let both = vec![
            RowTarget::PickEngine(SearchEngine::Google),
            RowTarget::PickEngine(SearchEngine::DuckDuckGo),
        ];
        // A blank query and a search: the choice comes first, no search row.
        assert_eq!(targets(&rows(None, &[], "")), both);
        let r = rows(None, &[], "cats");
        assert_eq!(targets(&r[..2]), both);
        assert!(
            !targets(&r).contains(&RowTarget::Omni(Target::Search("cats".into()))),
            "{r:?}"
        );
        assert!(r.iter().all(|r| r.kind != CommandKind::Search));
        // An address still opens first.
        let r = rows(None, &[], "example.com");
        assert_eq!(
            r[0].target,
            RowTarget::Omni(Target::Url("https://example.com/".into()))
        );
        assert_eq!(targets(&r[1..3]), both);
        // Once chosen, never again.
        for q in ["", "cats", "example.com"] {
            assert_eq!(picks(&rows(DDG, &[], q)), vec![], "{q}");
            assert_eq!(picks(&rows(Some(SearchEngine::Google), &[], q)), vec![]);
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
                engine: DDG,
                keymap: &keymap,
                suggestions: &["cats and dogs".into()],
                now_ms: 0,
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
                engine: DDG,
                keymap: &keymap,
                suggestions: &[],
                now_ms: 0,
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
                engine: DDG,
                keymap: &keymap,
                suggestions: &[],
                now_ms: 0,
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
        let google = Some(SearchEngine::Google);
        let cases = [
            (&url, BarMode::NewTab, DDG, Some(new_tab("https://a.test/"))),
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
                DDG,
                Some(navigate("https://duckduckgo.com/?q=a%20b")),
            ),
            (&search, BarMode::NewTab, None, None),
            (
                &RowTarget::Omni(Target::OpenTab(TabId(9))),
                BarMode::Navigate(t),
                DDG,
                Some(Choice::Model(Command::ActivateTab(TabId(9)))),
            ),
            (
                &RowTarget::Omni(Target::Action(Action::ReopenClosedTab)),
                BarMode::NewTab,
                DDG,
                Some(Choice::Perform(Action::ReopenClosedTab)),
            ),
            (
                &url,
                BarMode::Beside,
                DDG,
                Some(Choice::Model(Command::OpenBeside {
                    url: "https://a.test/".into(),
                })),
            ),
            (
                &RowTarget::Omni(Target::OpenTab(TabId(9))),
                BarMode::Beside,
                DDG,
                Some(Choice::Model(Command::SplitWith(TabId(9)))),
            ),
            (
                &RowTarget::PickEngine(SearchEngine::Google),
                BarMode::NewTab,
                None,
                Some(Choice::SetEngine(SearchEngine::Google)),
            ),
        ];
        for (target, mode, engine, want) in cases {
            assert_eq!(
                target_command(target, mode, engine),
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
        assert!(!is_typed(&RowTarget::PickEngine(SearchEngine::Google)));
    }
}
