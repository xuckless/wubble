//! What the sidebar shows, derived from the model in one pure function.
//!
//! [`view_model`] runs after every change; `bind.rs` hands its result to
//! Slint wholesale. Nothing patches the sidebar row by row.

use wubble_core::model::Model;
use wubble_core::space::{Item, Rgb, SpaceId};
use wubble_core::tab::{Tab, TabId};
use wubble_core::url;

/// One tab row or pinned tile.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TabRow {
    pub id: i32,
    pub title: String,
    pub glyph: String,
    pub tint: Rgb,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpaceRow {
    pub id: i32,
    pub name: String,
    pub tint: Rgb,
}

/// The sidebar for one model. Ids that do not fit Slint's `int` are
/// skipped (TODO.md "Slint ids"); `-1` means none.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShellView {
    pub pinned: Vec<TabRow>,
    /// The active Space's other tabs, folder tabs inline (no folder UI yet).
    pub tabs: Vec<TabRow>,
    pub spaces: Vec<SpaceRow>,
    pub active_tab: i32,
    pub active_space: i32,
    /// The address pill: the visible tab's host and the rest of its URL.
    pub host: String,
    pub path: String,
}

/// Placeholder glyph for a tab without a host to take a letter from.
const NO_GLYPH: &str = "•";

pub fn view_model(model: &Model) -> ShellView {
    let space = model.spaces.iter().find(|s| s.id == model.active_space);
    let tint = space.map_or(Rgb(0, 0, 0), |s| s.tint);
    let rows = |ids: &mut dyn Iterator<Item = TabId>| -> Vec<TabRow> {
        ids.filter_map(|id| {
            let tab = model.tabs.get(&id)?;
            Some(TabRow {
                id: slint_id(id.0)?,
                title: tab_title(tab),
                glyph: glyph(&tab.url),
                tint,
            })
        })
        .collect()
    };
    let (pinned, tabs) = match space {
        Some(s) => (
            rows(&mut s.pinned.iter().copied()),
            rows(&mut s.items.iter().flat_map(|item| match item {
                Item::Tab(t) => vec![*t],
                Item::Folder(f) => f.tabs.clone(),
            })),
        ),
        None => (vec![], vec![]),
    };
    let spaces = model
        .spaces
        .iter()
        .filter_map(|s| {
            Some(SpaceRow {
                id: slint_id(s.id.0)?,
                name: s.name.clone(),
                tint: s.tint,
            })
        })
        .collect();
    let visible = model.visible_tab();
    let address = visible
        .and_then(|t| model.tabs.get(&t))
        .map_or("", |t| t.url.as_str());
    let (host, path) = url::address_parts(address);
    ShellView {
        pinned,
        tabs,
        spaces,
        active_tab: visible.and_then(|t| slint_id(t.0)).unwrap_or(-1),
        active_space: slint_id(model.active_space.0).unwrap_or(-1),
        host: host.into(),
        path: path.into(),
    }
}

/// What a tab is called: its title, else its host, else "New tab".
pub fn tab_title(tab: &Tab) -> String {
    let host = url::address_parts(&tab.url).0;
    [tab.title.trim(), host]
        .into_iter()
        .find(|s| !s.is_empty())
        .unwrap_or("New tab")
        .to_string()
}

/// The host's first letter or digit, upper-cased.
fn glyph(address: &str) -> String {
    url::address_parts(address)
        .0
        .chars()
        .find(|c| c.is_alphanumeric())
        .map_or_else(|| NO_GLYPH.into(), |c| c.to_uppercase().collect())
}

fn slint_id(id: impl TryInto<i32>) -> Option<i32> {
    id.try_into().ok()
}

/// A tab id from Slint; negative ids name nothing.
pub fn tab_id(id: i32) -> Option<TabId> {
    u64::try_from(id).ok().map(TabId)
}

pub fn space_id(id: i32) -> Option<SpaceId> {
    u32::try_from(id).ok().map(SpaceId)
}

#[cfg(test)]
mod tests {
    use super::*;
    use wubble_core::model::{Command, Dest, apply};
    use wubble_core::space::ContainerId;

    fn new_tab(m: Model, url: &str) -> (Model, TabId) {
        let (m, fx) = apply(
            m,
            Command::NewTab {
                url: url.into(),
                container: None,
                opener: None,
            },
        );
        let tab = m.visible_tab().expect("new tab is visible");
        assert!(!fx.is_empty());
        (m, tab)
    }

    fn titles(rows: &[TabRow]) -> Vec<&str> {
        rows.iter().map(|r| r.title.as_str()).collect()
    }

    #[test]
    fn an_empty_model_shows_nothing_active() {
        let v = view_model(&Model::new());
        assert_eq!((v.pinned.len(), v.tabs.len()), (0, 0));
        assert_eq!(v.active_tab, -1);
        assert_eq!(v.active_space, 0);
        assert_eq!(v.spaces.len(), 1);
        assert_eq!((v.host.as_str(), v.path.as_str()), ("", ""));
    }

    #[test]
    fn pinned_then_normal_then_folder_tabs_in_order() {
        let (m, a) = new_tab(Model::new(), "https://a.test/");
        let (m, b) = new_tab(m, "https://b.test/");
        let (m, c) = new_tab(m, "https://c.test/");
        let (m, _) = apply(
            m,
            Command::NewFolder {
                space: SpaceId(0),
                name: "f".into(),
            },
        );
        let (m, _) = apply(m, Command::PinTab(a));
        let folder = m.spaces[0]
            .items
            .iter()
            .find_map(|i| match i {
                Item::Folder(f) => Some(f.id),
                Item::Tab(_) => None,
            })
            .expect("folder");
        let (m, _) = apply(
            m,
            Command::MoveTab {
                tab: b,
                to: Dest::Folder { folder, index: 0 },
            },
        );
        let v = view_model(&m);
        assert_eq!(titles(&v.pinned), ["a.test"]);
        assert_eq!(titles(&v.tabs), ["c.test", "b.test"]);
        assert_eq!(v.active_tab, i32::try_from(c.0).unwrap());
        assert_eq!((v.host.as_str(), v.path.as_str()), ("c.test", "/"));
    }

    #[test]
    fn only_the_active_space_is_listed() {
        let (m, _) = new_tab(Model::new(), "https://a.test/");
        let (m, _) = apply(
            m,
            Command::NewSpace {
                name: "Work".into(),
                tint: Rgb(1, 2, 3),
                container: ContainerId::DEFAULT,
            },
        );
        let (m, _) = apply(m, Command::SwitchSpace(SpaceId(1)));
        let (m, _) = new_tab(m, "https://w.test/");
        let v = view_model(&m);
        assert_eq!(titles(&v.tabs), ["w.test"]);
        assert_eq!(v.tabs[0].tint, Rgb(1, 2, 3));
        assert_eq!(v.active_space, 1);
        let names: Vec<_> = v.spaces.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["Space 1", "Work"]);
    }

    #[test]
    fn titles_fall_back_to_host_then_new_tab() {
        let (m, t) = new_tab(Model::new(), "https://example.com/x");
        assert_eq!(view_model(&m).tabs[0].title, "example.com");
        let (m, _) = apply(
            m,
            Command::TitleChanged {
                tab: t,
                title: "Example".into(),
            },
        );
        assert_eq!(view_model(&m).tabs[0].title, "Example");
        let (m, _) = new_tab(m, "");
        assert_eq!(view_model(&m).tabs[1].title, "New tab");
    }

    #[test]
    fn glyphs_come_from_the_host() {
        assert_eq!(glyph("https://example.com/"), "E");
        assert_eq!(glyph("https://www.x.test/"), "W");
        assert_eq!(glyph("data:text/html,x"), "D");
        assert_eq!(glyph("https://ünï.test/"), "Ü");
        assert_eq!(glyph(""), NO_GLYPH);
        assert_eq!(glyph("file:///tmp/x"), NO_GLYPH);
    }

    #[test]
    fn ids_that_do_not_fit_are_skipped() {
        assert_eq!(slint_id(u64::from(u32::MAX)), None);
        assert_eq!(slint_id(7u64), Some(7));
        assert_eq!(slint_id(i32::MAX as u32), Some(i32::MAX));
    }

    #[test]
    fn slint_ids_back_to_core_ids() {
        assert_eq!(tab_id(3), Some(TabId(3)));
        assert_eq!(tab_id(-1), None);
        assert_eq!(space_id(0), Some(SpaceId(0)));
        assert_eq!(space_id(-5), None);
    }
}
