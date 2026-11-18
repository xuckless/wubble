//! The browser model the shell binds to: Spaces, containers, folders and tabs.
//!
//! All mutation goes through [`apply`], a pure function from (model, command) to
//! (model, effects). It is total: unknown ids and rejected commands return the model
//! unchanged with no effects; indices are clamped. The engine executes the effects.
//!
//! Rules ported, no code copied, from:
//! - Chromium `chrome/browser/ui/tabs/tab_strip_model.cc` (BSD-3-Clause):
//!   `DetermineNewSelectedIndex` (selection after close: opener child, opener, next,
//!   else previous), `ConstrainInsertionIndex` (pinned tabs form a prefix; indices
//!   clamp into their section), pinned tabs never join a group (here: folder), and
//!   `ForgetAllOpeners` on a user tab switch (from memory of the file, unverified).
//! - Zen `src/zen/spaces/ZenSpaceManager.mjs` (MPL-2.0): a workspace has a default
//!   container (0 = none), deleting a container resets workspaces using it, deleting
//!   a workspace closes its tabs (`#deleteWorkspaceOwnedTabs`), and each workspace
//!   remembers its selected tab (`lastSelectedWorkspaceTabs`).
//!
//! Placement is stored once, in the `Space` lists; `tabs` holds data only, so a tab
//! cannot be in two places. What types cannot express is checked by [`Model::check`].

use std::collections::BTreeMap;
use std::time::Instant;

use crate::lifecycle::{self, Activity, Lifecycle, Pin};
use crate::sleep::{Pressure, SleepPolicy};
use crate::space::{Container, ContainerId, Folder, FolderId, Item, Rgb, Space, SpaceId};
use crate::tab::{Liveness, Tab, TabId};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Place {
    Pinned(usize),
    /// Index into the Space's top-level items.
    Normal(usize),
    Folder(FolderId, usize),
}

/// Where a moved tab goes. Pin state follows the destination; indices are clamped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dest {
    Pinned { space: SpaceId, index: usize },
    Normal { space: SpaceId, index: usize },
    Folder { folder: FolderId, index: usize },
}

#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    /// Ctrl+T: only shows the command bar; a tab exists once something is chosen.
    OpenCommandBar,
    /// Into the active Space and shown. With a live `opener` (a popup), into the
    /// opener's Space right after it, shown only if the opener is the visible tab.
    /// `container: None` uses the Space default.
    NewTab {
        url: String,
        container: Option<ContainerId>,
        opener: Option<TabId>,
    },
    CloseTab(TabId),
    ActivateTab(TabId),
    MoveTab {
        tab: TabId,
        to: Dest,
    },
    PinTab(TabId),
    UnpinTab(TabId),
    NewFolder {
        space: SpaceId,
        name: String,
    },
    /// The folder's tabs stay, at the folder's position.
    DeleteFolder(FolderId),
    NewSpace {
        name: String,
        tint: Rgb,
        container: ContainerId,
    },
    RenameSpace {
        space: SpaceId,
        name: String,
    },
    SetSpaceContainer {
        space: SpaceId,
        container: ContainerId,
    },
    /// Closes the Space's tabs (Zen). The UI must confirm. The last Space is refused.
    DeleteSpace(SpaceId),
    SwitchSpace(SpaceId),
    NewContainer {
        name: String,
    },
    DeleteContainer(ContainerId),
    /// Replaces the tab, in place, with a new one in `container`.
    ReopenInContainer {
        tab: TabId,
        container: ContainerId,
    },
    /// The user asked this tab to load `url`.
    Navigate {
        tab: TabId,
        url: String,
    },
    /// The engine reports the tab's committed URL. Nothing is echoed back.
    UrlChanged {
        tab: TabId,
        url: String,
    },
    /// The engine reports the tab's title.
    TitleChanged {
        tab: TabId,
        title: String,
    },
    /// The engine reports whether the tab's page is loading. A loading tab is
    /// busy: it is not frozen or discarded until the load ends.
    LoadingChanged {
        tab: TabId,
        loading: bool,
    },
    /// The tab's renderer process died.
    RendererGone(TabId),
    /// Coarse lifecycle tick: hidden tabs are put to sleep by the sleep policy.
    /// `now` also becomes the model's clock, the time later hides are stamped with.
    Tick {
        now: Instant,
        pressure: Pressure,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    ShowCommandBar,
    CreateTab {
        tab: TabId,
        url: String,
        container: ContainerId,
    },
    CloseTab(TabId),
    /// The engine shows this tab and hides the previous one.
    ActivateTab(TabId),
    SpaceChanged(SpaceId),
    /// Load `url` in the tab's live page.
    Navigate {
        tab: TabId,
        url: String,
    },
    /// What the tab's lifecycle asks of the engine (`lifecycle::transition`).
    Lifecycle {
        tab: TabId,
        effect: lifecycle::Effect,
    },
}

/// One per window.
#[derive(Debug, Clone, PartialEq)]
pub struct Model {
    pub spaces: Vec<Space>,
    /// `containers[0]` is Default.
    pub containers: Vec<Container>,
    pub tabs: BTreeMap<TabId, Tab>,
    pub active_space: SpaceId,
    /// The time hides are stamped with: set at creation and by `Command::Tick`,
    /// so `apply` itself never reads a clock.
    clock: Instant,
    /// The memory pressure of the last `Command::Tick`. With `clock` it lets a
    /// hidden tab that becomes eligible for sleep between ticks be judged at once.
    pressure: Pressure,
    next_tab: u64,
    next_space: u32,
    next_folder: u32,
    next_container: u32,
}

impl Model {
    /// One empty Space and the Default container; `now` starts the model's clock.
    pub fn new(now: Instant) -> Self {
        let space = Space {
            id: SpaceId(0),
            name: "Space 1".into(),
            tint: Rgb(0x6a, 0x8f, 0xff),
            container: ContainerId::DEFAULT,
            pinned: vec![],
            items: vec![],
            active: None,
        };
        Self {
            active_space: space.id,
            spaces: vec![space],
            containers: vec![Container {
                id: ContainerId::DEFAULT,
                name: "Default".into(),
            }],
            tabs: BTreeMap::new(),
            clock: now,
            pressure: Pressure::None,
            next_tab: 1,
            next_space: 1,
            next_folder: 0,
            next_container: 1,
        }
    }

    /// Rebuilds a model from stored parts. The id counters are not part of the parts: they are
    /// derived as the highest id in use plus one, so new ids never collide with restored ones.
    /// `now` starts the model's clock. The result must pass [`Model::check`].
    pub fn from_parts(
        spaces: Vec<Space>,
        containers: Vec<Container>,
        tabs: Vec<Tab>,
        active_space: SpaceId,
        now: Instant,
    ) -> Result<Model, String> {
        let next = |max: Option<u64>, floor: u64| max.map_or(floor, |m| m + 1);
        let next_folder = spaces
            .iter()
            .flat_map(|s| &s.items)
            .filter_map(|i| match i {
                Item::Folder(f) => Some(u64::from(f.id.0)),
                Item::Tab(_) => None,
            })
            .max();
        let to_u32 = |v: u64| u32::try_from(v).map_err(|_| "id counter overflow".to_string());
        let m = Model {
            next_tab: next(tabs.iter().map(|t| t.id.0).max(), 1),
            next_space: to_u32(next(spaces.iter().map(|s| u64::from(s.id.0)).max(), 1))?,
            next_folder: to_u32(next(next_folder, 0))?,
            next_container: to_u32(next(containers.iter().map(|c| u64::from(c.id.0)).max(), 1))?,
            spaces,
            containers,
            tabs: tabs.into_iter().map(|t| (t.id, t)).collect(),
            active_space,
            clock: now,
            pressure: Pressure::None,
        };
        m.check()?;
        Ok(m)
    }

    fn space_idx(&self, id: SpaceId) -> Option<usize> {
        self.spaces.iter().position(|s| s.id == id)
    }

    fn has_container(&self, id: ContainerId) -> bool {
        self.containers.iter().any(|c| c.id == id)
    }

    fn folder_space(&self, folder: FolderId) -> Option<usize> {
        self.spaces.iter().position(|s| {
            s.items
                .iter()
                .any(|i| matches!(i, Item::Folder(f) if f.id == folder))
        })
    }

    fn place_of(&self, tab: TabId) -> Option<(usize, Place)> {
        for (si, s) in self.spaces.iter().enumerate() {
            if let Some(i) = s.pinned.iter().position(|&t| t == tab) {
                return Some((si, Place::Pinned(i)));
            }
            for (i, item) in s.items.iter().enumerate() {
                match item {
                    Item::Tab(t) if *t == tab => return Some((si, Place::Normal(i))),
                    Item::Folder(f) => {
                        if let Some(j) = f.tabs.iter().position(|&t| t == tab) {
                            return Some((si, Place::Folder(f.id, j)));
                        }
                    }
                    Item::Tab(_) => {}
                }
            }
        }
        None
    }

    /// Where a tab is. Linear scan: fine at tab-count scale.
    pub fn location(&self, tab: TabId) -> Option<(SpaceId, Place)> {
        self.place_of(tab).map(|(si, p)| (self.spaces[si].id, p))
    }

    /// Visual order of a Space's tabs: pinned, then items with folder tabs inline.
    pub fn order(&self, space: SpaceId) -> Vec<TabId> {
        self.space_idx(space)
            .map_or_else(Vec::new, |si| self.order_idx(si))
    }

    fn order_idx(&self, si: usize) -> Vec<TabId> {
        let s = &self.spaces[si];
        let mut out = s.pinned.clone();
        for item in &s.items {
            match item {
                Item::Tab(t) => out.push(*t),
                Item::Folder(f) => out.extend(&f.tabs),
            }
        }
        out
    }

    /// Pinned state is owned by placement; `Lifecycle::pin` mirrors it, synced
    /// by `move_tab`, the only way a tab enters or leaves the pinned section.
    pub fn is_pinned(&self, tab: TabId) -> bool {
        matches!(self.place_of(tab), Some((_, Place::Pinned(_))))
    }

    /// The tab on screen: the active tab of the active Space.
    pub fn visible_tab(&self) -> Option<TabId> {
        self.space_idx(self.active_space)
            .and_then(|si| self.spaces[si].active)
    }

    /// Tabs whose page is alive (not discarded): each has one engine view.
    pub fn live_tabs(&self) -> usize {
        self.tabs
            .values()
            .filter(|t| t.lifecycle.liveness != Liveness::Discarded)
            .count()
    }

    /// Verifies what the types cannot: the invariants every `apply` result must hold.
    pub fn check(&self) -> Result<(), String> {
        let mut placed = BTreeMap::new();
        let mut space_ids = vec![];
        let mut folder_ids = vec![];
        if self.spaces.is_empty() {
            return Err("no spaces".into());
        }
        if self.space_idx(self.active_space).is_none() {
            return Err("active space missing".into());
        }
        if self.containers.first().map(|c| c.id) != Some(ContainerId::DEFAULT) {
            return Err("containers[0] is not Default".into());
        }
        for (i, c) in self.containers.iter().enumerate() {
            if self.containers[..i].iter().any(|o| o.id == c.id) {
                return Err(format!("duplicate container {:?}", c.id));
            }
        }
        for (si, s) in self.spaces.iter().enumerate() {
            if space_ids.contains(&s.id) {
                return Err(format!("duplicate space {:?}", s.id));
            }
            space_ids.push(s.id);
            if !self.has_container(s.container) {
                return Err(format!("space {:?} has unknown container", s.id));
            }
            let mut place = |t: TabId| -> Result<(), String> {
                if placed.insert(t, si).is_some() {
                    return Err(format!("tab {t:?} placed twice"));
                }
                let tab = self.tabs.get(&t).ok_or(format!("tab {t:?} not in map"))?;
                if tab.id != t {
                    return Err(format!("tab {t:?} has id {:?}", tab.id));
                }
                if !self.has_container(tab.container) {
                    return Err(format!("tab {t:?} has unknown container"));
                }
                if tab.opener.is_some_and(|o| !self.tabs.contains_key(&o)) {
                    return Err(format!("tab {t:?} opener is gone"));
                }
                Ok(())
            };
            for &t in &s.pinned {
                place(t)?;
            }
            for item in &s.items {
                match item {
                    Item::Tab(t) => place(*t)?,
                    Item::Folder(f) => {
                        if folder_ids.contains(&f.id) {
                            return Err(format!("duplicate folder {:?}", f.id));
                        }
                        folder_ids.push(f.id);
                        for &t in &f.tabs {
                            place(t)?;
                        }
                    }
                }
            }
            let order = self.order_idx(si);
            match s.active {
                Some(a) if order.contains(&a) => {}
                None if order.is_empty() => {}
                a => return Err(format!("space {:?} bad active {a:?}", s.id)),
            }
        }
        if let Some(t) = self.tabs.keys().find(|t| !placed.contains_key(t)) {
            return Err(format!("tab {t:?} in map but unplaced"));
        }
        let visible = self.visible_tab();
        for t in self.tabs.values() {
            let shown = Some(t.id) == visible;
            if shown != (t.lifecycle.liveness == Liveness::Active) {
                return Err(format!(
                    "tab {:?} visible={shown} but {:?}",
                    t.id, t.lifecycle.liveness
                ));
            }
            if (t.lifecycle.pin == Pin::Pinned) != self.is_pinned(t.id) {
                return Err(format!("tab {:?} pin out of sync", t.id));
            }
        }
        Ok(())
    }

    // ----- helpers that mutate; callers validate first so a reject never changes the model

    fn fresh_tab(&mut self) -> TabId {
        self.next_tab += 1;
        TabId(self.next_tab - 1)
    }

    /// The tab to show once `tab` leaves its Space (Chromium `DetermineNewSelectedIndex`).
    fn successor(&self, si: usize, tab: TabId, use_openers: bool) -> Option<TabId> {
        let order = self.order_idx(si);
        let i = order.iter().position(|&t| t == tab)?;
        if use_openers {
            let is_child = |t: &&TabId| self.tabs[*t].opener == Some(tab);
            if let Some(&c) = order[i + 1..]
                .iter()
                .find(is_child)
                .or_else(|| order[..i].iter().rev().find(is_child))
            {
                return Some(c);
            }
            if let Some(o) = self.tabs[&tab].opener.filter(|o| order.contains(o)) {
                return Some(o);
            }
        }
        order.get(i + 1).or_else(|| order[..i].last()).copied()
    }

    /// Removes a tab from its list (not from the map). Returns its Space index.
    fn detach(&mut self, tab: TabId) -> Option<usize> {
        let (si, place) = self.place_of(tab)?;
        let s = &mut self.spaces[si];
        match place {
            Place::Pinned(i) => {
                s.pinned.remove(i);
            }
            Place::Normal(i) => {
                s.items.remove(i);
            }
            Place::Folder(f, i) => {
                if let Some(fo) = s.folder_mut(f) {
                    fo.tabs.remove(i);
                }
            }
        }
        Some(si)
    }

    fn dest_space(&self, to: Dest) -> Option<usize> {
        match to {
            Dest::Pinned { space, .. } | Dest::Normal { space, .. } => self.space_idx(space),
            Dest::Folder { folder, .. } => self.folder_space(folder),
        }
    }

    /// Inserts a detached tab at `to` (clamped). `si` is the dest Space.
    fn attach(&mut self, tab: TabId, to: Dest, si: usize) {
        let s = &mut self.spaces[si];
        match to {
            Dest::Pinned { index, .. } => s.pinned.insert(index.min(s.pinned.len()), tab),
            Dest::Normal { index, .. } => s.items.insert(index.min(s.items.len()), Item::Tab(tab)),
            Dest::Folder { folder, index } => {
                if let Some(f) = s.folder_mut(folder) {
                    f.tabs.insert(index.min(f.tabs.len()), tab);
                }
            }
        }
    }

    /// Drops a closed tab from the map and from every opener reference.
    fn forget(&mut self, gone: &[TabId]) {
        for t in gone {
            self.tabs.remove(t);
        }
        for t in self.tabs.values_mut() {
            if t.opener.is_some_and(|o| gone.contains(&o)) {
                t.opener = None;
            }
        }
    }

    /// Effects for the visible tab becoming `tab` (or nothing left) in the active Space.
    fn show(&self, tab: Option<TabId>) -> Effect {
        tab.map_or(Effect::ShowCommandBar, Effect::ActivateTab)
    }

    fn new_tab(
        &mut self,
        url: String,
        container: Option<ContainerId>,
        opener: Option<TabId>,
    ) -> Vec<Effect> {
        if container.is_some_and(|c| !self.has_container(c)) {
            return vec![];
        }
        let opener_space = opener.and_then(|o| self.place_of(o)).map(|(s, _)| s);
        let opener = opener.filter(|_| opener_space.is_some());
        let si = opener_space
            .unwrap_or_else(|| self.space_idx(self.active_space).expect("active space"));
        // A popup from a hidden tab must not take over the screen.
        let show = opener.is_none() || opener == self.visible_tab();
        let default = self.spaces[si].container;
        let id = self.fresh_tab();
        let eff = container.unwrap_or(default);
        let tab = Tab {
            id,
            url: url.clone(),
            title: String::new(),
            lifecycle: Lifecycle::new(),
            container: eff,
            opener,
        };
        self.tabs.insert(id, tab);
        // New tabs follow their opener, otherwise go last; pinned openers do not count.
        let dest = match opener.and_then(|o| self.place_of(o)) {
            Some((_, Place::Normal(i))) => Dest::Normal {
                space: self.spaces[si].id,
                index: i + 1,
            },
            Some((_, Place::Folder(f, i))) => Dest::Folder {
                folder: f,
                index: i + 1,
            },
            _ => Dest::Normal {
                space: self.spaces[si].id,
                index: usize::MAX,
            },
        };
        self.attach(id, dest, si);
        let mut fx = vec![Effect::CreateTab {
            tab: id,
            url,
            container: eff,
        }];
        if show {
            self.spaces[si].active = Some(id);
            fx.push(Effect::ActivateTab(id));
        }
        fx
    }

    fn close_tab(&mut self, tab: TabId) -> Vec<Effect> {
        let Some((si, _)) = self.place_of(tab) else {
            return vec![];
        };
        let was_active = self.spaces[si].active == Some(tab);
        let next = if was_active {
            self.successor(si, tab, true)
        } else {
            self.spaces[si].active
        };
        self.detach(tab);
        self.forget(&[tab]);
        self.spaces[si].active = next;
        let mut fx = vec![Effect::CloseTab(tab)];
        if was_active && self.spaces[si].id == self.active_space {
            fx.push(self.show(next));
        }
        fx
    }

    fn activate_tab(&mut self, tab: TabId) -> Vec<Effect> {
        let Some((si, _)) = self.place_of(tab) else {
            return vec![];
        };
        let id = self.spaces[si].id;
        if self.spaces[si].active == Some(tab) && self.active_space == id {
            return vec![];
        }
        let mut fx = vec![];
        if self.active_space != id {
            self.active_space = id;
            fx.push(Effect::SpaceChanged(id));
        }
        self.spaces[si].active = Some(tab);
        for t in self.tabs.values_mut() {
            t.opener = None;
        }
        fx.push(Effect::ActivateTab(tab));
        fx
    }

    fn move_tab(&mut self, tab: TabId, to: Dest) -> Vec<Effect> {
        let (Some((src, _)), Some(dst)) = (self.place_of(tab), self.dest_space(to)) else {
            return vec![];
        };
        let src_was_active = self.spaces[src].active == Some(tab);
        let src_next = (src_was_active && src != dst)
            .then(|| self.successor(src, tab, false))
            .flatten();
        self.detach(tab);
        self.attach(tab, to, dst);
        let pin = if matches!(to, Dest::Pinned { .. }) {
            Pin::Pinned
        } else {
            Pin::Unpinned
        };
        let mut fx = self.lifecycle(tab, lifecycle::Event::PinnedChanged(pin));
        if src != dst {
            if src_was_active {
                self.spaces[src].active = src_next;
            }
            if self.spaces[dst].active.is_none() {
                self.spaces[dst].active = Some(tab);
            }
            // Only the active Space's visible tab matters to the engine.
            let shown = self.spaces[self.space_idx(self.active_space).expect("active")].active;
            if src_was_active && self.spaces[src].id == self.active_space {
                fx.push(self.show(shown));
            } else if self.spaces[dst].id == self.active_space && shown == Some(tab) {
                fx.push(Effect::ActivateTab(tab));
            }
        }
        fx
    }

    fn set_pinned(&mut self, tab: TabId, pin: bool) -> Vec<Effect> {
        let Some((si, place)) = self.place_of(tab) else {
            return vec![];
        };
        if matches!(place, Place::Pinned(_)) == pin {
            return vec![];
        }
        let space = self.spaces[si].id;
        let to = if pin {
            Dest::Pinned {
                space,
                index: usize::MAX,
            }
        } else {
            Dest::Normal { space, index: 0 }
        };
        self.move_tab(tab, to)
    }

    fn delete_folder(&mut self, folder: FolderId) -> Vec<Effect> {
        let Some(si) = self.folder_space(folder) else {
            return vec![];
        };
        let items = std::mem::take(&mut self.spaces[si].items);
        self.spaces[si].items = items
            .into_iter()
            .flat_map(|item| match item {
                Item::Folder(f) if f.id == folder => {
                    f.tabs.into_iter().map(Item::Tab).collect::<Vec<_>>()
                }
                other => vec![other],
            })
            .collect();
        vec![]
    }

    fn set_space_container(&mut self, space: SpaceId, container: ContainerId) -> Vec<Effect> {
        let Some(si) = self.space_idx(space) else {
            return vec![];
        };
        if !self.has_container(container) {
            return vec![];
        }
        // Existing tabs keep their live context: the default only seeds new tabs.
        self.spaces[si].container = container;
        vec![]
    }

    /// Closes the Space's tabs (Zen `#deleteWorkspaceOwnedTabs`). The alternative,
    /// moving them to a neighbour, would merge unrelated contexts: change it here only.
    fn delete_space(&mut self, space: SpaceId) -> Vec<Effect> {
        let Some(si) = self.space_idx(space) else {
            return vec![];
        };
        if self.spaces.len() == 1 {
            return vec![];
        }
        let gone = self.order_idx(si);
        let mut fx: Vec<Effect> = gone.iter().map(|&t| Effect::CloseTab(t)).collect();
        self.spaces.remove(si);
        self.forget(&gone);
        if self.active_space == space {
            let ni = si.saturating_sub(1);
            self.active_space = self.spaces[ni].id;
            fx.push(Effect::SpaceChanged(self.active_space));
            fx.push(self.show(self.spaces[ni].active));
        }
        fx
    }

    fn switch_space(&mut self, space: SpaceId) -> Vec<Effect> {
        let Some(si) = self.space_idx(space) else {
            return vec![];
        };
        if self.active_space == space {
            return vec![];
        }
        self.active_space = space;
        let mut fx = vec![Effect::SpaceChanged(space)];
        fx.push(self.show(self.spaces[si].active));
        fx
    }

    fn delete_container(&mut self, container: ContainerId) -> Vec<Effect> {
        if container == ContainerId::DEFAULT || !self.has_container(container) {
            return vec![];
        }
        // Live pages in the dead context are replaced by Default ones, as in a reopen.
        let affected: Vec<TabId> = self
            .tabs
            .values()
            .filter(|t| t.container == container)
            .map(|t| t.id)
            .collect();
        let fx = affected
            .into_iter()
            .flat_map(|t| self.reopen_in_container(t, ContainerId::DEFAULT))
            .collect();
        self.containers.retain(|c| c.id != container);
        for s in &mut self.spaces {
            if s.container == container {
                s.container = ContainerId::DEFAULT;
            }
        }
        fx
    }

    fn reopen_in_container(&mut self, tab: TabId, container: ContainerId) -> Vec<Effect> {
        let Some((si, _)) = self.place_of(tab) else {
            return vec![];
        };
        if !self.has_container(container) || self.tabs[&tab].container == container {
            return vec![];
        }
        let new = self.fresh_tab();
        let old = self.tabs[&tab].clone();
        self.tabs.insert(
            new,
            Tab {
                id: new,
                container,
                opener: None,
                lifecycle: Lifecycle {
                    pin: old.lifecycle.pin,
                    ..Lifecycle::new()
                },
                ..old.clone()
            },
        );
        let s = &mut self.spaces[si];
        let swap = |t: &mut TabId| {
            if *t == tab {
                *t = new;
            }
        };
        s.pinned.iter_mut().for_each(swap);
        for item in &mut s.items {
            match item {
                Item::Tab(t) => swap(t),
                Item::Folder(f) => f.tabs.iter_mut().for_each(swap),
            }
        }
        let was_active = s.active == Some(tab);
        if was_active {
            s.active = Some(new);
        }
        self.forget(&[tab]);
        let mut fx = vec![
            Effect::CloseTab(tab),
            Effect::CreateTab {
                tab: new,
                url: old.url,
                container,
            },
        ];
        if was_active && self.spaces[si].id == self.active_space {
            fx.push(Effect::ActivateTab(new));
        }
        fx
    }

    /// Runs `event` through the tab's lifecycle and returns its effects.
    fn lifecycle(&mut self, tab: TabId, event: lifecycle::Event) -> Vec<Effect> {
        let Some(t) = self.tabs.get_mut(&tab) else {
            return vec![];
        };
        let (next, fx) = step(tab, t.lifecycle, event);
        t.lifecycle = next;
        fx
    }

    /// Brings every lifecycle in line with the model after a command:
    /// tabs `gone` since `before` are closed, every tab but the visible one
    /// is hidden and the visible one shown. Each transition is a no-op for a
    /// tab already in that state.
    fn settle(&mut self, before: &BTreeMap<TabId, Lifecycle>) -> Vec<Effect> {
        let mut fx = vec![];
        for (&tab, &state) in before {
            if !self.tabs.contains_key(&tab) {
                fx.extend(step(tab, state, lifecycle::Event::Closed).1);
            }
        }
        let ids: Vec<TabId> = self.tabs.keys().copied().collect();
        let visible = self.visible_tab();
        let now = self.clock;
        for &tab in ids.iter().filter(|&&t| Some(t) != visible) {
            fx.extend(self.lifecycle(tab, lifecycle::Event::Hidden { now }));
        }
        if let Some(tab) = visible {
            fx.extend(self.lifecycle(tab, lifecycle::Event::Shown));
        }
        fx
    }

    /// Judges again, by the last tick's time and pressure, every tab that the
    /// command made eligible for sleep (it stopped being busy, or was unpinned),
    /// so it is not left awake until a next tick that may never come. Runs after
    /// [`Self::settle`], so a tab the same command brings into view is Active by
    /// then and the tick leaves it alone.
    fn rejudge(&mut self, before: &BTreeMap<TabId, Lifecycle>) -> Vec<Effect> {
        let eligible: Vec<TabId> = before
            .iter()
            .filter(|&(id, was)| {
                self.tabs.get(id).is_some_and(|t| {
                    let now = t.lifecycle;
                    (was.activity == Activity::Busy && now.activity == Activity::Idle)
                        || (was.pin == Pin::Pinned && now.pin == Pin::Unpinned)
                })
            })
            .map(|(&id, _)| id)
            .collect();
        let tick = lifecycle::Event::Tick {
            now: self.clock,
            pressure: self.pressure,
        };
        eligible
            .into_iter()
            .flat_map(|tab| self.lifecycle(tab, tick))
            .collect()
    }

    fn set_url(&mut self, tab: TabId, url: &str) -> bool {
        match self.tabs.get_mut(&tab) {
            Some(t) => {
                url.clone_into(&mut t.url);
                true
            }
            None => false,
        }
    }
}

/// The one call into `lifecycle::transition`, under the one sleep policy.
fn step(tab: TabId, state: Lifecycle, event: lifecycle::Event) -> (Lifecycle, Vec<Effect>) {
    let (next, fx) = lifecycle::transition(&SleepPolicy::default(), state, event);
    let fx = fx
        .into_iter()
        .map(|effect| Effect::Lifecycle { tab, effect })
        .collect();
    (next, fx)
}

/// The only way to change a [`Model`].
///
/// Tab lifecycles are settled at the end: whatever the command, the result has
/// exactly one visible, Active tab (if any), every other tab hidden, and the
/// lifecycle effects of getting there appended as [`Effect::Lifecycle`]
/// (closed tabs first, then hides, then the show, then the sleep of any hidden
/// tab the command made eligible for it).
pub fn apply(mut m: Model, command: Command) -> (Model, Vec<Effect>) {
    let before: BTreeMap<TabId, Lifecycle> =
        m.tabs.iter().map(|(&id, t)| (id, t.lifecycle)).collect();
    let mut fx = match command {
        Command::OpenCommandBar => vec![Effect::ShowCommandBar],
        Command::NewTab {
            url,
            container,
            opener,
        } => m.new_tab(url, container, opener),
        Command::CloseTab(t) => m.close_tab(t),
        Command::ActivateTab(t) => m.activate_tab(t),
        Command::MoveTab { tab, to } => m.move_tab(tab, to),
        Command::PinTab(t) => m.set_pinned(t, true),
        Command::UnpinTab(t) => m.set_pinned(t, false),
        Command::NewFolder { space, name } => {
            if let Some(si) = m.space_idx(space) {
                let id = FolderId(m.next_folder);
                m.next_folder += 1;
                m.spaces[si].items.push(Item::Folder(Folder {
                    id,
                    name,
                    tabs: vec![],
                }));
            }
            vec![]
        }
        Command::DeleteFolder(f) => m.delete_folder(f),
        Command::NewSpace {
            name,
            tint,
            container,
        } => {
            if m.has_container(container) {
                let id = SpaceId(m.next_space);
                m.next_space += 1;
                m.spaces.push(Space {
                    id,
                    name,
                    tint,
                    container,
                    pinned: vec![],
                    items: vec![],
                    active: None,
                });
            }
            vec![]
        }
        Command::RenameSpace { space, name } => {
            if let Some(si) = m.space_idx(space) {
                m.spaces[si].name = name;
            }
            vec![]
        }
        Command::SetSpaceContainer { space, container } => m.set_space_container(space, container),
        Command::DeleteSpace(s) => m.delete_space(s),
        Command::SwitchSpace(s) => m.switch_space(s),
        Command::NewContainer { name } => {
            let id = ContainerId(m.next_container);
            m.next_container += 1;
            m.containers.push(Container { id, name });
            vec![]
        }
        Command::DeleteContainer(c) => m.delete_container(c),
        Command::ReopenInContainer { tab, container } => m.reopen_in_container(tab, container),
        Command::Navigate { tab, url } => {
            if m.set_url(tab, &url) {
                vec![Effect::Navigate { tab, url }]
            } else {
                vec![]
            }
        }
        Command::UrlChanged { tab, url } => {
            m.set_url(tab, &url);
            vec![]
        }
        Command::TitleChanged { tab, title } => {
            if let Some(t) = m.tabs.get_mut(&tab) {
                t.title = title;
            }
            vec![]
        }
        Command::LoadingChanged { tab, loading } => {
            let activity = if loading {
                Activity::Busy
            } else {
                Activity::Idle
            };
            m.lifecycle(tab, lifecycle::Event::BusyChanged(activity))
        }
        Command::RendererGone(t) => m.lifecycle(t, lifecycle::Event::RendererGone),
        Command::Tick { now, pressure } => {
            m.clock = now;
            m.pressure = pressure;
            let ids: Vec<TabId> = m.tabs.keys().copied().collect();
            ids.into_iter()
                .flat_map(|t| m.lifecycle(t, lifecycle::Event::Tick { now, pressure }))
                .collect()
        }
    };
    fx.extend(m.settle(&before));
    fx.extend(m.rejudge(&before));
    (m, fx)
}

#[cfg(test)]
mod tests;
