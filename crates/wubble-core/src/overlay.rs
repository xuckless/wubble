//! Every surface the shell draws over a page, as one table ([`ROWS`]), and the rules between the
//! overlays among them: which is on top, and what opening one, Escape, a tab switch or a
//! navigation does to the others. The one owner of the surfaces' static properties, of the
//! overlays' order and scope, and of the answer to "what shell UI covers this page now"
//! ([`covers`]).
//!
//! Each surface keeps its own owner for whether it shows and what it shows (the menu's machine,
//! the print sheet's settings, the find bar's query, the hover clocks, the driver's tweens); this
//! module owns the rules between them. The host derives the [`Stack`] from those owners (what is
//! open, for which tab, and what is still drawn while it leaves) and runs [`transition`] or
//! [`covers`]; the effects close, hide or reveal overlays through their own machines. So whether
//! a surface is open has one owner (its machine), which may be open together has one owner
//! (here), and the stack is a value derived each time, never a copy kept beside them.
//!
//! The rules (Chromium's: one tab-modal sheet or bubble at a time, the find bar belongs to its
//! tab, Escape closes the innermost) govern the overlays, the rows whose [`Group`] is not
//! [`Group::Free`]; a free surface (a tooltip, the video strip, a drag's ghost) is in the stack
//! only for [`covers`]:
//! - **Exclusive** overlays show one at a time: opening one closes the other visible one. A
//!   page's dialog is never closed by another: while one shows, the user's overlays are refused
//!   (as Chromium's tab-modal dialog), and the dialog itself closes the user's overlays when it
//!   arrives.
//! - The **find bar** coexists under any exclusive overlay but a modal sheet (dialog, Add
//!   bookmark, Print): it is refused under one, and opening it closes a menu, the command bar,
//!   link hints or the screenshot picker.
//! - A **notice** (SponsorBlock's question) sits under everything, takes Escape only when nothing
//!   above it is open and the shell has the keys, and one shows at a time.
//! - A **permission prompt** belongs to its tab and coexists under other overlays; Escape
//!   dismisses it (denied, not remembered) only when the shell has the keys.
//! - **Escape** closes the topmost visible overlay that takes it; a second Escape the next. A
//!   notice and a permission prompt never take the page's Escape ([`Event::PageEscape`]): a key
//!   meant for the page (leaving fullscreen, the site's own dialog) must not answer them
//!   (no keyjacking, docs/research/mcp-server.md).
//! - **Tab switch**: menus, sheets, link hints and the screenshot picker close; the find bar, a
//!   page's dialog and a notice stay with their tab, hidden, and are revealed only on it. The
//!   window's command bar and downloads popover stay.
//! - **Navigation** (a new document): the tab's sheets, find bar, link hints, screenshot picker
//!   and notice close; a page's dialog is the engine's to drop.
//! - A surface **leaving** (its owner closed it, its kit still draws the exit) is closed for
//!   these rules and still covers the page ([`Stack::leaving`]).
//!
//! A pointer press outside an overlay is the kit's: the menu's and the command bar's scrims
//! dismiss them, a sheet's scrim holds the page (modal); none of it needs a rule here. A
//! dismissing press is consumed whole (kit `Scrim`), so a press on the popup's own toggle (the
//! "^", the downloads button, a chip, a Select's trigger) closes it and is no click that opens
//! it again.

use crate::tab::TabId;
use crate::window::WindowKind;

/// A surface. The overlays come first, in their Escape order: a later one is closed by Escape
/// before an earlier one. Drawing order is each row's [`Row::z`]. Add a surface as a variant at
/// the end and its row at the end of [`ROWS`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Kind {
    /// A lasting notice of the page (SponsorBlock's "Skip sponsor segments?"), on the toast.
    Notice,
    /// A tab's permission prompt (camera, mic, notifications, external link).
    Permission,
    /// The find bar.
    Find,
    /// A page's JS dialog or sign-in sheet (the engine waits for its answer).
    Dialog,
    /// Add bookmark (and the bookmarks page's rename, move and new-folder sheets).
    BookmarkSheet,
    PrintSheet,
    /// The extension install prompt: what an extension asks for, Allow or Cancel.
    ExtensionSheet,
    /// An agent session's approval sheet (`agent`): the window's, modal, one at a time.
    AgentSheet,
    /// The screenshot region picker.
    Region,
    /// Keyboard link hints (drawn by the page itself, `hints.rs`).
    Hints,
    /// The command bar over the page.
    CommandBar,
    /// The sidebar's downloads popover.
    Downloads,
    /// The sidebar footer's extensions popover, and the action popup that replaces its list.
    Extensions,
    /// The sidebar footer's Spaces popup (the "^": every Space, then New Space).
    Spaces,
    /// A context menu: a tab's, a folder's or the page's.
    Menu,
    // ----- free surfaces: outside the overlays' rules
    /// The page's error panel in place of the page (a failed load, a refused certificate).
    ErrorPanel,
    /// The page's own popup widget (a `<select>` list), CEF's second frame stream.
    Popup,
    /// A brief notice ("125%", "Press Esc to exit full screen", a notice with a button).
    Toast,
    /// The one tooltip (a shell button's, a page's `title`).
    Tooltip,
    /// The tab hover preview card.
    Preview,
    /// The hovered link's status bubble at its pane's foot.
    Status,
    /// Password autofill's key chip.
    AutofillChip,
    /// The key chip's menu of saved logins.
    AutofillMenu,
    /// The save or update prompt after a sign-in.
    AutofillPrompt,
    /// The video controls' strip over the key pane's video.
    VideoStrip,
    /// The collapsed strip's tab at the pane's edge.
    VideoTab,
    /// The strip's speed menu.
    VideoSpeeds,
    /// The picture-in-picture window's strip, on hover.
    PipStrip,
    /// A Main window's card for its tab playing in picture-in-picture.
    PipCard,
    /// Reader mode's typography strip.
    Reader,
    /// The Boost editor docked at the page's right edge.
    Boost,
    /// The window's peek (Shift+click): a page in a card over the page.
    Glance,
    /// The touchpad history swipe's chip.
    Swipe,
    /// The page a tab switch hid, leaving under the shown one.
    ExitPage,
    /// The shown page's entrance after a tab switch (it moves and scales the page).
    Entrance,
    /// A hidden sidebar peeking over the page.
    Peek,
    /// The bowed seam string over the page's edge, the page's frame cut along it.
    SeamBow,
    /// The split's focus ring round the focused pane.
    SplitRing,
    /// The split's seams between the panes.
    SplitSeams,
    /// The docked DevTools panel's close button, over its own pane.
    DevTools,
    /// A tab dragged out of the list: its title follows the pointer.
    TabGhost,
    /// The content edge a dragged tab would split into.
    DropZone,
    /// A page's drag: its image or label follows the pointer.
    PageGhost,
    /// The shell target a page's drag would drop on.
    DropMark,
    /// Middle-click autoscroll's own pointer.
    Autoscroll,
    /// The gallery's agent widgets specimen.
    Specimen,
}

/// Whom a surface belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    /// The window: shown over whichever tab is visible (and over every pane).
    Window,
    /// One tab: shown only while it is the window's visible one (over every pane).
    Tab,
    /// One pane on screen, its tab's: shown over that pane only.
    Pane,
}

/// How a surface lives with the others.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Group {
    /// One at a time: opening one closes the other.
    Exclusive,
    /// Exclusive, and holds the page under a scrim: the find bar waits for it.
    Modal,
    /// Modal, and answers someone outside the window (the page's engine, an agent's client):
    /// no other overlay closes it, and while one shows the user's overlays are refused.
    Holds,
    /// Under the overlays' rules (Escape, a tab switch, a navigation), but it closes nothing and
    /// nothing replaces it.
    Coexist,
    /// Outside the overlays' rules: its owner alone shows and hides it.
    Free,
}

/// Whether a surface takes the keyboard.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Keys {
    /// As it opens.
    Take,
    /// When asked (Mod+F for the find bar, a click in a field).
    OnAsk,
    /// Never: a key meant for the page must not answer it (keyjacking).
    Never,
}

/// What a surface does with the pointer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pointer {
    /// A scrim holds the page under it.
    Scrim,
    /// A press outside dismisses it (and is consumed).
    DismissOutside,
    /// It takes the pointer over itself only.
    OwnRect,
    /// The pointer passes through to the page.
    PassThrough,
    /// It holds the pointer while a drag or a pick runs.
    Grab,
}

/// What a surface needs of the page under it. Ordered: [`covers`] answers the most any surface
/// over the pane needs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Coverage {
    /// Nothing of the shell over the page.
    None,
    /// Shell pixels over part of the page (or all of it): the page shows around and under them.
    Part,
    /// The shell needs the page's own pixels: glass blurring it, an entrance moving it, a mask
    /// cutting it.
    Pixels,
}

/// How long a surface lives, and who ends it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Life {
    /// Until answered.
    Answer,
    /// Until dismissed (a click outside, Escape, its own close).
    Dismiss,
    /// For this many ms, unless replaced.
    Timed(u64),
    /// While hovered (with its owner's delays).
    Hover,
    /// While a drag or gesture runs.
    Drag,
    /// As its owner's state says (the page's error, the reader on, a transition's run).
    Owner,
}

/// Whether Escape closes a surface.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Esc {
    Never,
    /// Only while the shell has the keys: the page's Escape never answers it.
    ShellOnly,
    Any,
}

/// One surface's static properties.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Row {
    pub kind: Kind,
    pub scope: Scope,
    /// Its place in its window's drawing order (app.slint's), back to front; distinct in a
    /// window.
    pub z: u16,
    pub group: Group,
    pub keys: Keys,
    pub pointer: Pointer,
    /// What it needs of a page under it with the scene's glass off.
    pub cover: Coverage,
    /// Its scene glass, the `GlassKeys` name: with the scene's glass on it reads the page's
    /// pixels.
    pub glass: Option<&'static str>,
    pub life: Life,
    /// The windows it can show in.
    pub windows: &'static [WindowKind],
    pub escape: Esc,
    /// Closes when its tab stops being the visible one (a window overlay: on any switch); else
    /// it stays with its tab, hidden.
    pub on_leave: bool,
    /// Closes when its tab's main frame commits a new document.
    pub on_nav: bool,
}

/// How long a plain toast stays (wubble-only: no design counterpart).
pub const TOAST_SHOWN_MS: u64 = 1_500;

const ALL: &[WindowKind] = &[
    WindowKind::Main,
    WindowKind::Little,
    WindowKind::Popup,
    WindowKind::Pip,
];
const PAGES: &[WindowKind] = &[WindowKind::Main, WindowKind::Little, WindowKind::Popup];
const MAIN: &[WindowKind] = &[WindowKind::Main];
const PIP: &[WindowKind] = &[WindowKind::Pip];

/// A popover or menu the user opened: one at a time, a press outside closes it.
const DISMISSED: Row = Row {
    kind: Kind::Menu,
    scope: Scope::Window,
    z: 0,
    group: Group::Exclusive,
    keys: Keys::Take,
    pointer: Pointer::DismissOutside,
    cover: Coverage::Part,
    glass: None,
    life: Life::Dismiss,
    windows: ALL,
    escape: Esc::Any,
    on_leave: true,
    on_nav: false,
};

/// A modal sheet: it asks, the page waits under its scrim.
const SHEET: Row = Row {
    group: Group::Modal,
    pointer: Pointer::Scrim,
    life: Life::Answer,
    ..DISMISSED
};

/// A free surface: shown by its owner, over the page, taking nothing.
const FREE: Row = Row {
    group: Group::Free,
    keys: Keys::Never,
    pointer: Pointer::PassThrough,
    life: Life::Owner,
    escape: Esc::Never,
    on_leave: false,
    ..DISMISSED
};

/// Every surface, in [`Kind`]'s order (one row each: the test checks it). Every property of a
/// surface is a field here; a new surface is a new row, a new behaviour a new field, never a
/// branch on a kind elsewhere.
#[rustfmt::skip]
pub const ROWS: &[Row] = &[
    Row { kind: Kind::Notice, scope: Scope::Tab, z: 63, group: Group::Coexist, keys: Keys::Never, pointer: Pointer::OwnRect,
        glass: Some("toast"), life: Life::Answer, escape: Esc::ShellOnly, on_leave: false, on_nav: true, ..DISMISSED },
    Row { kind: Kind::Permission, scope: Scope::Tab, z: 94, group: Group::Coexist, keys: Keys::Never, pointer: Pointer::OwnRect,
        glass: Some("permission"), life: Life::Answer, escape: Esc::ShellOnly, on_leave: false, ..DISMISSED },
    Row { kind: Kind::Find, scope: Scope::Tab, z: 50, group: Group::Coexist, keys: Keys::OnAsk, pointer: Pointer::OwnRect,
        glass: Some("find"), on_leave: false, on_nav: true, ..DISMISSED },
    Row { kind: Kind::Dialog, scope: Scope::Tab, z: 72, group: Group::Holds, glass: Some("dialog"), on_leave: false, ..SHEET },
    Row { kind: Kind::BookmarkSheet, scope: Scope::Tab, z: 66, glass: Some("bookmark-sheet"), on_nav: true, ..SHEET },
    Row { kind: Kind::PrintSheet, scope: Scope::Tab, z: 68, glass: Some("print-sheet"), on_nav: true, ..SHEET },
    Row { kind: Kind::ExtensionSheet, z: 67, glass: Some("extension-sheet"), ..SHEET },
    Row { kind: Kind::AgentSheet, z: 69, group: Group::Holds, glass: Some("agent-sheet"), on_leave: false, ..SHEET },
    Row { kind: Kind::Region, scope: Scope::Tab, z: 56, pointer: Pointer::Grab, glass: Some("shot"), on_nav: true, ..DISMISSED },
    // the page draws the labels: nothing of the shell over it
    Row { kind: Kind::Hints, scope: Scope::Tab, z: 5, pointer: Pointer::PassThrough, cover: Coverage::None, on_nav: true, ..DISMISSED },
    Row { kind: Kind::CommandBar, z: 74, glass: Some("command-bar"), on_leave: false, ..DISMISSED },
    Row { kind: Kind::Downloads, z: 100, glass: Some("downloads"), on_leave: false, ..DISMISSED },
    Row { kind: Kind::Extensions, z: 102, glass: Some("extensions"), ..DISMISSED },
    Row { kind: Kind::Spaces, z: 104, glass: Some("spaces"), windows: MAIN, ..DISMISSED },
    Row { kind: Kind::Menu, z: 110, glass: Some("menu"), ..DISMISSED },
    // ----- free
    Row { kind: Kind::ErrorPanel, scope: Scope::Tab, z: 60, keys: Keys::Take, pointer: Pointer::OwnRect, glass: Some("error"), ..FREE },
    Row { kind: Kind::Popup, z: 90, pointer: Pointer::OwnRect, ..FREE },
    Row { kind: Kind::Toast, z: 62, glass: Some("toast"), life: Life::Timed(TOAST_SHOWN_MS), ..FREE },
    Row { kind: Kind::Tooltip, z: 114, life: Life::Hover, ..FREE },
    Row { kind: Kind::Preview, z: 52, glass: Some("preview"), life: Life::Hover, ..FREE },
    Row { kind: Kind::Status, scope: Scope::Pane, z: 30, life: Life::Hover, ..FREE },
    Row { kind: Kind::AutofillChip, scope: Scope::Tab, z: 96, pointer: Pointer::OwnRect, ..FREE },
    Row { kind: Kind::AutofillMenu, scope: Scope::Tab, z: 97, pointer: Pointer::OwnRect, glass: Some("autofill-menu"), life: Life::Dismiss, ..FREE },
    Row { kind: Kind::AutofillPrompt, scope: Scope::Tab, z: 98, pointer: Pointer::OwnRect, glass: Some("autofill-prompt"), life: Life::Answer, ..FREE },
    Row { kind: Kind::VideoStrip, scope: Scope::Pane, z: 40, pointer: Pointer::OwnRect, glass: Some("video-strip"), life: Life::Hover, windows: PAGES, ..FREE },
    Row { kind: Kind::VideoTab, scope: Scope::Pane, z: 41, pointer: Pointer::OwnRect, glass: Some("video-tab"), windows: PAGES, ..FREE },
    Row { kind: Kind::VideoSpeeds, scope: Scope::Pane, z: 42, pointer: Pointer::OwnRect, glass: Some("video-speeds"), life: Life::Dismiss, windows: PAGES, ..FREE },
    Row { kind: Kind::PipStrip, z: 44, pointer: Pointer::OwnRect, glass: Some("pip-strip"), life: Life::Hover, windows: PIP, ..FREE },
    Row { kind: Kind::PipCard, scope: Scope::Tab, z: 46, pointer: Pointer::OwnRect, windows: MAIN, ..FREE },
    Row { kind: Kind::Reader, scope: Scope::Tab, z: 54, pointer: Pointer::OwnRect, glass: Some("reader"), ..FREE },
    Row { kind: Kind::Boost, scope: Scope::Tab, z: 58, keys: Keys::OnAsk, pointer: Pointer::OwnRect, glass: Some("boost"), life: Life::Dismiss, ..FREE },
    Row { kind: Kind::Glance, z: 64, keys: Keys::OnAsk, pointer: Pointer::Scrim, glass: Some("glance"), life: Life::Dismiss, escape: Esc::Any, ..FREE },
    Row { kind: Kind::Swipe, z: 70, life: Life::Drag, ..FREE },
    Row { kind: Kind::ExitPage, z: 10, ..FREE },
    Row { kind: Kind::Entrance, z: 20, cover: Coverage::Pixels, ..FREE },
    Row { kind: Kind::Peek, z: 80, pointer: Pointer::OwnRect, glass: Some("peek"), life: Life::Hover, windows: PAGES, ..FREE },
    Row { kind: Kind::SeamBow, z: 82, pointer: Pointer::Grab, cover: Coverage::Pixels, life: Life::Drag, windows: PAGES, ..FREE },
    Row { kind: Kind::SplitRing, z: 34, ..FREE },
    Row { kind: Kind::SplitSeams, z: 36, pointer: Pointer::Grab, ..FREE },
    Row { kind: Kind::DevTools, scope: Scope::Pane, z: 32, pointer: Pointer::OwnRect, ..FREE },
    Row { kind: Kind::TabGhost, z: 108, life: Life::Drag, escape: Esc::Any, ..FREE },
    Row { kind: Kind::DropZone, z: 106, life: Life::Drag, ..FREE },
    Row { kind: Kind::PageGhost, z: 109, life: Life::Drag, escape: Esc::Any, ..FREE },
    Row { kind: Kind::DropMark, z: 107, life: Life::Drag, ..FREE },
    Row { kind: Kind::Autoscroll, z: 92, ..FREE },
    Row { kind: Kind::Specimen, z: 112, ..FREE },
];

impl Kind {
    /// Its row.
    pub fn row(self) -> &'static Row {
        &ROWS[self as usize]
    }

    pub fn scope(self) -> Scope {
        self.row().scope
    }

    /// One at a time.
    pub fn exclusive(self) -> bool {
        matches!(self.row().group, Group::Exclusive | Group::Modal | Group::Holds)
    }

    /// Holds the page under a scrim: the find bar waits for it.
    fn modal(self) -> bool {
        matches!(self.row().group, Group::Modal | Group::Holds)
    }

    fn holds(self) -> bool {
        self.row().group == Group::Holds
    }

    /// Under the overlays' rules.
    fn ruled(self) -> bool {
        self.row().group != Group::Free
    }

    /// Whether Escape closes it; `from_page`: the page has the keys.
    fn takes_escape(self, from_page: bool) -> bool {
        match self.row().escape {
            Esc::Never => false,
            Esc::ShellOnly => !from_page,
            Esc::Any => true,
        }
    }
}

/// One surface on screen: its kind and, unless the window's, its tab (a pane's: the pane's).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Layer {
    pub kind: Kind,
    pub tab: Option<TabId>,
}

impl Layer {
    /// `kind` for `tab`; a window's surface keeps no tab.
    pub fn new(kind: Kind, tab: Option<TabId>) -> Self {
        let tab = match kind.scope() {
            Scope::Window => None,
            Scope::Tab | Scope::Pane => tab,
        };
        Layer { kind, tab }
    }
}

/// What is open, what is still drawn as it leaves, and which tab is visible: derived by the host
/// from each surface's owner whenever it is needed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Stack {
    pub shown: Option<TabId>,
    pub layers: Vec<Layer>,
    /// Closed by their owners, still drawn by their kit until the exit ends: no rule sees them,
    /// and they cover the page.
    pub leaving: Vec<Layer>,
}

impl Stack {
    /// Drawn now: a window's and a pane's surface (the host derives only the panes on screen)
    /// always, a tab's while its tab is visible.
    fn visible(&self, layer: &Layer) -> bool {
        layer.kind.scope() == Scope::Pane || layer.tab.is_none() || layer.tab == self.shown
    }

    /// The topmost visible overlay.
    pub fn top(&self) -> Option<Layer> {
        self.layers
            .iter()
            .filter(|l| l.kind.ruled() && self.visible(l))
            .max_by_key(|l| l.kind)
            .copied()
    }

    pub fn is_open(&self, kind: Kind) -> bool {
        self.layers.iter().any(|l| l.kind == kind)
    }

    /// This stack with what the kit still draws (`drawn`: open, or its exit not yet over): a
    /// drawn surface with no visible layer of its kind open is leaving.
    pub fn with_drawn(mut self, drawn: &[Layer]) -> Stack {
        for d in drawn {
            if !self.layers.iter().any(|l| l.kind == d.kind && self.visible(l)) {
                self.leaving.push(*d);
            }
        }
        self
    }
}

/// What a page's pane is covered by that its owner, not the stack, knows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Facts {
    /// The scene draws the overlays' glass in this window (`GlassLayer.over-on`): a glass
    /// surface reads the page's pixels.
    pub glass: bool,
    /// The window's peek is open or leaving (`Model::glance_of`, its card's exit).
    pub glance: bool,
    /// The visible tab plays in picture-in-picture: its card stands in for it.
    pub pip_card: Option<TabId>,
}

/// What the shell draws over the page in pane `pane` of the window `stack` and `facts` describe:
/// the most any visible surface over it needs, a leaving one included.
pub fn covers(stack: &Stack, facts: &Facts, pane: TabId) -> Coverage {
    let owned = [
        facts.glance.then(|| Layer::new(Kind::Glance, None)),
        facts.pip_card.map(|t| Layer::new(Kind::PipCard, Some(t))),
    ];
    stack
        .layers
        .iter()
        .chain(&stack.leaving)
        .copied()
        .chain(owned.into_iter().flatten())
        .filter(|l| stack.visible(l) && (l.kind.scope() != Scope::Pane || l.tab == Some(pane)))
        .map(|l| {
            let row = l.kind.row();
            match row.glass {
                Some(_) if facts.glass && row.cover != Coverage::None => Coverage::Pixels,
                _ => row.cover,
            }
        })
        .max()
        .unwrap_or(Coverage::None)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    /// The user, a key or the page asks for an overlay.
    Open(Layer),
    /// Escape while the shell has the keys.
    Escape,
    /// Escape while the page has the keys.
    PageEscape,
    /// This tab is now the visible one (none: the window shows no tab).
    Switched(Option<TabId>),
    /// The tab's main frame committed a new document.
    Navigated(TabId),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effect {
    /// Close it through its machine (a sheet or dialog as cancelled, a menu as dismissed).
    Close(Layer),
    /// It stays with its tab, which is no longer visible.
    Hide(Layer),
    /// Its tab is visible again.
    Reveal(Layer),
    /// The asked overlay may open (no effect at all: refused).
    Open(Layer),
}

/// One step of the overlays.
pub fn transition(stack: Stack, event: Event) -> (Stack, Vec<Effect>) {
    match event {
        Event::Open(layer) => open(stack, layer),
        Event::Escape | Event::PageEscape => match stack.top() {
            Some(top) if top.kind.takes_escape(event == Event::PageEscape) => {
                close(stack, vec![top], vec![])
            }
            _ => (stack, vec![]),
        },
        Event::Switched(to) => switched(stack, to),
        Event::Navigated(tab) => {
            let gone = stack
                .layers
                .iter()
                .filter(|l| l.tab == Some(tab) && l.kind.row().on_nav)
                .copied()
                .collect();
            close(stack, gone, vec![])
        }
    }
}

fn open(stack: Stack, layer: Layer) -> (Stack, Vec<Effect>) {
    let layer = Layer::new(layer.kind, layer.tab);
    // already open: its machine decides (Mod+F again selects the query)
    if stack.layers.contains(&layer) {
        return (stack, vec![Effect::Open(layer)]);
    }
    // where it shows: a hidden tab's overlay meets nothing that is visible
    let shows = stack.visible(&layer);
    let visible: Vec<Layer> = stack
        .layers
        .iter()
        .filter(|l| shows && stack.visible(l))
        .copied()
        .collect();
    let refused = match layer.kind {
        Kind::Notice | Kind::Permission | Kind::Dialog => false,
        Kind::Find => visible.iter().any(|l| l.kind.modal()),
        // an agent's sheet never replaces the user's sheet, which it waits for
        Kind::AgentSheet => visible.iter().any(|l| l.kind.holds() || l.kind.modal()),
        _ => visible.iter().any(|l| l.kind.holds()),
    };
    if refused {
        return (stack, vec![]);
    }
    let gone: Vec<Layer> = match layer.kind {
        // one notice in the window
        Kind::Notice => stack
            .layers
            .iter()
            .filter(|l| l.kind == Kind::Notice)
            .copied()
            .collect(),
        Kind::Permission => vec![],
        Kind::Find => visible
            .into_iter()
            .filter(|l| l.kind.exclusive() && !l.kind.modal())
            .collect(),
        // a held overlay (a page's dialog, an agent's sheet) is never closed by another
        _ => visible
            .into_iter()
            .filter(|l| l.kind.exclusive() && !l.kind.holds())
            .collect(),
    };
    let (mut next, effects) = close(stack, gone, vec![]);
    next.layers.push(layer);
    let mut effects = effects;
    effects.push(Effect::Open(layer));
    (next, effects)
}

fn switched(stack: Stack, to: Option<TabId>) -> (Stack, Vec<Effect>) {
    let from = stack.shown;
    if from == to {
        return (stack, vec![]);
    }
    let mut gone = vec![];
    let mut effects = vec![];
    for l in stack.layers.iter().filter(|l| l.kind.ruled()) {
        let mine = l.tab.is_some() && l.tab == from;
        if l.kind.row().on_leave && (mine || l.tab.is_none()) {
            gone.push(*l);
        } else if mine {
            effects.push(Effect::Hide(*l));
        } else if l.tab.is_some() && l.tab == to {
            effects.push(Effect::Reveal(*l));
        }
    }
    close(Stack { shown: to, ..stack }, gone, effects)
}

/// Removes `gone` from the stack, each a [`Effect::Close`], before `then`.
fn close(stack: Stack, gone: Vec<Layer>, then: Vec<Effect>) -> (Stack, Vec<Effect>) {
    let Stack {
        shown,
        layers,
        leaving,
    } = stack;
    let layers = layers.into_iter().filter(|l| !gone.contains(l)).collect();
    let mut effects: Vec<Effect> = gone.into_iter().map(Effect::Close).collect();
    effects.extend(then);
    (
        Stack {
            shown,
            layers,
            leaving,
        },
        effects,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use Kind::*;

    const A: TabId = TabId(1);
    const B: TabId = TabId(2);

    fn on(kind: Kind, tab: TabId) -> Layer {
        Layer::new(kind, Some(tab))
    }

    fn win(kind: Kind) -> Layer {
        Layer::new(kind, None)
    }

    fn stack(layers: &[Layer]) -> Stack {
        Stack {
            shown: Some(A),
            layers: layers.to_vec(),
            ..Stack::default()
        }
    }

    fn step(layers: &[Layer], event: Event) -> (Vec<Layer>, Vec<Effect>) {
        let (next, effects) = transition(stack(layers), event);
        (next.layers, effects)
    }

    #[test]
    fn a_window_overlay_keeps_no_tab() {
        assert_eq!(Layer::new(Menu, Some(A)).tab, None);
        assert_eq!(Layer::new(Find, Some(A)).tab, Some(A));
    }

    #[test]
    fn the_spaces_popup_is_one_window_overlay_among_the_others() {
        assert_eq!(Layer::new(Spaces, Some(A)).tab, None);
        // it replaces the downloads popover, and a menu replaces it
        let (layers, fx) = step(&[win(Downloads)], Event::Open(win(Spaces)));
        assert_eq!(layers, [win(Spaces)]);
        assert_eq!(
            fx,
            [Effect::Close(win(Downloads)), Effect::Open(win(Spaces))]
        );
        let (layers, _) = step(&[win(Spaces)], Event::Open(win(Menu)));
        assert_eq!(layers, [win(Menu)]);
        // refused over a page's dialog; Escape closes it first; a tab switch closes it
        let (_, fx) = step(&[on(Dialog, A)], Event::Open(win(Spaces)));
        assert!(fx.is_empty());
        let (layers, fx) = step(&[on(Find, A), win(Spaces)], Event::Escape);
        assert_eq!(layers, [on(Find, A)]);
        assert_eq!(fx, [Effect::Close(win(Spaces))]);
        let (layers, _) = step(&[win(Spaces)], Event::Switched(Some(B)));
        assert!(layers.is_empty());
    }

    #[test]
    fn the_extensions_popover_is_a_window_overlay_that_follows_the_rules() {
        assert_eq!(Layer::new(Extensions, Some(A)).tab, None);
        // it replaces the downloads popover, and the Spaces popup replaces it
        let (layers, _) = step(&[win(Downloads)], Event::Open(win(Extensions)));
        assert_eq!(layers, [win(Extensions)]);
        let (layers, _) = step(&[win(Extensions)], Event::Open(win(Spaces)));
        assert_eq!(layers, [win(Spaces)]);
        // refused over a page's dialog; Escape and a tab switch close it
        let (_, fx) = step(&[on(Dialog, A)], Event::Open(win(Extensions)));
        assert!(fx.is_empty());
        let (layers, fx) = step(&[win(Extensions)], Event::Escape);
        assert!(layers.is_empty());
        assert_eq!(fx, [Effect::Close(win(Extensions))]);
        let (layers, _) = step(&[win(Extensions)], Event::Switched(Some(B)));
        assert!(layers.is_empty());
    }

    #[test]
    fn the_install_prompt_is_a_modal_sheet_that_holds_the_find_bar_off() {
        assert_eq!(Layer::new(ExtensionSheet, Some(A)).tab, None);
        let (_, fx) = step(&[win(ExtensionSheet)], Event::Open(on(Find, A)));
        assert!(fx.is_empty());
        // it replaces a popover; Escape cancels it
        let (layers, _) = step(&[win(Extensions)], Event::Open(win(ExtensionSheet)));
        assert_eq!(layers, [win(ExtensionSheet)]);
        let (layers, fx) = step(&[win(ExtensionSheet)], Event::Escape);
        assert!(layers.is_empty());
        assert_eq!(fx, [Effect::Close(win(ExtensionSheet))]);
    }

    #[test]
    fn print_over_add_bookmark_replaces_it() {
        let (layers, fx) = step(&[on(BookmarkSheet, A)], Event::Open(on(PrintSheet, A)));
        assert_eq!(layers, [on(PrintSheet, A)]);
        assert_eq!(
            fx,
            [
                Effect::Close(on(BookmarkSheet, A)),
                Effect::Open(on(PrintSheet, A))
            ]
        );
    }

    #[test]
    fn every_exclusive_overlay_replaces_the_shown_one() {
        let exclusive = [
            on(BookmarkSheet, A),
            on(PrintSheet, A),
            win(ExtensionSheet),
            on(Region, A),
            on(Hints, A),
            win(CommandBar),
            win(Downloads),
            win(Extensions),
            win(Menu),
        ];
        for old in exclusive {
            for new in exclusive.iter().filter(|n| **n != old) {
                let (layers, fx) = step(&[old], Event::Open(*new));
                assert_eq!(layers, [*new], "{new:?} over {old:?}");
                assert_eq!(fx, [Effect::Close(old), Effect::Open(*new)]);
            }
        }
    }

    #[test]
    fn opening_what_is_open_closes_nothing() {
        let (layers, fx) = step(&[on(Find, A)], Event::Open(on(Find, A)));
        assert_eq!(layers, [on(Find, A)]);
        assert_eq!(fx, [Effect::Open(on(Find, A))]);
    }

    #[test]
    fn a_page_dialog_refuses_the_user_s_overlays_and_closes_them_on_arrival() {
        for kind in [
            BookmarkSheet,
            PrintSheet,
            ExtensionSheet,
            Region,
            Hints,
            CommandBar,
            Downloads,
            Extensions,
            Menu,
            Find,
        ] {
            let layer = Layer::new(kind, Some(A));
            let (layers, fx) = step(&[on(Dialog, A)], Event::Open(layer));
            assert_eq!(layers, [on(Dialog, A)], "{kind:?}");
            assert!(fx.is_empty(), "{kind:?} opened over a dialog");
        }
        // it closes the user's sheet and menu, keeps the find bar under it
        let (layers, fx) = step(
            &[on(Find, A), on(BookmarkSheet, A)],
            Event::Open(on(Dialog, A)),
        );
        assert_eq!(layers, [on(Find, A), on(Dialog, A)]);
        assert_eq!(
            fx,
            [
                Effect::Close(on(BookmarkSheet, A)),
                Effect::Open(on(Dialog, A))
            ]
        );
    }

    #[test]
    fn a_hidden_tab_s_dialog_blocks_nothing_and_closes_nothing() {
        let (layers, fx) = step(&[on(Dialog, B)], Event::Open(on(PrintSheet, A)));
        assert_eq!(layers, [on(Dialog, B), on(PrintSheet, A)]);
        assert_eq!(fx, [Effect::Open(on(PrintSheet, A))]);
        let (layers, fx) = step(&[win(Menu)], Event::Open(on(Dialog, B)));
        assert_eq!(layers, [win(Menu), on(Dialog, B)]);
        assert_eq!(fx, [Effect::Open(on(Dialog, B))]);
    }

    #[test]
    fn the_find_bar_stays_under_a_menu_and_waits_for_a_sheet() {
        // a menu over the find bar: both
        let (layers, fx) = step(&[on(Find, A)], Event::Open(win(Menu)));
        assert_eq!(layers, [on(Find, A), win(Menu)]);
        assert_eq!(fx, [Effect::Open(win(Menu))]);
        // the find bar opened under a menu or the command bar closes them
        for kind in [Menu, CommandBar, Downloads] {
            let (layers, fx) = step(&[win(kind)], Event::Open(on(Find, A)));
            assert_eq!(layers, [on(Find, A)]);
            assert_eq!(fx, [Effect::Close(win(kind)), Effect::Open(on(Find, A))]);
        }
        // refused under a modal sheet
        for kind in [BookmarkSheet, PrintSheet] {
            let (layers, fx) = step(&[on(kind, A)], Event::Open(on(Find, A)));
            assert_eq!(layers, [on(kind, A)]);
            assert!(fx.is_empty());
        }
    }

    #[test]
    fn an_agent_sheet_is_a_window_modal_that_nothing_replaces() {
        let sheet = win(AgentSheet);
        assert_eq!(Layer::new(AgentSheet, Some(A)).tab, None);
        // it replaces a menu or the command bar
        for kind in [Menu, CommandBar, Downloads] {
            let (layers, fx) = step(&[win(kind)], Event::Open(sheet));
            assert_eq!(layers, [sheet]);
            assert_eq!(fx, [Effect::Close(win(kind)), Effect::Open(sheet)]);
        }
        // it waits for the user's sheet and for a page's dialog
        for other in [on(BookmarkSheet, A), on(PrintSheet, A), on(Dialog, A)] {
            let (layers, fx) = step(&[other], Event::Open(sheet));
            assert_eq!(layers, [other]);
            assert!(fx.is_empty());
        }
        // while it shows the user's overlays are refused, and a page's dialog does not close it
        for kind in [Menu, CommandBar, Downloads, Spaces] {
            let (layers, fx) = step(&[sheet], Event::Open(win(kind)));
            assert_eq!(layers, [sheet]);
            assert!(fx.is_empty(), "{kind:?}");
        }
        let (layers, fx) = step(&[sheet], Event::Open(on(Find, A)));
        assert_eq!(layers, [sheet]);
        assert!(fx.is_empty());
        let (layers, fx) = step(&[sheet], Event::Open(on(Dialog, A)));
        assert_eq!(layers, [sheet, on(Dialog, A)]);
        assert_eq!(fx, [Effect::Open(on(Dialog, A))]);
        // Escape closes it first (the topmost); a tab switch or navigation leaves it
        let (layers, fx) = step(&[sheet, on(Dialog, A)], Event::Escape);
        assert_eq!(layers, [on(Dialog, A)]);
        assert_eq!(fx, [Effect::Close(sheet)]);
        let (layers, fx) = step(&[sheet], Event::Switched(Some(B)));
        assert_eq!(layers, [sheet]);
        assert!(fx.is_empty());
        let (layers, fx) = step(&[sheet], Event::Navigated(A));
        assert_eq!(layers, [sheet]);
        assert!(fx.is_empty());
    }

    #[test]
    fn the_install_prompt_and_an_agent_sheet_show_one_at_a_time() {
        let agent = win(AgentSheet);
        let prompt = win(ExtensionSheet);
        // an agent's sheet waits for the install prompt
        let (layers, fx) = step(&[prompt], Event::Open(agent));
        assert_eq!(layers, [prompt]);
        assert!(fx.is_empty());
        // while an agent's sheet shows, the install prompt and the popover are refused
        for kind in [ExtensionSheet, Extensions] {
            let (layers, fx) = step(&[agent], Event::Open(win(kind)));
            assert_eq!(layers, [agent], "{kind:?}");
            assert!(fx.is_empty(), "{kind:?}");
        }
        // Escape closes each
        for sheet in [prompt, agent] {
            let (layers, fx) = step(&[sheet], Event::Escape);
            assert!(layers.is_empty());
            assert_eq!(fx, [Effect::Close(sheet)]);
        }
    }

    #[test]
    fn one_notice_in_the_window_under_everything() {
        let (layers, fx) = step(
            &[on(Notice, B), on(PrintSheet, A)],
            Event::Open(on(Notice, A)),
        );
        assert_eq!(layers, [on(PrintSheet, A), on(Notice, A)]);
        assert_eq!(
            fx,
            [Effect::Close(on(Notice, B)), Effect::Open(on(Notice, A))]
        );
        // it opens over a dialog too, and is never the top
        let s = stack(&[on(Dialog, A), on(Notice, A)]);
        assert_eq!(s.top(), Some(on(Dialog, A)));
    }

    #[test]
    fn escape_closes_the_topmost_then_the_next() {
        let layers = [on(Notice, A), on(Permission, A), on(Find, A), win(Menu)];
        let (layers, fx) = step(&layers, Event::Escape);
        assert_eq!(fx, [Effect::Close(win(Menu))]);
        let (layers, fx) = step(&layers, Event::Escape);
        assert_eq!(fx, [Effect::Close(on(Find, A))]);
        let (layers, fx) = step(&layers, Event::Escape);
        assert_eq!(fx, [Effect::Close(on(Permission, A))]);
        // Notice is lowest: Escape dismisses it once nothing sits above it.
        let (layers, fx) = step(&layers, Event::Escape);
        assert_eq!(fx, [Effect::Close(on(Notice, A))]);
        assert!(layers.is_empty());
        // With nothing left, Escape goes on to the page.
        let (layers, fx) = step(&layers, Event::Escape);
        assert!(fx.is_empty());
        assert!(layers.is_empty());
    }

    /// The page's Escape closes a shell overlay above, but never answers a notice or a
    /// permission prompt: it goes on to the page (fullscreen, the site's dialog).
    #[test]
    fn the_pages_escape_never_answers_a_notice_or_a_prompt() {
        let layers = [on(Notice, A), on(Permission, A), win(Menu)];
        let (layers, fx) = step(&layers, Event::PageEscape);
        assert_eq!(fx, [Effect::Close(win(Menu))]);
        let (layers, fx) = step(&layers, Event::PageEscape);
        assert!(fx.is_empty());
        assert_eq!(layers, [on(Notice, A), on(Permission, A)]);
        let (layers, fx) = step(&[on(Notice, A)], Event::PageEscape);
        assert!(fx.is_empty());
        assert_eq!(layers, [on(Notice, A)]);
        // with the shell's keys, each closes
        let (_, fx) = step(&[on(Permission, A)], Event::Escape);
        assert_eq!(fx, [Effect::Close(on(Permission, A))]);
        let (_, fx) = step(&[on(Notice, A)], Event::Escape);
        assert_eq!(fx, [Effect::Close(on(Notice, A))]);
    }

    #[test]
    fn permission_is_tab_scoped_and_coexists_with_other_overlays() {
        assert_eq!(Layer::new(Permission, Some(A)).tab, Some(A));
        assert!(!Permission.exclusive());
        // Opening a menu or find bar leaves a permission prompt in place.
        let (layers, fx) = step(&[on(Permission, A)], Event::Open(win(Menu)));
        assert_eq!(layers, [on(Permission, A), win(Menu)]);
        assert_eq!(fx, [Effect::Open(win(Menu))]);
        // Switching tabs hides the permission prompt rather than closing it.
        let (next, fx) = transition(stack(&[on(Permission, A)]), Event::Switched(Some(B)));
        assert_eq!(next.layers, [on(Permission, A)]);
        assert_eq!(fx, [Effect::Hide(on(Permission, A))]);
        // Escape on tab B does not touch tab A's permission prompt.
        let (after, fx) = transition(next, Event::Escape);
        assert_eq!(after.layers, [on(Permission, A)]);
        assert!(fx.is_empty());
    }

    #[test]
    fn escape_closes_each_kind() {
        for kind in [
            Notice,
            Permission,
            Find,
            Dialog,
            BookmarkSheet,
            PrintSheet,
            Region,
            Hints,
            CommandBar,
            Downloads,
            Menu,
        ] {
            let layer = Layer::new(kind, Some(A));
            let (layers, fx) = step(&[layer], Event::Escape);
            assert!(layers.is_empty(), "{kind:?}");
            assert_eq!(fx, [Effect::Close(layer)]);
        }
    }

    #[test]
    fn escape_skips_a_hidden_tab_s_overlays() {
        let (layers, fx) = step(&[on(Find, B)], Event::Escape);
        assert!(fx.is_empty());
        assert_eq!(layers, [on(Find, B)]);
    }

    #[test]
    fn a_tab_switch_closes_sheets_and_menus_and_hides_what_stays() {
        let layers = [
            on(Notice, A),
            on(Find, A),
            on(Dialog, A),
            on(BookmarkSheet, A),
            win(CommandBar),
            win(Menu),
            on(Find, B),
        ];
        let (next, fx) = transition(stack(&layers), Event::Switched(Some(B)));
        assert_eq!(next.shown, Some(B));
        assert_eq!(
            next.layers,
            [
                on(Notice, A),
                on(Find, A),
                on(Dialog, A),
                win(CommandBar),
                on(Find, B)
            ]
        );
        assert_eq!(
            fx,
            [
                Effect::Close(on(BookmarkSheet, A)),
                Effect::Close(win(Menu)),
                Effect::Hide(on(Notice, A)),
                Effect::Hide(on(Find, A)),
                Effect::Hide(on(Dialog, A)),
                Effect::Reveal(on(Find, B)),
            ]
        );
        // back: A's find bar, dialog and notice come back, B's find bar hides
        let (back, fx) = transition(next, Event::Switched(Some(A)));
        assert_eq!(back.shown, Some(A));
        assert_eq!(
            fx,
            [
                Effect::Reveal(on(Notice, A)),
                Effect::Reveal(on(Find, A)),
                Effect::Reveal(on(Dialog, A)),
                Effect::Hide(on(Find, B)),
            ]
        );
        // the other tab's overlays never show on this one
        assert_eq!(back.top(), Some(win(CommandBar)));
    }

    #[test]
    fn print_hints_and_region_close_with_their_tab() {
        for kind in [PrintSheet, Hints, Region] {
            let (next, fx) = transition(stack(&[on(kind, A)]), Event::Switched(Some(B)));
            assert!(next.layers.is_empty(), "{kind:?}");
            assert_eq!(fx, [Effect::Close(on(kind, A))]);
        }
    }

    #[test]
    fn the_same_tab_again_is_no_switch() {
        let (next, fx) = transition(stack(&[win(Menu)]), Event::Switched(Some(A)));
        assert_eq!(next.layers, [win(Menu)]);
        assert!(fx.is_empty());
    }

    #[test]
    fn navigation_closes_the_tab_s_overlays() {
        let layers = [
            on(Notice, A),
            on(Find, A),
            on(Dialog, A),
            on(PrintSheet, A),
            win(Downloads),
            on(Find, B),
        ];
        let (layers, fx) = step(&layers, Event::Navigated(A));
        assert_eq!(layers, [on(Dialog, A), win(Downloads), on(Find, B)]);
        assert_eq!(
            fx,
            [
                Effect::Close(on(Notice, A)),
                Effect::Close(on(Find, A)),
                Effect::Close(on(PrintSheet, A)),
            ]
        );
        for kind in [BookmarkSheet, Hints, Region] {
            let (layers, fx) = step(&[on(kind, A)], Event::Navigated(A));
            assert!(layers.is_empty());
            assert_eq!(fx, [Effect::Close(on(kind, A))]);
        }
    }

    #[test]
    fn the_top_is_the_latest_drawn_visible_kind() {
        let s = stack(&[win(Menu), on(Find, A), on(PrintSheet, B)]);
        assert_eq!(s.top(), Some(win(Menu)));
        assert!(s.is_open(PrintSheet));
        assert_eq!(Stack::default().top(), None);
    }

    /// A free surface and a leaving one are in the stack only for `covers`: no rule sees them.
    #[test]
    fn free_and_leaving_surfaces_meet_no_rule() {
        let free = [
            win(Tooltip),
            on(ErrorPanel, A),
            Layer::new(Status, Some(A)),
            win(TabGhost),
        ];
        // Escape skips them and closes the find bar; again, nothing
        let mut with_find = free.to_vec();
        with_find.push(on(Find, A));
        let (layers, fx) = step(&with_find, Event::Escape);
        assert_eq!(fx, [Effect::Close(on(Find, A))]);
        let (_, fx) = step(&layers, Event::Escape);
        assert!(fx.is_empty());
        // a menu opens over them and closes none; a switch and a navigation leave them
        let (_, fx) = step(&free, Event::Open(win(Menu)));
        assert_eq!(fx, [Effect::Open(win(Menu))]);
        let (next, fx) = transition(stack(&free), Event::Switched(Some(B)));
        assert!(fx.is_empty());
        assert_eq!(next.layers, free);
        let (_, fx) = step(&free, Event::Navigated(A));
        assert!(fx.is_empty());
        // a sheet and a menu leaving neither refuse the find bar nor answer Escape
        let fading = Stack {
            shown: Some(A),
            leaving: vec![on(PrintSheet, A), win(Menu)],
            ..Stack::default()
        };
        let (next, fx) = transition(fading.clone(), Event::Open(on(Find, A)));
        assert_eq!(fx, [Effect::Open(on(Find, A))]);
        assert_eq!(next.leaving, fading.leaving);
        let (_, fx) = transition(fading, Event::Escape);
        assert!(fx.is_empty());
    }

    /// What the kit still draws is leaving unless a visible layer of its kind is open.
    #[test]
    fn what_is_drawn_and_not_open_is_leaving() {
        let with = |open: &[Layer], drawn: &[Layer]| stack(open).with_drawn(drawn).leaving;
        // open and drawn: not leaving
        assert!(with(&[on(Find, A)], &[on(Find, A)]).is_empty());
        // closed, still drawn: leaving
        assert_eq!(with(&[], &[on(Find, A), win(Menu)]), [on(Find, A), win(Menu)]);
        // open on a hidden tab, drawn over the visible one as its bar fades: leaving
        assert_eq!(with(&[on(Find, B)], &[on(Find, A)]), [on(Find, A)]);
        // a pane's surface open on another pane counts as open: it is drawn there
        let strip = Layer::new(VideoStrip, Some(B));
        assert!(with(&[strip], &[Layer::new(VideoStrip, Some(A))]).is_empty());
        // nothing drawn, nothing leaving; the open layers stay as they are
        assert!(with(&[on(Find, A)], &[]).is_empty());
        assert_eq!(stack(&[on(Find, A)]).with_drawn(&[win(Menu)]).layers, [on(Find, A)]);
    }

    /// Each row sits at its kind's index (`Kind::row` reads it so); `Specimen` is the last kind,
    /// so a kind without a row fails here.
    #[test]
    fn one_row_per_kind() {
        for (i, row) in ROWS.iter().enumerate() {
            assert_eq!(row.kind as usize, i, "{:?} out of place", row.kind);
        }
        assert_eq!(ROWS.len(), Specimen as usize + 1);
    }

    #[test]
    fn drawing_orders_are_distinct_in_each_window() {
        for window in ALL {
            let mut seen = std::collections::BTreeMap::new();
            for row in ROWS.iter().filter(|r| r.windows.contains(window)) {
                if let Some(other) = seen.insert(row.z, row.kind) {
                    panic!("{:?} and {other:?} share z {} in {window:?}", row.kind, row.z);
                }
            }
        }
    }

    /// What `kind` needs of the page it is over (pane A, the visible tab's) and of another pane
    /// of the window (B), with the scene's glass on. A match, so a new kind needs its line.
    #[rustfmt::skip]
    fn over(kind: Kind) -> (Coverage, Coverage) {
        use Coverage::{None as No, Part, Pixels};
        match kind {
            Notice | Permission | Find | Dialog | BookmarkSheet | PrintSheet | ExtensionSheet
            | AgentSheet | Region | CommandBar | Downloads | Extensions | Spaces | Menu => {
                (Pixels, Pixels)
            }
            Hints => (No, No),
            ErrorPanel | Toast | Preview | AutofillMenu | AutofillPrompt | PipStrip | Reader
            | Boost | Glance | Peek | Entrance | SeamBow => (Pixels, Pixels),
            VideoStrip | VideoTab | VideoSpeeds => (Pixels, No),
            Status | DevTools => (Part, No),
            Popup | Tooltip | AutofillChip | PipCard | Swipe | ExitPage | SplitRing | SplitSeams
            | TabGhost | DropZone | PageGhost | DropMark | Autoscroll | Specimen => (Part, Part),
        }
    }

    /// Each kind alone on screen, over its pane and another one (the peek and the
    /// picture-in-picture card come from their owners, as `Facts`).
    #[test]
    #[rustfmt::skip]
    fn each_kind_covers_the_panes_it_is_over() {
        let lit = Facts {
            glass: true,
            ..Facts::default()
        };
        for row in ROWS {
            let kind = row.kind;
            let (stack, facts) = match kind {
                Glance => (stack(&[]), Facts { glance: true, ..lit }),
                PipCard => (stack(&[]), Facts { pip_card: Some(A), ..lit }),
                _ => (stack(&[Layer::new(kind, Some(A))]), lit),
            };
            assert_eq!(
                (covers(&stack, &facts, A), covers(&stack, &facts, B)),
                over(kind),
                "{kind:?}"
            );
        }
    }

    #[test]
    fn coverage_is_the_most_any_drawn_surface_needs() {
        let lit = Facts {
            glass: true,
            ..Facts::default()
        };
        let plain = Facts::default();
        let leaving = |l: &[Layer]| Stack {
            shown: Some(A),
            leaving: l.to_vec(),
            ..Stack::default()
        };
        #[rustfmt::skip]
        let rows: &[(&str, Stack, Facts, Coverage)] = &[
            ("nothing", stack(&[]), lit, Coverage::None),
            ("a tooltip: the pointer passes, its pixels cover", stack(&[win(Tooltip)]), lit, Coverage::Part),
            ("the find bar's glass reads the page", stack(&[on(Find, A)]), lit, Coverage::Pixels),
            ("the find bar without the scene's glass", stack(&[on(Find, A)]), plain, Coverage::Part),
            ("a sheet fading out still covers", leaving(&[on(PrintSheet, A)]), lit, Coverage::Pixels),
            ("a menu fading out, no glass", leaving(&[win(Menu)]), plain, Coverage::Part),
            ("another tab's find bar is not drawn", stack(&[on(Find, B)]), lit, Coverage::None),
            ("nor its fading sheet", leaving(&[on(PrintSheet, B)]), lit, Coverage::None),
            ("link hints are the page's own", stack(&[on(Hints, A)]), lit, Coverage::None),
            ("the most of several", stack(&[win(Tooltip), on(Find, A)]), lit, Coverage::Pixels),
            ("another pane's status bubble", stack(&[Layer::new(Status, Some(B))]), lit, Coverage::None),
            ("an entrance moves the page, glass or not", stack(&[win(Entrance)]), plain, Coverage::Pixels),
            ("the card of a tab not shown", stack(&[]), Facts { pip_card: Some(B), ..lit }, Coverage::None),
            ("the peek, open or leaving, no glass", stack(&[]), Facts { glance: true, ..plain }, Coverage::Part),
        ];
        for (name, stack, facts, want) in rows {
            assert_eq!(covers(stack, facts, A), *want, "{name}");
        }
    }
}
