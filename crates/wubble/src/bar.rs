//! The command bar's rows and what choosing one does, as pure functions over
//! the model.
//!
//! Slint's bar fires `result-chosen` only for an existing row, so a
//! non-empty query always gets a first "go to" or "search" row: Enter on
//! free text chooses it.

use wubble_core::model::{Command, Model};
use wubble_core::tab::TabId;
use wubble_core::url;
use wubble_ui::CommandKind;

use crate::shell::tab_title;

/// Search engine for text that is not an address: its name and query URL.
/// Fixed until the first-run choice exists (TODO.md "Search").
const SEARCH_NAME: &str = "DuckDuckGo";
const SEARCH_URL: &str = "https://duckduckgo.com/?q=";

/// What Enter on the query does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BarMode {
    /// Ctrl+T: open a new tab.
    NewTab,
    /// Ctrl+L: load in this tab.
    Navigate(TabId),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BarAction {
    OpenUrl(String),
    Navigate { tab: TabId, url: String },
    Activate(TabId),
}

#[derive(Clone, Debug, PartialEq)]
pub struct BarRow {
    pub kind: CommandKind,
    pub title: String,
    pub subtitle: String,
    pub action: BarAction,
}

/// What typed text stands for. Both carry the URL to load.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Resolved {
    Address(String),
    Search(String),
}

impl Resolved {
    pub fn url(&self) -> &str {
        match self {
            Self::Address(url) | Self::Search(url) => url,
        }
    }
}

/// The one input classifier (a minimal stand-in until the core lane's real
/// one): text with a scheme, a dot or `localhost` and no spaces is an
/// address, anything else is searched.
pub fn resolve(input: &str) -> Resolved {
    let text = input.trim();
    let as_url = url::from_cli_arg(text);
    let has_scheme = as_url == text;
    let lower = text.to_ascii_lowercase();
    let host_like = text.contains('.')
        || lower == "localhost"
        || lower.starts_with("localhost:")
        || lower.starts_with("localhost/");
    if !text.is_empty() && !text.contains(char::is_whitespace) && (has_scheme || host_like) {
        Resolved::Address(as_url)
    } else {
        let mut search = String::from(SEARCH_URL);
        url::percent_encode_into(text, &mut search);
        Resolved::Search(search)
    }
}

/// The rows for `query`: the go/search row, then the open tabs whose title
/// or URL contains it (any case), active Space first. Nothing for a blank query.
pub fn bar_rows(model: &Model, query: &str, mode: BarMode) -> Vec<BarRow> {
    let text = query.trim();
    if text.is_empty() {
        return vec![];
    }
    let resolved = resolve(text);
    let action = match mode {
        BarMode::NewTab => BarAction::OpenUrl(resolved.url().into()),
        BarMode::Navigate(tab) => BarAction::Navigate {
            tab,
            url: resolved.url().into(),
        },
    };
    let first = match &resolved {
        Resolved::Address(url) => BarRow {
            kind: CommandKind::Action,
            title: url.clone(),
            subtitle: "Go to address".into(),
            action,
        },
        Resolved::Search(_) => BarRow {
            kind: CommandKind::Search,
            title: text.into(),
            subtitle: format!("Search {SEARCH_NAME}"),
            action,
        },
    };
    let needle = text.to_lowercase();
    let mut spaces: Vec<_> = model.spaces.iter().map(|s| s.id).collect();
    spaces.sort_by_key(|&s| s != model.active_space);
    let tabs = spaces
        .into_iter()
        .flat_map(|s| model.order(s))
        .filter_map(|id| model.tabs.get(&id))
        .filter(|t| {
            t.title.to_lowercase().contains(&needle) || t.url.to_lowercase().contains(&needle)
        })
        .map(|t| BarRow {
            kind: CommandKind::OpenTab,
            title: tab_title(t),
            subtitle: url::address_parts(&t.url).0.into(),
            action: BarAction::Activate(t.id),
        });
    std::iter::once(first).chain(tabs).collect()
}

/// The model command a chosen row stands for.
pub fn action_command(action: BarAction) -> Command {
    match action {
        BarAction::OpenUrl(url) => Command::NewTab {
            url,
            container: None,
            opener: None,
        },
        BarAction::Navigate { tab, url } => Command::Navigate { tab, url },
        BarAction::Activate(tab) => Command::ActivateTab(tab),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wubble_core::model::apply;
    use wubble_core::space::{ContainerId, Rgb, SpaceId};

    fn address(url: &str) -> Resolved {
        Resolved::Address(url.into())
    }

    fn search(encoded: &str) -> Resolved {
        Resolved::Search(format!("{SEARCH_URL}{encoded}"))
    }

    #[test]
    fn resolve_tells_addresses_from_searches() {
        let cases = [
            ("example.com", address("https://example.com")),
            (
                "  example.com/a?b=1  ",
                address("https://example.com/a?b=1"),
            ),
            ("https://x", address("https://x")),
            ("about:blank", address("about:blank")),
            ("localhost:3000", address("https://localhost:3000")),
            ("localhost", address("https://localhost")),
            ("rust lifetimes", search("rust%20lifetimes")),
            ("hello", search("hello")),
            ("what is example.com", search("what%20is%20example.com")),
            ("über", search("%C3%BCber")),
            ("a?&#b", search("a%3F%26%23b")),
        ];
        for (input, want) in cases {
            assert_eq!(resolve(input), want, "{input}");
        }
    }

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

    fn actions(rows: &[BarRow]) -> Vec<BarAction> {
        rows.iter().map(|r| r.action.clone()).collect()
    }

    #[test]
    fn a_blank_query_has_no_rows() {
        let (m, _) = tab(Model::new(), "https://a.test/", "A");
        assert_eq!(bar_rows(&m, "", BarMode::NewTab), vec![]);
        assert_eq!(bar_rows(&m, "   ", BarMode::NewTab), vec![]);
    }

    #[test]
    fn the_first_row_opens_or_navigates() {
        let (m, t) = tab(Model::new(), "https://a.test/", "A");
        let rows = bar_rows(&m, "example.com", BarMode::NewTab);
        assert_eq!(rows[0].kind, CommandKind::Action);
        assert_eq!(
            rows[0].action,
            BarAction::OpenUrl("https://example.com".into())
        );
        let rows = bar_rows(&m, "cats", BarMode::Navigate(t));
        assert_eq!(rows[0].kind, CommandKind::Search);
        assert_eq!(rows[0].title, "cats");
        assert_eq!(
            rows[0].action,
            BarAction::Navigate {
                tab: t,
                url: format!("{SEARCH_URL}cats"),
            }
        );
    }

    #[test]
    fn open_tabs_match_title_or_url_active_space_first() {
        let (m, a) = tab(Model::new(), "https://mail.test/", "Inbox");
        let (m, b) = tab(m, "https://docs.test/", "Docs about MAIL");
        let (m, _) = tab(m, "https://other.test/", "Other");
        let (m, _) = apply(
            m,
            Command::NewSpace {
                name: "W".into(),
                tint: Rgb(0, 0, 0),
                container: ContainerId::DEFAULT,
            },
        );
        let (m, _) = apply(m, Command::SwitchSpace(SpaceId(1)));
        let (m, w) = tab(m, "https://w.test/mail", "W");
        let rows = bar_rows(&m, "Mail", BarMode::NewTab);
        assert_eq!(
            actions(&rows[1..]),
            vec![
                BarAction::Activate(w),
                BarAction::Activate(a),
                BarAction::Activate(b),
            ]
        );
        assert_eq!(rows[1].kind, CommandKind::OpenTab);
        assert_eq!(rows[2].title, "Inbox");
        assert_eq!(rows[2].subtitle, "mail.test");
    }

    #[test]
    fn actions_map_to_model_commands() {
        let t = TabId(4);
        assert_eq!(
            action_command(BarAction::OpenUrl("u".into())),
            Command::NewTab {
                url: "u".into(),
                container: None,
                opener: None,
            }
        );
        assert_eq!(
            action_command(BarAction::Navigate {
                tab: t,
                url: "u".into()
            }),
            Command::Navigate {
                tab: t,
                url: "u".into()
            }
        );
        assert_eq!(
            action_command(BarAction::Activate(t)),
            Command::ActivateTab(t)
        );
    }
}
