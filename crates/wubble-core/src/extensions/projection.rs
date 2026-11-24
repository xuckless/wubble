//! wubble's windows and tabs as extensions see them (`chrome.windows`, `chrome.tabs`): a pure
//! projection of the [`Model`], pushed whole to the patched CEF (`wubble_engine::ext_api`),
//! which turns it into Chromium's extension window controllers and diffs it for the tab events.
//! The Model stays the one owner; nothing here is stored.
//!
//! Rules (docs/decisions.md, "Extension tabs model"):
//! - A Main window is a `normal` window listing its Space's tabs in sidebar order: pinned, then
//!   items, folders flattened. Its active tab is the Space's.
//! - Tabs of Spaces no Main window shows sit at the end of the focused Main window, never
//!   active: one window holds every workspace's tabs, the way Zen's workspaces do.
//! - A Little or Popup window is a `popup` window with its one tab, active at index 0; that tab
//!   is left out of its Space's window. Pip windows are left out (their tab stays its Space's).
//! - Only live tabs (with a page) are listed; indices stay contiguous.
//! - The focused window is the Model's last one.
//! - Private windows (a private Space's Main window, a private tab's Little or Popup window) and
//!   their tabs are left out. The projection is one for every extension, so it cannot follow a
//!   per-extension "Allow in Private Windows" ([`crate::extensions::reaches`]); an allowed
//!   extension's view of private tabs is the engine's half (docs/research/extensions.md,
//!   "Private windows").

use crate::model::Model;
use crate::pages::InternalPage;
use crate::space::{Item, Side, Space, SpaceId};
use crate::tab::TabId;
use crate::window::{Shown, WindowId};

/// `chrome.windows.WindowType`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExtWindowKind {
    Normal,
    Popup,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExtWindow {
    pub id: WindowId,
    pub focused: bool,
    pub kind: ExtWindowKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExtTab {
    pub tab: TabId,
    pub window: WindowId,
    pub index: u32,
    pub active: bool,
    pub pinned: bool,
}

/// Windows least recently focused first, as [`Model::windows`]; tabs by window, then index.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Projection {
    pub windows: Vec<ExtWindow>,
    pub tabs: Vec<ExtTab>,
}

/// The projection of `model`, listing only the tabs `live` accepts.
pub fn project(model: &Model, live: impl Fn(TabId) -> bool) -> Projection {
    let focused = model.windows.last().map(|w| w.id);
    // Tabs shown in a window of their own are not their Space's window's.
    let own: Vec<TabId> = model.windows.iter().filter_map(|w| w.tab()).collect();
    let shown: Vec<SpaceId> = model.windows.iter().filter_map(|w| w.space()).collect();
    let focused_main = model
        .windows
        .iter()
        .rev()
        .filter(|w| model.side_of_window(w.id) != Side::Private)
        .find_map(|w| w.space().map(|_| w.id));
    let mut out = Projection::default();
    for w in &model.windows {
        if model.side_of_window(w.id) == Side::Private {
            continue;
        }
        let kind = match w.shown {
            Shown::Space(_) => ExtWindowKind::Normal,
            Shown::Little(_) | Shown::Popup(_) => ExtWindowKind::Popup,
            Shown::Pip(_) => continue,
        };
        out.windows.push(ExtWindow {
            id: w.id,
            focused: Some(w.id) == focused,
            kind,
        });
        // (tab, active, pinned) in window order.
        let mut listed: Vec<(TabId, bool, bool)> = vec![];
        match w.shown {
            Shown::Space(space) => {
                let mine = model.spaces.iter().filter(|s| s.id == space);
                for s in mine {
                    listed.extend(space_tabs(s, true));
                }
                if Some(w.id) == focused_main {
                    for s in model.spaces.iter().filter(|s| {
                        model.side_of_space(s.id) == Side::Normal && !shown.contains(&s.id)
                    }) {
                        listed.extend(space_tabs(s, false));
                    }
                }
            }
            Shown::Little(t) | Shown::Popup(t) => {
                listed.push((t, true, model.is_pinned(t)));
            }
            Shown::Pip(_) => {}
        }
        let windowed = !matches!(w.shown, Shown::Space(_));
        let mut index = 0;
        for (tab, active, pinned) in listed {
            if !live(tab) || (!windowed && own.contains(&tab)) {
                continue;
            }
            out.tabs.push(ExtTab {
                tab,
                window: w.id,
                index,
                active,
                pinned,
            });
            index += 1;
        }
    }
    out
}

/// The page a tab an extension opens on `url` shows: Chrome's new-tab page (what `tabs.create`
/// without a URL asks for) is wubble's start page; any other URL is itself.
pub fn opened_url(url: &str) -> &str {
    let page = url.trim().trim_end_matches('/');
    let new_tab = ["chrome://newtab", "chrome://new-tab-page"];
    if new_tab.iter().any(|n| n.eq_ignore_ascii_case(page)) {
        InternalPage::Start.url()
    } else {
        url
    }
}

/// `space`'s tabs in sidebar order with (active, pinned); active only when `may_be_active`.
fn space_tabs(space: &Space, may_be_active: bool) -> Vec<(TabId, bool, bool)> {
    let active = |t: TabId| may_be_active && space.active == Some(t);
    let pinned = space.pinned.iter().map(|&t| (t, active(t), true));
    let items = space.items.iter().flat_map(|item| match item {
        Item::Tab(t) => vec![*t],
        Item::Folder(f) => f.tabs.clone(),
    });
    pinned.chain(items.map(|t| (t, active(t), false))).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::space::{Folder, FolderId};
    use crate::window::Window;
    use std::time::Instant;

    fn t(n: u64) -> TabId {
        TabId(n)
    }

    /// One Main window on Space 0 with `pinned` and `items`; no tab is checked against the
    /// Model's own rules, which `project` does not read.
    fn model(pinned: &[u64], items: Vec<Item>, active: Option<u64>) -> Model {
        let mut m = Model::new(Instant::now());
        m.spaces[0].pinned = pinned.iter().map(|&n| t(n)).collect();
        m.spaces[0].items = items;
        m.spaces[0].active = active.map(t);
        m
    }

    fn space(m: &Model, id: u32, items: &[u64], active: Option<u64>) -> Space {
        Space {
            id: SpaceId(id),
            pinned: vec![],
            items: items.iter().map(|&n| Item::Tab(t(n))).collect(),
            active: active.map(t),
            ..m.spaces[0].clone()
        }
    }

    fn tab(n: u64, window: u32, index: u32, active: bool, pinned: bool) -> ExtTab {
        ExtTab {
            tab: t(n),
            window: WindowId(window),
            index,
            active,
            pinned,
        }
    }

    fn all(_: TabId) -> bool {
        true
    }

    #[test]
    fn a_main_window_lists_pinned_then_items_with_folders_flattened() {
        let folder = Item::Folder(Folder {
            id: FolderId(0),
            name: "F".into(),
            tabs: vec![t(3), t(4)],
            collapsed: true,
        });
        let m = model(
            &[1],
            vec![Item::Tab(t(2)), folder, Item::Tab(t(5))],
            Some(4),
        );
        let w = m.windows[0].id;
        let p = project(&m, all);
        assert_eq!(
            p.windows,
            [ExtWindow {
                id: w,
                focused: true,
                kind: ExtWindowKind::Normal
            }]
        );
        assert_eq!(
            p.tabs,
            [
                tab(1, w.0, 0, false, true),
                tab(2, w.0, 1, false, false),
                tab(3, w.0, 2, false, false),
                tab(4, w.0, 3, true, false),
                tab(5, w.0, 4, false, false),
            ]
        );
    }

    #[test]
    fn sleeping_tabs_are_left_out_and_indices_stay_contiguous() {
        let m = model(&[1], vec![Item::Tab(t(2)), Item::Tab(t(3))], Some(3));
        let w = m.windows[0].id.0;
        let p = project(&m, |tab| tab != t(2));
        assert_eq!(
            p.tabs,
            [tab(1, w, 0, false, true), tab(3, w, 1, true, false)]
        );
    }

    #[test]
    fn two_main_windows_show_their_own_spaces_and_the_last_is_focused() {
        let mut m = model(&[], vec![Item::Tab(t(1))], Some(1));
        let s1 = space(&m, 1, &[2, 3], Some(2));
        m.spaces.push(s1);
        m.windows.push(Window::main(WindowId(7), SpaceId(1)));
        let first = m.windows[0].id.0;
        let p = project(&m, all);
        let focus: Vec<_> = p.windows.iter().map(|w| (w.id.0, w.focused)).collect();
        assert_eq!(focus, [(first, false), (7, true)]);
        assert_eq!(
            p.tabs,
            [
                tab(1, first, 0, true, false),
                tab(2, 7, 0, true, false),
                tab(3, 7, 1, false, false),
            ]
        );
    }

    #[test]
    fn tabs_of_an_unshown_space_go_to_the_focused_main_window_never_active() {
        let mut m = model(&[], vec![Item::Tab(t(1))], Some(1));
        let hidden = space(&m, 1, &[2, 3], Some(2));
        m.spaces.push(hidden);
        let w = m.windows[0].id.0;
        let p = project(&m, all);
        assert_eq!(
            p.tabs,
            [
                tab(1, w, 0, true, false),
                tab(2, w, 1, false, false),
                tab(3, w, 2, false, false),
            ]
        );
    }

    #[test]
    fn little_and_popup_windows_are_popups_with_their_tab_and_pip_is_left_out() {
        let mut m = model(
            &[],
            vec![Item::Tab(t(1)), Item::Tab(t(2)), Item::Tab(t(3))],
            Some(1),
        );
        // tabs the model knows: a window of an unknown tab is taken as private and left out
        for n in 1..=3 {
            m.tabs.insert(
                t(n),
                crate::tab::Tab {
                    id: t(n),
                    url: String::new(),
                    title: String::new(),
                    lifecycle: crate::lifecycle::Lifecycle::new(),
                    container: crate::space::ContainerId::DEFAULT,
                    opener: None,
                    last_active_ms: 0,
                    busy: crate::busy::Busy::NONE,
                },
            );
        }
        let main = m.windows[0].id.0;
        m.windows.insert(
            0,
            Window {
                id: WindowId(8),
                shown: Shown::Little(t(2)),
                geometry: None,
            },
        );
        m.windows.push(Window {
            id: WindowId(9),
            shown: Shown::Popup(t(3)),
            geometry: None,
        });
        m.windows.push(Window {
            id: WindowId(10),
            shown: Shown::Pip(t(1)),
            geometry: None,
        });
        let p = project(&m, all);
        let kinds: Vec<_> = p
            .windows
            .iter()
            .map(|w| (w.id.0, w.kind, w.focused))
            .collect();
        // The Pip window is the focused one and is left out: no listed window is focused.
        assert_eq!(
            kinds,
            [
                (8, ExtWindowKind::Popup, false),
                (main, ExtWindowKind::Normal, false),
                (9, ExtWindowKind::Popup, false),
            ]
        );
        assert_eq!(
            p.tabs,
            [
                tab(2, 8, 0, true, false),
                tab(1, main, 0, true, false),
                tab(3, 9, 0, true, false),
            ]
        );
    }

    #[test]
    fn chromes_new_tab_page_opens_as_the_start_page() {
        for url in [
            "chrome://newtab/",
            "chrome://newtab",
            "chrome://new-tab-page/",
        ] {
            assert_eq!(opened_url(url), "wubble://start");
        }
        assert_eq!(opened_url("https://a.test/"), "https://a.test/");
        assert_eq!(opened_url("chrome://settings/"), "chrome://settings/");
    }

    #[test]
    fn an_empty_model_has_one_window_and_no_tabs() {
        let m = Model::new(Instant::now());
        let p = project(&m, all);
        assert_eq!(p.windows.len(), 1);
        assert!(p.windows[0].focused);
        assert!(p.tabs.is_empty());
    }
}
