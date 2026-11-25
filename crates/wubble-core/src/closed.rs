//! Closed tabs, windows, folders and Spaces coming back: reopen-closed (Mod+Shift+T, a row of
//! the recently-closed list) and Mod+Z after a close (undo close), one machine for all.
//!
//! The list's shape is Chromium's `TabRestoreService` (`components/sessions/core/
//! tab_restore_service.h`, BSD-3-Clause; behaviour only, no code): entries newest first, each a
//! tab, a whole window or a group (here a folder, and a Space, wubble's own grouping), at most
//! [`RECENTLY_CLOSED_MAX`]; reopen takes the newest entry of whatever kind; a window comes back
//! as a window, a group as a group with its tabs in their order. A closed Main window keeps its
//! Space (its tabs stay open in the sidebar), so its entry reopens a window on that Space at its
//! old geometry, and only when the Space is gone too does it build the Space again from what the
//! entry kept of it.
//!
//! The Mod+Z rule is Safari's Edit > Undo Close Tab (Cmd+Z, Apple's responder-chain undo;
//! behaviour only, no code): the key is the focused text field's undo first, always; only when
//! no field has the keys and the page left the key alone does it reopen the last thing closed in
//! this session, and pressed again the one before, back through every close of the session.
//! Switching tabs or Spaces and navigating end nothing; closes from an earlier run are not
//! undoable (they stay on the recently-closed list for Mod+Shift+T). Who has the keys is the
//! router's (`input::key_route` keeps the key in a shell field); whether the page's focused
//! element is editable comes with the page's unhandled-key report (CEF
//! `CefKeyEvent::focus_on_editable_field`), so a field with nothing to undo keeps the key too.
//!
//! The recently-closed list itself is the store's (persisted, newest first); this machine
//! keeps, for the closes of this run still on it, where each tab sat, so the one restore
//! (`Host::reopen_closed`) puts it back in its Space at its place (pinned, in its folder). A
//! tab an agent session drives or an agent Space's is never on the list
//! (`Model::closes_into_recent`), so never here; nor is an agent Space as a Space or a window.

use crate::model::{Command, Dest, Model, Place};
use crate::space::{ContainerId, FolderId, Item, Rgb, SpaceIcon, SpaceId, SpaceKind};
use crate::tab::TabId;
use crate::window::{Geometry, Shown, WindowId, WindowKind};

/// Most entries the recently-closed list keeps; older ones are dropped on a push. Chromium's
/// `TabRestoreService` keeps 25 (`kMaxEntries`).
pub const RECENTLY_CLOSED_MAX: usize = 25;

// ----------------------------------------------------------------------------- entries

/// A closed tab on its own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClosedTab {
    pub url: String,
    pub title: String,
    pub container: ContainerId,
    pub space: Option<SpaceId>,
    /// Unix ms, supplied by the caller.
    pub closed_at_ms: i64,
}

/// Where a tab of a grouped entry sat in its Space.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Slot {
    Pinned,
    /// A top-level tab.
    Normal,
    /// In the Space's `folder`-th folder (counted in the Space's order), named `name`.
    Folder {
        folder: u32,
        name: String,
    },
}

/// One tab of a grouped entry, in the entry's order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TabSnap {
    pub url: String,
    pub title: String,
    pub container: ContainerId,
    pub slot: Slot,
}

/// A Space as an entry keeps it: enough to build it again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpaceSnap {
    pub name: String,
    pub tint: Rgb,
    pub icon: SpaceIcon,
    pub container: ContainerId,
    /// Pinned first, then the Space's items, a folder's tabs inline.
    pub tabs: Vec<TabSnap>,
}

/// A closed Main window: the Space it showed and its geometry; the Space as it was, should it
/// be gone by the time the window is reopened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClosedWindow {
    pub space: SpaceId,
    pub geometry: Option<Geometry>,
    pub snap: SpaceSnap,
    pub closed_at_ms: i64,
}

/// A closed folder ("Close Folder") and its tabs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClosedFolder {
    pub name: String,
    /// The Space it was in; it comes back in the active one when that is gone.
    pub space: Option<SpaceId>,
    /// In the folder's order; their slot is `Slot::Normal`.
    pub tabs: Vec<TabSnap>,
    pub closed_at_ms: i64,
}

/// A deleted Space and its tabs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClosedSpace {
    pub snap: SpaceSnap,
    pub closed_at_ms: i64,
}

/// One entry of the recently-closed list (Chromium `TabRestoreService::Entry`: TAB, WINDOW,
/// GROUP).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Entry {
    Tab(ClosedTab),
    Window(ClosedWindow),
    Folder(ClosedFolder),
    Space(ClosedSpace),
}

impl From<ClosedTab> for Entry {
    fn from(tab: ClosedTab) -> Entry {
        Entry::Tab(tab)
    }
}

/// An entry as the store lists it, with its row id (a list row reopens or deletes by it).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listed {
    pub id: i64,
    pub entry: Entry,
}

fn tabs_word(n: usize) -> String {
    format!("{n} tab{}", if n == 1 { "" } else { "s" })
}

impl Entry {
    pub fn closed_at_ms(&self) -> i64 {
        match self {
            Entry::Tab(t) => t.closed_at_ms,
            Entry::Window(w) => w.closed_at_ms,
            Entry::Folder(f) => f.closed_at_ms,
            Entry::Space(s) => s.closed_at_ms,
        }
    }

    /// With the close time, what names this entry to the undo machine (a tab's URL; a group's
    /// kind and name).
    pub fn key(&self) -> String {
        match self {
            Entry::Tab(t) => t.url.clone(),
            Entry::Window(w) => format!("window:{}", w.space.0),
            Entry::Folder(f) => format!("folder:{}", f.name),
            Entry::Space(s) => format!("space:{}", s.snap.name),
        }
    }

    /// The tabs a grouped entry holds; empty for a tab.
    pub fn tabs(&self) -> &[TabSnap] {
        match self {
            Entry::Tab(_) => &[],
            Entry::Window(w) => &w.snap.tabs,
            Entry::Folder(f) => &f.tabs,
            Entry::Space(s) => &s.snap.tabs,
        }
    }

    /// Every page the entry would open, in order.
    pub fn urls(&self) -> Vec<&str> {
        match self {
            Entry::Tab(t) => vec![t.url.as_str()],
            other => other.tabs().iter().map(|t| t.url.as_str()).collect(),
        }
    }

    /// The page whose icon a row of the entry shows: the tab's, or a group's first.
    pub fn icon_url(&self) -> Option<&str> {
        self.urls().first().copied()
    }

    /// A row's title (Chromium's recent tabs menu: a tab's title, a window's "N tabs", a group's
    /// name). An untitled tab shows its host.
    pub fn title(&self) -> String {
        match self {
            Entry::Tab(t) => {
                let host = crate::url::address_parts(&t.url).0;
                [t.title.trim(), host, t.url.as_str()]
                    .into_iter()
                    .find(|s| !s.is_empty())
                    .unwrap_or_default()
                    .to_owned()
            }
            Entry::Window(w) => format!("Window: {}", w.snap.name),
            Entry::Folder(f) if f.name.trim().is_empty() => "Unnamed folder".to_owned(),
            Entry::Folder(f) => f.name.clone(),
            Entry::Space(s) => s.snap.name.clone(),
        }
    }

    /// A row's note: a tab's address, a group's kind and size.
    pub fn note(&self) -> String {
        match self {
            Entry::Tab(t) => t.url.clone(),
            Entry::Window(w) => tabs_word(w.snap.tabs.len()),
            Entry::Folder(f) => format!("Folder · {}", tabs_word(f.tabs.len())),
            Entry::Space(s) => format!("Space · {}", tabs_word(s.snap.tabs.len())),
        }
    }

    /// Whether a search for `query_lower` (lower case, trimmed) finds the entry: its title, or
    /// any of its pages' addresses or titles. An empty query finds every entry.
    pub fn matches(&self, query_lower: &str) -> bool {
        if query_lower.is_empty() {
            return true;
        }
        let hit = |s: &str| s.to_lowercase().contains(query_lower);
        hit(&self.title())
            || match self {
                Entry::Tab(t) => hit(&t.url),
                other => other.tabs().iter().any(|t| hit(&t.url) || hit(&t.title)),
            }
    }

    /// The Space `model` would close as an entry: its tabs that `keep` lets on the list, in
    /// order. None for an agent Space, an unknown one, or one with no such tab.
    pub fn space_of(
        model: &Model,
        space: SpaceId,
        closed_at_ms: i64,
        keep: impl Fn(TabId) -> bool,
    ) -> Option<Entry> {
        let snap = snap_space(model, space, keep)?;
        Some(Entry::Space(ClosedSpace { snap, closed_at_ms }))
    }

    /// Main window `window` as an entry, if closing it closes a window (not the last Main one)
    /// on a Space of the user's with a tab `keep` lets on the list. Its geometry is the model's:
    /// the host records the live one first.
    pub fn window_of(
        model: &Model,
        window: WindowId,
        closed_at_ms: i64,
        keep: impl Fn(TabId) -> bool,
    ) -> Option<Entry> {
        let w = model.window(window)?;
        let Shown::Space(space) = w.shown else {
            return None;
        };
        let mains = model
            .windows
            .iter()
            .filter(|w| w.kind() == WindowKind::Main)
            .count();
        if mains < 2 {
            return None;
        }
        let snap = snap_space(model, space, keep)?;
        Some(Entry::Window(ClosedWindow {
            space,
            geometry: w.geometry.clone(),
            snap,
            closed_at_ms,
        }))
    }

    /// Folder `folder` as an entry: its tabs that `keep` lets on the list. None for an unknown
    /// folder, an agent Space's, or one with no such tab.
    pub fn folder_of(
        model: &Model,
        folder: FolderId,
        closed_at_ms: i64,
        keep: impl Fn(TabId) -> bool,
    ) -> Option<Entry> {
        let (space, f) = model
            .spaces
            .iter()
            .filter(|s| s.kind == SpaceKind::Normal)
            .find_map(|s| {
                s.items.iter().find_map(|i| match i {
                    Item::Folder(f) if f.id == folder => Some((s.id, f)),
                    _ => None,
                })
            })?;
        let tabs: Vec<TabSnap> = f
            .tabs
            .iter()
            .filter(|&&t| keep(t) && !model.is_glance(t))
            .filter_map(|&t| snap_tab(model, t, Slot::Normal))
            .collect();
        (!tabs.is_empty()).then(|| {
            Entry::Folder(ClosedFolder {
                name: f.name.clone(),
                space: Some(space),
                tabs,
                closed_at_ms,
            })
        })
    }
}

fn snap_tab(model: &Model, tab: TabId, slot: Slot) -> Option<TabSnap> {
    let t = model.tabs.get(&tab)?;
    (!t.url.is_empty()).then(|| TabSnap {
        url: t.url.clone(),
        title: t.title.clone(),
        container: t.container,
        slot,
    })
}

/// The Space as an entry keeps it: pinned tabs, then its items with each folder's tabs inline,
/// those `keep` refuses and the peeks left out. None for an agent's or a private Space, or one
/// with none left.
fn snap_space(model: &Model, space: SpaceId, keep: impl Fn(TabId) -> bool) -> Option<SpaceSnap> {
    let s = model
        .spaces
        .iter()
        .find(|s| s.id == space && s.kind == SpaceKind::Normal)?;
    let keep = |t: TabId| keep(t) && !model.is_glance(t);
    let mut tabs: Vec<TabSnap> = s
        .pinned
        .iter()
        .filter(|&&t| keep(t))
        .filter_map(|&t| snap_tab(model, t, Slot::Pinned))
        .collect();
    let mut folders = 0u32;
    for item in &s.items {
        match item {
            Item::Tab(t) if keep(*t) => tabs.extend(snap_tab(model, *t, Slot::Normal)),
            Item::Tab(_) => {}
            Item::Folder(f) => {
                let slot = Slot::Folder {
                    folder: folders,
                    name: f.name.clone(),
                };
                folders += 1;
                tabs.extend(
                    f.tabs
                        .iter()
                        .filter(|&&t| keep(t))
                        .filter_map(|&t| snap_tab(model, t, slot.clone())),
                );
            }
        }
    }
    (!tabs.is_empty()).then(|| SpaceSnap {
        name: s.name.clone(),
        tint: s.tint,
        icon: s.icon,
        container: s.container,
        tabs,
    })
}

// ----------------------------------------------------------------------------- the machine

/// One close of this run, as the store recorded it (`key` and `closed_at_ms` name the entry).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Closed {
    /// [`Entry::key`].
    pub key: String,
    pub closed_at_ms: i64,
    /// A tab's Space and its place there, before it closed; `None` for a grouped entry.
    pub at: Option<(SpaceId, Dest)>,
}

impl Closed {
    /// `tab` as it sits in `model` before `command` (about to run) closes it, if the close goes
    /// on the recently-closed list as far as the model knows (`Model::closes_into_recent`: not
    /// an agent Space's being deleted, not a peek's, not an empty tab).
    pub fn of(model: &Model, tab: TabId, command: &Command, closed_at_ms: i64) -> Option<Closed> {
        if !model.closes_into_recent(tab, command) {
            return None;
        }
        let url = model.tabs.get(&tab)?.url.clone();
        let (space, place) = model.location(tab)?;
        let place = match place {
            Place::Pinned(index) => Dest::Pinned { space, index },
            Place::Normal(index) => Dest::Normal { space, index },
            Place::Folder(folder, index) => Dest::Folder { folder, index },
        };
        Some(Closed {
            key: url,
            closed_at_ms,
            at: Some((space, place)),
        })
    }

    /// A grouped entry's close.
    pub fn entry(entry: &Entry) -> Closed {
        Closed {
            key: entry.key(),
            closed_at_ms: entry.closed_at_ms(),
            at: None,
        }
    }
}

/// This run's closes still on the list, oldest first, and how many list entries the host has
/// asked the store for and not yet heard back about.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Undo {
    closes: Vec<Closed>,
    popping: usize,
}

impl Undo {
    /// Closes a press of Mod+Z could still reopen.
    pub fn undoable(&self) -> usize {
        self.closes.len().saturating_sub(self.popping)
    }

    fn take(&mut self, key: &str, closed_at_ms: i64) -> Option<Closed> {
        self.closes
            .iter()
            .rposition(|c| c.key == key && c.closed_at_ms == closed_at_ms)
            .map(|i| self.closes.remove(i))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// A tab, window, folder or Space went on the recently-closed list.
    Closed(Closed),
    /// Mod+Shift+T or the command bar's "Reopen closed tab": the newest entry, whichever run
    /// closed it.
    Reopen,
    /// A row of the list (the history page's, the command bar's): that entry, by its store id.
    Take(i64),
    /// A row's delete: the entry leaves the list without opening.
    Remove {
        id: i64,
        key: String,
        closed_at_ms: i64,
    },
    /// Mod+Z that no shell field took and the page left alone; `editable`: the page's focus is
    /// on an editable element (CEF's `focus_on_editable_field`).
    Key { editable: bool },
    /// The command bar's "Undo close tab": no field to hand the key to.
    Command,
    /// The store took an entry off the list (the newest, or the one asked for): this one, or none.
    Popped(Option<Entry>),
    /// History was deleted, and with it entries of the list: what is left is not known here.
    Forget,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    /// Take the newest entry off the store's list (an [`Event::Popped`] answers).
    Pop,
    /// Take this entry off the store's list (an [`Event::Popped`] answers).
    Take(i64),
    /// Delete this entry from the store's list.
    Delete(i64),
    /// Open the popped tab: in `space` at `place` when this run closed it, else in the active
    /// Space, last, as any reopen.
    Restore {
        tab: ClosedTab,
        space: Option<SpaceId>,
        place: Option<Dest>,
    },
    /// A Main window on its Space again, at its geometry (the Space rebuilt if it is gone):
    /// `Command::ReopenWindow`.
    RestoreWindow(ClosedWindow),
    /// The folder and its tabs again: `Command::ReopenFolder`.
    RestoreFolder(ClosedFolder),
    /// The Space and its tabs again: `Command::ReopenSpace`.
    RestoreSpace(ClosedSpace),
    /// The key is the page's: its focused frame's undo, as Mod+Z without a close.
    FieldUndo,
    /// Asked from the command bar with nothing to undo.
    NothingToUndo,
}

impl Effect {
    /// The model command a grouped restore is (a tab's goes through the host's reopen).
    pub fn command(self) -> Option<Command> {
        match self {
            Effect::RestoreWindow(w) => Some(Command::ReopenWindow(w)),
            Effect::RestoreFolder(f) => Some(Command::ReopenFolder(f)),
            Effect::RestoreSpace(s) => Some(Command::ReopenSpace(s)),
            _ => None,
        }
    }
}

fn restore(entry: Entry, this_run: Option<Closed>) -> Effect {
    match entry {
        Entry::Tab(tab) => {
            let at = this_run.and_then(|c| c.at);
            Effect::Restore {
                tab,
                space: at.map(|(space, _)| space),
                place: at.map(|(_, place)| place),
            }
        }
        Entry::Window(w) => Effect::RestoreWindow(w),
        Entry::Folder(f) => Effect::RestoreFolder(f),
        Entry::Space(s) => Effect::RestoreSpace(s),
    }
}

pub fn transition(mut state: Undo, event: Event) -> (Undo, Vec<Effect>) {
    let effects = match event {
        Event::Closed(closed) => {
            state.closes.push(closed);
            if state.closes.len() > RECENTLY_CLOSED_MAX {
                state.closes.remove(0);
            }
            vec![]
        }
        Event::Reopen => {
            state.popping += 1;
            vec![Effect::Pop]
        }
        Event::Take(id) => {
            state.popping += 1;
            vec![Effect::Take(id)]
        }
        Event::Remove {
            id,
            key,
            closed_at_ms,
        } => {
            state.take(&key, closed_at_ms);
            vec![Effect::Delete(id)]
        }
        Event::Key { editable: true } => vec![Effect::FieldUndo],
        Event::Key { editable: false } | Event::Command if state.undoable() > 0 => {
            state.popping += 1;
            vec![Effect::Pop]
        }
        Event::Key { editable: false } => vec![Effect::FieldUndo],
        Event::Command => vec![Effect::NothingToUndo],
        Event::Popped(entry) => {
            state.popping = state.popping.saturating_sub(1);
            match entry {
                None => vec![],
                Some(entry) => {
                    let this_run = state.take(&entry.key(), entry.closed_at_ms());
                    vec![restore(entry, this_run)]
                }
            }
        }
        Event::Forget => {
            state.closes.clear();
            vec![]
        }
    };
    (state, effects)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn closed(url: &str, at: i64, index: usize) -> Closed {
        let space = SpaceId(1);
        Closed {
            key: url.to_owned(),
            closed_at_ms: at,
            at: Some((space, Dest::Normal { space, index })),
        }
    }

    fn tab_of(c: &Closed) -> ClosedTab {
        ClosedTab {
            url: c.key.clone(),
            title: String::new(),
            container: ContainerId::DEFAULT,
            space: None,
            closed_at_ms: c.closed_at_ms,
        }
    }

    fn restored(c: &Closed) -> Effect {
        Effect::Restore {
            tab: tab_of(c),
            space: c.at.map(|(s, _)| s),
            place: c.at.map(|(_, p)| p),
        }
    }

    fn run(state: Undo, events: impl IntoIterator<Item = Event>) -> (Undo, Vec<Effect>) {
        events.into_iter().fold((state, vec![]), |(s, mut all), e| {
            let (s, effects) = transition(s, e);
            all.extend(effects);
            (s, all)
        })
    }

    fn popped(c: &Closed) -> Event {
        Event::Popped(Some(Entry::Tab(tab_of(c))))
    }

    #[test]
    fn a_close_then_the_key_restores_it_at_its_place() {
        let a = closed("https://a.test/", 10, 2);
        let (s, effects) = run(
            Undo::default(),
            [Event::Closed(a.clone()), Event::Key { editable: false }],
        );
        assert_eq!(effects, [Effect::Pop]);
        let (s, effects) = transition(s, popped(&a));
        assert_eq!(effects, [restored(&a)]);
        assert_eq!(s, Undo::default(), "nothing left to undo");
    }

    #[test]
    fn nothing_closed_passes_the_key_on() {
        let (s, effects) = transition(Undo::default(), Event::Key { editable: false });
        assert_eq!(effects, [Effect::FieldUndo]);
        assert_eq!(s, Undo::default());
        let (_, effects) = transition(s, Event::Command);
        assert_eq!(effects, [Effect::NothingToUndo]);
    }

    #[test]
    fn an_editable_focus_keeps_the_key_whatever_was_closed() {
        let a = closed("https://a.test/", 10, 0);
        let (s, effects) = run(
            Undo::default(),
            [Event::Closed(a), Event::Key { editable: true }],
        );
        assert_eq!(effects, [Effect::FieldUndo]);
        assert_eq!(s.undoable(), 1, "the close stays undoable");
    }

    #[test]
    fn presses_step_back_through_several_closes_newest_first() {
        let (a, b) = (
            closed("https://a.test/", 10, 1),
            closed("https://b.test/", 20, 1),
        );
        let (s, effects) = run(
            Undo::default(),
            [
                Event::Closed(a.clone()),
                Event::Closed(b.clone()),
                Event::Key { editable: false },
                // pressed again before the store answered: the next one, not the same twice
                Event::Key { editable: false },
                Event::Key { editable: false },
            ],
        );
        assert_eq!(effects, [Effect::Pop, Effect::Pop, Effect::FieldUndo]);
        let (s, effects) = run(s, [popped(&b), popped(&a)]);
        assert_eq!(effects, [restored(&b), restored(&a)]);
        assert_eq!(s.undoable(), 0);
    }

    #[test]
    fn reopen_takes_this_runs_close_and_its_place_too() {
        let a = closed("https://a.test/", 10, 3);
        let (s, effects) = run(Undo::default(), [Event::Closed(a.clone()), Event::Reopen]);
        assert_eq!(effects, [Effect::Pop]);
        assert_eq!(s.undoable(), 0, "the close is being reopened");
        let (s, effects) = transition(s, popped(&a));
        assert_eq!(effects, [restored(&a)]);
        let (_, effects) = transition(s, Event::Key { editable: false });
        assert_eq!(effects, [Effect::FieldUndo], "nothing left for the key");
    }

    #[test]
    fn an_earlier_runs_close_reopens_only_by_reopen_and_without_a_place() {
        // the list holds an entry from before this run: the key leaves it alone
        let (s, effects) = transition(Undo::default(), Event::Key { editable: false });
        assert_eq!(effects, [Effect::FieldUndo]);
        let old = Closed {
            key: "https://old.test/".into(),
            closed_at_ms: 1,
            at: None,
        };
        let (s, effects) = run(s, [Event::Reopen, popped(&old)]);
        assert_eq!(effects, [Effect::Pop, restored(&old)]);
        let (_, effects) = run(s, [Event::Reopen, Event::Popped(None)]);
        assert_eq!(effects, [Effect::Pop], "an empty list restores nothing");
    }

    #[test]
    fn deleted_history_ends_the_undo() {
        let a = closed("https://a.test/", 10, 0);
        let (s, _) = run(Undo::default(), [Event::Closed(a), Event::Forget]);
        assert_eq!(s.undoable(), 0);
        let (_, effects) = transition(s, Event::Key { editable: false });
        assert_eq!(effects, [Effect::FieldUndo]);
    }

    #[test]
    fn the_run_keeps_as_many_closes_as_the_list() {
        let events = (0..RECENTLY_CLOSED_MAX as i64 + 5)
            .map(|i| Event::Closed(closed("https://a.test/", i, 0)))
            .collect::<Vec<_>>();
        let (s, _) = run(Undo::default(), events);
        assert_eq!(s.undoable(), RECENTLY_CLOSED_MAX);
    }

    fn open(m: Model, space: Option<SpaceId>, url: &str) -> (Model, TabId) {
        let command = match space {
            Some(space) => Command::OpenInSpace {
                space,
                url: url.into(),
            },
            None => Command::NewTab {
                url: url.into(),
                container: None,
                opener: None,
            },
        };
        let (m, _) = crate::model::apply(m, command);
        let tab = *m.tabs.keys().max().unwrap();
        (m, tab)
    }

    #[test]
    fn a_tab_closes_with_its_place_and_an_agent_space_s_not_at_all() {
        let m = Model::new(std::time::Instant::now());
        let (m, _) = open(m, None, "https://a.test/");
        let (m, b) = open(m, None, "https://b.test/");
        let space = m.active_space();
        let c = Closed::of(&m, b, &Command::CloseTab(b), 5).unwrap();
        assert_eq!(
            (c.key.as_str(), c.at),
            (
                "https://b.test/",
                Some((space, Dest::Normal { space, index: 1 }))
            )
        );
        let (m, _) = crate::model::apply(
            m,
            Command::NewSpace {
                name: "agent".into(),
                tint: crate::space::Rgb(1, 2, 3),
                container: crate::space::ContainerId::DEFAULT,
            },
        );
        let agent = m.spaces.last().unwrap().id;
        let (m, _) = crate::model::apply(m, Command::MarkAgentSpace(agent));
        let (m, t) = open(m, Some(agent), "https://agent.test/");
        // a session's teardown: never on the list, so never undoable
        assert_eq!(Closed::of(&m, t, &Command::DeleteSpace(agent), 5), None);
        assert_eq!(Entry::space_of(&m, agent, 5, |_| true), None);
    }
}
