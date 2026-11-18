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
            .find(|w| w.shown == Shown::Tab(tab))
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
    pub fn window_tabs(&self, id: WindowId) -> Vec<TabId> {
        match self.window(id).map(|w| w.shown) {
            Some(Shown::Space(s)) => self
                .space_idx(s)
                .map_or_else(Vec::new, |si| self.space_tabs(si)),
            Some(Shown::Tab(t)) => vec![t],
            None => vec![],
        }
    }

    /// The split a window shows, if any.
    pub fn window_split(&self, id: WindowId) -> Option<&SplitGroup> {
        self.space_split(self.space_idx(self.window(id)?.space()?)?)
    }

    /// The page of `w` that gets keys: its own tab, or its Space's active tab.
    pub(super) fn focused_tab(&self, w: &Window) -> Option<TabId> {
        match w.shown {
            Shown::Tab(t) => Some(t),
            Shown::Space(s) => self.space_idx(s).and_then(|si| self.spaces[si].active),
        }
    }

    /// The index of the focused Main window.
    fn main_idx(&self) -> usize {
        self.windows
            .iter()
            .rposition(|w| w.kind == WindowKind::Main)
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
    fn push_window(
        &mut self,
        kind: WindowKind,
        shown: Shown,
        geometry: Option<Geometry>,
    ) -> WindowId {
        let id = WindowId(self.next_window);
        self.next_window += 1;
        // before the focused one: `focus` then moves it last
        self.windows.push(Window {
            id,
            kind,
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
        let id = self.push_window(WindowKind::Main, Shown::Space(space), None);
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
            Shown::Tab(t) => self.close_tab(t),
            Shown::Space(_) => {
                if self
                    .windows
                    .iter()
                    .filter(|w| w.kind == WindowKind::Main)
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

    /// Closes the Little and Popup windows whose tab is gone.
    pub(super) fn prune_windows(&mut self) -> Vec<Effect> {
        let gone: Vec<WindowId> = self
            .windows
            .iter()
            .filter(|w| w.tab().is_some_and(|t| !self.tabs.contains_key(&t)))
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
                Some(win) if win.kind == WindowKind::Main => w,
                _ => return vec![],
            },
            WindowTarget::New => {
                let space = self.space_for_window();
                let id = self.push_window(WindowKind::Main, Shown::Space(space), None);
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
        let geometry = size.map(|(width, height)| Geometry {
            width,
            height,
            pos: None,
        });
        let id = self.push_window(WindowKind::Popup, Shown::Tab(tab), geometry);
        self.focus(id);
        vec![create, Effect::WindowOpened(id)]
    }

    pub(super) fn open_little(&mut self, url: String, space: Option<SpaceId>) -> Vec<Effect> {
        let Some((tab, _, create)) = self.place_new_tab(url, None, None, space) else {
            return vec![];
        };
        let id = self.push_window(WindowKind::Little, Shown::Tab(tab), None);
        self.focus(id);
        vec![create, Effect::WindowOpened(id)]
    }

    pub(super) fn promote_little(&mut self, window: WindowId, space: SpaceId) -> Vec<Effect> {
        let tab = match self.window(window) {
            Some(w) if w.kind == WindowKind::Little && self.space_idx(space).is_some() => {
                w.tab().expect("a Little window shows a tab")
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

    /// The window invariants (part of [`Model::check`]).
    pub(super) fn check_windows(&self) -> Result<(), String> {
        if !self.windows.iter().any(|w| w.kind == WindowKind::Main) {
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
            match (w.kind, w.shown) {
                (WindowKind::Main, Shown::Space(s)) => {
                    if self.space_idx(s).is_none() {
                        return Err(format!("window {:?} shows unknown space", w.id));
                    }
                    if self.shown_by(s) != Some(i) {
                        return Err(format!("space {s:?} shown in two windows"));
                    }
                }
                (WindowKind::Little | WindowKind::Popup, Shown::Tab(t)) => {
                    if !self.tabs.contains_key(&t) {
                        return Err(format!("window {:?} shows unknown tab", w.id));
                    }
                }
                _ => return Err(format!("window {:?} shows the wrong kind of thing", w.id)),
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
