//! The overlays' one store: which is open and which is leaving, for the app. Window overlays
//! are keyed by their window, a tab's by its tab (a tab moved to another window keeps its find
//! bar). [`reduce`] is the one place an overlay opens or closes; the selectors answer who has
//! the keys, the pointer, Escape and the accessibility tree.
//!
//! What other owners know is never kept here: the visible tab (the model's), the window's kind
//! (its host's), whether the page has the keys (the shell's) come as an [`Env`].
//!
//! The hover and timed surfaces (a tooltip, the tab card, the link status bubble, the toast and
//! its notice, the video strip, the picture-in-picture strip, the sidebar's peek) are one
//! `hovercard` machine per [`Surface`] (window, kind), stepped by the kit's discrete hover cues
//! and their owners' shows ([`Action::Hover`]) and by the host's ONE timer ([`Action::Tick`]),
//! which [`Effect::Arm`] sets to the soonest deadline of them all. The store never reads a
//! clock: `now` comes with each action.
//!
//! Prior art: Zed `crates/workspace/src/modal_layer.rs` (one layer owning the modal's
//! open/closed and the focus to return to; here the focus is derived, [`Ui::key_owner`] after
//! the close), the Elm architecture's `update(model, msg) -> (model, cmds)`.

use std::collections::BTreeMap;

use super::{Esc, Group, Keys, Kind, Layer, Pointer, Stack};
use crate::hovercard::{self, Event, Hover};
use crate::tab::TabId;
use crate::window::{WindowId, WindowKind};

/// An overlay in the store; absent is closed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Open,
    /// Closed, its kit still draws the exit (until [`Action::Exited`]).
    Leaving,
}

/// Whose an entry is: its window's, or its tab's (wherever that tab shows).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Home {
    Window(WindowId),
    Tab(TabId),
}

fn home(w: WindowId, layer: Layer) -> Home {
    layer.tab.map_or(Home::Window(w), Home::Tab)
}

/// One window's hover or timed surface (a kind with [`Kind::timing`]): its slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Surface {
    pub w: WindowId,
    pub kind: Kind,
}

/// What a hover slot shows for: replacing it in its slot is a move, never a hide and a show.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    /// The slot's one thing (a toast, the peek, the picture-in-picture strip).
    None,
    /// A tab: its row's card, its pane's status bubble or video strip.
    Tab(TabId),
    /// The page in this tab's `title` under the pointer.
    Page(TabId),
    /// A shell button that runs this action.
    Shell(crate::action::Action),
    /// The notice asked over this tab, held on the toast until answered.
    Notice(TabId),
}

impl Key {
    fn tab(self) -> Option<TabId> {
        match self {
            Key::Tab(t) | Key::Page(t) | Key::Notice(t) => Some(t),
            Key::None | Key::Shell(_) => None,
        }
    }
}

/// What the kit's hover or a surface's owner says of a slot; never a pointer position.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cue {
    /// The pointer came onto the key: it shows after the slot's delay.
    Enter,
    /// The pointer left the key (a stale leave, of a key the slot is no longer on, is nothing).
    Leave,
    /// Shown now and held (the peek's edge, the pointer on a strip, a notice).
    Show,
    /// Shown now for this many ms (a toast, the video strip after a move over the video).
    ShowFor(u64),
    /// The pointer is back on the key on its way out: it stays.
    Keep,
    /// A press, a key, a tab switch, an answer: gone now (for its key; `Key::None`, whatever
    /// shows).
    Dismiss,
}

/// The overlays the store owns, for every window and tab of the app, and its hover slots.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Ui {
    entries: BTreeMap<(Home, Kind), Phase>,
    /// The hover and timed surfaces; absent is `Hover::Idle`.
    hover: BTreeMap<Surface, Hover<Key>>,
    /// What the host's one timer is armed for (the store's own timer bookkeeping).
    armed: Option<u64>,
}

/// What other owners know, read when an action runs; never stored.
pub trait Env {
    /// The window's visible tab (the model's).
    fn shown(&self, w: WindowId) -> Option<TabId>;
    /// The window that shows this tab, if one does.
    fn showing(&self, tab: TabId) -> Option<WindowId>;
    fn kind(&self, w: WindowId) -> WindowKind;
    /// The page has the keys (the shell's focus): its Escape never answers a `ShellOnly` row.
    fn page_has_keys(&self, w: WindowId) -> bool;
    /// TRANSITIONAL (until each kind moves into the store): the overlays open in `w` whose
    /// machine still owns their openness, read from it.
    fn others(&self, w: WindowId) -> Vec<Layer>;
    /// TRANSITIONAL: `kind`'s openness is still its machine's.
    fn machine(&self, kind: Kind) -> bool;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// The user, a key or the page asks for an overlay.
    Open { w: WindowId, layer: Layer },
    /// Its owner closed it (answered, picked, dismissed): nothing to cancel.
    Close { w: WindowId, layer: Layer },
    /// The kit drew its exit to the end.
    Exited { w: WindowId, layer: Layer },
    /// Escape in window `w` (who has the keys is the [`Env`]'s).
    Escape { w: WindowId },
    /// Window `w` shows `to` in place of `from`.
    Switched {
        w: WindowId,
        from: Option<TabId>,
        to: Option<TabId>,
    },
    /// The tab's main frame committed a new document (`w` the window acting).
    Navigated { w: WindowId, tab: TabId },
    /// The tab's view went: the engine answered everything it held.
    ViewGone(TabId),
    /// The window closed.
    WindowGone(WindowId),
    /// A hover or timed surface's cue, at `now` on the host's clock.
    Hover {
        at: Surface,
        key: Key,
        cue: Cue,
        now: u64,
    },
    /// The host's one timer fired.
    Tick { now: u64 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyOwner {
    Page,
    Layer(Layer),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PointerOwner {
    Page,
    Layer(Layer),
}

/// What the accessibility tree shows: the modal that holds it, and every overlay by drawing order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AxRoots {
    pub modal: Option<Layer>,
    pub roots: Vec<Layer>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effect {
    /// It opens: its content comes to be.
    Mount(Layer),
    /// It is gone: its content goes.
    Unmount(Layer),
    /// A rule closed it: its owner answers as cancelled (a sheet, a dialog, a prompt denied).
    Cancel(Layer),
    /// The keys go to this owner.
    Focus(WindowId, KeyOwner),
    /// It stays with its tab, which is no longer visible.
    Hide(Layer),
    /// Its tab is visible again.
    Reveal(Layer),
    /// A hover slot shows for this key (moved to it, when it showed another).
    Show(Surface, Key),
    /// A hover slot hides (its kit draws the fade).
    Unshow(Surface),
    /// The host's one timer: fire at this time, or stop.
    Arm(Option<u64>),
}

impl Ui {
    pub fn phase(&self, w: WindowId, layer: Layer) -> Option<Phase> {
        self.entries.get(&(home(w, layer), layer.kind)).copied()
    }

    /// The entries window `w` can show: its own and every tab's.
    fn in_window(&self, w: WindowId) -> impl Iterator<Item = (Layer, Phase)> + '_ {
        self.entries
            .iter()
            .filter_map(move |(&(h, kind), &p)| match h {
                Home::Window(x) if x != w => None,
                Home::Window(_) => Some((Layer::new(kind, None), p)),
                Home::Tab(t) => Some((Layer::new(kind, Some(t)), p)),
            })
    }

    /// The open overlays in `w`, the store's and the machines' (`Env::others`).
    fn live(&self, env: &impl Env, w: WindowId) -> Vec<Layer> {
        self.in_window(w)
            .filter(|&(_, p)| p == Phase::Open)
            .map(|(l, _)| l)
            .chain(env.others(w))
            .collect()
    }

    /// The open overlays drawn in `w` now (a hidden tab's are not).
    fn visible(&self, env: &impl Env, w: WindowId) -> Vec<Layer> {
        let shown = env.shown(w);
        let mut v = self.live(env, w);
        v.retain(|l| l.tab.is_none() || l.tab == shown);
        v
    }

    /// The window's stack, for [`super::covers`]: open (every tab's: the hidden ones are not
    /// drawn) and leaving.
    pub fn stack(&self, env: &impl Env, w: WindowId) -> Stack {
        Stack {
            shown: env.shown(w),
            layers: self.live(env, w),
            leaving: self
                .in_window(w)
                .filter(|&(_, p)| p == Phase::Leaving)
                .map(|(l, _)| l)
                .collect(),
        }
    }

    /// Who has the keys: the topmost visible overlay that takes them as it opens, else the page.
    /// After a close it is the focus to return to (a leaving one is outside the chain).
    pub fn key_owner(&self, env: &impl Env, w: WindowId) -> KeyOwner {
        keys_among(self.visible(env, w))
    }

    /// What Escape closes: the topmost visible overlay (in [`Kind`]'s order) that takes it from
    /// whoever has the keys; none, the key reaches the page.
    pub fn escape_target(&self, env: &impl Env, w: WindowId) -> Option<Layer> {
        let from_page = env.page_has_keys(w);
        self.visible(env, w)
            .into_iter()
            .filter(|l| match l.kind.row().escape {
                Esc::Never => false,
                Esc::ShellOnly => !from_page,
                Esc::Any => true,
            })
            .max_by_key(|l| l.kind)
    }

    /// Who takes a press outside every overlay's own rect: the topmost scrim or dismissing one.
    pub fn pointer_owner(&self, env: &impl Env, w: WindowId) -> PointerOwner {
        self.visible(env, w)
            .into_iter()
            .filter(|l| {
                matches!(
                    l.kind.row().pointer,
                    Pointer::Scrim | Pointer::DismissOutside
                )
            })
            .max_by_key(|l| l.kind.row().z)
            .map_or(PointerOwner::Page, PointerOwner::Layer)
    }

    pub fn ax_roots(&self, env: &impl Env, w: WindowId) -> AxRoots {
        let mut roots = self.visible(env, w);
        roots.sort_by_key(|l| l.kind.row().z);
        let modal = roots
            .iter()
            .rev()
            .find(|l| matches!(l.kind.row().group, Group::Modal | Group::Holds))
            .copied();
        AxRoots { modal, roots }
    }

    /// Nothing leaving, no hover deadline: no exit for the kit to report, no timer armed.
    pub fn idle(&self) -> bool {
        self.entries.values().all(|&p| p == Phase::Open) && self.deadline().is_none()
    }

    /// A hover slot's state.
    pub fn hover(&self, at: Surface) -> Hover<Key> {
        self.hover.get(&at).copied().unwrap_or_default()
    }

    /// The key a hover slot is up for (also on its way out).
    pub fn shown(&self, at: Surface) -> Option<Key> {
        hovercard::shown(self.hover(at))
    }

    fn deadline(&self) -> Option<u64> {
        self.hover
            .values()
            .filter_map(|&h| hovercard::deadline(h))
            .min()
    }

    /// One slot's step, its effect pushed.
    fn slot(&mut self, at: Surface, event: Event<Key>, fx: &mut Vec<Effect>) {
        let Some(timing) = at.kind.timing() else {
            return;
        };
        let (next, effect) = hovercard::transition(self.hover(at), event, &timing);
        if next == Hover::Idle {
            self.hover.remove(&at);
        } else {
            self.hover.insert(at, next);
        }
        fx.extend(effect.map(|e| match e {
            hovercard::Effect::Show(key) => Effect::Show(at, key),
            hovercard::Effect::Hide => Effect::Unshow(at),
        }));
    }

    fn cue(&mut self, at: Surface, key: Key, cue: Cue, now: u64, fx: &mut Vec<Effect>) {
        let on = match self.hover(at) {
            Hover::Pending { key: k, .. }
            | Hover::Shown { key: k }
            | Hover::Leaving { key: k, .. } => k == key,
            Hover::Idle | Hover::Cooling { .. } => false,
        };
        let event = match cue {
            Cue::Enter => Event::Enter { key, now },
            Cue::Leave if on => Event::Leave { now },
            Cue::Keep if on => Event::Keep,
            // `Key::None` dismisses whatever the slot is on
            Cue::Dismiss if on || key == Key::None => Event::Dismiss,
            // a leave, a return or a dismissal of a key the slot is not on: a stale one
            Cue::Leave | Cue::Keep | Cue::Dismiss => return,
            Cue::Show => Event::Show { key },
            Cue::ShowFor(ms) => Event::ShowFor { key, now, ms },
        };
        self.slot(at, event, fx);
    }

    /// Every slot whose deadline passed, through as many of its steps as are due (a hide
    /// with no warm time rests in the same tick).
    fn tick(&mut self, now: u64, fx: &mut Vec<Effect>) {
        let slots: Vec<Surface> = self.hover.keys().copied().collect();
        for at in slots {
            while self
                .hover
                .get(&at)
                .is_some_and(|&h| hovercard::deadline(h).is_some_and(|d| d <= now))
            {
                self.slot(at, Event::Tick { now }, fx);
            }
        }
    }

    /// The slots for `tab` go (`nav`: only the ones whose row closes on a navigation).
    fn drop_tab(&mut self, tab: TabId, nav: bool, fx: &mut Vec<Effect>) {
        let slots: Vec<Surface> = self
            .hover
            .iter()
            .filter(|&(s, &h)| {
                (!nav || s.kind.row().on_nav)
                    && [hovercard::shown(h), hovercard::active(h)]
                        .iter()
                        .any(|k| k.and_then(Key::tab) == Some(tab))
            })
            .map(|(&s, _)| s)
            .collect();
        for at in slots {
            self.slot(at, Event::Dismiss, fx);
        }
    }

    /// The timer for the soonest deadline, when it changed. A deadline that only moved later
    /// keeps the timer armed (it fires early, and that tick arms the true one): a pointer
    /// moving over a video never re-arms it. `fired`: the timer just ran out.
    fn arm(&mut self, fired: bool) -> Option<Effect> {
        let next = self.deadline();
        let armed = if fired { None } else { self.armed };
        if matches!((armed, next), (Some(a), Some(n)) if n > a) {
            return None;
        }
        self.armed = next;
        (next != armed).then_some(Effect::Arm(next))
    }

    /// The leaving overlays of `w` the kit no longer draws (`drawn` false, or their tab not
    /// shown): the host sends [`Action::Exited`] for each, after every action and on the kit's
    /// `mounted-changed`, so a close before the first render ends too.
    pub fn gone(&self, env: &impl Env, w: WindowId, drawn: impl Fn(Layer) -> bool) -> Vec<Layer> {
        let shown = env.shown(w);
        self.in_window(w)
            .filter(|&(l, p)| {
                p == Phase::Leaving
                    && match l.tab {
                        None => !drawn(l),
                        Some(t) if shown == Some(t) => !drawn(l),
                        Some(t) => env.showing(t).is_none(),
                    }
            })
            .map(|(l, _)| l)
            .collect()
    }
}

/// The topmost of `layers` that takes the keys as it opens, else the page.
fn keys_among(layers: Vec<Layer>) -> KeyOwner {
    layers
        .into_iter()
        .filter(|l| l.kind.row().keys == Keys::Take)
        .max_by_key(|l| l.kind.row().z)
        .map_or(KeyOwner::Page, KeyOwner::Layer)
}

/// One action's run: the store as it changes and the effects so far.
struct Step<'a, E> {
    ui: Ui,
    env: &'a E,
    w: WindowId,
    fx: Vec<Effect>,
    /// An overlay that takes the keys opened, or one that had them closed: the keys move.
    focus: bool,
}

impl<E: Env> Step<'_, E> {
    /// Closes `l` (`cancel`: by a rule, so its owner answers): leaving while its kit draws the
    /// exit, else gone. A machine's overlay is only cancelled (its machine closes it).
    fn close(&mut self, l: Layer, cancel: bool) {
        let key = (home(self.w, l), l.kind);
        let open = self.ui.entries.get(&key) == Some(&Phase::Open);
        // a rule closes only live overlays: the store's open ones, or a machine's
        if cancel && (open || self.env.machine(l.kind)) {
            self.fx.push(Effect::Cancel(l));
        }
        if !open {
            return;
        }
        let row = l.kind.row();
        if row.tail && l.tab.is_none_or(|t| self.env.showing(t).is_some()) {
            self.ui.entries.insert(key, Phase::Leaving);
        } else {
            self.ui.entries.remove(&key);
            self.fx.push(Effect::Unmount(l));
        }
        self.focus |= row.keys != Keys::Never;
    }

    fn open(&mut self, l: Layer) {
        let (w, env) = (self.w, self.env);
        let row = l.kind.row();
        let key = (home(w, l), l.kind);
        let was = self.ui.entries.get(&key).copied();
        // already open: its owner takes the request (a second permission queues, Mod+F selects)
        if was == Some(Phase::Open) || env.others(w).contains(&l) {
            return;
        }
        if !row.windows.contains(&env.kind(w)) {
            return;
        }
        // where it shows: a hidden tab's overlay meets nothing that is visible
        let seen = if l.tab.is_none() || l.tab == env.shown(w) {
            self.ui.visible(env, w)
        } else {
            vec![]
        };
        if seen
            .iter()
            .any(|o| row.refused_under.contains(&o.kind.row().group))
        {
            return;
        }
        let mut gone: Vec<Layer> = seen
            .into_iter()
            .filter(|o| row.closes.contains(&o.kind.row().group))
            .collect();
        if row.one_per_window {
            // this window's: its own, its tab's and the hidden tabs' (another window's stay)
            let here = |o: &Layer| o.tab.is_none_or(|t| env.showing(t).is_none_or(|x| x == w));
            let same: Vec<Layer> = self
                .ui
                .live(env, w)
                .into_iter()
                .filter(|o| o.kind == l.kind && here(o) && !gone.contains(o))
                .collect();
            gone.extend(same);
        }
        for g in gone {
            self.close(g, true);
        }
        if env.machine(l.kind) {
            // admitted: its machine opens it
            self.fx.push(Effect::Mount(l));
            return;
        }
        if was == Some(Phase::Leaving) {
            self.fx.push(Effect::Unmount(l));
        }
        self.ui.entries.insert(key, Phase::Open);
        self.fx.push(Effect::Mount(l));
        self.focus |= row.keys == Keys::Take;
    }
}

impl Ui {
    /// Unmounts every entry of `home` (`cancel`: an open one is cancelled first: its owner
    /// still waits; a view gone answered everything it held).
    fn drop_home(&mut self, home: Home, cancel: bool) -> Vec<Effect> {
        let gone: Vec<_> = self
            .entries
            .iter()
            .filter(|((h, _), _)| *h == home)
            .map(|(&k, &p)| (k, p))
            .collect();
        let mut fx = vec![];
        for ((h, kind), p) in gone {
            let l = match h {
                Home::Window(_) => Layer::new(kind, None),
                Home::Tab(t) => Layer::new(kind, Some(t)),
            };
            if cancel && p == Phase::Open {
                fx.push(Effect::Cancel(l));
            }
            self.entries.remove(&(h, kind));
            fx.push(Effect::Unmount(l));
        }
        fx
    }
}

/// One step of the store: the hover slots (a cue, the timer, a navigation, a tab or window
/// gone), then the overlays, then the timer armed for what is due next.
pub fn reduce(mut ui: Ui, env: &impl Env, action: Action) -> (Ui, Vec<Effect>) {
    let mut fx = vec![];
    match action {
        Action::Hover { at, key, cue, now } => ui.cue(at, key, cue, now, &mut fx),
        Action::Tick { now } => ui.tick(now, &mut fx),
        Action::Navigated { tab, .. } => ui.drop_tab(tab, true, &mut fx),
        Action::ViewGone(tab) => ui.drop_tab(tab, false, &mut fx),
        // its slots go with it, nothing left to hide
        Action::WindowGone(w) => ui.hover.retain(|s, _| s.w != w),
        _ => {}
    }
    if !matches!(action, Action::Hover { .. } | Action::Tick { .. }) {
        let (next, more) = overlays(ui, env, action);
        ui = next;
        fx.extend(more);
    }
    fx.extend(ui.arm(matches!(action, Action::Tick { .. })));
    (ui, fx)
}

/// One step of the overlays: every (phase, action) cell is decided here (absent, Open,
/// Leaving), each by the acting row's data. A refused open changes nothing; an action on a
/// leaving overlay is ignored but its reopening (fresh content: Unmount, Mount) and its
/// `Exited`. Add behaviour as a [`super::Row`] field, not a branch on a kind.
fn overlays(mut ui: Ui, env: &impl Env, action: Action) -> (Ui, Vec<Effect>) {
    // a tab's overlay meets the overlays of the window that shows its tab, wherever it was asked
    let at = |w: WindowId, tab: Option<TabId>| tab.and_then(|t| env.showing(t)).unwrap_or(w);
    let w = match action {
        Action::Open { w, layer } | Action::Close { w, layer } | Action::Exited { w, layer } => {
            at(w, layer.tab)
        }
        Action::Navigated { w, tab } => at(w, Some(tab)),
        Action::Escape { w } | Action::Switched { w, .. } => w,
        Action::ViewGone(tab) => {
            let fx = ui.drop_home(Home::Tab(tab), false);
            return (ui, fx);
        }
        Action::WindowGone(w) => {
            let fx = ui.drop_home(Home::Window(w), true);
            return (ui, fx);
        }
        Action::Hover { .. } | Action::Tick { .. } => return (ui, vec![]),
    };
    let mut s = Step {
        ui,
        env,
        w,
        fx: vec![],
        focus: false,
    };
    match action {
        Action::Open { layer, .. } => s.open(Layer::new(layer.kind, layer.tab)),
        Action::Close { layer, .. } => s.close(Layer::new(layer.kind, layer.tab), false),
        Action::Exited { layer, .. } => {
            let key = (home(w, layer), layer.kind);
            if s.ui.entries.get(&key) == Some(&Phase::Leaving) {
                s.ui.entries.remove(&key);
                s.fx.push(Effect::Unmount(layer));
            }
        }
        Action::Escape { .. } => {
            if let Some(top) = s.ui.escape_target(env, w) {
                s.close(top, true);
            }
        }
        Action::Switched { from, to, .. } => {
            let mut stays = vec![];
            for l in s.ui.live(env, w).into_iter().filter(|_| from != to) {
                let mine = l.tab.is_some() && l.tab == from;
                if l.kind.row().on_leave && (mine || l.tab.is_none()) {
                    s.close(l, true);
                } else if mine {
                    stays.push(Effect::Hide(l));
                } else if l.tab.is_some() && l.tab == to {
                    stays.push(Effect::Reveal(l));
                }
            }
            s.fx.extend(stays);
        }
        Action::Navigated { tab, .. } => {
            for l in s.ui.live(env, w) {
                if l.tab == Some(tab) && l.kind.row().on_nav {
                    s.close(l, true);
                }
            }
        }
        Action::ViewGone(_)
        | Action::WindowGone(_)
        | Action::Hover { .. }
        | Action::Tick { .. } => {
            unreachable!("answered above")
        }
    }
    if s.focus {
        // a machine's overlay cancelled here is still in `others` until its machine closes it
        let mut still = s.ui.visible(env, w);
        still.retain(|l| !s.fx.contains(&Effect::Cancel(*l)));
        s.fx.push(Effect::Focus(w, keys_among(still)));
    }
    (s.ui, s.fx)
}

#[cfg(test)]
mod tests;
