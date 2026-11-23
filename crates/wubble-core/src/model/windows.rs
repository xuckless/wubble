//! The window half of [`Model`]: which window shows what, and the window commands.

use super::{DEFAULT_TINT, Dest, Effect, Model};
use crate::space::{ContainerId, Rgb, Space, SpaceIcon, SpaceId, SplitGroup};
use crate::tab::TabId;
use crate::window::{Geometry, Shown, Window, WindowId, WindowKind, WindowTarget};

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
    pub(super) fn set_active_space(&mut self, space: SpaceId) -> Vec<Effect> {
        if self.active_space() == space {
            return vec![];
        }
        let mut fx = vec![];
        match self.shown_by(space) {
            Some(i) => fx.extend(self.focus(self.windows[i].id)),
            None => {
                let i = self.main_idx();
                self.windows[i].shown = Shown::Space(space);
            }
        }
        fx.push(Effect::SpaceChanged(space));
        fx
    }

    /// The Space no Main window shows that is nearest to index `near`, lower first.
    pub(super) fn free_space(&self, near: usize) -> Option<SpaceId> {
        let free = |i: &usize| self.shown_by(self.spaces[*i].id).is_none();
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
            agent: false,
        });
        id
    }

    /// A Space for a new Main window: the first no window shows, else a new one.
    fn space_for_window(&mut self) -> SpaceId {
        if let Some(s) = self.free_space(0) {
            return s;
        }
        let name = format!("Space {}", self.spaces.len() + 1);
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
        let space = match space {
            Some(s) if self.space_idx(s).is_none() => return vec![],
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
            Shown::Space(_) => {
                if self
                    .windows
                    .iter()
                    .filter(|w| w.kind() == WindowKind::Main)
                    .count()
                    == 1
                {
                    return vec![Effect::LastWindowClosed];
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
        let mut fx = vec![];
        let target = match to {
            WindowTarget::Existing(w) => match self.window(w) {
                Some(win) if win.kind() == WindowKind::Main => w,
                _ => return vec![],
            },
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
        fx.push(Effect::ActivateTab(tab));
        fx
    }

    pub(super) fn open_popup_window(
        &mut self,
        url: String,
        opener: TabId,
        size: Option<(u32, u32)>,
    ) -> Vec<Effect> {
        let Some((tab, _, create)) = self.place_new_tab(url, None, Some(opener), None) else {
            return vec![];
        };
        // `size` is the page's (window.open's width and height: Chromium's content size); the
        // shell sets the page card `Chrome.inset` in from each edge of the window
        let frame = 2 * crate::kit_gen::tokens::chrome::inset as u32;
        let geometry = size.map(|(width, height)| Geometry {
            width: width.saturating_add(frame),
            height: height.saturating_add(frame),
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
        let Some((tab, _, create)) = self.place_new_tab(url, container, None, space) else {
            return vec![];
        };
        let id = self.push_window(Shown::Little(tab), None);
        self.focus(id);
        vec![create, Effect::WindowOpened(id)]
    }

    pub(super) fn promote_little(&mut self, window: WindowId, space: SpaceId) -> Vec<Effect> {
        let tab = match self.window(window).map(|w| w.shown) {
            Some(Shown::Little(t)) if self.space_idx(space).is_some() => t,
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
