//! The browser model the shell binds to: Spaces, containers, folders and tabs.
//!
//! All mutation goes through [`apply`], a pure function from (model, command) to
//! (model, effects). It is total: unknown ids and rejected commands return the model
//! unchanged with no effects; indices are clamped. The engine executes the effects.
//!
//! Rules ported, no code copied, from:
//! - Chromium `chrome/browser/ui/tabs/tab_strip_model.cc` (BSD-3-Clause, read at main
//!   2026-11): the opener rules. `DetermineNewSelectedIndex` (selection after closing the
//!   active tab: a tab it opened, else a tab its opener opened, else its opener, else the next,
//!   else the previous; each search looks after the tab first, then before it),
//!   `FixOpeners` (a closed tab's children inherit its opener), `AddWebContents` /
//!   `InsertTabAtImpl` (a page's tab remembers its opener; one opened in the foreground first
//!   makes every other tab forget theirs, `ForgetAllOpeners`) and `OnActiveTabChanged` (a user
//!   switch forgets every opener unless it moves between a tab and its opener or between two
//!   tabs of one opener). Not ported: the Ctrl+T "quick look-up" opener (wubble's shell tabs
//!   have none) and `TabNavigating` (wubble does not tell typed navigations from link ones).
//!   Also `ConstrainInsertionIndex` (pinned tabs form a prefix; indices clamp into their
//!   section) and pinned tabs never joining a group (here: folder).
//! - Zen `src/zen/spaces/ZenSpaceManager.mjs` (MPL-2.0): a workspace has a default
//!   container (0 = none), deleting a container resets workspaces using it, deleting
//!   a workspace closes its tabs (`#deleteWorkspaceOwnedTabs`), and each workspace
//!   remembers its selected tab (`lastSelectedWorkspaceTabs`).
//!
//! Placement is stored once, in the `Space` lists; `tabs` holds data only, so a tab
//! cannot be in two places. What types cannot express is checked by [`Model::check`].

use std::collections::BTreeMap;
use std::time::Instant;

use crate::busy::{Busy, Reason};
use crate::lifecycle::{self, Activity, Keep, Lifecycle, Pin};
use crate::pages::InternalPage;
use crate::routing::host_in;
use crate::sleep::{Pressure, SleepPolicy};
use crate::space::{
    Container, ContainerId, Folder, FolderId, Item, MAX_PANES, Rgb, Space, SpaceIcon, SpaceId,
    SplitGroup, even,
};
use crate::tab::{Liveness, Tab, TabId};
use crate::tidy;
use crate::window::{Geometry, Shown, Window, WindowId, WindowTarget};

mod windows;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Place {
    Pinned(usize),
    /// Index into the Space's top-level items.
    Normal(usize),
    Folder(FolderId, usize),
}

/// A link opened in a peek over its window's page (Shift+click): a card the user can close or
/// promote. Its tab sits right after the opener like a background tab, but it is no row of the
/// sidebar and no part of a saved session; it is on screen (Active) while the peek is open.
/// Written new (the design bucket has no peek).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Glance {
    pub window: WindowId,
    /// The page the link was on; the peek closes when it leaves the window's screen.
    pub opener: TabId,
    pub tab: TabId,
}

/// What a peek becomes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GlanceTo {
    /// A tab of its Space, shown.
    Tab,
    /// The pane beside its opener.
    Split,
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
    /// A link opened in the background (middle click, Ctrl+click): placed right after its
    /// `opener`, never shown, the opener relation kept.
    BackgroundTab {
        url: String,
        opener: TabId,
    },
    /// A tab an extension opened (tabs.create, runtime.openOptionsPage, clients.openWindow):
    /// last in `space` (`None`: the active Space), in the asking extension's `container` (its
    /// Chrome profile, so the tab is the extension's to see), shown when `show`.
    ExtensionTab {
        url: String,
        container: ContainerId,
        space: Option<SpaceId>,
        show: bool,
    },
    /// A tab at the end of `space`, in its container, never shown: an agent session's tab
    /// (`agent`), which lives in a Space no window may show. The Space's active tab when it
    /// has none, so showing the Space shows it.
    OpenInSpace {
        space: SpaceId,
        url: String,
    },
    CloseTab(TabId),
    /// Closes each of these tabs (the tab menu's "Close Tabs Below", a folder's "Close Folder").
    CloseTabs(Vec<TabId>),
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
    /// Fold the folder's tabs away in the sidebar, or show them.
    SetFolderCollapsed {
        folder: FolderId,
        collapsed: bool,
    },
    RenameFolder {
        folder: FolderId,
        name: String,
    },
    /// A new folder holding the tab, where the tab was (the tab menu's "New folder with tab").
    /// A pinned tab or one in a folder goes into a new folder at the end of its Space.
    FolderWithTab {
        tab: TabId,
        name: String,
    },
    /// Open `url` in a peek over `window`'s page `opener` (one peek per window: an open one is
    /// closed first). A tab instead unless the window shows a Space with the opener on screen
    /// and the opener is not a peek.
    OpenGlance {
        window: WindowId,
        opener: TabId,
        url: String,
    },
    /// Close the window's peek (its tab closes).
    CloseGlance(WindowId),
    /// The window's peek becomes a tab, or the pane beside its opener.
    PromoteGlance {
        window: WindowId,
        to: GlanceTo,
    },
    /// A new tab on the same page in the same container, right after it, shown.
    DuplicateTab(TabId),
    /// Closes every other tab of the tab's Space except the pinned ones (Chromium
    /// `CloseOtherTabs`), and shows it.
    CloseOthers(TabId),
    NewSpace {
        name: String,
        tint: Rgb,
        container: ContainerId,
    },
    RenameSpace {
        space: SpaceId,
        name: String,
    },
    /// The Space's colour (the backdrop's dye follows it).
    SetSpaceTint {
        space: SpaceId,
        tint: Rgb,
    },
    SetSpaceIcon {
        space: SpaceId,
        icon: SpaceIcon,
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
    /// One reason the tab is busy started (`on`) or ended. A tab with any reason is not
    /// frozen or discarded until the last one ends (`busy::Busy`).
    Busy {
        tab: TabId,
        reason: Reason,
        on: bool,
    },
    /// Discard a hidden tab now (Task manager). Refused for a visible or busy tab, and for a
    /// pinned one the policy keeps alive.
    DiscardNow(TabId),
    /// The tab's renderer process died.
    RendererGone(TabId),
    /// Put `tab` (of the active Space) beside the focused page: opens a split of the two, or
    /// adds a pane after the focused one (at most [`MAX_PANES`]); the shares even out and `tab`
    /// takes focus. The design's `split_with` (browser/model.ts).
    SplitWith(TabId),
    /// A new tab loading `url`, put beside the focused page (the design's `open_beside`).
    OpenBeside {
        url: String,
    },
    /// Take the tab out of its split; it stays open. A split of one is no split. If it was the
    /// focused page, focus passes to the page that grows into its place.
    Unsplit(TabId),
    /// Where the Space's seams rest (from the split physics). Refused unless they fit its panes.
    ResizeSplit {
        space: SpaceId,
        stops: Vec<f64>,
    },
    /// Swap the tab with its neighbour in its split (`right`: the one to its right). The seams
    /// stay where they are.
    SwapPane {
        tab: TabId,
        right: bool,
    },
    /// Shows an internal page: the active Space's tab already on it, else a new tab.
    OpenInternal(InternalPage),
    /// A new Main window, focused, on `space` if no other window shows it (else that window
    /// is focused), or on the first Space no window shows, or on a new Space.
    NewWindow {
        space: Option<SpaceId>,
    },
    /// The last Main window is not closed: [`Effect::LastWindowClosed`] asks the host to quit.
    /// A Little or Popup window closes its tab with it.
    CloseWindow(WindowId),
    /// The platform focused this window.
    FocusWindow(WindowId),
    /// The window's size or position changed (kept for the session).
    ResizeWindow {
        window: WindowId,
        geometry: Geometry,
    },
    /// Shows `tab` in a Main window (moved into the Space it shows) and focuses it; a tab of a
    /// Little or Popup window leaves that window, which closes.
    MoveTabToWindow {
        tab: TabId,
        to: WindowTarget,
    },
    /// A page's `window.open` with a size: a new tab beside `opener`, shown in a Popup window.
    /// `pos` is the window's outer position (physical px; its `left`/`top`), used only with a
    /// size, which gives the window a geometry.
    OpenPopupWindow {
        url: String,
        opener: TabId,
        size: Option<(u32, u32)>,
        pos: Option<(i32, i32)>,
    },
    /// A link from another app: a new tab in `space` (`None`: the active one) shown in a
    /// Little window, in `container` (`None`: the Space's own).
    OpenLittle {
        url: String,
        space: Option<SpaceId>,
        container: Option<ContainerId>,
    },
    /// The Little window's tab becomes an ordinary tab of `space`, shown; the window closes.
    PromoteLittle {
        window: WindowId,
        space: SpaceId,
    },
    /// Picture-in-picture: `tab`'s frames in a Pip window of their own, unfocused (the tab stays
    /// its Space's; its Main window shows a card for it). Ignored for an unknown tab, a peek, a
    /// Little or Popup window's tab, or while a Pip window is open.
    OpenPip {
        tab: TabId,
    },
    /// The Pip window closes; its tab is shown in its Space's window again.
    ClosePip,
    /// The Pip window closes and its tab is shown and focused where it lives.
    ReturnPip,
    /// Hidden tabs are judged by this policy from the next tick on.
    SetSleepPolicy(SleepPolicy),
    /// Tabs of this Space are never put to sleep (or are again).
    SetSpaceNeverSleep {
        space: SpaceId,
        on: bool,
    },
    /// An agent session made the Space: an agent Space for the rest of its life
    /// ([`Space::agent`]); nothing clears it.
    MarkAgentSpace(SpaceId),
    /// Tabs on these sites (a host covers its subdomains) are never put to sleep.
    SetNeverSleepSites(Vec<String>),
    /// Groups the Space's unpinned top-level tabs into one folder per site (`tidy::plan_groups`).
    /// [`Effect::Tidied`] carries the layout before, for [`Command::SetLayout`] to undo it.
    TidyTabs {
        space: SpaceId,
    },
    /// Puts the Space's unpinned tabs back in `items` (a layout `Effect::Tidied` gave): tabs and
    /// folders that are gone are dropped, tabs it does not mention (opened since) follow at the
    /// end, and a folder left empty is dropped.
    SetLayout {
        space: SpaceId,
        items: Vec<Item>,
    },
    /// Coarse lifecycle tick: hidden tabs are put to sleep by the sleep policy.
    /// `now` also becomes the model's clock, the time later hides are stamped with; `wall_ms`
    /// (unix milliseconds, 0 when unknown) is the wall clock that stamps `Tab::last_active_ms`.
    Tick {
        now: Instant,
        wall_ms: i64,
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
    /// Open a platform window for this model window.
    WindowOpened(WindowId),
    /// Close this window's platform window.
    WindowClosed(WindowId),
    /// Raise and focus this window.
    FocusWindow(WindowId),
    /// The user closed the last Main window: quit.
    LastWindowClosed,
    /// Tidy grouped tabs of this Space; `before` is the layout to undo it with.
    Tidied {
        space: SpaceId,
        before: Vec<Item>,
    },
    /// The container is gone: its cookies, storage and cache go with it.
    ContainerDeleted(ContainerId),
}

/// One per app: every window's tabs and Spaces.
#[derive(Debug, Clone, PartialEq)]
pub struct Model {
    pub spaces: Vec<Space>,
    /// `containers[0]` is Default.
    pub containers: Vec<Container>,
    pub tabs: BTreeMap<TabId, Tab>,
    /// The open windows, least recently focused first: the last one is focused, and the last
    /// Main one shows the active Space ([`Model::active_space`]). Never without a Main window.
    pub windows: Vec<Window>,
    /// The open peeks, at most one per window (`Command::OpenGlance`).
    pub glances: Vec<Glance>,
    /// Where the last Pip window was (`Command::ResizeWindow` on it), for the next one in this
    /// run.
    pub pip_geometry: Option<Geometry>,
    /// The time hides are stamped with: set at creation and by `Command::Tick`,
    /// so `apply` itself never reads a clock.
    clock: Instant,
    /// The memory pressure of the last `Command::Tick`. With `clock` it lets a
    /// hidden tab that becomes eligible for sleep between ticks be judged at once.
    pressure: Pressure,
    /// The wall clock (unix milliseconds) of the last tick; 0 until the first one. Hides are
    /// stamped with it ([`Tab::last_active_ms`]).
    wall_ms: i64,
    /// When hidden tabs go to sleep (the settings' sleep rows).
    policy: SleepPolicy,
    /// Sites whose tabs never sleep (the `sleep.never_sites` row).
    never_sites: Vec<String>,
    next_tab: u64,
    next_space: u32,
    next_folder: u32,
    next_container: u32,
    next_window: u32,
}

/// The tint of the first Space and of Spaces a new window makes.
const DEFAULT_TINT: Rgb = Rgb(0x6a, 0x8f, 0xff);

impl Model {
    /// The Space Settings' "+" and the New Space action add: [`crate::space::next_space`]
    /// beside the Spaces there are, in the Default container.
    pub fn new_space(&self) -> Command {
        let (name, tint) =
            crate::space::next_space(self.spaces.iter().map(|s| (s.name.as_str(), s.tint)));
        Command::NewSpace {
            name,
            tint,
            container: ContainerId::DEFAULT,
        }
    }
}

impl Model {
    /// One empty Space and the Default container; `now` starts the model's clock.
    pub fn new(now: Instant) -> Self {
        let space = Space {
            id: SpaceId(0),
            name: "Space 1".into(),
            tint: DEFAULT_TINT,
            icon: SpaceIcon::Personal,
            container: ContainerId::DEFAULT,
            pinned: vec![],
            items: vec![],
            active: None,
            split: None,
            never_sleep: false,
            agent: false,
        };
        Self {
            windows: vec![Window::main(WindowId(0), space.id)],
            spaces: vec![space],
            containers: vec![Container {
                id: ContainerId::DEFAULT,
                name: "Default".into(),
            }],
            tabs: BTreeMap::new(),
            glances: vec![],
            pip_geometry: None,
            clock: now,
            pressure: Pressure::None,
            wall_ms: 0,
            policy: SleepPolicy::default(),
            never_sites: vec![],
            next_tab: 1,
            next_space: 1,
            next_folder: 0,
            next_container: 1,
            next_window: 1,
        }
    }

    /// The sleep policy hidden tabs are judged by.
    pub fn policy(&self) -> SleepPolicy {
        self.policy
    }

    /// The sites whose tabs never sleep.
    pub fn never_sites(&self) -> &[String] {
        &self.never_sites
    }

    /// The model's clock: the time it was started with or last ticked to.
    pub fn clock(&self) -> Instant {
        self.clock
    }

    /// Rebuilds a model from stored parts. The id counters are not part of the parts: they are
    /// derived as the highest id in use plus one, so new ids never collide with restored ones.
    /// `now` starts the model's clock. The result must pass [`Model::check`].
    pub fn from_parts(
        spaces: Vec<Space>,
        containers: Vec<Container>,
        tabs: Vec<Tab>,
        windows: Vec<Window>,
        now: Instant,
    ) -> Result<Model, String> {
        Self::build_parts(
            spaces,
            containers,
            tabs,
            windows,
            vec![],
            None,
            now,
            Pressure::None,
            0,
            SleepPolicy::default(),
            vec![],
        )
    }

    /// Rebuilds this model with updated Spaces, containers, tabs and windows while keeping live
    /// runtime state (open peeks, PiP geometry, sleep policy, never-sleep sites, clock and
    /// pressure). Used when merging imported Spaces into a live model.
    pub fn with_parts(
        &self,
        spaces: Vec<Space>,
        containers: Vec<Container>,
        tabs: Vec<Tab>,
        windows: Vec<Window>,
    ) -> Result<Model, String> {
        Self::build_parts(
            spaces,
            containers,
            tabs,
            windows,
            self.glances.clone(),
            self.pip_geometry.clone(),
            self.clock,
            self.pressure,
            self.wall_ms,
            self.policy,
            self.never_sites.clone(),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn build_parts(
        spaces: Vec<Space>,
        containers: Vec<Container>,
        tabs: Vec<Tab>,
        windows: Vec<Window>,
        glances: Vec<Glance>,
        pip_geometry: Option<Geometry>,
        clock: Instant,
        pressure: Pressure,
        wall_ms: i64,
        policy: SleepPolicy,
        never_sites: Vec<String>,
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
            next_window: to_u32(next(windows.iter().map(|w| u64::from(w.id.0)).max(), 0))?,
            spaces,
            containers,
            tabs: tabs.into_iter().map(|t| (t.id, t)).collect(),
            windows,
            glances,
            pip_geometry,
            clock,
            pressure,
            wall_ms,
            policy,
            never_sites,
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

    /// Whether `tab` is in `space` and lives in `container`: what an agent session reaches (a
    /// tab moved into its Space keeps its own container, so stays out of reach).
    pub fn holds(&self, space: SpaceId, container: ContainerId, tab: TabId) -> bool {
        self.location(tab).is_some_and(|(s, _)| s == space)
            && self
                .tabs
                .get(&tab)
                .is_some_and(|t| t.container == container)
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

    /// The focused page of the focused window: a Little or Popup window's tab, else the active
    /// tab of the active Space.
    pub fn visible_tab(&self) -> Option<TabId> {
        self.windows.last().and_then(|w| self.focused_tab(w))
    }

    /// Every tab on screen in any window: each window's [`Model::window_tabs`], focused window
    /// last. Exactly these are Active.
    pub fn visible_tabs(&self) -> Vec<TabId> {
        self.windows
            .iter()
            .flat_map(|w| self.window_tabs(w.id))
            .collect()
    }

    /// The split shown in the active Space, if any.
    pub fn shown_split(&self) -> Option<&SplitGroup> {
        self.space_split(self.space_idx(self.active_space())?)
    }

    /// The Space's split if it is shown: its active tab is one of the panes.
    fn space_split(&self, si: usize) -> Option<&SplitGroup> {
        let s = &self.spaces[si];
        s.split
            .as_ref()
            .filter(|g| s.active.is_some_and(|a| g.tabs.contains(&a)))
    }

    /// What a Space shows in the window that shows it, left to right: its split when its
    /// active tab is in it, otherwise just the active tab.
    fn space_tabs(&self, si: usize) -> Vec<TabId> {
        match self.space_split(si) {
            Some(g) => g.tabs.clone(),
            None => self.spaces[si].active.into_iter().collect(),
        }
    }

    /// Whether the active Space's window has room for another pane. The limit is per window:
    /// what other windows show does not count.
    pub fn pane_room(&self) -> bool {
        self.space_idx(self.active_space())
            .is_some_and(|si| self.space_tabs(si).len() < MAX_PANES)
    }

    /// The split `tab` is a pane of, shown or not.
    pub fn split_of(&self, tab: TabId) -> Option<&SplitGroup> {
        self.spaces
            .iter()
            .filter_map(|s| s.split.as_ref())
            .find(|g| g.tabs.contains(&tab))
    }

    /// The tab "split with next tab" puts beside the focused page: the next one down in the
    /// active Space's order, wrapping, that is not already in its split (the design's
    /// `split_next`). `None` when there is none, or the shown split is full.
    pub fn split_candidate(&self) -> Option<TabId> {
        if !self.pane_room() {
            return None;
        }
        let si = self.space_idx(self.active_space())?;
        let s = &self.spaces[si];
        let order = self.order_idx(si);
        let a = order.iter().position(|&t| Some(t) == s.active)?;
        let in_split = |t: &TabId| s.split.as_ref().is_some_and(|g| g.tabs.contains(t));
        order[a + 1..]
            .iter()
            .chain(&order[..a])
            .copied()
            .find(|t| !in_split(t))
    }

    /// The tab `delta` steps from the visible one in the active Space's
    /// order (pinned first), wrapping at both ends.
    pub fn tab_step(&self, delta: isize) -> Option<TabId> {
        cycle(&self.order(self.active_space()), self.visible_tab(), delta)
    }

    /// The active Space's tab at `index` (0-based) in its order, if there is one.
    pub fn nth_tab(&self, index: usize) -> Option<TabId> {
        self.order(self.active_space()).get(index).copied()
    }

    /// The active Space's last tab in its order.
    pub fn last_tab(&self) -> Option<TabId> {
        self.order(self.active_space()).last().copied()
    }

    /// The Space `delta` steps from the active one, wrapping at both ends.
    pub fn space_step(&self, delta: isize) -> Option<SpaceId> {
        let ids: Vec<SpaceId> = self.spaces.iter().map(|s| s.id).collect();
        cycle(&ids, Some(self.active_space()), delta)
    }

    /// The Space at `index` (0-based), if there is one.
    pub fn nth_space(&self, index: usize) -> Option<SpaceId> {
        self.spaces.get(index).map(|s| s.id)
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
            // a tab shown in its own window is never the Space's active tab
            let windowed = |t: &TabId| self.windowed(*t).is_some();
            match s.active {
                Some(a) if order.contains(&a) && !windowed(&a) => {}
                None if order.iter().all(windowed) => {}
                a => return Err(format!("space {:?} bad active {a:?}", s.id)),
            }
            if let Some(g) = &s.split {
                if !(2..=MAX_PANES).contains(&g.tabs.len()) {
                    return Err(format!("space {:?} split of {}", s.id, g.tabs.len()));
                }
                if !SplitGroup::stops_fit(&g.stops, g.tabs.len()) {
                    return Err(format!("space {:?} bad stops {:?}", s.id, g.stops));
                }
                for (i, t) in g.tabs.iter().enumerate() {
                    if !order.contains(t) || g.tabs[..i].contains(t) || windowed(t) {
                        return Err(format!("space {:?} bad split pane {t:?}", s.id));
                    }
                }
            }
        }
        if let Some(t) = self.tabs.keys().find(|t| !placed.contains_key(t)) {
            return Err(format!("tab {t:?} in map but unplaced"));
        }
        self.check_windows()?;
        for (i, g) in self.glances.iter().enumerate() {
            if self.glances[..i].iter().any(|o| o.window == g.window) {
                return Err(format!("two peeks in window {:?}", g.window));
            }
            if !self.tabs.contains_key(&g.tab) || !self.tabs.contains_key(&g.opener) {
                return Err(format!("peek {g:?} names a gone tab"));
            }
            if self.spaces.iter().any(|s| s.active == Some(g.tab)) || self.split_of(g.tab).is_some()
            {
                return Err(format!("peek {g:?} is an ordinary tab"));
            }
        }
        let visible = self.visible_tabs();
        for t in self.tabs.values() {
            let shown = visible.contains(&t.id);
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
        // A tab shown in its own window is never a Space's active tab.
        let order: Vec<TabId> = self
            .order_idx(si)
            .into_iter()
            .filter(|&t| t == tab || self.windowed(t).is_none())
            .collect();
        let i = order.iter().position(|&t| t == tab)?;
        if use_openers {
            // `GetIndexOfNextWebContentsOpenedBy`: after the tab, then before it
            let opened_by = |o: TabId| {
                let by = |t: &&TabId| self.tabs[*t].opener == Some(o);
                order[i + 1..]
                    .iter()
                    .find(by)
                    .or_else(|| order[..i].iter().rev().find(by))
                    .copied()
            };
            let opener = self.tabs[&tab].opener.filter(|o| order.contains(o));
            if let Some(t) = opened_by(tab)
                .or_else(|| opener.and_then(opened_by))
                .or(opener)
            {
                return Some(t);
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

    /// Drops closed tabs from the map; the tabs one opened inherit its opener (Chromium
    /// `FixOpeners`), and no opener names a closed tab.
    fn forget(&mut self, gone: &[TabId]) {
        for g in gone {
            let Some(closed) = self.tabs.remove(g) else {
                continue;
            };
            for t in self.tabs.values_mut() {
                if t.opener == Some(*g) {
                    t.opener = closed.opener.filter(|&o| o != t.id);
                }
            }
        }
        for t in self.tabs.values_mut() {
            if t.opener.is_some_and(|o| gone.contains(&o)) {
                t.opener = None;
            }
        }
    }

    /// Every tab of the Space forgets its opener (Chromium `ForgetAllOpeners`; a Space is a
    /// tab strip).
    fn forget_openers(&mut self, si: usize) {
        for t in self.order_idx(si) {
            if let Some(t) = self.tabs.get_mut(&t) {
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
        // A popup from a hidden tab must not take over the screen.
        let show =
            opener.is_none_or(|o| self.windows.iter().any(|w| self.focused_tab(w) == Some(o)));
        let Some((id, si, create)) = self.place_new_tab(url, container, opener, None) else {
            return vec![];
        };
        // A page's tab opened in the foreground: one opener relation at a time (Chromium
        // `InsertTabAtImpl`, ADD_INHERIT_OPENER with ADD_ACTIVE).
        if show && let Some(o) = self.tabs[&id].opener {
            self.forget_openers(si);
            if let Some(t) = self.tabs.get_mut(&id) {
                t.opener = Some(o);
            }
        }
        let mut fx = vec![create];
        // one opened from a Little or Popup window's page shows in the Main window
        if show {
            self.spaces[si].active = Some(id);
            fx.push(Effect::ActivateTab(id));
        }
        fx
    }

    fn extension_tab(
        &mut self,
        url: String,
        container: ContainerId,
        space: Option<SpaceId>,
        show: bool,
    ) -> Vec<Effect> {
        let Some((id, si, create)) = self.place_new_tab(url, Some(container), None, space) else {
            return vec![];
        };
        let mut fx = vec![create];
        if show {
            self.spaces[si].active = Some(id);
            fx.push(Effect::ActivateTab(id));
        }
        fx
    }

    /// A new tab, placed but not shown: after its live `opener`, else last in `space` (`None`:
    /// the active Space). Returns it, its Space and its `CreateTab` effect.
    fn place_new_tab(
        &mut self,
        url: String,
        container: Option<ContainerId>,
        opener: Option<TabId>,
        space: Option<SpaceId>,
    ) -> Option<(TabId, usize, Effect)> {
        if container.is_some_and(|c| !self.has_container(c)) {
            return None;
        }
        let opener_space = opener.and_then(|o| self.place_of(o)).map(|(s, _)| s);
        let opener = opener.filter(|_| opener_space.is_some());
        let si = match space {
            Some(space) => self.space_idx(space)?,
            None => opener_space
                .unwrap_or_else(|| self.space_idx(self.active_space()).expect("active space")),
        };
        // A page's pop-up or link keeps its opener's cookies: the opener's container.
        let inherited = opener.and_then(|o| self.tabs.get(&o)).map(|t| t.container);
        let opener = opener.filter(|_| opener_space == Some(si));
        let default = self.spaces[si].container;
        let id = self.fresh_tab();
        let eff = container.or(inherited).unwrap_or(default);
        let tab = Tab {
            id,
            url: url.clone(),
            title: String::new(),
            lifecycle: Lifecycle::new(),
            container: eff,
            opener,
            last_active_ms: 0,
            busy: Busy::NONE,
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
        let create = Effect::CreateTab {
            tab: id,
            url,
            container: eff,
        };
        Some((id, si, create))
    }

    fn close_tab(&mut self, tab: TabId) -> Vec<Effect> {
        let Some((si, _)) = self.place_of(tab) else {
            return vec![];
        };
        // its peek goes first, so it is never the page that takes the closed one's place
        let mut fx = vec![];
        if let Some(g) = self.glances.iter().find(|g| g.opener == tab).copied() {
            self.glances.retain(|o| o.tab != g.tab);
            fx.extend(self.close_tab(g.tab));
        }
        self.glances.retain(|g| g.tab != tab);
        let was_active = self.spaces[si].active == Some(tab);
        // A closed pane leaves its split; the page that grows into its place takes focus.
        let heir = self.leave_split(tab);
        let next = if was_active {
            heir.or_else(|| self.successor(si, tab, true))
        } else {
            self.spaces[si].active
        };
        self.detach(tab);
        self.forget(&[tab]);
        self.spaces[si].active = next;
        fx.push(Effect::CloseTab(tab));
        if was_active && self.spaces[si].id == self.active_space() {
            fx.push(self.show(next));
        }
        fx
    }

    fn duplicate_tab(&mut self, tab: TabId) -> Vec<Effect> {
        let Some(t) = self.tabs.get(&tab) else {
            return vec![];
        };
        let (url, container) = (t.url.clone(), t.container);
        let mut fx = self.new_tab(url, Some(container), Some(tab));
        if let Some(Effect::CreateTab { tab: new, .. }) = fx.first().cloned()
            && !fx.contains(&Effect::ActivateTab(new))
        {
            fx.extend(self.activate_tab(new));
        }
        fx
    }

    fn close_others(&mut self, tab: TabId) -> Vec<Effect> {
        let Some((si, _)) = self.place_of(tab) else {
            return vec![];
        };
        let others: Vec<TabId> = self
            .order_idx(si)
            .into_iter()
            .filter(|&t| t != tab && !self.is_pinned(t))
            .collect();
        let mut fx = self.activate_tab(tab);
        for t in others {
            fx.extend(self.close_tab(t));
        }
        fx
    }

    fn activate_tab(&mut self, tab: TabId) -> Vec<Effect> {
        let Some((si, _)) = self.place_of(tab) else {
            return vec![];
        };
        // A Little or Popup window's tab is shown there: bring that window forward.
        if let Some(w) = self.windowed(tab) {
            return self.focus(w).into_iter().collect();
        }
        let id = self.spaces[si].id;
        if self.spaces[si].active == Some(tab) && self.active_space() == id {
            return vec![];
        }
        let mut fx = self.set_active_space(id);
        // Chromium `OnActiveTabChanged` on a user's switch: the openers stay only while the
        // switch is between a tab and its opener, or between two tabs of one opener.
        let old = self.spaces[si].active;
        let opener = |t: TabId| self.tabs.get(&t).and_then(|t| t.opener);
        let (was, now) = (old.and_then(opener), opener(tab));
        if was != now && now != old && was != Some(tab) {
            self.forget_openers(si);
        }
        self.spaces[si].active = Some(tab);
        fx.push(Effect::ActivateTab(tab));
        fx
    }

    /// The window's peek, if one is open.
    pub fn glance_of(&self, window: WindowId) -> Option<Glance> {
        self.glances.iter().copied().find(|g| g.window == window)
    }

    /// Whether `tab` is a peek's (no sidebar row, not saved).
    pub fn is_glance(&self, tab: TabId) -> bool {
        self.glances.iter().any(|g| g.tab == tab)
    }

    /// Whether closing `tab` by `command` (applied next) puts it on the user's recently-closed
    /// list: not a peek's, not an empty tab, and not a tab of an agent Space that `command`
    /// deletes (a session's teardown or the owner's dustbin: its pages are none of the user's).
    /// A tab an agent session drives is excluded by the caller, which knows the sessions.
    pub fn closes_into_recent(&self, tab: TabId, command: &Command) -> bool {
        let deleted_agent_space = match command {
            Command::DeleteSpace(space) => {
                self.location(tab).is_some_and(|(at, _)| at == *space)
                    && self.spaces.iter().any(|s| s.id == *space && s.agent)
            }
            _ => false,
        };
        !self.is_glance(tab)
            && !deleted_agent_space
            && self.tabs.get(&tab).is_some_and(|t| !t.url.is_empty())
    }

    /// The model as a session keeps it: without the peeks and their tabs, and without the
    /// `transient` Spaces (an agent session's), their tabs and their own containers (a restart
    /// never brings one back).
    pub fn for_session(&self, transient: &[SpaceId]) -> Model {
        let mut m = self.clone();
        let gone: Vec<TabId> = m.glances.drain(..).map(|g| g.tab).collect();
        for &t in &gone {
            m.detach(t);
        }
        m.forget(&gone);
        for &space in transient {
            let Some(container) = m.space_idx(space).map(|si| m.spaces[si].container) else {
                continue;
            };
            m = apply(m, Command::DeleteSpace(space)).0;
            m = apply(m, Command::DeleteContainer(container)).0;
        }
        m
    }

    /// A peek needs a window showing a Space and an opener on screen there that is not itself a
    /// peek; otherwise the link opens as a tab, as a plain click on it would (a Little or Popup
    /// window, a link inside a peek).
    fn open_glance(&mut self, window: WindowId, opener: TabId, url: String) -> Vec<Effect> {
        let peekable = matches!(self.window(window).map(|w| w.shown), Some(Shown::Space(_)))
            && self.window_tabs(window).contains(&opener)
            && !self.is_glance(opener);
        if !peekable {
            return self.new_tab(url, None, Some(opener));
        }
        let mut fx = match self.glance_of(window) {
            Some(g) => self.close_tab(g.tab),
            None => vec![],
        };
        if let Some((tab, _, create)) = self.place_new_tab(url, None, Some(opener), None) {
            self.glances.push(Glance {
                window,
                opener,
                tab,
            });
            fx.push(create);
        }
        fx
    }

    fn promote_glance(&mut self, window: WindowId, to: GlanceTo) -> Vec<Effect> {
        let Some(g) = self.glance_of(window) else {
            return vec![];
        };
        self.glances.retain(|o| o.window != window);
        if to == GlanceTo::Split {
            let fx = self.split_with(g.tab);
            if !fx.is_empty() || self.window_tabs(window).contains(&g.tab) {
                return fx;
            }
        }
        self.activate_tab(g.tab)
    }

    /// Peeks whose tab is gone are forgotten; one whose tab became an ordinary on-screen tab
    /// (activated) is promoted; one whose opener left its window's screen (closed, another tab
    /// or Space shown, the window closed) closes.
    fn tidy_glances(&mut self) -> Vec<Effect> {
        self.glances.retain(|g| self.tabs.contains_key(&g.tab));
        let shown = |m: &Model, g: &Glance| -> Vec<TabId> {
            match m.window(g.window).map(|w| w.shown) {
                // an opener in picture-in-picture is off its window's screen
                Some(Shown::Space(s)) => m.space_idx(s).map_or(vec![], |si| {
                    let pip = m.pip_tab();
                    let mut tabs = m.space_tabs(si);
                    tabs.retain(|t| Some(*t) != pip);
                    tabs
                }),
                _ => vec![],
            }
        };
        let promoted: Vec<TabId> = self
            .glances
            .iter()
            .filter(|g| {
                shown(self, g).contains(&g.tab)
                    || self.spaces.iter().any(|s| s.active == Some(g.tab))
                    || self.split_of(g.tab).is_some()
            })
            .map(|g| g.tab)
            .collect();
        self.glances.retain(|g| !promoted.contains(&g.tab));
        let stale: Vec<TabId> = self
            .glances
            .iter()
            .filter(|g| !shown(self, g).contains(&g.opener))
            .map(|g| g.tab)
            .collect();
        let mut fx = vec![];
        for tab in stale {
            fx.extend(self.close_tab(tab));
        }
        self.glances.retain(|g| self.tabs.contains_key(&g.tab));
        fx
    }

    fn folder_with_tab(&mut self, tab: TabId, name: String) -> Vec<Effect> {
        let Some((si, place)) = self.place_of(tab) else {
            return vec![];
        };
        let id = FolderId(self.next_folder);
        self.next_folder += 1;
        let folder = |tabs| {
            Item::Folder(Folder {
                id,
                name,
                tabs,
                collapsed: false,
            })
        };
        if let Place::Normal(i) = place {
            self.spaces[si].items[i] = folder(vec![tab]);
            return vec![];
        }
        self.spaces[si].items.push(folder(vec![]));
        self.move_tab(
            tab,
            Dest::Folder {
                folder: id,
                index: 0,
            },
        )
    }

    fn move_tab(&mut self, tab: TabId, to: Dest) -> Vec<Effect> {
        let (Some((src, _)), Some(dst)) = (self.place_of(tab), self.dest_space(to)) else {
            return vec![];
        };
        let src_was_active = self.spaces[src].active == Some(tab);
        // A pane moved to another Space leaves its split there.
        let heir = (src != dst).then(|| self.leave_split(tab)).flatten();
        let src_next = (src_was_active && src != dst)
            .then(|| heir.or_else(|| self.successor(src, tab, false)))
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
            if self.spaces[dst].active.is_none() && self.windowed(tab).is_none() {
                self.spaces[dst].active = Some(tab);
            }
            // Only the active Space's visible tab matters to the engine.
            let shown = self.spaces[self.space_idx(self.active_space()).expect("active")].active;
            if src_was_active && self.spaces[src].id == self.active_space() {
                fx.push(self.show(shown));
            } else if self.spaces[dst].id == self.active_space() && shown == Some(tab) {
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
        let focused = self.active_space() == space;
        let shown_in = self.shown_by(space);
        self.spaces.remove(si);
        self.forget(&gone);
        // The window that showed it takes the nearest Space no other window shows; with none
        // left it closes (it is not the last Main window: that one has every other Space free).
        if let Some(wi) = shown_in {
            match self.free_space(si.saturating_sub(1)) {
                Some(next) => self.windows[wi].shown = crate::window::Shown::Space(next),
                None => fx.push(Effect::WindowClosed(self.windows.remove(wi).id)),
            }
        }
        if focused {
            let active = self.active_space();
            fx.push(Effect::SpaceChanged(active));
            fx.push(self.show(self.spaces[self.space_idx(active).expect("active")].active));
        }
        fx
    }

    fn switch_space(&mut self, space: SpaceId) -> Vec<Effect> {
        let Some(si) = self.space_idx(space) else {
            return vec![];
        };
        if self.active_space() == space {
            return vec![];
        }
        let mut fx = self.set_active_space(space);
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
        let mut fx: Vec<Effect> = affected
            .into_iter()
            .flat_map(|t| self.reopen_in_container(t, ContainerId::DEFAULT))
            .collect();
        fx.push(Effect::ContainerDeleted(container));
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
                busy: Busy::NONE,
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
        if let Some(g) = &mut s.split {
            g.tabs.iter_mut().for_each(swap);
        }
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
        for w in &mut self.windows {
            use crate::window::Shown;
            w.shown = match w.shown {
                Shown::Little(t) if t == tab => Shown::Little(new),
                Shown::Popup(t) if t == tab => Shown::Popup(new),
                Shown::Pip(t) if t == tab => Shown::Pip(new),
                shown => shown,
            };
        }
        for g in &mut self.glances {
            if g.opener == tab {
                g.opener = new;
            }
            if g.tab == tab {
                g.tab = new;
            }
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
        if was_active && self.spaces[si].id == self.active_space() {
            fx.push(Effect::ActivateTab(new));
        }
        fx
    }

    /// Takes `tab` out of its split (the design's `unsplit`): the seam on its far side goes with
    /// it, and a split of one is no split. Returns the heir, the pane that grows into its place,
    /// while any pane is left.
    fn leave_split(&mut self, tab: TabId) -> Option<TabId> {
        let s = self
            .spaces
            .iter_mut()
            .find(|s| s.split.as_ref().is_some_and(|g| g.tabs.contains(&tab)))?;
        let g = s.split.as_mut()?;
        let i = g.tabs.iter().position(|&t| t == tab)?;
        g.tabs.remove(i);
        g.stops.remove(i.min(g.stops.len() - 1));
        let heir = g.tabs.get(i.min(g.tabs.len().saturating_sub(1))).copied();
        if g.tabs.len() < 2 {
            s.split = None;
        }
        heir
    }

    fn split_with(&mut self, tab: TabId) -> Vec<Effect> {
        let Some(si) = self.space_idx(self.active_space()) else {
            return vec![];
        };
        let Some(focused) = self.spaces[si].active.filter(|&f| f != tab) else {
            return vec![];
        };
        if !self.order_idx(si).contains(&tab)
            || !self.order_idx(si).contains(&focused)
            || self.is_internal(tab)
            || self.is_internal(focused)
            || self.windowed(tab).is_some()
            || self.windowed(focused).is_some()
            || self.pip_tab().is_some_and(|p| p == tab || p == focused)
        {
            return vec![];
        }
        let s = &mut self.spaces[si];
        match &mut s.split {
            Some(g) if g.tabs.contains(&focused) => {
                if g.tabs.len() >= MAX_PANES || g.tabs.contains(&tab) {
                    return vec![];
                }
                let at = g.tabs.iter().position(|&t| t == focused).unwrap_or(0) + 1;
                g.tabs.insert(at, tab);
                g.stops = even(g.tabs.len());
            }
            // a hidden split is replaced, as in the design
            _ => {
                s.split = Some(SplitGroup {
                    tabs: vec![focused, tab],
                    stops: even(2),
                })
            }
        }
        // the new page takes focus
        s.active = Some(tab);
        vec![Effect::ActivateTab(tab)]
    }

    /// A new tab beside the focused page (the design's `open_beside`). Into an empty Space, or
    /// beside a full split, it is a plain new tab shown alone.
    fn open_beside(&mut self, url: String) -> Vec<Effect> {
        let focused = self
            .space_idx(self.active_space())
            .and_then(|si| self.spaces[si].active);
        let mut fx = self.new_tab(url, None, None);
        let (Some(Effect::CreateTab { tab, .. }), Some(f)) = (fx.first().cloned(), focused) else {
            return fx;
        };
        // back to the page the new one goes beside
        let si = self.space_idx(self.active_space()).expect("active space");
        self.spaces[si].active = Some(f);
        let beside = self.split_with(tab);
        if beside.is_empty() {
            self.spaces[si].active = Some(tab);
        } else {
            fx.retain(|e| *e != Effect::ActivateTab(tab));
            fx.extend(beside);
        }
        fx
    }

    fn unsplit(&mut self, tab: TabId) -> Vec<Effect> {
        let Some((si, _)) = self.place_of(tab) else {
            return vec![];
        };
        let was_active = self.spaces[si].active == Some(tab);
        let Some(heir) = self.leave_split(tab) else {
            return vec![];
        };
        if !was_active {
            return vec![];
        }
        self.spaces[si].active = Some(heir);
        if self.spaces[si].id == self.active_space() {
            vec![Effect::ActivateTab(heir)]
        } else {
            vec![]
        }
    }

    fn resize_split(&mut self, space: SpaceId, stops: Vec<f64>) -> Vec<Effect> {
        if let Some(g) = self
            .space_idx(space)
            .and_then(|si| self.spaces[si].split.as_mut())
            .filter(|g| SplitGroup::stops_fit(&stops, g.tabs.len()))
        {
            g.stops = stops;
        }
        vec![]
    }

    fn swap_pane(&mut self, tab: TabId, right: bool) -> Vec<Effect> {
        let Some(g) = self
            .spaces
            .iter_mut()
            .find_map(|s| s.split.as_mut().filter(|g| g.tabs.contains(&tab)))
        else {
            return vec![];
        };
        let i = g.tabs.iter().position(|&t| t == tab).unwrap_or(0);
        let j = if right { i + 1 } else { i.wrapping_sub(1) };
        if j < g.tabs.len() {
            g.tabs.swap(i, j);
        }
        vec![]
    }

    /// Adds or removes one busy reason; the lifecycle hears only when the tab turns busy or idle.
    fn set_busy(&mut self, tab: TabId, reason: Reason, on: bool) -> Vec<Effect> {
        let Some(t) = self.tabs.get_mut(&tab) else {
            return vec![];
        };
        t.busy = t.busy.with(reason, on);
        let activity = if t.busy.is_empty() {
            Activity::Idle
        } else {
            Activity::Busy
        };
        if activity == t.lifecycle.activity {
            return vec![];
        }
        self.lifecycle(tab, lifecycle::Event::BusyChanged(activity))
    }

    /// A hidden, idle tab judged as if memory were critical: the policy discards it unless it is
    /// pinned and kept alive, which is refused here rather than frozen.
    fn discard_now(&mut self, tab: TabId) -> Vec<Effect> {
        if self.is_internal(tab) {
            return vec![];
        }
        let Some(l) = self.tabs.get(&tab).map(|t| t.lifecycle) else {
            return vec![];
        };
        let target = self.policy.target(
            std::time::Duration::ZERO,
            l.pin,
            l.activity,
            l.keep,
            Pressure::Critical,
        );
        if target != Liveness::Discarded {
            return vec![];
        }
        let now = self.clock;
        self.lifecycle(
            tab,
            lifecycle::Event::Tick {
                now,
                pressure: Pressure::Critical,
            },
        )
    }

    /// Runs `event` through the tab's lifecycle and returns its effects.
    fn lifecycle(&mut self, tab: TabId, event: lifecycle::Event) -> Vec<Effect> {
        let Some(t) = self.tabs.get_mut(&tab) else {
            return vec![];
        };
        let (next, fx) = step(&self.policy, tab, t.lifecycle, event);
        t.lifecycle = next;
        fx
    }

    /// Brings every lifecycle in line with the model after a command:
    /// tabs `gone` since `before` are closed, every tab not on screen is
    /// hidden and each one on screen (every pane of a shown split) shown.
    /// Each transition is a no-op for a tab already in that state.
    fn settle(&mut self, before: &BTreeMap<TabId, Lifecycle>) -> Vec<Effect> {
        let mut fx = vec![];
        for (&tab, &state) in before {
            if !self.tabs.contains_key(&tab) {
                fx.extend(step(&self.policy, tab, state, lifecycle::Event::Closed).1);
            }
        }
        let ids: Vec<TabId> = self.tabs.keys().copied().collect();
        let visible = self.visible_tabs();
        let now = self.clock;
        for &tab in ids.iter().filter(|t| !visible.contains(t)) {
            // a tab that was on screen and is not now: its last visit is this moment
            if before
                .get(&tab)
                .is_some_and(|l| l.liveness == Liveness::Active)
            {
                self.tabs.get_mut(&tab).expect("listed").last_active_ms = self.wall_ms;
            }
            fx.extend(self.lifecycle(tab, lifecycle::Event::Hidden { now }));
        }
        for tab in visible {
            fx.extend(self.lifecycle(tab, lifecycle::Event::Shown));
        }
        fx
    }

    /// Brings every tab's [`Keep`] in line with the never-sleep Spaces and sites: a tab is
    /// kept awake when its Space is on the list or its address is on a listed site.
    fn sync_keep(&mut self) -> Vec<Effect> {
        let mut want = vec![];
        for si in 0..self.spaces.len() {
            let space_on = self.spaces[si].never_sleep;
            for t in self.order_idx(si) {
                let on = space_on
                    || (!self.never_sites.is_empty()
                        && host_in(&self.never_sites, &self.tabs[&t].url));
                want.push((t, if on { Keep::Awake } else { Keep::Normal }));
            }
        }
        want.retain(|(t, keep)| self.tabs[t].lifecycle.keep != *keep);
        want.into_iter()
            .flat_map(|(t, keep)| self.lifecycle(t, lifecycle::Event::KeepChanged(keep)))
            .collect()
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
                        || (was.keep == Keep::Awake && now.keep == Keep::Normal)
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

    fn tidy_tabs(&mut self, space: SpaceId) -> Vec<Effect> {
        let Some(si) = self.space_idx(space) else {
            return vec![];
        };
        let groups = tidy::plan_groups(&self.spaces[si], &self.tabs);
        if groups.is_empty() {
            return vec![];
        }
        let before = self.spaces[si].items.clone();
        for g in groups {
            let existing = self.spaces[si]
                .items
                .iter()
                .position(|i| matches!(i, Item::Folder(f) if f.name.eq_ignore_ascii_case(&g.name)));
            let items = &mut self.spaces[si].items;
            match existing {
                Some(fi) => {
                    items.retain(|i| !matches!(i, Item::Tab(t) if g.tabs.contains(t)));
                    // retain moved the folder only if a tab stood before it
                    let fi = items
                        .iter()
                        .position(|i| matches!(i, Item::Folder(f) if f.name.eq_ignore_ascii_case(&g.name)))
                        .unwrap_or(fi);
                    if let Item::Folder(f) = &mut items[fi] {
                        f.tabs.extend(&g.tabs);
                    }
                }
                None => {
                    let id = FolderId(self.next_folder);
                    self.next_folder += 1;
                    let at = items
                        .iter()
                        .position(|i| matches!(i, Item::Tab(t) if *t == g.tabs[0]))
                        .expect("planned tab is top-level");
                    items[at] = Item::Folder(Folder {
                        id,
                        name: g.name.clone(),
                        tabs: g.tabs.clone(),
                        collapsed: false,
                    });
                    items.retain(|i| !matches!(i, Item::Tab(t) if g.tabs[1..].contains(t)));
                }
            }
        }
        vec![Effect::Tidied { space, before }]
    }

    fn set_layout(&mut self, space: SpaceId, items: Vec<Item>) -> Vec<Effect> {
        let Some(si) = self.space_idx(space) else {
            return vec![];
        };
        let current = std::mem::take(&mut self.spaces[si].items);
        let mut allowed = vec![];
        for item in &current {
            match item {
                Item::Tab(t) => allowed.push(*t),
                Item::Folder(f) => allowed.extend(&f.tabs),
            }
        }
        let mut seen: Vec<TabId> = vec![];
        let mut out: Vec<Item> = vec![];
        let take = |tabs: &[TabId], seen: &mut Vec<TabId>| -> Vec<TabId> {
            let kept: Vec<TabId> = tabs
                .iter()
                .copied()
                .filter(|t| allowed.contains(t) && !seen.contains(t))
                .collect();
            seen.extend(&kept);
            kept
        };
        // the wanted layout first, then what it does not mention, in the current order
        for item in items.iter().chain(&current) {
            match item {
                Item::Tab(t) => {
                    if !take(&[*t], &mut seen).is_empty() {
                        out.push(Item::Tab(*t));
                    }
                }
                Item::Folder(f) => {
                    if out
                        .iter()
                        .any(|o| matches!(o, Item::Folder(o) if o.id == f.id))
                    {
                        continue;
                    }
                    let tabs = take(&f.tabs, &mut seen);
                    if !tabs.is_empty() || f.tabs.is_empty() {
                        out.push(Item::Folder(Folder {
                            id: f.id,
                            name: f.name.clone(),
                            tabs,
                            collapsed: f.collapsed,
                        }));
                    }
                }
            }
        }
        self.spaces[si].items = out;
        vec![]
    }

    /// Whether the tab shows an internal page (no web view; never a split pane).
    pub fn is_internal(&self, tab: TabId) -> bool {
        self.tabs
            .get(&tab)
            .is_some_and(|t| InternalPage::from_url(&t.url).is_some())
    }

    fn open_internal(&mut self, page: InternalPage) -> Vec<Effect> {
        let si = self.space_idx(self.active_space()).expect("active space");
        let existing = self
            .order_idx(si)
            .into_iter()
            .find(|t| InternalPage::from_url(&self.tabs[t].url) == Some(page));
        match existing {
            Some(tab) => self.activate_tab(tab),
            None => self.new_tab(page.url().to_owned(), None, None),
        }
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

/// The one call into `lifecycle::transition`, under the model's sleep policy.
fn step(
    policy: &SleepPolicy,
    tab: TabId,
    state: Lifecycle,
    event: lifecycle::Event,
) -> (Lifecycle, Vec<Effect>) {
    let (next, fx) = lifecycle::transition(policy, state, event);
    let fx = fx
        .into_iter()
        .map(|effect| Effect::Lifecycle { tab, effect })
        .collect();
    (next, fx)
}

/// The only way to change a [`Model`].
///
/// Tab lifecycles are settled at the end: whatever the command, the result has
/// exactly the tabs on screen ([`Model::visible_tabs`]) Active, every other tab hidden, and the
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
        Command::BackgroundTab { url, opener } => m
            .place_new_tab(url, None, Some(opener), None)
            .map(|(_, _, create)| create)
            .into_iter()
            .collect(),
        Command::ExtensionTab {
            url,
            container,
            space,
            show,
        } => m.extension_tab(url, container, space, show),
        Command::OpenInSpace { space, url } => m
            .place_new_tab(url, None, None, Some(space))
            .map(|(id, si, create)| {
                m.spaces[si].active.get_or_insert(id);
                create
            })
            .into_iter()
            .collect(),
        Command::CloseTab(t) => m.close_tab(t),
        Command::CloseTabs(tabs) => tabs.into_iter().flat_map(|t| m.close_tab(t)).collect(),
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
                    collapsed: false,
                }));
            }
            vec![]
        }
        Command::DeleteFolder(f) => m.delete_folder(f),
        Command::SetFolderCollapsed { folder, collapsed } => {
            if let Some(si) = m.folder_space(folder)
                && let Some(f) = m.spaces[si].folder_mut(folder)
            {
                f.collapsed = collapsed;
            }
            vec![]
        }
        Command::RenameFolder { folder, name } => {
            if let Some(si) = m.folder_space(folder)
                && let Some(f) = m.spaces[si].folder_mut(folder)
            {
                f.name = name;
            }
            vec![]
        }
        Command::FolderWithTab { tab, name } => m.folder_with_tab(tab, name),
        Command::OpenGlance {
            window,
            opener,
            url,
        } => m.open_glance(window, opener, url),
        Command::CloseGlance(window) => m
            .glance_of(window)
            .map(|g| m.close_tab(g.tab))
            .unwrap_or_default(),
        Command::PromoteGlance { window, to } => m.promote_glance(window, to),
        Command::DuplicateTab(t) => m.duplicate_tab(t),
        Command::CloseOthers(t) => m.close_others(t),
        Command::NewSpace {
            name,
            tint,
            container,
        } => {
            if m.has_container(container) {
                m.add_space(name, tint, container);
            }
            vec![]
        }
        Command::RenameSpace { space, name } => {
            if let Some(si) = m.space_idx(space) {
                m.spaces[si].name = name;
            }
            vec![]
        }
        Command::SetSpaceTint { space, tint } => {
            if let Some(si) = m.space_idx(space) {
                m.spaces[si].tint = tint;
            }
            vec![]
        }
        Command::SetSpaceIcon { space, icon } => {
            if let Some(si) = m.space_idx(space) {
                m.spaces[si].icon = icon;
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
        // A web tab keeps its view and history: an internal page typed into it opens in its own tab.
        Command::Navigate { tab, url } => match InternalPage::from_url(&url) {
            Some(page) if m.tabs.contains_key(&tab) && !m.is_internal(tab) => m.open_internal(page),
            _ if m.set_url(tab, &url) => vec![Effect::Navigate { tab, url }],
            _ => vec![],
        },
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
        Command::Busy { tab, reason, on } => m.set_busy(tab, reason, on),
        Command::DiscardNow(tab) => m.discard_now(tab),
        Command::RendererGone(t) => m.lifecycle(t, lifecycle::Event::RendererGone),
        Command::SplitWith(t) => m.split_with(t),
        Command::OpenBeside { url } => m.open_beside(url),
        Command::Unsplit(t) => m.unsplit(t),
        Command::ResizeSplit { space, stops } => m.resize_split(space, stops),
        Command::SwapPane { tab, right } => m.swap_pane(tab, right),
        Command::OpenInternal(page) => m.open_internal(page),
        Command::NewWindow { space } => m.new_window(space),
        Command::CloseWindow(w) => m.close_window(w),
        Command::FocusWindow(w) => {
            m.focus(w);
            vec![]
        }
        Command::ResizeWindow { window, geometry } => {
            m.resize_window(window, geometry);
            vec![]
        }
        Command::OpenPip { tab } => m.open_pip(tab),
        Command::ClosePip => m.close_pip(),
        Command::ReturnPip => m.return_pip(),
        Command::MoveTabToWindow { tab, to } => m.move_tab_to_window(tab, to),
        Command::OpenPopupWindow {
            url,
            opener,
            size,
            pos,
        } => m.open_popup_window(url, opener, size, pos),
        Command::OpenLittle {
            url,
            space,
            container,
        } => m.open_little(url, space, container),
        Command::PromoteLittle { window, space } => m.promote_little(window, space),
        Command::SetSleepPolicy(policy) => {
            m.policy = policy;
            vec![]
        }
        Command::SetSpaceNeverSleep { space, on } => {
            if let Some(si) = m.space_idx(space) {
                m.spaces[si].never_sleep = on;
            }
            vec![]
        }
        Command::MarkAgentSpace(space) => {
            if let Some(si) = m.space_idx(space) {
                m.spaces[si].agent = true;
            }
            vec![]
        }
        Command::SetNeverSleepSites(sites) => {
            m.never_sites = sites;
            vec![]
        }
        Command::TidyTabs { space } => m.tidy_tabs(space),
        Command::SetLayout { space, items } => m.set_layout(space, items),
        Command::Tick {
            now,
            wall_ms,
            pressure,
        } => {
            m.clock = now;
            m.pressure = pressure;
            m.wall_ms = wall_ms;
            if wall_ms > 0 {
                // tabs of unknown last visit (restored before this field existed) start now
                let visible = m.visible_tabs();
                for t in m.tabs.values_mut() {
                    if t.last_active_ms == 0 && !visible.contains(&t.id) {
                        t.last_active_ms = wall_ms;
                    }
                }
            }
            // Oldest hidden first, so a critical sweep discards in that order (Firefox
            // `TabUnloader.sys.mjs` sorts by last access).
            let mut ids: Vec<TabId> = m.tabs.keys().copied().collect();
            ids.sort_by_key(|t| (m.tabs[t].lifecycle.hidden_since, *t));
            ids.into_iter()
                .flat_map(|t| m.lifecycle(t, lifecycle::Event::Tick { now, pressure }))
                .collect()
        }
    };
    fx.extend(m.prune_windows());
    fx.extend(m.tidy_glances());
    fx.extend(m.settle(&before));
    fx.extend(m.sync_keep());
    fx.extend(m.rejudge(&before));
    (m, fx)
}

/// The item `delta` places from `current` in `items`, wrapping. Without a
/// current item (or one not in `items`) the first step lands on the first
/// item going forward and on the last going back.
fn cycle<T: Copy + PartialEq>(items: &[T], current: Option<T>, delta: isize) -> Option<T> {
    let len = isize::try_from(items.len()).ok().filter(|&n| n > 0)?;
    let from = current.and_then(|c| items.iter().position(|&i| i == c));
    let index = match from {
        Some(i) => (i as isize + delta).rem_euclid(len),
        None if delta >= 0 => 0,
        None => len - 1,
    };
    items.get(index as usize).copied()
}

#[cfg(test)]
mod tests;
