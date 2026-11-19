//! What the sidebar shows, derived from the model in one pure function.
//!
//! [`view_model`] runs after every change; `bind.rs` hands its result to
//! Slint wholesale. Nothing patches the sidebar row by row.

use std::collections::BTreeMap;

use wubble_core::model::{Dest, Model};
use wubble_core::pages::InternalPage;
use wubble_core::space::{FolderId, Item, Rgb, SpaceIcon, SpaceId};
use wubble_core::tab::{Tab, TabId};
use wubble_core::url;
use wubble_core::window::WindowId;

/// What a sidebar row is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RowKind {
    #[default]
    Tab,
    /// A folder's header (its id is [`folder_row_id`]): the chevron, the name, the count.
    Folder,
}

/// One tab row or pinned tile, or a folder's header.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TabRow {
    pub kind: RowKind,
    /// The tab's id, or a folder header's negative id.
    pub id: i32,
    /// 1 for a tab in a folder.
    pub depth: u8,
    /// A folder header: how many tabs it holds, and whether they are folded away.
    pub count: u32,
    pub collapsed: bool,
    pub title: String,
    pub url: String,
    pub audible: bool,
    pub muted: bool,
    /// Requests the content blocker stopped since the page's main-frame request; 0 for a tab
    /// without a view. Data for the shields badge, which the shell does not draw yet.
    pub blocked: u32,
    /// The tab is a pane of its Space's split, shown or not (the design's `split?.includes`).
    pub in_split: bool,
}

/// Whether a tab's page is muted, as its view last reported (`PageEvent::MutedChanged`). Gone
/// with the view. Whether it plays sound is a busy reason in the model (`Reason::Audible`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Audio {
    pub muted: bool,
}

pub type AudioMap = BTreeMap<TabId, Audio>;

/// Requests blocked per live tab (`Blocking::blocked` of its view).
pub type BlockedMap = BTreeMap<TabId, u32>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpaceRow {
    pub id: i32,
    pub name: String,
    pub tint: Rgb,
    pub icon: SpaceIcon,
}

/// The sidebar for one model. Ids that do not fit Slint's `int` are
/// skipped (TODO.md "Slint ids"); `-1` means none.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShellView {
    pub pinned: Vec<TabRow>,
    /// The active Space's other rows in order: tabs, and each folder's header followed by its
    /// tabs (one level in) unless it is collapsed.
    pub tabs: Vec<TabRow>,
    pub spaces: Vec<SpaceRow>,
    pub active_tab: i32,
    pub active_space: i32,
    /// The address pill: the visible tab's host and the rest of its URL.
    pub host: String,
    pub path: String,
    /// The pill's lock (a site over https) or its bubble.
    pub secure: bool,
}

/// The shell of `window`: its Space's rows (the active Space for a Little or Popup window, whose
/// sidebar is hidden) and its page's address.
pub fn view_model(
    model: &Model,
    window: WindowId,
    audio: &AudioMap,
    blocked: &BlockedMap,
) -> ShellView {
    let space_id = model
        .window(window)
        .and_then(|w| w.space())
        .unwrap_or_else(|| model.active_space());
    let space = model.spaces.iter().find(|s| s.id == space_id);
    // a peek's tab is no row: the card over the page shows it
    let rows = |ids: &mut dyn Iterator<Item = TabId>| -> Vec<TabRow> {
        ids.filter(|&id| !model.is_glance(id))
            .filter_map(|id| {
                let tab = model.tabs.get(&id)?;
                let sound = audio.get(&id).copied().unwrap_or_default();
                Some(TabRow {
                    id: slint_id(id.0)?,
                    title: tab_title(tab),
                    url: tab.url.clone(),
                    audible: tab.busy.contains(wubble_core::busy::Reason::Audible),
                    muted: sound.muted,
                    blocked: blocked.get(&id).copied().unwrap_or(0),
                    in_split: model.split_of(id).is_some(),
                    ..TabRow::default()
                })
            })
            .collect()
    };
    let (pinned, tabs) = match space {
        Some(s) => {
            let mut list = vec![];
            for item in &s.items {
                match item {
                    Item::Tab(t) => list.extend(rows(&mut std::iter::once(*t))),
                    Item::Folder(f) => {
                        list.push(TabRow {
                            kind: RowKind::Folder,
                            id: folder_row_id(f.id),
                            title: f.name.clone(),
                            count: u32::try_from(f.tabs.len()).unwrap_or(u32::MAX),
                            collapsed: f.collapsed,
                            ..TabRow::default()
                        });
                        if !f.collapsed {
                            list.extend(
                                rows(&mut f.tabs.iter().copied())
                                    .into_iter()
                                    .map(|r| TabRow { depth: 1, ..r }),
                            );
                        }
                    }
                }
            }
            (rows(&mut s.pinned.iter().copied()), list)
        }
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
                icon: s.icon,
            })
        })
        .collect();
    let visible = model.window_tab(window);
    let address = visible
        .and_then(|t| model.tabs.get(&t))
        .map_or("", |t| t.url.as_str());
    let (host, path) = wubble_core::chrome::address_parts(address);
    ShellView {
        pinned,
        tabs,
        spaces,
        active_tab: visible.and_then(|t| slint_id(t.0)).unwrap_or(-1),
        active_space: slint_id(space_id.0).unwrap_or(-1),
        host,
        path,
        secure: wubble_core::chrome::is_secure(address),
    }
}

/// Where a row dragged to slot `to` of the sidebar's rows ([`ShellView::tabs`]: the active
/// Space's items, each folder's header followed by its tabs unless collapsed) goes in the model.
/// Dragged up, it goes before the row now at `to`; dragged down, after the row it passed (at `to`
/// once it is taken out). Before a folder's header is the top level, above the folder; after an
/// open folder's header is the folder's first place; after a collapsed one, the top level below
/// it. The very last slot is the end of the Space at the top level, out of any folder. `None`
/// for a tab that is not a row (pinned, another Space's, folded away, unknown).
pub fn move_dest(model: &Model, tab: TabId, to: usize) -> Option<Dest> {
    // the row's own Space: the one the window it was dragged in shows
    let (space, _) = model.location(tab)?;
    let space = model.spaces.iter().find(|s| s.id == space)?;
    // The other rows in order, as (before it, after it), indices as after the detach.
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
            // a peek's tab is no row, but it holds its place among the items
            Item::Tab(t) if model.is_glance(*t) => items += 1,
            Item::Tab(_) => {
                rows.push((normal(items), normal(items + 1)));
                items += 1;
            }
            Item::Folder(f) => {
                let at = |index| Dest::Folder {
                    folder: f.id,
                    index,
                };
                if f.collapsed {
                    rows.push((normal(items), normal(items + 1)));
                } else {
                    rows.push((normal(items), at(0)));
                    let mut index = 0;
                    for &t in &f.tabs {
                        if t == tab {
                            from = Some(rows.len());
                            continue;
                        }
                        if !model.is_glance(t) {
                            rows.push((at(index), at(index + 1)));
                        }
                        index += 1;
                    }
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

/// A folder header's row id: negative, so it never names a tab (`-2 - id`; -1 means none).
pub fn folder_row_id(folder: FolderId) -> i32 {
    i32::try_from(folder.0).map_or(i32::MIN, |id| -2 - id)
}

/// The folder a header row id names.
pub fn folder_of_row(id: i32) -> Option<FolderId> {
    (id <= -2)
        .then(|| u32::try_from(-2_i64 - i64::from(id)).ok())
        .flatten()
        .map(FolderId)
}

/// What a tab is called: its title, else its host, else "New tab".
pub fn tab_title(tab: &Tab) -> String {
    if let Some(page) = InternalPage::from_url(&tab.url) {
        return page.title().to_owned();
    }
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

/// A tab's id in Slint (a pane's id is its tab's); -1 if it does not fit.
pub fn pane_id(tab: TabId) -> i32 {
    slint_id(tab.0).unwrap_or(-1)
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
        let v = view_model(
            &Model::new(std::time::Instant::now()),
            wubble_core::window::WindowId(0),
            &AudioMap::new(),
            &BlockedMap::new(),
        );
        assert_eq!((v.pinned.len(), v.tabs.len()), (0, 0));
        assert_eq!(v.active_tab, -1);
        assert_eq!(v.active_space, 0);
        assert_eq!(v.spaces.len(), 1);
        // no page: the design's start address (chrome.tsx `addressOf`)
        assert_eq!((v.host.as_str(), v.path.as_str()), ("wubble", "://start"));
        assert!(!v.secure);
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
        let v = view_model(&m, m.focused(), &AudioMap::new(), &BlockedMap::new());
        assert_eq!(titles(&v.pinned), ["a.test"]);
        assert_eq!(titles(&v.tabs), ["c.test", "f", "b.test"]);
        let kinds: Vec<_> = v.tabs.iter().map(|r| (r.kind, r.depth)).collect();
        assert_eq!(
            kinds,
            [(RowKind::Tab, 0), (RowKind::Folder, 0), (RowKind::Tab, 1)]
        );
        assert_eq!(v.tabs[1].id, folder_row_id(folder));
        assert_eq!(v.tabs[1].count, 1);
        assert_eq!(v.active_tab, i32::try_from(c.0).unwrap());
        // collapsed, the folder's tabs are folded away
        let (m, _) = apply(
            m,
            Command::SetFolderCollapsed {
                folder,
                collapsed: true,
            },
        );
        let v = view_model(&m, m.focused(), &AudioMap::new(), &BlockedMap::new());
        assert_eq!(titles(&v.tabs), ["c.test", "f"]);
        assert!(v.tabs[1].collapsed);
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
        let v = view_model(&m, m.focused(), &AudioMap::new(), &BlockedMap::new());
        assert_eq!(titles(&v.tabs), ["w.test"]);
        assert_eq!(v.tabs[0].url, "https://w.test/");
        assert_eq!(v.spaces[1].tint, Rgb(1, 2, 3));
        assert_eq!(v.active_space, 1);
        let names: Vec<_> = v.spaces.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["Space 1", "Work"]);
    }

    #[test]
    fn rows_show_split_panes_also_while_the_split_is_hidden() {
        let (m, a) = new_tab(Model::new(std::time::Instant::now()), "https://a.test/");
        let (m, b) = new_tab(m, "https://b.test/");
        let (m, c) = new_tab(m, "https://c.test/");
        let (m, _) = apply(m, Command::ActivateTab(a));
        let (m, _) = apply(m, Command::SplitWith(b));
        let split = |m: &Model| -> Vec<bool> {
            view_model(m, m.focused(), &AudioMap::new(), &BlockedMap::new())
                .tabs
                .iter()
                .map(|r| r.in_split)
                .collect()
        };
        assert_eq!(split(&m), [true, true, false]);
        let (m, _) = apply(m, Command::ActivateTab(c));
        assert_eq!(split(&m), [true, true, false]);
    }

    #[test]
    fn rows_carry_their_tab_s_sound() {
        let (m, a) = new_tab(Model::new(std::time::Instant::now()), "https://a.test/");
        let (m, b) = new_tab(m, "https://b.test/");
        let (m, _) = apply(
            m,
            Command::Busy {
                tab: a,
                reason: wubble_core::busy::Reason::Audible,
                on: true,
            },
        );
        let audio = AudioMap::from([(b, Audio { muted: true })]);
        let v = view_model(&m, m.focused(), &audio, &BlockedMap::new());
        let sound: Vec<_> = v.tabs.iter().map(|r| (r.audible, r.muted)).collect();
        assert_eq!(sound, [(true, false), (false, true)]);
        let empty = Model::new(std::time::Instant::now());
        let quiet = view_model(&empty, empty.focused(), &audio, &BlockedMap::new());
        assert!(quiet.tabs.is_empty());
        let unmuted = view_model(&m, m.focused(), &AudioMap::new(), &BlockedMap::new());
        assert!(unmuted.tabs.iter().all(|r| !r.muted));
    }

    #[test]
    fn rows_carry_their_tab_s_blocked_count() {
        let (m, a) = new_tab(Model::new(std::time::Instant::now()), "https://a.test/");
        let (m, _b) = new_tab(m, "https://b.test/");
        let v = view_model(
            &m,
            m.focused(),
            &AudioMap::new(),
            &BlockedMap::from([(a, 4)]),
        );
        let counts: Vec<_> = v.tabs.iter().map(|r| r.blocked).collect();
        assert_eq!(counts, [4, 0]);
    }

    #[test]
    fn titles_fall_back_to_host_then_new_tab() {
        let (m, t) = new_tab(
            Model::new(std::time::Instant::now()),
            "https://example.com/x",
        );
        assert_eq!(
            view_model(&m, m.focused(), &AudioMap::new(), &BlockedMap::new()).tabs[0].title,
            "example.com"
        );
        let (m, _) = apply(
            m,
            Command::TitleChanged {
                tab: t,
                title: "Example".into(),
            },
        );
        assert_eq!(
            view_model(&m, m.focused(), &AudioMap::new(), &BlockedMap::new()).tabs[0].title,
            "Example"
        );
        let (m, _) = new_tab(m, "");
        assert_eq!(
            view_model(&m, m.focused(), &AudioMap::new(), &BlockedMap::new()).tabs[1].title,
            "New tab"
        );
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
        let v = view_model(&m, m.focused(), &AudioMap::new(), &BlockedMap::new());
        assert_eq!(
            titles(&v.tabs),
            ["a.test", "b.test", "c.test", "f", "d.test", "e.test"]
        );
        (m, [a, b, c, d, e, p], folder)
    }

    fn moved(m: Model, tab: TabId, to: usize) -> Vec<String> {
        let dest = move_dest(&m, tab, to).expect("a row");
        let (m, _) = apply(m, Command::MoveTab { tab, to: dest });
        view_model(&m, m.focused(), &AudioMap::new(), &BlockedMap::new())
            .tabs
            .into_iter()
            .map(|r| r.title)
            .collect()
    }

    #[test]
    fn a_dragged_row_lands_on_its_slot_of_the_rows() {
        let (m, [a, b, c, d, e, _], f) = with_folder();
        let s = SpaceId(0);
        // a down to slot 2: b c a f d e
        assert_eq!(
            move_dest(&m, a, 2),
            Some(Dest::Normal { space: s, index: 2 })
        );
        assert_eq!(
            moved(m.clone(), a, 2),
            ["b.test", "c.test", "a.test", "f", "d.test", "e.test"]
        );
        // c up to the top
        assert_eq!(
            moved(m.clone(), c, 0),
            ["c.test", "a.test", "b.test", "f", "d.test", "e.test"]
        );
        // a to the last slot (the last `TabRows` reports): after the trailing folder, at the
        // top level
        let last = Some(Dest::Normal {
            space: s,
            index: usize::MAX,
        });
        assert_eq!(move_dest(&m, a, 5), last);
        assert_eq!(
            moved(m.clone(), a, 5),
            ["b.test", "c.test", "f", "d.test", "e.test", "a.test"]
        );
        // d out of the trailing folder the same way
        assert_eq!(move_dest(&m, d, 5), last);
        // a into the trailing folder: before its last tab
        assert_eq!(
            move_dest(&m, a, 4),
            Some(Dest::Folder {
                folder: f,
                index: 1
            })
        );
        // a just below the folder's header: its first place
        assert_eq!(
            moved(m.clone(), a, 3),
            ["b.test", "c.test", "f", "a.test", "d.test", "e.test"]
        );
        // e within its folder, before d
        assert_eq!(
            moved(m.clone(), e, 4),
            ["a.test", "b.test", "c.test", "f", "e.test", "d.test"]
        );
        // d up onto the header's slot: out of the folder, above it
        assert_eq!(
            moved(m.clone(), d, 3),
            ["a.test", "b.test", "c.test", "d.test", "f", "e.test"]
        );
        // d out of the folder, up between a and b
        assert_eq!(
            moved(m.clone(), d, 1),
            ["a.test", "d.test", "b.test", "c.test", "f", "e.test"]
        );
        // b down to where d is: into the folder after d
        assert_eq!(
            moved(m.clone(), b, 4),
            ["a.test", "c.test", "f", "d.test", "b.test", "e.test"]
        );
    }

    #[test]
    fn a_collapsed_folder_is_one_row() {
        let (m, [a, _, _, d, ..], f) = with_folder();
        let (m, _) = apply(
            m,
            Command::SetFolderCollapsed {
                folder: f,
                collapsed: true,
            },
        );
        // its tabs are no rows
        assert_eq!(move_dest(&m, d, 0), None);
        // a down past the header: after the folder, at the top level
        assert_eq!(moved(m.clone(), a, 3), ["b.test", "c.test", "f", "a.test"]);
        assert_eq!(moved(m.clone(), a, 2), ["b.test", "c.test", "a.test", "f"]);
    }

    #[test]
    fn folder_row_ids_are_negative_and_round_trip() {
        for id in [0, 1, 7, 1_000_000] {
            let row = folder_row_id(FolderId(id));
            assert!(row <= -2);
            assert_eq!(tab_id(row), None);
            assert_eq!(folder_of_row(row), Some(FolderId(id)));
        }
        assert_eq!(folder_of_row(-1), None);
        assert_eq!(folder_of_row(3), None);
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
