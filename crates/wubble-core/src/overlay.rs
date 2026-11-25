//! Every surface the shell draws over a page, as one table ([`ROWS`]), and the one store of the
//! overlays among them ([`Ui`], `store.rs`): which is open or leaving, and what opening one,
//! Escape, a tab switch or a navigation does to the others ([`reduce`]). The one owner of the
//! surfaces' static properties, of the overlays' order and scope, and of the answer to "what
//! shell UI covers this page now" ([`covers`]).
//!
//! A surface's content stays with its machine (the menu's entries, the print sheet's settings,
//! the find bar's query); the store owns whether it is open. The free surfaces (a tooltip, the
//! video strip, a drag's ghost) keep their own owners and are in the [`Stack`] only for
//! [`covers`].
//!
//! The rules (Chromium's: one tab-modal sheet or bubble at a time, the find bar belongs to its
//! tab, Escape closes the innermost) are each row's data ([`Row::refused_under`],
//! [`Row::closes`], [`Row::escape`], [`Row::on_leave`], [`Row::on_nav`]):
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
//!   notice and a permission prompt never take the page's Escape ([`Esc::ShellOnly`]): a key
//!   meant for the page (leaving fullscreen, the site's own dialog) must not answer them
//!   (no keyjacking, docs/research/mcp-server.md).
//! - **Tab switch**: menus, sheets, link hints and the screenshot picker close; the find bar, a
//!   page's dialog, a notice, the error panel and the Boost editor stay with their tab, hidden,
//!   and are revealed only on it. The window's command bar and downloads popover stay.
//! - **Navigation** (a new document): the tab's sheets, find bar, link hints, screenshot picker
//!   and notice close; a page's dialog is the engine's to drop.
//! - A surface **leaving** (closed, its kit still draws the exit) is closed for these rules and
//!   still covers the page ([`Stack::leaving`]).
//!
//! A pointer press outside an overlay is the kit's: the menu's and the command bar's scrims
//! dismiss them, a sheet's scrim holds the page (modal); none of it needs a rule here. A
//! dismissing press is consumed whole (kit `Scrim`), so a press on the popup's own toggle (the
//! "^", the downloads button, a chip, a Select's trigger) closes it and is no click that opens
//! it again.

mod store;

pub use store::{
    Action, AxRoots, Cue, Effect, Env, Key, KeyOwner, Phase, PointerOwner, Surface, Ui, acting,
    reduce,
};

use crate::hovercard::{self, Timing};
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
    // ----- the tab's panels: under the rules (a switch hides them) and closing nothing
    /// The page's error panel in place of the page (a failed load, a refused certificate).
    ErrorPanel,
    /// The Boost editor docked at the page's right edge.
    Boost,
    // ----- free surfaces: outside the overlays' rules
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
    /// Glass blurring the page: a slowly refreshed copy of it will do (a page on its OS layer
    /// stays there, `page_layer::Hold::Frost`).
    Glass,
    /// The shell draws the page's own pixels: an entrance moving it, a mask cutting it.
    Pixels,
}

/// How long a surface lives, and who ends it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Life {
    /// Until answered.
    Answer,
    /// Until dismissed (a click outside, Escape, its own close).
    Dismiss,
    /// For this many ms, unless replaced: a hover slot of the store shown for a time.
    Timed(u64),
    /// While hovered, with these delays: a hover slot of the store ([`Surface`]).
    Hover(Timing),
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
    /// Its kit draws an exit: closed, it is [`Phase::Leaving`] until the kit says it is gone
    /// ([`Action::Exited`]).
    pub tail: bool,
    /// Opened while it still leaves, its content is fresh (Unmount, Mount); else the open only
    /// cancels the exit (a surface whose content its owner rewrites in place).
    pub remount: bool,
    /// It may not open while a visible overlay of one of these groups shows.
    pub refused_under: &'static [Group],
    /// Opening it closes the visible overlays of these groups.
    pub closes: &'static [Group],
    /// One in the window: opening it closes its kind's other one, on this window's tab or a
    /// hidden one (another window's stays).
    pub one_per_window: bool,
    /// Opening it closes these surfaces when they are open. A [`Pointer::Grab`] row opens only
    /// when every other open grab is among them: one owner holds the pointer, and a held
    /// button is never stolen by a key.
    pub ends: &'static [Kind],
    /// Its window losing the keys (focus gone) closes it, its owner answering as cancelled
    /// ([`Action::Lost`]). A drag's rows are not here: `drag::window_lost_actions` decides
    /// whether its drag survives (a tab torn off keeps going), and the rows follow the drag.
    pub on_lost: bool,
}

/// How long a plain toast stays (wubble-only: no design counterpart).
pub const TOAST_SHOWN_MS: u64 = 1_500;

/// How long a toast shows, by `[button][sticky]`: a plain one [`TOAST_SHOWN_MS`], one with a
/// button long enough to reach it ([`crate::sponsor::TOAST_MS`]), a sticky one until its owner
/// hides it (`None`). A new toast time is a cell here.
const TOAST_TIMES: [[Option<u64>; 2]; 2] = [
    [Some(TOAST_SHOWN_MS), None],
    [Some(crate::sponsor::TOAST_MS), None],
];

/// The host's timer for a toast shown now: `None`, no timer.
pub fn toast_ms(button: bool, sticky: bool) -> Option<u64> {
    TOAST_TIMES[usize::from(button)][usize::from(sticky)]
}

/// The video strip: shown at once or for a while by the video machine (`video::hold`), gone
/// [`crate::video::LEAVE_MS`] after the pointer left.
const VIDEO: Timing = Timing {
    sticky_ms: crate::video::LEAVE_MS,
    ..hovercard::AT_ONCE
};

/// The hidden sidebar's peek (chrome.tsx): [`crate::chrome::PEEK_IN_MS`] after a move in its
/// strip (at once from its edge), [`crate::chrome::PEEK_OUT_MS`] after the pointer left.
const PEEK: Timing = Timing {
    show_ms: crate::chrome::PEEK_IN_MS as u64,
    warm_ms: 0,
    sticky_ms: crate::chrome::PEEK_OUT_MS as u64,
};

const ALL: &[WindowKind] = &[
    WindowKind::Main,
    WindowKind::Little,
    WindowKind::Popup,
    WindowKind::Pip,
];
const PAGES: &[WindowKind] = &[WindowKind::Main, WindowKind::Little, WindowKind::Popup];
const MAIN: &[WindowKind] = &[WindowKind::Main];
const PIP: &[WindowKind] = &[WindowKind::Pip];

/// The user's overlays: what a popover, a menu or a sheet replaces.
const USERS: &[Group] = &[Group::Exclusive, Group::Modal];
const HOLDS: &[Group] = &[Group::Holds];
/// What a modal sheet holds off: the find bar and an agent's sheet wait for either.
const MODALS: &[Group] = &[Group::Modal, Group::Holds];
const POPOVERS: &[Group] = &[Group::Exclusive];
/// What a drag's start ends: a region pick and an autoscroll.
const DRAGS_END: &[Kind] = &[Kind::Region, Kind::Autoscroll];

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
    tail: true,
    remount: true,
    refused_under: HOLDS,
    closes: USERS,
    one_per_window: false,
    ends: &[],
    on_lost: false,
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
    tail: false,
    remount: false,
    refused_under: &[],
    closes: &[],
    ..DISMISSED
};

/// Every surface, in [`Kind`]'s order (one row each: the test checks it). Every property of a
/// surface is a field here; a new surface is a new row, a new behaviour a new field, never a
/// branch on a kind elsewhere.
#[rustfmt::skip]
pub const ROWS: &[Row] = &[
    Row { kind: Kind::Notice, scope: Scope::Tab, z: 63, group: Group::Coexist, keys: Keys::Never, pointer: Pointer::OwnRect,
        glass: Some("toast"), life: Life::Answer, escape: Esc::ShellOnly, on_leave: false, on_nav: true,
        // its toast's exit is the plain toast's (`Mounted.toast` is theirs)
        tail: false, refused_under: &[], closes: &[], one_per_window: true, ..DISMISSED },
    Row { kind: Kind::Permission, scope: Scope::Tab, z: 94, group: Group::Coexist, keys: Keys::Never, pointer: Pointer::OwnRect,
        glass: Some("permission"), life: Life::Answer, escape: Esc::ShellOnly, on_leave: false, refused_under: &[], closes: &[], ..DISMISSED },
    Row { kind: Kind::Find, scope: Scope::Tab, z: 50, group: Group::Coexist, keys: Keys::OnAsk, pointer: Pointer::OwnRect,
        glass: Some("find"), on_leave: false, on_nav: true, refused_under: MODALS, closes: POPOVERS, ..DISMISSED },
    // a page's dialog is refused by nothing, and closes no held overlay
    Row { kind: Kind::Dialog, scope: Scope::Tab, z: 72, group: Group::Holds, glass: Some("dialog"), on_leave: false, refused_under: &[], ..SHEET },
    Row { kind: Kind::BookmarkSheet, scope: Scope::Tab, z: 66, glass: Some("bookmark-sheet"), on_nav: true, ..SHEET },
    Row { kind: Kind::PrintSheet, scope: Scope::Tab, z: 68, glass: Some("print-sheet"), on_nav: true, ..SHEET },
    Row { kind: Kind::ExtensionSheet, z: 67, glass: Some("extension-sheet"), ..SHEET },
    // an agent's sheet never replaces the user's sheet, which it waits for
    Row { kind: Kind::AgentSheet, z: 69, group: Group::Holds, glass: Some("agent-sheet"), on_leave: false,
        refused_under: MODALS, closes: POPOVERS, ..SHEET },
    Row { kind: Kind::Region, scope: Scope::Tab, z: 56, pointer: Pointer::Grab, glass: Some("shot"), on_nav: true, ends: &[Kind::Autoscroll], on_lost: true, ..DISMISSED },
    // the page draws the labels: nothing of the shell over it, no exit of the kit's
    Row { kind: Kind::Hints, scope: Scope::Tab, z: 5, pointer: Pointer::PassThrough, cover: Coverage::None, on_nav: true, tail: false, ..DISMISSED },
    Row { kind: Kind::CommandBar, z: 74, glass: Some("command-bar"), on_leave: false, ..DISMISSED },
    Row { kind: Kind::Downloads, z: 100, glass: Some("downloads"), on_leave: false, ..DISMISSED },
    Row { kind: Kind::Extensions, z: 102, glass: Some("extensions"), ..DISMISSED },
    Row { kind: Kind::Spaces, z: 104, glass: Some("spaces"), windows: MAIN, ..DISMISSED },
    Row { kind: Kind::Menu, z: 110, glass: Some("menu"), ..DISMISSED },
    // ----- the tab's panels: its machine opens and closes them; Escape and the overlays leave them
    Row { kind: Kind::ErrorPanel, scope: Scope::Tab, z: 60, group: Group::Coexist, pointer: Pointer::OwnRect, glass: Some("error"),
        life: Life::Owner, escape: Esc::Never, on_leave: false, refused_under: &[], closes: &[], ..DISMISSED },
    Row { kind: Kind::Boost, scope: Scope::Tab, z: 58, group: Group::Coexist, keys: Keys::OnAsk, pointer: Pointer::OwnRect, glass: Some("boost"),
        escape: Esc::Never, on_leave: false, closes: &[], ..DISMISSED },
    // ----- free
    // the page's popup belongs to its tab (the peek's too) and ends with its document
    Row { kind: Kind::Popup, scope: Scope::Tab, z: 90, pointer: Pointer::OwnRect, on_nav: true, ..FREE },
    // the toast and the notice on it are one slot (`Key::Notice`): its navigation close is the notice's row's
    Row { kind: Kind::Toast, z: 62, glass: Some("toast"), life: Life::Timed(TOAST_SHOWN_MS), ..FREE },
    Row { kind: Kind::Tooltip, z: 114, life: Life::Hover(hovercard::TOOLTIP), on_nav: true, ..FREE },
    Row { kind: Kind::Preview, z: 52, glass: Some("preview"), life: Life::Hover(hovercard::TAB_CARD), ..FREE },
    // its fade is the kit's (`status-drawn` per pane): leaving until it reports
    Row { kind: Kind::Status, scope: Scope::Pane, z: 30, life: Life::Hover(hovercard::STATUS_BUBBLE), on_nav: true, tail: true, ..FREE },
    Row { kind: Kind::AutofillChip, scope: Scope::Tab, z: 96, pointer: Pointer::OwnRect, ..FREE },
    Row { kind: Kind::AutofillMenu, scope: Scope::Tab, z: 97, pointer: Pointer::OwnRect, glass: Some("autofill-menu"), life: Life::Dismiss, tail: true, ..FREE },
    Row { kind: Kind::AutofillPrompt, scope: Scope::Tab, z: 98, pointer: Pointer::OwnRect, glass: Some("autofill-prompt"), life: Life::Answer, tail: true, ..FREE },
    // the strip, its tab and its menu are one slot (the video machine says which)
    Row { kind: Kind::VideoStrip, scope: Scope::Pane, z: 40, pointer: Pointer::OwnRect, glass: Some("video-strip"), life: Life::Hover(VIDEO), windows: PAGES, on_nav: true, ..FREE },
    Row { kind: Kind::VideoTab, scope: Scope::Pane, z: 41, pointer: Pointer::OwnRect, glass: Some("video-tab"), windows: PAGES, ..FREE },
    Row { kind: Kind::VideoSpeeds, scope: Scope::Pane, z: 42, pointer: Pointer::OwnRect, glass: Some("video-speeds"), life: Life::Dismiss, windows: PAGES, ..FREE },
    Row { kind: Kind::PipStrip, z: 44, pointer: Pointer::OwnRect, glass: Some("pip-strip"), life: Life::Hover(hovercard::AT_ONCE), windows: PIP, ..FREE },
    Row { kind: Kind::PipCard, scope: Scope::Tab, z: 46, pointer: Pointer::OwnRect, windows: MAIN, ..FREE },
    Row { kind: Kind::Reader, scope: Scope::Tab, z: 54, pointer: Pointer::OwnRect, glass: Some("reader"), tail: true, ..FREE },
    Row { kind: Kind::Glance, z: 64, keys: Keys::OnAsk, pointer: Pointer::Scrim, glass: Some("glance"), life: Life::Dismiss, escape: Esc::Any, ..FREE },
    Row { kind: Kind::Swipe, z: 70, life: Life::Drag, on_lost: true, ..FREE },
    Row { kind: Kind::ExitPage, z: 10, ..FREE },
    Row { kind: Kind::Entrance, z: 20, cover: Coverage::Pixels, ..FREE },
    Row { kind: Kind::Peek, z: 80, pointer: Pointer::OwnRect, glass: Some("peek"), life: Life::Hover(PEEK), windows: PAGES, ..FREE },
    Row { kind: Kind::SeamBow, z: 82, pointer: Pointer::Grab, cover: Coverage::Pixels, life: Life::Drag, windows: PAGES, ..FREE },
    Row { kind: Kind::SplitRing, z: 34, ..FREE },
    // the handles take the pointer over themselves only: a drag opens beside them
    Row { kind: Kind::SplitSeams, z: 36, pointer: Pointer::OwnRect, ..FREE },
    Row { kind: Kind::DevTools, scope: Scope::Pane, z: 32, pointer: Pointer::OwnRect, ..FREE },
    // a drag holds the pointer until its release; its start closes a pick and an autoscroll
    Row { kind: Kind::TabGhost, z: 108, pointer: Pointer::Grab, life: Life::Drag, escape: Esc::Any, ends: DRAGS_END, ..FREE },
    Row { kind: Kind::DropZone, z: 106, life: Life::Drag, ..FREE },
    Row { kind: Kind::PageGhost, z: 109, pointer: Pointer::Grab, life: Life::Drag, escape: Esc::Any, ends: DRAGS_END, ..FREE },
    Row { kind: Kind::DropMark, z: 107, life: Life::Drag, ..FREE },
    // Chromium runs it and ends it on any press or key: the pointer is the page's
    Row { kind: Kind::Autoscroll, z: 92, on_lost: true, ..FREE },
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
        matches!(
            self.row().group,
            Group::Exclusive | Group::Modal | Group::Holds
        )
    }

    /// Its hover slot's delays, when it is a hover or timed surface of the store.
    pub fn timing(self) -> Option<Timing> {
        match self.row().life {
            Life::Hover(t) => Some(t),
            Life::Timed(_) => Some(hovercard::AT_ONCE),
            Life::Answer | Life::Dismiss | Life::Drag | Life::Owner => None,
        }
    }

    /// A hover slot whose thing is also a layer of its pane (the status bubble): shown, it is
    /// open; hidden, it leaves until its kit reports the fade over.
    pub fn slot_layer(self) -> bool {
        let row = self.row();
        row.tail && row.scope == Scope::Pane && matches!(row.life, Life::Hover(_))
    }

    /// Under the overlays' rules.
    fn ruled(self) -> bool {
        self.row().group != Group::Free
    }
}

/// One surface on screen: its kind and, unless the window's, its tab (a pane's: the pane's).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
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
    /// The window's peek tab, drawn over the shown one: its tab's layers show too.
    pub peeked: Option<TabId>,
    pub layers: Vec<Layer>,
    /// Closed by their owners, still drawn by their kit until the exit ends: no rule sees them,
    /// and they cover the page.
    pub leaving: Vec<Layer>,
}

impl Stack {
    /// Drawn now: a window's and a pane's surface (the host derives only the panes on screen)
    /// always, a tab's while its tab is visible.
    fn visible(&self, layer: &Layer) -> bool {
        layer.kind.scope() == Scope::Pane
            || layer.tab.is_none()
            || layer.tab == self.shown
            || layer.tab == self.peeked
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
            if !self
                .layers
                .iter()
                .any(|l| l.kind == d.kind && self.visible(l))
            {
                self.leaving.push(*d);
            }
        }
        self
    }
}

/// What a page's pane is covered by that its owner, not the stack, knows.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Facts {
    /// The scene draws the overlays' glass in this window (`GlassLayer.over-on`) and some of it
    /// lies over the pane asked about: a glass surface there reads the page's pixels. Per pane:
    /// a menu or a toast over the sidebar is no glass over a page (the host's `page_covers`).
    pub glass: bool,
    /// The window's peek is open or leaving (`Model::glance_of`, its card's exit).
    pub glance: bool,
    /// Where the open peek's card and scrim are (the host's `glance_rect`, computed per call:
    /// not the store's); none while it is closed or only its exit is drawn.
    pub glance_rect: Option<crate::tabdrag::Rect>,
    /// The visible tab plays in picture-in-picture: its card stands in for it.
    pub pip_card: Option<TabId>,
}

/// What the shell draws over the page in pane `pane` of the window `stack` and `facts` describe:
/// the most any visible surface over it needs, a leaving one included.
pub fn covers(stack: &Stack, facts: &Facts, pane: TabId) -> Coverage {
    coverers(stack, facts, pane)
        .map(|(_, cover)| cover)
        .max()
        .unwrap_or(Coverage::None)
}

/// Each visible surface over pane `pane`, a leaving one included, with what it needs there:
/// [`covers`] is the most of them; the page layer's log names them.
pub fn coverers<'a>(
    stack: &'a Stack,
    facts: &Facts,
    pane: TabId,
) -> impl Iterator<Item = (Kind, Coverage)> + 'a {
    let owned = [
        facts.glance.then(|| Layer::new(Kind::Glance, None)),
        facts.pip_card.map(|t| Layer::new(Kind::PipCard, Some(t))),
    ];
    let glass = facts.glass;
    stack
        .layers
        .iter()
        .chain(&stack.leaving)
        .copied()
        .chain(owned.into_iter().flatten())
        .filter(move |l| stack.visible(l) && (l.kind.scope() != Scope::Pane || l.tab == Some(pane)))
        .map(move |l| {
            let row = l.kind.row();
            let cover = match row.glass {
                Some(_) if glass && row.cover != Coverage::None => Coverage::Glass,
                _ => row.cover,
            };
            (l.kind, cover)
        })
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

    #[test]
    fn a_window_overlay_keeps_no_tab() {
        assert_eq!(Layer::new(Menu, Some(A)).tab, None);
        assert_eq!(Layer::new(Find, Some(A)).tab, Some(A));
    }

    /// What the kit still draws is leaving unless a visible layer of its kind is open.
    #[test]
    fn what_is_drawn_and_not_open_is_leaving() {
        let with = |open: &[Layer], drawn: &[Layer]| stack(open).with_drawn(drawn).leaving;
        // open and drawn: not leaving
        assert!(with(&[on(Find, A)], &[on(Find, A)]).is_empty());
        // closed, still drawn: leaving
        assert_eq!(
            with(&[], &[on(Find, A), win(Menu)]),
            [on(Find, A), win(Menu)]
        );
        // open on a hidden tab, drawn over the visible one as its bar fades: leaving
        assert_eq!(with(&[on(Find, B)], &[on(Find, A)]), [on(Find, A)]);
        // a pane's surface open on another pane counts as open: it is drawn there
        let strip = Layer::new(VideoStrip, Some(B));
        assert!(with(&[strip], &[Layer::new(VideoStrip, Some(A))]).is_empty());
        // nothing drawn, nothing leaving; the open layers stay as they are
        assert!(with(&[on(Find, A)], &[]).is_empty());
        assert_eq!(
            stack(&[on(Find, A)]).with_drawn(&[win(Menu)]).layers,
            [on(Find, A)]
        );
    }

    /// Each row sits at its kind's index (`Kind::row` reads it so); `Specimen` is the last kind,
    /// so a kind without a row fails here.
    #[test]
    fn one_row_per_kind() {
        for (i, row) in ROWS.iter().enumerate() {
            assert_eq!(row.kind as usize, i, "{:?} out of place", row.kind);
        }
        assert_eq!(ROWS.len(), Specimen as usize + 1);
        // the overlays come first (Kind's order is their Escape order), the free surfaces after
        let overlays = ROWS.iter().take_while(|r| r.group != Group::Free).count();
        assert_eq!(
            overlays,
            Boost as usize + 1,
            "the overlays are the first kinds"
        );
        assert!(
            ROWS[overlays..].iter().all(|r| r.group == Group::Free),
            "no overlay after the first free surface"
        );
    }

    #[test]
    fn a_toast_shows_for_its_time() {
        let rows = [
            ("plain", false, false, Some(TOAST_SHOWN_MS)),
            ("with a button", true, false, Some(crate::sponsor::TOAST_MS)),
            ("sticky", false, true, None),
            ("sticky with a button", true, true, None),
        ];
        for (name, button, sticky, want) in rows {
            assert_eq!(toast_ms(button, sticky), want, "{name}");
        }
        assert_eq!(
            (TOAST_SHOWN_MS, crate::sponsor::TOAST_MS),
            (1_500, 6_000),
            "the times as before"
        );
    }

    #[test]
    fn drawing_orders_are_distinct_in_each_window() {
        for window in ALL {
            let mut seen = std::collections::BTreeMap::new();
            for row in ROWS.iter().filter(|r| r.windows.contains(window)) {
                if let Some(other) = seen.insert(row.z, row.kind) {
                    panic!(
                        "{:?} and {other:?} share z {} in {window:?}",
                        row.kind, row.z
                    );
                }
            }
        }
    }

    /// What `kind` needs of the page it is over (pane A, the visible tab's) and of another pane
    /// of the window (B), with the scene's glass on. A match, so a new kind needs its line.
    #[rustfmt::skip]
    fn over(kind: Kind) -> (Coverage, Coverage) {
        use Coverage::{Glass, None as No, Part, Pixels};
        match kind {
            Notice | Permission | Find | Dialog | BookmarkSheet | PrintSheet | ExtensionSheet
            | AgentSheet | Region | CommandBar | Downloads | Extensions | Spaces | Menu => {
                (Glass, Glass)
            }
            Hints => (No, No),
            ErrorPanel | Toast | Preview | AutofillMenu | AutofillPrompt | PipStrip | Reader
            | Boost | Glance | Peek => (Glass, Glass),
            Entrance | SeamBow => (Pixels, Pixels),
            VideoStrip | VideoTab | VideoSpeeds => (Glass, No),
            Status | DevTools => (Part, No),
            Popup | Tooltip | AutofillChip | PipCard | Swipe | ExitPage | SplitRing | SplitSeams
            | TabGhost | DropZone | PageGhost | DropMark | Autoscroll | Specimen => (Part, Part),
        }
    }

    /// Each kind alone on screen, over its pane and another one (the peek and the
    /// picture-in-picture card come from their owners, as `Facts`): open, leaving (its exit
    /// still drawn: the same), and with the scene's glass off (its row's `cover`).
    #[test]
    #[rustfmt::skip]
    fn each_kind_covers_the_panes_it_is_over() {
        for row in ROWS {
            let kind = row.kind;
            for glass in [true, false] {
                let lit = Facts { glass, ..Facts::default() };
                let (open, facts) = match kind {
                    Glance => (stack(&[]), Facts { glance: true, ..lit }),
                    PipCard => (stack(&[]), Facts { pip_card: Some(A), ..lit }),
                    _ => (stack(&[Layer::new(kind, Some(A))]), lit),
                };
                let leaving = Stack { leaving: open.layers.clone(), layers: vec![], ..open.clone() };
                let want = if glass { over(kind) } else { (row.cover, over(kind).1.min(row.cover)) };
                for (state, stack) in [("open", &open), ("leaving", &leaving)] {
                    assert_eq!(
                        (covers(stack, &facts, A), covers(stack, &facts, B)),
                        want,
                        "{kind:?} {state}, glass {glass}"
                    );
                }
            }
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
            ("the find bar's glass reads the page", stack(&[on(Find, A)]), lit, Coverage::Glass),
            ("the find bar without the scene's glass", stack(&[on(Find, A)]), plain, Coverage::Part),
            ("a sheet fading out still covers", leaving(&[on(PrintSheet, A)]), lit, Coverage::Glass),
            ("a menu fading out, no glass", leaving(&[win(Menu)]), plain, Coverage::Part),
            ("another tab's find bar is not drawn", stack(&[on(Find, B)]), lit, Coverage::None),
            ("nor its fading sheet", leaving(&[on(PrintSheet, B)]), lit, Coverage::None),
            ("link hints are the page's own", stack(&[on(Hints, A)]), lit, Coverage::None),
            ("the most of several", stack(&[win(Tooltip), on(Find, A)]), lit, Coverage::Glass),
            ("glass and an entrance: the entrance draws the page", stack(&[on(Find, A), win(Entrance)]), lit, Coverage::Pixels),
            ("a window's glass not over this pane (the host's fact)", stack(&[win(Menu)]), plain, Coverage::Part),
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
