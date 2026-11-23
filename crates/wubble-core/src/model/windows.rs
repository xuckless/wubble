//! The window half of [`Model`]: which window shows what, and the window commands.

use super::{Crossing, DEFAULT_TINT, Dest, Effect, Model, Reach, crossing};
use crate::space::{
    Container, ContainerId, PRIVATE_NAME, Rgb, Side, Space, SpaceIcon, SpaceId, SpaceKind,
    SplitGroup, Storage,
};
use crate::tab::TabId;
use crate::window::{Closing, Geometry, Shown, Window, WindowId, WindowKind, WindowTarget};

/// A private Space's tint: the design's lilac swatch, Firefox private browsing's purple (the
/// window's dark tint and Private chip are the shell's, `kit/chrome/private-mark.slint`).
const PRIVATE_TINT: [u8; 3] = crate::kit_gen::spaces::SWATCHES[1];

impl Model {
    /// The Space of the focused Main window (the last focused one while a Little or Popup
    /// window has focus): where new tabs go and what the sidebar lists.
    pub fn active_space(&self) -> SpaceId {
        self.windows[self.main_idx()]
            .space()
            .expect("a Main window shows a Space")
    }

    /// The focused window.
    pub fn focused(&self) -> WindowId {
        self.windows.last().expect("a window").id
    }

    pub fn window(&self, id: WindowId) -> Option<&Window> {
        self.windows.iter().find(|w| w.id == id)
    }

    /// The Little or Popup window showing `tab`, if one does.
    pub fn windowed(&self, tab: TabId) -> Option<WindowId> {
        self.windows
            .iter()
            .find(|w| w.tab() == Some(tab))
            .map(|w| w.id)
    }

    /// The window `tab` is on screen in, if any.
    pub fn window_of(&self, tab: TabId) -> Option<WindowId> {
        self.windows
            .iter()
            .find(|w| self.window_tabs(w.id).contains(&tab))
            .map(|w| w.id)
    }

    /// The tabs a window shows, left to right: its Space's split or active tab, or its own tab.
    /// The tab in picture-in-picture is its Pip window's, never its Space's window's.
    pub fn window_tabs(&self, id: WindowId) -> Vec<TabId> {
        match self.window(id).map(|w| w.shown) {
            // the window's peek, over its page, last
            Some(Shown::Space(s)) => {
                let pip = self.pip_tab();
                let mut tabs: Vec<TabId> = self
                    .space_idx(s)
                    .map_or_else(Vec::new, |si| self.space_tabs(si))
                    .into_iter()
                    .filter(|t| Some(*t) != pip)
                    .collect();
                if let Some(g) = self.glance_of(id)
                    && !tabs.contains(&g.tab)
                {
                    tabs.push(g.tab);
                }
                tabs
            }
            Some(Shown::Little(t) | Shown::Popup(t) | Shown::Pip(t)) => vec![t],
            None => vec![],
        }
    }

    /// The page of window `id` that gets keys: its own tab, or its Space's active tab.
    pub fn window_tab(&self, id: WindowId) -> Option<TabId> {
        self.window(id).and_then(|w| self.focused_tab(w))
    }

    /// The page of window `id` that gets keys, if it is on screen there: a Main window whose
    /// active tab is in picture-in-picture shows a card for it and sends it no keys.
    pub fn window_key_tab(&self, id: WindowId) -> Option<TabId> {
        self.window_tab(id)
            .filter(|&t| self.window_of(t) == Some(id))
    }

    /// The split a window shows, if any.
    pub fn window_split(&self, id: WindowId) -> Option<&SplitGroup> {
        self.space_split(self.space_idx(self.window(id)?.space()?)?)
    }

    /// The page of `w` that gets keys: its own tab, or its Space's active tab.
    pub(super) fn focused_tab(&self, w: &Window) -> Option<TabId> {
        match w.shown {
            Shown::Little(t) | Shown::Popup(t) | Shown::Pip(t) => Some(t),
            Shown::Space(s) => self.space_idx(s).and_then(|si| self.spaces[si].active),
        }
    }

    /// The focused Main window (the last focused one while a Little or Popup window has
    /// focus): the one showing [`Model::active_space`], where its command bar and sidebar are.
    pub fn main_window(&self) -> WindowId {
        self.windows[self.main_idx()].id
    }

    /// The tab in picture-in-picture: the one a Pip window shows.
    pub fn pip_tab(&self) -> Option<TabId> {
        self.windows.iter().find_map(|w| match w.shown {
            Shown::Pip(t) => Some(t),
            _ => None,
        })
    }

    /// Whether `tab` can be shown in a Pip window: a known tab that is not a peek, not a Little
    /// or Popup window's and not a split pane (its pane would leave a hole in the split).
    pub fn can_pip(&self, tab: TabId) -> bool {
        self.tabs.contains_key(&tab)
            && !self.is_glance(tab)
            && self.windowed(tab).is_none()
            && self.split_of(tab).is_none()
    }

    /// The Pip window, if one is open.
    pub fn pip_window(&self) -> Option<WindowId> {
        self.windows
            .iter()
            .find(|w| w.kind() == WindowKind::Pip)
            .map(|w| w.id)
    }

    /// Where `container` keeps its data ([`Container::storage`]). An unknown container is
    /// [`Storage::Memory`]: what nobody knows is never written to disk (fails closed).
    pub fn storage_of(&self, container: ContainerId) -> Storage {
        self.containers
            .iter()
            .find(|c| c.id == container)
            .map_or(Storage::Memory, |c| c.storage)
    }

    /// The side of the private line `container` is on: its storage's.
    pub fn side_of_container(&self, container: ContainerId) -> Side {
        self.storage_of(container).side()
    }

    /// The side `space` is on: its container's (a private window's Space, [`SpaceKind::Private`],
    /// is in the Memory container, by [`Model::check`]). An unknown Space is private.
    pub fn side_of_space(&self, space: SpaceId) -> Side {
        self.space_idx(space).map_or(Side::Private, |si| {
            self.side_of_container(self.spaces[si].container)
        })
    }

    /// `tab`'s container; `None` for an unknown tab, which its caller refuses or cancels
    /// (never a Default stand-in).
    pub fn container_of(&self, tab: TabId) -> Option<ContainerId> {
        self.tabs.get(&tab).map(|t| t.container)
    }

    /// The side `tab` is on: its container's. An unknown tab is private: nothing of it is
    /// recorded, nothing of it crosses (fails closed).
    pub fn side_of_tab(&self, tab: TabId) -> Side {
        self.container_of(tab)
            .map_or(Side::Private, |c| self.side_of_container(c))
    }

    /// The side a record of `container` (a download row) is shown on: private only for a
    /// known Memory container. A row outlives its container (a deleted, agent or imported
    /// one), and stays the normal side's then, where it was stored.
    pub fn side_of_record(&self, container: ContainerId) -> Side {
        let memory = self
            .containers
            .iter()
            .any(|c| c.id == container && c.storage == Storage::Memory);
        if memory { Side::Private } else { Side::Normal }
    }

    /// The side window `id` is on: its Space's (a Main window), its own tab's (a Little, Popup
    /// or Pip window). An unknown window is private.
    pub fn side_of_window(&self, id: WindowId) -> Side {
        match self.window(id).map(|w| w.shown) {
            Some(Shown::Space(s)) => self.side_of_space(s),
            Some(Shown::Little(t) | Shown::Popup(t) | Shown::Pip(t)) => self.side_of_tab(t),
            None => Side::Private,
        }
    }

    /// The Spaces a window showing `space` lists (its sidebar's Spaces, the command bar's open
    /// tabs): a private window only its own, any other window every Space that is not private
    /// (Chrome's incognito window and its tab search stay within one profile).
    pub fn listed_spaces(&self, space: SpaceId) -> impl Iterator<Item = &Space> + '_ {
        let private = self.side_of_space(space) == Side::Private;
        self.spaces.iter().filter(move |s| {
            if private {
                s.id == space
            } else {
                s.kind != SpaceKind::Private
            }
        })
    }

    /// The Space of the last focused Main window that is not private, else the first Space that
    /// is not: where a link from another app, an extension's tab or an agent's lands while a
    /// private window has focus (Chrome opens those in the last normal window).
    pub fn normal_space(&self) -> SpaceId {
        self.windows
            .iter()
            .rev()
            .filter_map(Window::space)
            .find(|&s| self.side_of_space(s) != Side::Private)
            .or_else(|| {
                self.spaces
                    .iter()
                    .find(|s| s.kind != SpaceKind::Private)
                    .map(|s| s.id)
            })
            .expect("a Space that is not private")
    }

    /// The private tabs `closing` ends, whose running downloads go with them (the host asks
    /// first): a private Main window's Space's tabs, a private tab's own Popup window's tab;
    /// quitting, or closing the last Main window (which quits), every private tab. A normal
    /// window ends none: its Space and tabs stay.
    pub fn private_tabs_ended(&self, closing: Closing) -> Vec<TabId> {
        let mains = self
            .windows
            .iter()
            .filter(|w| w.kind() == WindowKind::Main)
            .count();
        let all = || self.tabs.keys().copied().collect::<Vec<_>>();
        let ended = match closing {
            Closing::Window(w) => match self.window(w).map(|w| w.shown) {
                Some(Shown::Space(_)) if mains == 1 => all(),
                Some(Shown::Space(s)) => self.order(s),
                Some(Shown::Little(t) | Shown::Popup(t)) => vec![t],
                // a Pip window's tab stays in its Space
                Some(Shown::Pip(_)) | None => vec![],
            },
            Closing::App | Closing::Restart => all(),
        };
        ended
            .into_iter()
            .filter(|&t| self.side_of_tab(t) == Side::Private)
            .collect()
    }

    /// The Main window showing `space`, if one does.
    pub fn space_window(&self, space: SpaceId) -> Option<WindowId> {
        self.shown_by(space).map(|i| self.windows[i].id)
    }

    /// The index of the focused Main window.
    fn main_idx(&self) -> usize {
        self.windows
            .iter()
            .rposition(|w| w.kind() == WindowKind::Main)
            .expect("a Main window")
    }

    /// The index of the Main window showing `space`.
    pub(super) fn shown_by(&self, space: SpaceId) -> Option<usize> {
        self.windows
            .iter()
            .position(|w| w.shown == Shown::Space(space))
    }

    /// Focuses window `id` (it moves to the end). The effect, if focus moved.
    pub(super) fn focus(&mut self, id: WindowId) -> Option<Effect> {
        let i = self.windows.iter().position(|w| w.id == id)?;
        if i + 1 == self.windows.len() {
            return None;
        }
        let w = self.windows.remove(i);
        self.windows.push(w);
        Some(Effect::FocusWindow(id))
    }

    /// The focused Main window shows `space`; if another Main window shows it already, that
    /// window takes focus instead (two windows never show one Space).
    ///
    /// A private window is locked to its Space: it is never turned to another one. A Space no
    /// window shows is then shown by the last focused Main window that is not private (focused),
    /// or by a new window when there is none.
    pub(super) fn set_active_space(&mut self, space: SpaceId) -> Vec<Effect> {
        if self.active_space() == space {
            return vec![];
        }
        let mut fx = vec![];
        match self.shown_by(space) {
            Some(i) => fx.extend(self.focus(self.windows[i].id)),
            None => {
                let main = self.main_idx();
                let normal = self.windows.iter().rposition(|w| {
                    w.space()
                        .is_some_and(|s| self.side_of_space(s) != Side::Private)
                });
                match normal {
                    Some(i) => {
                        self.windows[i].shown = Shown::Space(space);
                        if i != main {
                            fx.extend(self.focus(self.windows[i].id));
                        }
                    }
                    None => {
                        let id = self.push_window(Shown::Space(space), None);
                        fx.push(Effect::WindowOpened(id));
                    }
                }
            }
        }
        fx.push(Effect::SpaceChanged(space));
        fx
    }

    /// The Space no Main window shows that is nearest to index `near`, lower first.
    pub(super) fn free_space(&self, near: usize) -> Option<SpaceId> {
        // a private Space is its own window's, never another's
        let free = |i: &usize| {
            self.spaces[*i].kind != SpaceKind::Private
                && self.shown_by(self.spaces[*i].id).is_none()
        };
        let n = self.spaces.len();
        (0..=near.min(n.saturating_sub(1)))
            .rev()
            .chain(near + 1..n)
            .find(free)
            .map(|i| self.spaces[i].id)
    }

    pub(super) fn add_space(&mut self, name: String, tint: Rgb, container: ContainerId) -> SpaceId {
        let id = SpaceId(self.next_space);
        self.next_space += 1;
        let icon = SpaceIcon::nth(self.spaces.len());
        self.spaces.push(Space {
            id,
            name,
            tint,
            icon,
            container,
            pinned: vec![],
            items: vec![],
            active: None,
            split: None,
            never_sleep: false,
            kind: SpaceKind::Normal,
        });
        id
    }

    /// A Space for a new Main window: the first no window shows, else a new one.
    fn space_for_window(&mut self) -> SpaceId {
        if let Some(s) = self.free_space(0) {
            return s;
        }
        // private Spaces are not counted: they are no Space of the user's list
        let name = format!(
            "Space {}",
            self.spaces
                .iter()
                .filter(|s| s.kind != SpaceKind::Private)
                .count()
                + 1
        );
        self.add_space(name, DEFAULT_TINT, ContainerId::DEFAULT)
    }

    /// Adds an unfocused window. Returns its id.
    fn push_window(&mut self, shown: Shown, geometry: Option<Geometry>) -> WindowId {
        let id = WindowId(self.next_window);
        self.next_window += 1;
        // before the focused one: `focus` then moves it last
        self.windows.push(Window {
            id,
            shown,
            geometry,
        });
        id
    }

    pub(super) fn new_window(&mut self, space: Option<SpaceId>) -> Vec<Effect> {
        // with no window yet, the new one is a normal window
        let from = self
            .windows
            .last()
            .map_or(Side::Normal, |w| self.side_of_window(w.id));
        let space = match space {
            Some(s)
                if self.space_idx(s).is_none()
                    || crossing(Reach::NewWindow, from, self.side_of_space(s))
                        == Crossing::Refuse =>
            {
                return vec![];
            }
            Some(s) => match self.shown_by(s) {
                Some(i) => return self.focus(self.windows[i].id).into_iter().collect(),
                None => s,
            },
            None => self.space_for_window(),
        };
        // the size of the window it was opened from, cascaded (Chrome's `WindowSizer`)
        let geometry = self
            .window(self.main_window())
            .and_then(|w| w.geometry.as_ref())
            .map(Geometry::cascaded);
        let id = self.push_window(Shown::Space(space), geometry);
        let si = self.space_idx(space).expect("space");
        vec![
            Effect::WindowOpened(id),
            Effect::SpaceChanged(space),
            self.show(self.spaces[si].active),
        ]
    }

    pub(super) fn close_window(&mut self, id: WindowId) -> Vec<Effect> {
        let Some(w) = self.window(id).cloned() else {
            return vec![];
        };
        match w.shown {
            // its tab closes; `prune_windows` then closes the window
            Shown::Little(t) | Shown::Popup(t) => self.close_tab(t),
            // the tab stays, back in its Space's window
            Shown::Pip(_) => self.close_pip(),
            Shown::Space(s) => {
                if self
                    .windows
                    .iter()
                    .filter(|w| w.kind() == WindowKind::Main)
                    .count()
                    == 1
                {
                    return vec![Effect::LastWindowClosed];
                }
                if self.side_of_space(s) == Side::Private {
                    return self.close_private_window(id, s);
                }
                let was_focused = self.focused() == id;
                self.windows.retain(|w| w.id != id);
                let mut fx = vec![Effect::WindowClosed(id)];
                if was_focused {
                    fx.push(Effect::SpaceChanged(self.active_space()));
                }
                fx
            }
        }
    }

    /// A private window closes (Chrome `BrowserView` closing an incognito window): its tabs close,
    /// its Space goes, and with the last private window the ephemeral container goes too, so
    /// nothing of it outlives the window (Chrome destroys the off-the-record profile when its
    /// last browser closes, `ProfileDestroyer::DestroyOTRProfileWhenAppropriate`). The caller
    /// checked it is not the last Main window.
    fn close_private_window(&mut self, id: WindowId, space: SpaceId) -> Vec<Effect> {
        let Some(si) = self.space_idx(space) else {
            return vec![];
        };
        let gone = self.order_idx(si);
        self.glances
            .retain(|g| g.window != id && !gone.contains(&g.tab) && !gone.contains(&g.opener));
        let was_focused = self.focused() == id;
        let container = self.spaces[si].container;
        let mut fx: Vec<Effect> = gone.iter().map(|&t| Effect::CloseTab(t)).collect();
        self.spaces.remove(si);
        self.forget(&gone);
        self.windows.retain(|w| w.id != id);
        fx.push(Effect::WindowClosed(id));
        if !self.spaces.iter().any(|s| s.container == container)
            && !self.tabs.values().any(|t| t.container == container)
        {
            self.containers.retain(|c| c.id != container);
            fx.push(Effect::ContainerDeleted {
                id: container,
                storage: Storage::Memory,
            });
        }
        if was_focused {
            let active = self.active_space();
            fx.push(Effect::SpaceChanged(active));
            let si = self.space_idx(active).expect("active space");
            fx.push(self.show(self.spaces[si].active));
        }
        fx
    }

    /// A new private Space in the in-memory container private windows share (the one there
    /// is, else a new one) and the Main window showing it, focused.
    fn add_private_window(&mut self) -> (SpaceId, WindowId) {
        let shared = self
            .containers
            .iter()
            .find(|c| c.storage == Storage::Memory);
        let container = match shared {
            Some(c) => c.id,
            None => {
                let id = ContainerId(self.next_container);
                self.next_container += 1;
                self.containers.push(Container {
                    id,
                    name: PRIVATE_NAME.into(),
                    storage: Storage::Memory,
                });
                id
            }
        };
        let tint = {
            let [r, g, b] = PRIVATE_TINT;
            Rgb(r, g, b)
        };
        let space = self.add_space(PRIVATE_NAME.into(), tint, container);
        let si = self.space_idx(space).expect("just added");
        self.spaces[si].kind = SpaceKind::Private;
        let id = self.push_window(Shown::Space(space), None);
        (space, id)
    }

    /// Ctrl+Shift+N: a private window, focused, on a new private Space; on `url` (Open Link in
    /// Private Window) or, without one, with the command bar open (Chrome's incognito window
    /// opens on its new tab page).
    pub(super) fn new_private_window(&mut self, url: Option<String>) -> Vec<Effect> {
        let (space, id) = self.add_private_window();
        let mut fx = vec![Effect::WindowOpened(id), Effect::SpaceChanged(space)];
        match url.and_then(|u| self.place_new_tab(u, None, None, Some(space))) {
            Some((tab, si, create)) => {
                self.spaces[si].active = Some(tab);
                fx.push(create);
                fx.push(Effect::ActivateTab(tab));
            }
            None => fx.push(Effect::ShowCommandBar),
        }
        fx
    }

    /// Closes the Little, Popup and Pip windows whose tab is gone.
    pub(super) fn prune_windows(&mut self) -> Vec<Effect> {
        let gone: Vec<WindowId> = self
            .windows
            .iter()
            .filter(|w| w.owner().is_some_and(|t| !self.tabs.contains_key(&t)))
            .map(|w| w.id)
            .collect();
        self.windows.retain(|w| !gone.contains(&w.id));
        gone.into_iter().map(Effect::WindowClosed).collect()
    }

    pub(super) fn move_tab_to_window(&mut self, tab: TabId, to: WindowTarget) -> Vec<Effect> {
        if !self.tabs.contains_key(&tab) {
            return vec![];
        }
        // a tab never crosses between private and normal windows (Chrome keeps incognito tabs in
        // incognito windows: `TabStripModel` drags only within one profile)
        let side = self.side_of_tab(tab);
        // the private window it leaves, which closes once nothing is left in it
        let left = self
            .location(tab)
            .map(|(s, _)| s)
            .filter(|&s| self.side_of_space(s) == Side::Private)
            .and_then(|s| self.space_window(s));
        let mut fx = vec![];
        let target = match to {
            WindowTarget::Existing(w) => match self.window(w) {
                Some(win)
                    if win.kind() == WindowKind::Main
                        && crossing(Reach::MoveToWindow, side, self.side_of_window(w))
                            == Crossing::Allow =>
                {
                    w
                }
                _ => return vec![],
            },
            WindowTarget::New if crossing(Reach::TearOut, side, side) == Crossing::Refuse => {
                return vec![];
            }
            // a private tab dragged out makes a private window of its own
            WindowTarget::New if side == Side::Private => {
                let (_, id) = self.add_private_window();
                fx.push(Effect::WindowOpened(id));
                id
            }
            WindowTarget::New => {
                let space = self.space_for_window();
                let id = self.push_window(Shown::Space(space), None);
                fx.push(Effect::WindowOpened(id));
                id
            }
        };
        // out of its own window, which then has nothing to show
        if let Some(own) = self.windowed(tab) {
            self.windows.retain(|w| w.id != own);
            fx.push(Effect::WindowClosed(own));
        }
        let space = self.window(target).and_then(Window::space).expect("Main");
        if self.location(tab).map(|(s, _)| s) != Some(space) {
            let moved = self.move_tab(
                tab,
                Dest::Normal {
                    space,
                    index: usize::MAX,
                },
            );
            // the window it left keeps no command bar open for its emptied Space
            fx.extend(moved.into_iter().filter(|e| *e != Effect::ShowCommandBar));
        }
        let si = self.space_idx(space).expect("space");
        self.spaces[si].active = Some(tab);
        if matches!(to, WindowTarget::Existing(_)) {
            fx.extend(self.focus(target));
        } else {
            self.focus(target);
        }
        // An emptied private window closes, as Chrome closes a window whose last tab was
        // dragged out. A normal window stays: it shows a Space, which outlives its tabs.
        if let Some(w) = left.filter(|&w| w != target)
            && let Some(s) = self.window(w).and_then(Window::space)
            && self.order(s).is_empty()
        {
            fx.extend(self.close_window(w));
        }
        fx.push(Effect::ActivateTab(tab));
        fx
    }

    pub(super) fn open_popup_window(
        &mut self,
        url: String,
        opener: TabId,
        size: Option<(u32, u32)>,
        pos: Option<(i32, i32)>,
    ) -> Vec<Effect> {
        let Some((tab, _, create)) = self.place_new_tab(url, None, Some(opener), None) else {
            return vec![];
        };
        // the window's size, from the page's (`window::popup_window`), where it asked to be
        let geometry = size.map(|(width, height)| Geometry {
            width,
            height,
            pos,
            ..Geometry::default()
        });
        let id = self.push_window(Shown::Popup(tab), geometry);
        self.focus(id);
        vec![create, Effect::WindowOpened(id)]
    }

    pub(super) fn open_little(
        &mut self,
        url: String,
        space: Option<SpaceId>,
        container: Option<ContainerId>,
    ) -> Vec<Effect> {
        // a link from another app never opens in a private window (Chrome opens it in the last
        // normal one, as it does here with no Space named)
        let space = space.unwrap_or_else(|| self.normal_space());
        if crossing(Reach::OpenLittle, Side::Normal, self.side_of_space(space)) != Crossing::Allow {
            return vec![];
        }
        let Some((tab, _, create)) = self.place_new_tab(url, container, None, Some(space)) else {
            return vec![];
        };
        let id = self.push_window(Shown::Little(tab), None);
        self.focus(id);
        vec![create, Effect::WindowOpened(id)]
    }

    pub(super) fn promote_little(&mut self, window: WindowId, space: SpaceId) -> Vec<Effect> {
        let tab = match self.window(window).map(|w| w.shown) {
            Some(Shown::Little(t))
                if self.space_idx(space).is_some()
                    && crossing(
                        Reach::PromoteLittle,
                        self.side_of_tab(t),
                        self.side_of_space(space),
                    ) == Crossing::Allow =>
            {
                t
            }
            _ => return vec![],
        };
        self.windows.retain(|w| w.id != window);
        let mut fx = vec![Effect::WindowClosed(window)];
        fx.extend(self.move_tab(
            tab,
            Dest::Normal {
                space,
                index: usize::MAX,
            },
        ));
        fx.extend(self.activate_tab(tab));
        if let Some(i) = self.shown_by(space) {
            fx.extend(self.focus(self.windows[i].id));
        }
        fx
    }

    /// A Pip window for `tab`, unfocused (picture-in-picture takes no keys from the window the
    /// user is in), at the geometry the last one had. Refused for what [`Model::can_pip`]
    /// refuses, and while a Pip window is open.
    pub(super) fn open_pip(&mut self, tab: TabId) -> Vec<Effect> {
        if !self.can_pip(tab) || self.pip_window().is_some() {
            return vec![];
        }
        let id = WindowId(self.next_window);
        self.next_window += 1;
        let at = self.windows.len() - 1;
        self.windows.insert(
            at,
            Window {
                id,
                shown: Shown::Pip(tab),
                geometry: self.pip_geometry.clone(),
            },
        );
        vec![Effect::WindowOpened(id)]
    }

    /// Closes the Pip window; its tab is shown in its Space's window again.
    pub(super) fn close_pip(&mut self) -> Vec<Effect> {
        let Some(id) = self.pip_window() else {
            return vec![];
        };
        self.windows.retain(|w| w.id != id);
        vec![Effect::WindowClosed(id)]
    }

    /// Closes the Pip window and shows its tab where it lives (its Space, in its window).
    pub(super) fn return_pip(&mut self) -> Vec<Effect> {
        let Some(tab) = self.pip_tab() else {
            return vec![];
        };
        let mut fx = self.close_pip();
        fx.extend(self.activate_tab(tab));
        if let Some(i) = self.location(tab).and_then(|(s, _)| self.shown_by(s)) {
            fx.extend(self.focus(self.windows[i].id));
        }
        fx
    }

    /// The window's size, position, show state or monitor changed; kept as its restored
    /// bounds ([`Geometry::observed`]). A Pip window's is also remembered for the next.
    pub(super) fn resize_window(&mut self, id: WindowId, geometry: Geometry) {
        if let Some(w) = self.windows.iter_mut().find(|w| w.id == id) {
            let kept = Geometry::observed(w.geometry.as_ref(), geometry);
            if w.kind() == WindowKind::Pip {
                self.pip_geometry = Some(kept.clone());
            }
            w.geometry = Some(kept);
        }
    }

    /// The window invariants (part of [`Model::check`]).
    pub(super) fn check_windows(&self) -> Result<(), String> {
        if !self.windows.iter().any(|w| w.kind() == WindowKind::Main) {
            return Err("no Main window".into());
        }
        // a private Space lives exactly as long as its window
        if let Some(s) = self
            .spaces
            .iter()
            .find(|s| s.kind == SpaceKind::Private && self.shown_by(s.id).is_none())
        {
            return Err(format!("private space {:?} shown by no window", s.id));
        }
        // the in-memory container lives exactly as long as the private windows
        if self.containers.iter().any(|c| c.storage == Storage::Memory)
            != self.spaces.iter().any(|s| s.kind == SpaceKind::Private)
        {
            return Err("ephemeral container without private windows, or the reverse".into());
        }
        let mut seen: Vec<TabId> = vec![];
        for (i, w) in self.windows.iter().enumerate() {
            if self.windows[..i].iter().any(|o| o.id == w.id) {
                return Err(format!("duplicate window {:?}", w.id));
            }
            if w.id.0 >= self.next_window {
                return Err(format!("window {:?} past the counter", w.id));
            }
            match w.shown {
                Shown::Space(s) => {
                    if self.space_idx(s).is_none() {
                        return Err(format!("window {:?} shows unknown space", w.id));
                    }
                    if self.shown_by(s) != Some(i) {
                        return Err(format!("space {s:?} shown in two windows"));
                    }
                }
                Shown::Little(t) | Shown::Popup(t) => {
                    if !self.tabs.contains_key(&t) {
                        return Err(format!("window {:?} shows unknown tab", w.id));
                    }
                }
                Shown::Pip(t) => {
                    if self.windows[..i]
                        .iter()
                        .any(|o| o.kind() == WindowKind::Pip)
                    {
                        return Err("two Pip windows".into());
                    }
                    if !self.can_pip(t) {
                        return Err(format!("Pip window {:?} shows {t:?}", w.id));
                    }
                }
            }
            for t in self.window_tabs(w.id) {
                if seen.contains(&t) {
                    return Err(format!("tab {t:?} shown in two windows"));
                }
                seen.push(t);
            }
        }
        Ok(())
    }
}
