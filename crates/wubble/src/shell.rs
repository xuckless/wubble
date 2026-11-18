//! What the sidebar shows, derived from the model in one pure function.
//!
//! [`view_model`] runs after every change; `bind.rs` hands its result to
//! Slint wholesale. Nothing patches the sidebar row by row.

use wubble_core::model::{Dest, Model};
use wubble_core::space::{Item, Rgb, SpaceId};
use wubble_core::tab::{Tab, TabId};
use wubble_core::url;

/// One tab row or pinned tile.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TabRow {
    pub id: i32,
    pub title: String,
    pub url: String,
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

pub fn view_model(model: &Model) -> ShellView {
    let space = model.spaces.iter().find(|s| s.id == model.active_space);
    let rows = |ids: &mut dyn Iterator<Item = TabId>| -> Vec<TabRow> {
        ids.filter_map(|id| {
            let tab = model.tabs.get(&id)?;
            Some(TabRow {
                id: slint_id(id.0)?,
                title: tab_title(tab),
                url: tab.url.clone(),
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

/// Where a row dragged to slot `to` of the flat tab list ([`ShellView::tabs`]: the active
/// Space's items with folder tabs inline) goes in the model. Dragged up, it goes before the
/// row now at `to`; dragged down, after the row it passed (at `to` once it is taken out); in
/// that row's folder if it has one, so the list reads as dropped. The very last slot is the
/// end of the Space at the top level, out of any folder (a trailing folder can still be joined
/// before its last tab). `None` for a tab that is not a row (pinned, another Space's, unknown).
pub fn move_dest(model: &Model, tab: TabId, to: usize) -> Option<Dest> {
    let space = model.spaces.iter().find(|s| s.id == model.active_space)?;
    // The other rows in flat order, as (before it, after it), indices as after the detach.
    let mut rows = vec![];
    let mut from = None;
    let mut items = 0;
    let normal = |index| Dest::Normal {
        space: space.id,
        index,
    };
    for item in &space.items {
        match item {
            Item::Tab(t) if *t == tab => from = Some(rows.len()),
            Item::Tab(_) => {
                rows.push((normal(items), normal(items + 1)));
                items += 1;
            }
            Item::Folder(f) => {
                for (index, _) in f.tabs.iter().filter(|&&t| t != tab).enumerate() {
                    let at = |index| Dest::Folder {
                        folder: f.id,
                        index,
                    };
                    rows.push((at(index), at(index + 1)));
                }
                if let Some(i) = f.tabs.iter().position(|&t| t == tab) {
                    from = Some(rows.len() - (f.tabs.len() - 1 - i));
                }
                items += 1;
            }
        }
    }
    let from = from?;
    let row = if to > from {
        rows.get(to - 1).map(|r| r.1)
    } else {
        rows.get(to).map(|r| r.0)
    };
    // `rows` lacks the dragged one: slot `rows.len()` is the last
    Some(
        row.filter(|_| to < rows.len())
            .unwrap_or(normal(usize::MAX)),
    )
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
        let v = view_model(&Model::new(std::time::Instant::now()));
        assert_eq!((v.pinned.len(), v.tabs.len()), (0, 0));
        assert_eq!(v.active_tab, -1);
        assert_eq!(v.active_space, 0);
        assert_eq!(v.spaces.len(), 1);
        assert_eq!((v.host.as_str(), v.path.as_str()), ("", ""));
    }

    #[test]
    fn pinned_then_normal_then_folder_tabs_in_order() {
        let (m, a) = new_tab(Model::new(std::time::Instant::now()), "https://a.test/");
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
        let (m, _) = new_tab(Model::new(std::time::Instant::now()), "https://a.test/");
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
        assert_eq!(v.tabs[0].url, "https://w.test/");
        assert_eq!(v.spaces[1].tint, Rgb(1, 2, 3));
        assert_eq!(v.active_space, 1);
        let names: Vec<_> = v.spaces.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["Space 1", "Work"]);
    }

    #[test]
    fn titles_fall_back_to_host_then_new_tab() {
        let (m, t) = new_tab(
            Model::new(std::time::Instant::now()),
            "https://example.com/x",
        );
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

    /// Tabs a, b, c, then a folder holding d and e; p pinned.
    fn with_folder() -> (Model, [TabId; 6], wubble_core::space::FolderId) {
        let mut m = Model::new(std::time::Instant::now());
        let mut ids = vec![];
        for url in ["a", "b", "c", "d", "e", "p"] {
            let (next, t) = new_tab(m, &format!("https://{url}.test/"));
            m = next;
            ids.push(t);
        }
        let [a, b, c, d, e, p] = ids[..] else {
            unreachable!()
        };
        let (m, _) = apply(
            m,
            Command::NewFolder {
                space: SpaceId(0),
                name: "f".into(),
            },
        );
        let folder = m.spaces[0]
            .items
            .iter()
            .find_map(|i| match i {
                Item::Folder(f) => Some(f.id),
                Item::Tab(_) => None,
            })
            .expect("folder");
        let mut m = m;
        for (i, t) in [d, e].into_iter().enumerate() {
            m = apply(
                m,
                Command::MoveTab {
                    tab: t,
                    to: Dest::Folder { folder, index: i },
                },
            )
            .0;
        }
        let (m, _) = apply(m, Command::PinTab(p));
        let v = view_model(&m);
        assert_eq!(
            titles(&v.tabs),
            ["a.test", "b.test", "c.test", "d.test", "e.test"]
        );
        (m, [a, b, c, d, e, p], folder)
    }

    fn moved(m: Model, tab: TabId, to: usize) -> Vec<String> {
        let dest = move_dest(&m, tab, to).expect("a row");
        let (m, _) = apply(m, Command::MoveTab { tab, to: dest });
        view_model(&m).tabs.into_iter().map(|r| r.title).collect()
    }

    #[test]
    fn a_dragged_row_lands_on_its_slot_of_the_flat_list() {
        let (m, [a, b, c, d, e, _], f) = with_folder();
        let s = SpaceId(0);
        // a down to slot 2: b c a d e
        assert_eq!(
            move_dest(&m, a, 2),
            Some(Dest::Normal { space: s, index: 2 })
        );
        assert_eq!(
            moved(m.clone(), a, 2),
            ["b.test", "c.test", "a.test", "d.test", "e.test"]
        );
        // c up to the top
        assert_eq!(
            moved(m.clone(), c, 0),
            ["c.test", "a.test", "b.test", "d.test", "e.test"]
        );
        // a to the last slot (the last `TabRows` reports): after the trailing folder, at the
        // top level
        let last = Some(Dest::Normal {
            space: s,
            index: usize::MAX,
        });
        assert_eq!(move_dest(&m, a, 4), last);
        assert_eq!(
            moved(m.clone(), a, 4),
            ["b.test", "c.test", "d.test", "e.test", "a.test"]
        );
        // d out of the trailing folder the same way
        assert_eq!(move_dest(&m, d, 4), last);
        // a into the trailing folder: before its last tab
        assert_eq!(
            move_dest(&m, a, 3),
            Some(Dest::Folder {
                folder: f,
                index: 1
            })
        );
        // e within its folder, before d
        assert_eq!(
            moved(m.clone(), e, 3),
            ["a.test", "b.test", "c.test", "e.test", "d.test"]
        );
        // d out of the folder, up between a and b
        assert_eq!(
            moved(m.clone(), d, 1),
            ["a.test", "d.test", "b.test", "c.test", "e.test"]
        );
        // b down to where e is: into the folder before e
        assert_eq!(
            moved(m.clone(), b, 3),
            ["a.test", "c.test", "d.test", "b.test", "e.test"]
        );
    }

    #[test]
    fn pinned_and_unknown_tabs_are_not_rows() {
        let (m, [.., p], _) = with_folder();
        assert_eq!(move_dest(&m, p, 0), None);
        assert_eq!(move_dest(&m, TabId(999), 0), None);
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
