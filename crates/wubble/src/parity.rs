//! `--smoke parity`: the ordinary web-page interactions a Chromium window
//! gives for free, checked one by one through the host's real input paths on
//! the bundled `parity.html`. The checklist is one table, [`ITEMS`]: a name,
//! how to drive it ([`Step`]s) and what must be true afterwards ([`Expect`]s).
//! [`next`] is the pure driver; `host/parity.rs` executes its [`Action`]s.
//!
//! The page answers through its title: every script the smoke runs ends in
//! `parity:<serial>:ok` or `parity:<serial>:fail:<what it saw>`. Inputs and
//! scripts travel to the renderer on different channels, so a check that is
//! not yet true is asked again after a few runs of CEF work, up to
//! [`MAX_POLLS`] times. Everything is bounded by counts of CEF work runs,
//! never by time.
//!
//! Keys are built here as winit would report them on a US layout
//! ([`key_inputs`]): the host routes them exactly as raw winit key events
//! (`Host::key`), only the winit `KeyEvent` itself cannot be built outside
//! winit. The script names Linux chords; on macOS [`MAC_CHORDS`] rewrites them to
//! the Mac's own, and [`PLATFORM_GAPS`] lists what that OS has no equivalent for.

use slint::winit_030::winit::keyboard::{
    Key as WinitKey, KeyCode, KeyLocation, ModifiersState, NamedKey, PhysicalKey,
};
use slint::winit_030::winit::platform::scancode::PhysicalKeyExtScancode;
use wubble_core::keys::{Chord, Key, Platform};
use wubble_engine::input::{Buttons, KeyInput, Modifiers, Point};

/// Runs of CEF work one wait may take before its item fails.
pub const MAX_WAIT_RUNS: u32 = 400;
/// Times a check is asked before its item fails.
pub const MAX_POLLS: u32 = 12;
/// Times an `Until` step's check is asked before its item fails: setup waits on
/// state the window settles on its own (the first input after start takes longest).
pub const MAX_SETUP_POLLS: u32 = 60;
/// A `Retry` step runs its op again after this many unanswered asks.
pub const RETRY_EVERY: u32 = 10;
/// Runs of CEF work between two asks of the same check.
pub const POLL_GAP: u32 = 6;

/// Where `parity.html` puts things (view-relative logical px).
pub mod at {
    use wubble_engine::input::Point;
    const fn p(x: f32, y: f32) -> Point {
        Point { x, y }
    }
    /// `at` moved down by `dy`.
    pub const fn y(at: Point, dy: f32) -> Point {
        p(at.x, at.y + dy)
    }
    /// `at` moved to `x`.
    pub const fn x(at: Point, x: f32) -> Point {
        Point { x, y: at.y }
    }
    pub const INP: Point = p(140.0, 34.0);
    pub const INP2: Point = p(140.0, 74.0);
    pub const CE: Point = p(140.0, 200.0);
    /// On "alpha" of `alpha beta gamma` (16 px monospace from x 20).
    pub const WORD: Point = p(40.0, 250.0);
    /// On "gamma".
    pub const GAMMA: Point = p(170.0, 250.0);
    pub const LINK: Point = p(35.0, 299.0);
    pub const IMG: Point = p(40.0, 350.0);
    pub const VIDEO: Point = p(160.0, 360.0);
    pub const DRAG: Point = p(360.0, 40.0);
    pub const DROP: Point = p(400.0, 140.0);
    pub const FILEZONE: Point = p(400.0, 270.0);
    pub const COPY_BUTTON: Point = p(570.0, 35.0);
    pub const DOWNLOAD: Point = p(540.0, 89.0);
    pub const BLANK_LINK: Point = p(535.0, 129.0);
    pub const MAIL: Point = p(535.0, 169.0);
    pub const OPEN_BUTTON: Point = p(570.0, 255.0);
    pub const TIP: Point = p(570.0, 305.0);
    pub const BLANK: Point = p(700.0, 420.0);
    /// The page's empty right side (nothing sits right of x 620 at any scroll), where
    /// [`super::HostOp::Neutral`] clicks: first here ...
    pub const NEUTRAL_A: Point = p(660.0, 500.0);
    /// ... then here, the click the page counts (`seen.neutral`, x >= 720).
    pub const NEUTRAL: Point = p(760.0, 460.0);
    /// Left of the view: over the sidebar.
    pub const SIDEBAR: Point = p(-120.0, 200.0);
}

/// Held modifiers for a pointer step.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mods {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    /// Cmd on macOS (Super).
    pub cmd: bool,
}

pub const NONE: Mods = Mods {
    ctrl: false,
    shift: false,
    alt: false,
    cmd: false,
};
pub const CTRL: Mods = Mods { ctrl: true, ..NONE };
pub const SHIFT: Mods = Mods {
    shift: true,
    ..NONE
};

impl Mods {
    pub fn state(self) -> ModifiersState {
        let mut s = ModifiersState::empty();
        s.set(ModifiersState::CONTROL, self.ctrl);
        s.set(ModifiersState::SHIFT, self.shift);
        s.set(ModifiersState::ALT, self.alt);
        s.set(ModifiersState::SUPER, self.cmd);
        s
    }

    /// The modifiers a click holds on `platform`: macOS opens a link in the background with
    /// Cmd, as Ctrl+click is a right click there. Wheel steps keep Ctrl (pinch-zoom's chord).
    pub fn for_click(self, platform: Platform) -> Self {
        match platform {
            Platform::Mac => Self {
                ctrl: false,
                cmd: self.cmd || self.ctrl,
                ..self
            },
            Platform::Linux | Platform::Windows => self,
        }
    }
}

/// Which pointer button.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    Left,
    Middle,
    Right,
}

/// What the host does for the smoke that is not page input.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum HostOp {
    /// Close the command bar as its Escape does.
    CloseBar,
    /// Close the find bar as its Escape does.
    CloseFind,
    /// Put this text on the system clipboard, so a stale value never passes (items wait
    /// with an `Until` for the compositor to offer it back).
    SetClipboard(&'static str),
    /// A file from the OS dragged over the page at the point (winit's `HoveredFile`).
    HoverFile(Point),
    /// That file dropped at the point (`DroppedFile`).
    DropFile(Point),
    /// Back to the parity page's tab, closing every other tab.
    OnlyFirstTab,
    ZoomReset,
    /// The print sheet's Save / Print, as its button.
    AnswerPrint,
    /// Answers the visible page's permission prompt: allow (true) or block, not remembered.
    AnswerPrompt(bool),
    /// Undoes what input left: the page's menu, the command and find bars dismissed, a
    /// held button released, then a left click at [`at::NEUTRAL_A`] and one at
    /// [`at::NEUTRAL`]. The first ends a middle-click autoscroll (Chromium takes that
    /// press for itself); a new button far from any item's point ends every click
    /// series; the pointer rests where no item looks.
    Neutral,
}

/// How an item is driven, in order.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Step {
    /// A script in the page (`parity.*`); waits for its answer.
    Js(&'static str),
    Click(Button, Mods, Point),
    /// Two or three left clicks at one point.
    Clicks(u8, Point),
    Press(Point),
    MoveTo(Point),
    Release(Point),
    /// Space-separated chords as `wubble_core::keys::Chord` parses them.
    Keys(&'static str),
    /// Text typed key by key (US layout).
    Type(&'static str),
    Wheel(Mods, Point, f32, f32),
    /// Waits for the page's context menu and chooses the entry with this label.
    Choose(&'static str),
    /// Waits for the page's context menu, requires these labels in this order
    /// (others may sit between them), and dismisses it.
    MenuHas(&'static [&'static str]),
    Host(HostOp),
    /// Only the parity page's tab, at 100 %, on the parity page; waits for it to load if it
    /// had to be loaded again.
    Restore,
    /// Lets this many runs of CEF work pass.
    Settle(u32),
    /// Polls this check as an expect is polled, and goes on once it holds; the item
    /// fails if it never does.
    Until(Expect),
    /// Runs the host op, then polls the check as `Until` does, running the op again
    /// every [`RETRY_EVERY`] asks (input sent before the window takes it is lost).
    Retry(HostOp, Expect),
}

/// What the host can tell about itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostCheck {
    /// The system clipboard holds this text (read as a paste reads it).
    Clipboard(&'static str),
    /// The system clipboard offers an image.
    ClipboardImage,
    /// The primary selection holds this text.
    Primary(&'static str),
    BarOpen(bool),
    FindOpen(bool),
    /// The visible page's zoom is above (true) or at (false) 100 %.
    Zoomed(bool),
    /// Tabs in the active Space.
    Tabs(usize),
    /// The visible tab is the n-th (0-based) of the active Space.
    Visible(usize),
    /// Downloads and prints this item started, at least.
    Downloads(usize),
    /// The window's tooltip shows a page's `title` with this text.
    Tooltip(&'static str),
    /// The link status bubble shows this URL.
    Status(&'static str),
    /// The visible page's permission prompt is open (true) or not.
    Prompt(bool),
    /// The web content's cursor, by `wubble_engine::nav::Cursor` index.
    Cursor(i32),
    /// The last link handed to the OS starts with this.
    External(&'static str),
    /// Middle-click autoscroll: off (`None`), still (`Some(None)`), or moving this way.
    Autoscroll(Option<Option<wubble_core::autoscroll::Direction>>),
    /// Middle-click autoscroll scrolls down (straight, or a little sideways).
    AutoscrollDown,
    /// The autoscroll arrow is drawn longer than at rest (stretch above 1.05).
    AutoscrollStretched,
    /// A page drag runs (true) or none does (false).
    Drag(bool),
    /// The print sheet is open (true) or not.
    PrintSheet(bool),
    /// The page's entrance after a tab switch has ended: while it runs the pane is drawn
    /// shifted and scaled, and a click lands where it is drawn, not at its point.
    PageEntered,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Expect {
    /// A page expression: `true`, or what it saw instead.
    Js(&'static str),
    Host(HostCheck),
}

#[derive(Clone, Copy)]
pub struct Item {
    pub name: &'static str,
    pub steps: &'static [Step],
    pub expect: &'static [Expect],
    /// A known gap (TODO.md says why): expected to fail. Its failure is reported but
    /// does not fail the smoke; its passing does, so the table stays honest.
    pub gap: bool,
}

use Button::{Left, Middle, Right};
use Expect::{Host as H, Js as J};
use HostCheck as C;
use Step::*;

/// Clears what earlier items left, each part waited for by count: other tabs, the view at its
/// full size (the window settles after start), zoom (until the page shows it), a page navigated away from (reloaded); menus, bars, held buttons, an autoscroll, click series and
/// the pointer ([`HostOp::Neutral`]), until the page saw the neutral click and the host
/// runs neither an autoscroll nor a page drag; then the page's records, focus, selection and scroll.
const RESET: [Step; 8] = [
    Restore,
    Until(H(C::PageEntered)),
    Until(J("parity.sized()")),
    Until(J("parity.unzoomed()")),
    Retry(
        HostOp::Neutral,
        J("seen.neutral > 0 || 'no neutral click yet'"),
    ),
    Until(H(C::Autoscroll(None))),
    Until(H(C::Drag(false))),
    Js("parity.reset()"),
];

macro_rules! item {
    (@gap $gap:literal, $name:literal, [$($step:expr),* $(,)?], [$($expect:expr),* $(,)?]) => {
        Item {
            name: $name,
            steps: &[
                RESET[0], RESET[1], RESET[2], RESET[3], RESET[4], RESET[5], RESET[6], RESET[7],
                $($step),*
            ],
            expect: &[$($expect),*],
            gap: $gap,
        }
    };
    ($($t:tt)*) => { item!(@gap false, $($t)*) };
}

/// An item for a known gap: see [`Item::gap`]. None is known now.
#[allow(unused_macros)]
macro_rules! gap {
    ($($t:tt)*) => { item!(@gap true, $($t)*) };
}

/// The checklist, A (keyboard) to F (page features), in the order they run.
pub const ITEMS: &[Item] = &[
    // A. Keyboard in the page.
    item!(
        "A.type-input",
        [Click(Left, NONE, at::INP), Type("Hello, World!")],
        [J("eq($('inp').value, 'Hello, World!')")]
    ),
    item!(
        "A.backspace-delete",
        [
            Js("parity.field('inp', 'abcd')"),
            Keys("BackSpace Home Delete")
        ],
        [J("eq($('inp').value, 'bc')")]
    ),
    item!(
        "A.enter-textarea",
        [Js("parity.field('ta', 'one')"), Keys("Enter"), Type("two")],
        [J("eq($('ta').value, 'one\\ntwo')")]
    ),
    item!(
        "A.tab-traversal",
        [Js("parity.field('inp', '')"), Keys("Tab")],
        [J("eq(document.activeElement.id, 'inp2')")]
    ),
    item!(
        "A.shift-tab",
        [Js("parity.field('inp2', '')"), Keys("Shift+Tab")],
        [J("eq(document.activeElement.id, 'inp')")]
    ),
    item!(
        "A.arrows-home-end",
        [
            Js("parity.field('inp', 'abc')"),
            Keys("Home"),
            Type("X"),
            Keys("End Left"),
            Type("Y")
        ],
        [J("eq($('inp').value, 'XabYc')")]
    ),
    item!(
        "A.pagedown-scrolls",
        [Click(Left, NONE, at::BLANK), Keys("PageDown")],
        [J("scrollY > 100 || 'scrollY ' + scrollY")]
    ),
    item!(
        "A.space-scrolls",
        [Click(Left, NONE, at::BLANK), Keys("Space")],
        [J("scrollY > 100 || 'scrollY ' + scrollY")]
    ),
    item!(
        "A.select-all",
        [
            Js("parity.field('inp', 'old text')"),
            Keys("Ctrl+A"),
            // macOS selects through the host's edit command, a round trip after the key
            Until(J(
                "$('inp').selectionEnd - $('inp').selectionStart == 8 || 'not all selected'"
            )),
            Type("z")
        ],
        [J("eq($('inp').value, 'z')")]
    ),
    item!(
        "A.copy-paste-keys",
        [
            Host(HostOp::SetClipboard("stale")),
            Until(H(C::Clipboard("stale"))),
            Js("parity.field('inp', 'copyme')"),
            Keys("Ctrl+A Ctrl+C"),
            // The window's clipboard hears of its own new selection a round trip later.
            Until(H(C::Clipboard("copyme"))),
            Js("parity.field('inp2', '')"),
            Keys("Ctrl+V")
        ],
        [
            J("eq($('inp2').value, 'copyme')"),
            H(C::Clipboard("copyme"))
        ]
    ),
    item!(
        "A.cut-keys",
        [
            Js("parity.field('inp', 'cutme')"),
            Keys("Ctrl+A Ctrl+X"),
            // The window's clipboard hears of its own new selection a round trip later.
            Until(H(C::Clipboard("cutme"))),
            Js("parity.field('inp2', '')"),
            Keys("Ctrl+V")
        ],
        [J("eq($('inp').value + '|' + $('inp2').value, '|cutme')")]
    ),
    item!(
        "A.undo-redo",
        [Js("parity.field('inp', '')"), Type("ab"), Keys("Ctrl+Z")],
        [J("eq($('inp').value, '')")]
    ),
    item!(
        "A.redo-shift-z",
        [
            Js("parity.field('inp', '')"),
            Type("ab"),
            Keys("Ctrl+Z Ctrl+Shift+Z")
        ],
        [J("eq($('inp').value, 'ab')")]
    ),
    item!(
        "A.redo-y",
        [
            Js("parity.field('inp', '')"),
            Type("ab"),
            Keys("Ctrl+Z Ctrl+Y")
        ],
        [J("eq($('inp').value, 'ab')")]
    ),
    item!(
        "A.word-moves",
        [
            Js("parity.field('inp', 'one two')"),
            Keys("Ctrl+Left"),
            Type("X"),
            Keys("Ctrl+Right"),
            Type("Y")
        ],
        [J("eq($('inp').value, 'one XtwoY')")]
    ),
    item!(
        "A.shift-select",
        [
            Js("parity.field('inp', 'abcd')"),
            Keys("Shift+Left Shift+Left"),
            Type("Z")
        ],
        [J("eq($('inp').value, 'abZ')")]
    ),
    item!(
        "A.ctrl-backspace",
        [Js("parity.field('inp', 'one two')"), Keys("Ctrl+BackSpace")],
        [J("eq($('inp').value, 'one ')")]
    ),
    item!(
        "A.contenteditable",
        [Click(Left, NONE, at::CE), Type("Edit me")],
        [J("eq($('ce').textContent, 'Edit me')")]
    ),
    item!(
        "A.page-owns-ctrl-f",
        [
            Click(Left, NONE, at::BLANK),
            Js("seen.grab = true"),
            Keys("Ctrl+F"),
            Settle(20)
        ],
        [
            J("seen.keys.includes('grabbed') || seen.keys.join()"),
            H(C::FindOpen(false))
        ]
    ),
    item!(
        "A.ctrl-f-find",
        [Click(Left, NONE, at::BLANK), Keys("Ctrl+F")],
        [H(C::FindOpen(true))]
    ),
    item!(
        "A.ctrl-t-bar",
        [
            Host(HostOp::CloseFind),
            Click(Left, NONE, at::BLANK),
            Keys("Ctrl+T")
        ],
        [H(C::BarOpen(true))]
    ),
    item!(
        "A.ctrl-l-bar",
        [
            Host(HostOp::CloseBar),
            Click(Left, NONE, at::BLANK),
            Keys("Ctrl+L")
        ],
        [H(C::BarOpen(true))]
    ),
    item!(
        "A.zoom-keys",
        [
            Host(HostOp::CloseBar),
            Click(Left, NONE, at::BLANK),
            Keys("Ctrl+=")
        ],
        [H(C::Zoomed(true))]
    ),
    item!(
        "A.zoom-reset",
        [Click(Left, NONE, at::BLANK), Keys("Ctrl+0")],
        [H(C::Zoomed(false))]
    ),
    item!(
        "A.alt-left-back",
        [
            Js("history.pushState(null, '', '#one')"),
            Js("history.pushState(null, '', '#two')"),
            Click(Left, NONE, at::BLANK),
            Keys("Alt+Left")
        ],
        [J("eq(location.hash, '#one')")]
    ),
    item!(
        "A.alt-right-forward",
        [Click(Left, NONE, at::BLANK), Keys("Alt+Right")],
        [J("eq(location.hash, '#two')")]
    ),
    // The print sheet opens; Save writes the PDF and lists it as a finished download.
    item!(
        "A.ctrl-p-print",
        [
            Click(Left, NONE, at::BLANK),
            Keys("Ctrl+P"),
            Until(H(C::PrintSheet(true))),
            Host(HostOp::AnswerPrint),
            Until(H(C::PrintSheet(false)))
        ],
        [H(C::Downloads(1))]
    ),
    // B. Clipboard through the context menu, and the page's own copy.
    item!(
        "B.menu-copy-selection",
        [
            Host(HostOp::SetClipboard("stale")),
            Until(H(C::Clipboard("stale"))),
            Js("parity.select(0, 5)"),
            Click(Right, NONE, at::WORD),
            Choose("Copy")
        ],
        [H(C::Clipboard("alpha"))]
    ),
    item!(
        "B.menu-paste",
        [
            Host(HostOp::SetClipboard("pasted")),
            Until(H(C::Clipboard("pasted"))),
            Js("parity.field('inp', '')"),
            Click(Right, NONE, at::INP),
            Choose("Paste")
        ],
        [J("eq($('inp').value, 'pasted')")]
    ),
    item!(
        "B.menu-cut",
        [
            Host(HostOp::SetClipboard("stale")),
            Until(H(C::Clipboard("stale"))),
            Js("parity.field('inp', 'menucut')"),
            Js("$('inp').select()"),
            Click(Right, NONE, at::INP),
            Choose("Cut")
        ],
        [J("eq($('inp').value, '')"), H(C::Clipboard("menucut"))]
    ),
    item!(
        "B.menu-select-all",
        [
            Js("parity.field('inp', 'all of it')"),
            Click(Right, NONE, at::INP),
            Choose("Select All"),
            Type("q")
        ],
        [J("eq($('inp').value, 'q')")]
    ),
    item!(
        "B.menu-undo",
        [
            Js("parity.field('inp', '')"),
            Type("xy"),
            Click(Right, NONE, at::INP),
            Choose("Undo")
        ],
        [J("eq($('inp').value, '')")]
    ),
    item!(
        "B.menu-paste-plain",
        [
            Host(HostOp::SetClipboard("plain")),
            Until(H(C::Clipboard("plain"))),
            Js("parity.field('ce', '')"),
            Click(Right, NONE, at::CE),
            Choose("Paste as Plain Text")
        ],
        [J("eq($('ce').textContent, 'plain')")]
    ),
    item!(
        "B.copy-link-address",
        [
            Host(HostOp::SetClipboard("stale")),
            Until(H(C::Clipboard("stale"))),
            Click(Right, NONE, at::LINK),
            Choose("Copy Link Address")
        ],
        [H(C::Clipboard("https://example.invalid/linked"))]
    ),
    item!(
        "B.copy-image",
        [
            Host(HostOp::SetClipboard("stale")),
            Until(H(C::Clipboard("stale"))),
            Click(Right, NONE, at::IMG),
            Choose("Copy Image")
        ],
        [H(C::ClipboardImage)]
    ),
    item!(
        "B.page-execcommand-copy",
        [
            Host(HostOp::SetClipboard("stale")),
            Until(H(C::Clipboard("stale"))),
            Click(Left, NONE, at::COPY_BUTTON)
        ],
        [H(C::Clipboard("from execCommand"))]
    ),
    item!(
        "B.primary-selection",
        [
            Js("parity.field('inp', 'primary text')"),
            Keys("Ctrl+A"),
            Settle(10)
        ],
        [H(C::Primary("primary text"))]
    ),
    item!(
        "B.middle-click-paste",
        [
            Js("parity.field('inp2', '')"),
            Js("parity.field('inp', 'middle')"),
            Keys("Ctrl+A"),
            Until(H(C::Primary("middle"))),
            Click(Middle, NONE, at::INP2)
        ],
        [J("eq($('inp2').value, 'middle')")]
    ),
    // C. Context menu entries, as Chromium lists them.
    item!(
        "C.menu-link",
        [
            Click(Right, NONE, at::LINK),
            MenuHas(&[
                "Open Link in New Tab",
                "Save Link As\u{2026}",
                "Copy Link Address"
            ])
        ],
        []
    ),
    item!(
        "C.menu-image",
        [
            Click(Right, NONE, at::IMG),
            MenuHas(&[
                "Open Image in New Tab",
                "Save Image As\u{2026}",
                "Copy Image",
                "Copy Image Address"
            ])
        ],
        []
    ),
    item!(
        "C.menu-selection",
        [
            Js("parity.select(6, 10)"),
            Click(Right, NONE, at::x(at::GAMMA, 110.0)),
            MenuHas(&[
                "Copy",
                "Search the Web for \u{201c}beta\u{201d}",
                "Print\u{2026}"
            ])
        ],
        []
    ),
    item!(
        "C.menu-editable",
        [
            Js("parity.field('inp', 'x')"),
            Click(Right, NONE, at::INP),
            MenuHas(&[
                "Undo",
                "Redo",
                "Cut",
                "Copy",
                "Paste",
                "Paste as Plain Text",
                "Select All"
            ])
        ],
        []
    ),
    item!(
        "C.menu-page",
        [
            Click(Right, NONE, at::BLANK),
            MenuHas(&[
                "Back",
                "Forward",
                "Reload",
                "Save Page As\u{2026}",
                "Print\u{2026}",
                "View Page Source",
                "Inspect"
            ])
        ],
        []
    ),
    item!(
        "C.menu-video",
        [
            Click(Right, NONE, at::VIDEO),
            MenuHas(&["Loop", "Show Controls"])
        ],
        []
    ),
    item!(
        "C.save-link-downloads",
        [
            Click(Right, NONE, at::DOWNLOAD),
            Choose("Save Link As\u{2026}"),
            Settle(20)
        ],
        [H(C::Downloads(1))]
    ),
    // D. Drag and drop.
    item!(
        "D.html5-drag",
        [
            Press(at::DRAG),
            MoveTo(at::x(at::DRAG, 375.0)),
            Until(H(C::Drag(true))),
            MoveTo(at::x(at::DROP, 380.0)),
            MoveTo(at::DROP),
            Settle(10),
            Release(at::DROP)
        ],
        [J("eq(seen.drops.join(), 'payload')")]
    ),
    // A type the page set itself (`application/x-wubble`) reaches the drop: the page's own drag
    // data is re-entered as it was (welding change 48).
    item!(
        "D.custom-type-in-window",
        [
            Press(at::DRAG),
            MoveTo(at::x(at::DRAG, 375.0)),
            Until(H(C::Drag(true))),
            MoveTo(at::x(at::DROP, 380.0)),
            MoveTo(at::DROP),
            Settle(10),
            Release(at::DROP)
        ],
        [J("eq(seen.custom.join(), 'custom')")]
    ),
    // Selected text dragged from one field into another moves: the page's chosen operation
    // (CEF `UpdateDragCursor`) ends the drag, so the source field deletes it.
    item!(
        "D.text-move-between-fields",
        [
            Js("parity.field('inp2', '')"),
            Js("parity.selectField('inp', 'alpha')"),
            Press(at::x(at::INP, 32.0)),
            MoveTo(at::x(at::INP, 60.0)),
            Until(H(C::Drag(true))),
            MoveTo(at::x(at::INP2, 100.0)),
            MoveTo(at::INP2),
            Settle(10),
            Release(at::INP2),
            Settle(10)
        ],
        [
            J("eq($('inp2').value, 'alpha')"),
            J("eq($('inp').value, '')"),
            J("eq(seen.effect, 'move')")
        ]
    ),
    item!(
        "D.text-drag-to-input",
        [
            Js("parity.field('inp2', '')"),
            Js("parity.select(0, 5)"),
            Press(at::WORD),
            MoveTo(at::x(at::WORD, 50.0)),
            Until(H(C::Drag(true))),
            MoveTo(at::x(at::INP2, 100.0)),
            MoveTo(at::INP2),
            Settle(10),
            Release(at::INP2)
        ],
        [J("eq($('inp2').value, 'alpha')")]
    ),
    item!(
        "D.link-to-sidebar",
        [
            Press(at::LINK),
            MoveTo(at::x(at::LINK, 60.0)),
            Until(H(C::Drag(true))),
            MoveTo(at::x(at::SIDEBAR, -20.0)),
            MoveTo(at::SIDEBAR),
            Settle(10),
            Release(at::SIDEBAR),
            Settle(20)
        ],
        [H(C::Tabs(2))]
    ),
    item!(
        "D.os-file-drop",
        [
            Host(HostOp::OnlyFirstTab),
            Until(H(C::PageEntered)),
            Host(HostOp::HoverFile(at::FILEZONE)),
            Settle(10),
            Host(HostOp::DropFile(at::FILEZONE))
        ],
        [J("eq(seen.files.join(), 'parity-drop.txt')")]
    ),
    // E. Mouse.
    item!(
        "E.double-click-word",
        [Clicks(2, at::WORD)],
        [J("eq(getSelection().toString(), 'alpha')")]
    ),
    item!(
        "E.triple-click-line",
        [Clicks(3, at::WORD)],
        [J(
            "eq(getSelection().toString().trim(), 'alpha beta gamma')"
        )]
    ),
    item!(
        "E.shift-click-extends",
        [Clicks(2, at::WORD), Click(Left, SHIFT, at::GAMMA)],
        [J(
            "getSelection().toString().startsWith('alpha beta g') || getSelection().toString()"
        )]
    ),
    item!(
        "E.ctrl-click-background-tab",
        [Click(Left, CTRL, at::LINK), Settle(20)],
        [
            H(C::Tabs(2)),
            H(C::Visible(0)),
            J("eq(location.href.startsWith('data:'), true)")
        ]
    ),
    item!(
        "E.middle-click-background-tab",
        [
            Host(HostOp::OnlyFirstTab),
            Until(H(C::PageEntered)),
            Click(Middle, NONE, at::LINK),
            Settle(20)
        ],
        [H(C::Tabs(2)), H(C::Visible(0))]
    ),
    item!(
        "E.shift-wheel-horizontal",
        [
            Host(HostOp::OnlyFirstTab),
            Until(H(C::PageEntered)),
            Wheel(SHIFT, at::BLANK, 0.0, -120.0),
            Settle(10)
        ],
        [J("scrollX > 0 || 'scrollX ' + scrollX")]
    ),
    item!(
        "E.ctrl-wheel-zoom",
        [Wheel(CTRL, at::BLANK, 0.0, 120.0), Settle(10)],
        [H(C::Zoomed(true))]
    ),
    // Middle-click autoscroll on the tall page: a click starts it still, a click ends it,
    // moving below the anchor scrolls down, a key ends it.
    item!(
        "E.autoscroll-still",
        [MoveTo(at::BLANK), Click(Middle, NONE, at::BLANK)],
        [H(C::Autoscroll(Some(None)))]
    ),
    item!(
        "E.autoscroll-click-ends",
        [
            MoveTo(at::BLANK),
            Click(Middle, NONE, at::BLANK),
            Until(H(C::Autoscroll(Some(None)))),
            Click(Left, NONE, at::BLANK)
        ],
        [H(C::Autoscroll(None))]
    ),
    item!(
        "E.autoscroll-down",
        [
            MoveTo(at::BLANK),
            Click(Middle, NONE, at::BLANK),
            Until(H(C::Autoscroll(Some(None)))),
            MoveTo(at::y(at::BLANK, 150.0)),
            Settle(10),
            MoveTo(at::y(at::BLANK, 160.0)),
            Settle(30)
        ],
        [
            H(C::AutoscrollDown),
            H(C::AutoscrollStretched),
            J("scrollY > 0 || 'scrollY ' + scrollY")
        ]
    ),
    item!(
        "E.autoscroll-key-ends",
        [
            MoveTo(at::BLANK),
            Click(Middle, NONE, at::BLANK),
            Until(H(C::Autoscroll(Some(None)))),
            Keys("a")
        ],
        [H(C::Autoscroll(None))]
    ),
    item!(
        "E.hover-cursor",
        [MoveTo(at::BLANK), MoveTo(at::LINK)],
        [H(C::Cursor(2))]
    ),
    item!(
        "E.link-status",
        [MoveTo(at::BLANK), MoveTo(at::LINK)],
        [H(C::Status("https://example.invalid/linked"))]
    ),
    item!(
        "E.title-tooltip",
        [MoveTo(at::BLANK), MoveTo(at::TIP), Settle(30)],
        [H(C::Tooltip("tip text"))]
    ),
    // F. Page features that need the embedder.
    item!(
        "F.a-download",
        [Click(Left, NONE, at::DOWNLOAD), Settle(20)],
        [H(C::Downloads(1))]
    ),
    item!(
        "F.mailto-to-os",
        [
            Click(Left, NONE, at::MAIL),
            // the click landed on the link (the page records every press)
            Until(J(
                "(seen.down || '').startsWith('mail') || ('down ' + seen.down)"
            )),
            Until(H(C::Prompt(true))),
            Host(HostOp::AnswerPrompt(true)),
            Settle(5)
        ],
        [H(C::External("mailto:nobody@example.invalid"))]
    ),
    item!(
        "F.target-blank",
        [Click(Left, NONE, at::BLANK_LINK), Settle(20)],
        [H(C::Tabs(2))]
    ),
    item!(
        "F.window-open",
        [
            Host(HostOp::OnlyFirstTab),
            Until(H(C::PageEntered)),
            Click(Left, NONE, at::OPEN_BUTTON),
            Settle(20)
        ],
        [H(C::Tabs(2))]
    ),
    // A click straight after a tab switch: it once landed 11 px off, during the page's
    // entrance (now waited for with `PageEntered`).
    item!(
        "F.click-after-popup",
        [
            Host(HostOp::OnlyFirstTab),
            Until(H(C::PageEntered)),
            Click(Left, NONE, at::INP),
            Settle(5)
        ],
        [J("eq((seen.down || '').split('/')[0], 'inp@140,34')")]
    ),
];

// ----------------------------------------------------------------- keys

/// A key on a US layout: its code, logical key, and the characters it types unshifted and
/// shifted (none for a named key). The OS's scan code comes from winit (`to_scancode`).
type UsKey = (KeyCode, WinitKey, Option<(char, char)>);

fn us_key(key: Key) -> Option<UsKey> {
    use KeyCode as K;
    let named = |n| WinitKey::Named(n);
    let ch = |c: char| WinitKey::Character(c.to_string().into());
    const LETTERS: [KeyCode; 26] = [
        K::KeyA,
        K::KeyB,
        K::KeyC,
        K::KeyD,
        K::KeyE,
        K::KeyF,
        K::KeyG,
        K::KeyH,
        K::KeyI,
        K::KeyJ,
        K::KeyK,
        K::KeyL,
        K::KeyM,
        K::KeyN,
        K::KeyO,
        K::KeyP,
        K::KeyQ,
        K::KeyR,
        K::KeyS,
        K::KeyT,
        K::KeyU,
        K::KeyV,
        K::KeyW,
        K::KeyX,
        K::KeyY,
        K::KeyZ,
    ];
    const DIGITS: [(KeyCode, char); 10] = [
        (K::Digit0, ')'),
        (K::Digit1, '!'),
        (K::Digit2, '@'),
        (K::Digit3, '#'),
        (K::Digit4, '$'),
        (K::Digit5, '%'),
        (K::Digit6, '^'),
        (K::Digit7, '&'),
        (K::Digit8, '*'),
        (K::Digit9, '('),
    ];
    const PUNCT: [(KeyCode, char, char); 11] = [
        (K::Minus, '-', '_'),
        (K::Equal, '=', '+'),
        (K::BracketLeft, '[', '{'),
        (K::BracketRight, ']', '}'),
        (K::Semicolon, ';', ':'),
        (K::Quote, '\'', '"'),
        (K::Backquote, '`', '~'),
        (K::Backslash, '\\', '|'),
        (K::Comma, ',', '<'),
        (K::Period, '.', '>'),
        (K::Slash, '/', '?'),
    ];
    Some(match key {
        Key::Char(c @ 'a'..='z') => {
            let code = LETTERS[(c as u8 - b'a') as usize];
            (code, ch(c), Some((c, c.to_ascii_uppercase())))
        }
        Key::Char(c @ '0'..='9') => {
            let (code, shifted) = DIGITS[(c as u8 - b'0') as usize];
            (code, ch(c), Some((c, shifted)))
        }
        // A digit's shifted symbol (`!` is Shift+1), or punctuation.
        Key::Char(c) => match DIGITS.iter().find(|d| d.1 == c) {
            Some(&(code, shifted)) => {
                let plain = char::from(b'0' + DIGITS.iter().position(|d| d.1 == c)? as u8);
                (code, ch(plain), Some((plain, shifted)))
            }
            None => {
                let &(code, plain, shifted) = PUNCT.iter().find(|p| p.1 == c || p.2 == c)?;
                (code, ch(plain), Some((plain, shifted)))
            }
        },
        Key::Space => (K::Space, ch(' '), Some((' ', ' '))),
        Key::Enter => (K::Enter, named(NamedKey::Enter), Some(('\r', '\r'))),
        Key::Tab => (K::Tab, named(NamedKey::Tab), Some(('\t', '\t'))),
        Key::Escape => (K::Escape, named(NamedKey::Escape), None),
        Key::Backspace => (K::Backspace, named(NamedKey::Backspace), None),
        Key::Delete => (K::Delete, named(NamedKey::Delete), None),
        Key::Left => (K::ArrowLeft, named(NamedKey::ArrowLeft), None),
        Key::Right => (K::ArrowRight, named(NamedKey::ArrowRight), None),
        Key::Up => (K::ArrowUp, named(NamedKey::ArrowUp), None),
        Key::Down => (K::ArrowDown, named(NamedKey::ArrowDown), None),
        Key::Home => (K::Home, named(NamedKey::Home), None),
        Key::End => (K::End, named(NamedKey::End), None),
        Key::PageUp => (K::PageUp, named(NamedKey::PageUp), None),
        Key::PageDown => (K::PageDown, named(NamedKey::PageDown), None),
        Key::F(n @ 1..=10) => {
            const F: [KeyCode; 10] = [
                K::F1,
                K::F2,
                K::F3,
                K::F4,
                K::F5,
                K::F6,
                K::F7,
                K::F8,
                K::F9,
                K::F10,
            ];
            const N: [NamedKey; 10] = [
                NamedKey::F1,
                NamedKey::F2,
                NamedKey::F3,
                NamedKey::F4,
                NamedKey::F5,
                NamedKey::F6,
                NamedKey::F7,
                NamedKey::F8,
                NamedKey::F9,
                NamedKey::F10,
            ];
            let i = usize::from(n - 1);
            (F[i], named(N[i]), None)
        }
        Key::F(_) => return None,
    })
}

/// The modifier keys a chord holds, in press order: code, key, state bit. Cmd is Super.
const MODIFIER_KEYS: [(KeyCode, NamedKey, ModifiersState); 4] = [
    (
        KeyCode::ControlLeft,
        NamedKey::Control,
        ModifiersState::CONTROL,
    ),
    (KeyCode::AltLeft, NamedKey::Alt, ModifiersState::ALT),
    (KeyCode::ShiftLeft, NamedKey::Shift, ModifiersState::SHIFT),
    (KeyCode::SuperLeft, NamedKey::Super, ModifiersState::SUPER),
];

fn input(
    pressed: bool,
    code: KeyCode,
    unmodified: WinitKey,
    text: Option<String>,
    location: KeyLocation,
    keys: ModifiersState,
) -> KeyInput {
    KeyInput {
        pressed,
        repeat: false,
        physical: PhysicalKey::Code(code),
        scancode: code.to_scancode(),
        unmodified,
        text,
        location,
        modifiers: Modifiers {
            keys,
            buttons: Buttons::default(),
            locks: Default::default(),
        },
    }
}

/// The key events of one chord as winit reports them with a US layout: each
/// modifier pressed (the state includes it from its own press on, as Chromium's
/// X11 and Wayland events do), the key pressed and released, the modifiers
/// released. winit's Linux `text` ignores Ctrl (Ctrl+C types "c"). The scan
/// code is this OS's (winit's `to_scancode`).
pub fn key_inputs(chord: Chord) -> Option<Vec<KeyInput>> {
    let (code, unmodified, chars) = us_key(chord.key)?;
    let held = [chord.ctrl, chord.alt, chord.shift, chord.cmd];
    let mut state = ModifiersState::empty();
    let mut out = vec![];
    for (on, &(code, named, bit)) in held.iter().zip(&MODIFIER_KEYS) {
        if *on {
            state |= bit;
            out.push(input(
                true,
                code,
                WinitKey::Named(named),
                None,
                KeyLocation::Left,
                state,
            ));
        }
    }
    let text = chars.map(|(plain, shifted)| if chord.shift { shifted } else { plain }.to_string());
    out.push(input(
        true,
        code,
        unmodified.clone(),
        text,
        KeyLocation::Standard,
        state,
    ));
    out.push(input(
        false,
        code,
        unmodified,
        None,
        KeyLocation::Standard,
        state,
    ));
    for (on, &(code, named, bit)) in held.iter().zip(&MODIFIER_KEYS).rev() {
        if *on {
            state.remove(bit);
            out.push(input(
                false,
                code,
                WinitKey::Named(named),
                None,
                KeyLocation::Left,
                state,
            ));
        }
    }
    Some(out)
}

/// The chords that type `text` on a US layout.
pub fn typed(text: &str) -> Option<Vec<Chord>> {
    text.chars()
        .map(|c| {
            let (_, _, chars) = us_key(if c == ' ' {
                Key::Space
            } else {
                Key::Char(c.to_ascii_lowercase())
            })?;
            let (plain, shifted) = chars?;
            let shift = c != plain && c == shifted;
            let key = if c == ' ' {
                Key::Space
            } else {
                Key::Char(plain)
            };
            Some(Chord {
                ctrl: false,
                alt: false,
                shift,
                cmd: false,
                key,
            })
        })
        .collect()
}

/// macOS's own chord for each Linux chord the script names: Cmd where Linux takes Ctrl (edit,
/// find, the shell's chords, zoom), Option for word moves, Cmd+[ and Cmd+] for history (as
/// Chromium and AppKit text fields on macOS). Data, applied by [`chords_on`].
pub const MAC_CHORDS: &[(&str, &str)] = &[
    ("Ctrl+A", "Cmd+A"),
    ("Ctrl+C", "Cmd+C"),
    ("Ctrl+V", "Cmd+V"),
    ("Ctrl+X", "Cmd+X"),
    ("Ctrl+Z", "Cmd+Z"),
    ("Ctrl+Shift+Z", "Cmd+Shift+Z"),
    // macOS has no Ctrl+Y redo (Cmd+Y is history); its redo is Cmd+Shift+Z
    ("Ctrl+Y", "Cmd+Shift+Z"),
    ("Ctrl+F", "Cmd+F"),
    ("Ctrl+T", "Cmd+T"),
    ("Ctrl+L", "Cmd+L"),
    ("Ctrl+P", "Cmd+P"),
    ("Ctrl+=", "Cmd+="),
    ("Ctrl+0", "Cmd+0"),
    ("Ctrl+Left", "Alt+Left"),
    ("Ctrl+Right", "Alt+Right"),
    ("Ctrl+BackSpace", "Alt+BackSpace"),
    ("Alt+Left", "Cmd+["),
    ("Alt+Right", "Cmd+]"),
];

/// Items an OS has no equivalent for, expected to fail there as [`Item::gap`] items do
/// everywhere: macOS and Windows have no primary selection.
pub const PLATFORM_GAPS: &[(Platform, &str)] = &[
    (Platform::Mac, "B.primary-selection"),
    (Platform::Mac, "B.middle-click-paste"),
    (Platform::Windows, "B.primary-selection"),
    (Platform::Windows, "B.middle-click-paste"),
];

/// `item` is expected to fail on `platform`.
pub fn is_gap(item: &Item, platform: Platform) -> bool {
    item.gap || PLATFORM_GAPS.contains(&(platform, item.name))
}

/// `Keys` steps name keys as `Chord::parse` does, with `BackSpace` for Backspace too, as
/// this OS's chords.
pub fn chords(spec: &str) -> Option<Vec<Chord>> {
    chords_on(spec, Platform::current())
}

/// [`chords`] on `platform`.
pub fn chords_on(spec: &str, platform: Platform) -> Option<Vec<Chord>> {
    spec.split_whitespace()
        .map(|c| {
            let c = match platform {
                Platform::Mac => MAC_CHORDS
                    .iter()
                    .find(|(linux, _)| *linux == c)
                    .map_or(c, |(_, mac)| mac),
                Platform::Linux | Platform::Windows => c,
            };
            Chord::parse(&c.replace("BackSpace", "Backspace")).ok()
        })
        .collect()
}

// ---------------------------------------------------------------- driver

#[derive(Clone, Debug, PartialEq)]
pub enum Wait {
    /// The step's script answer, by serial.
    Answer(u32),
    /// The page's context menu.
    Menu,
    /// Runs of CEF work left.
    Runs(u32),
    /// The check's answer, by serial.
    Check(u32),
    /// Runs before the check is asked again.
    Gap(u32),
    /// The page loaded (`Step::Restore`).
    Load,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub enum Parity {
    /// The page is loading.
    #[default]
    Load,
    At {
        item: usize,
        /// Index into the item's steps, then (past them) its expects.
        step: usize,
        wait: Option<Wait>,
        /// Runs of CEF work spent waiting.
        runs: u32,
        polls: u32,
        /// What the last ask of a check saw.
        last: String,
    },
    Done,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Input<'a> {
    /// CEF did a run of work.
    Worked,
    /// The page finished loading.
    Loaded,
    Title(&'a str),
    /// The page's context menu is open with these entries (label, enabled).
    Menu(&'a [(String, bool)]),
    /// The answer to the last `Action::Check`.
    Checked(Result<(), String>),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    /// Run `parity.run(serial, ...)` / `parity.check(...)` in the page.
    Script(String),
    Step(Step),
    /// Choose entry `i` of the open menu.
    Choose(usize),
    DismissMenu,
    /// Ask the host; it answers with `Input::Checked` at once.
    Check(HostCheck),
    /// One item finished.
    Report {
        name: &'static str,
        result: Result<(), String>,
        /// The item is a known gap ([`Item::gap`]).
        gap: bool,
    },
    /// Every item ran.
    Finish,
}

fn serial(item: usize, step: usize, polls: u32) -> u32 {
    (item as u32) * 10_000 + (step as u32) * 100 + polls
}

/// The answer in a title `parity:<serial>:...`, if it is that serial's.
fn answer(title: &str, want: u32) -> Option<Result<(), String>> {
    let rest = title.strip_prefix("parity:")?;
    let (serial, verdict) = rest.split_once(':')?;
    if serial.parse::<u32>().ok()? != want {
        return None;
    }
    Some(match verdict {
        "ok" => Ok(()),
        other => Err(other.strip_prefix("fail:").unwrap_or(other).to_owned()),
    })
}

/// Requires `want` to appear in `labels` in order.
pub fn menu_lists(labels: &[(String, bool)], want: &[&str]) -> Result<(), String> {
    let mut it = labels.iter();
    for w in want {
        if !it.any(|(l, _)| l == w) {
            let all: Vec<&str> = labels
                .iter()
                .map(|(l, _)| l.as_str())
                .filter(|l| !l.is_empty())
                .collect();
            return Err(format!("no {w:?} (in order) in {all:?}"));
        }
    }
    Ok(())
}

/// The items whose names contain one of `filter`'s comma-separated parts (all without one):
/// `WUBBLE_PARITY=B.,D.html` runs a few by hand.
pub fn selected(filter: Option<&str>) -> Vec<Item> {
    ITEMS
        .iter()
        .filter(|i| filter.is_none_or(|f| f.split(',').any(|part| i.name.contains(part))))
        .copied()
        .collect()
}

pub fn next_in(items: &[Item], state: Parity, input: Input) -> (Parity, Vec<Action>) {
    match (state, input) {
        (Parity::Load, Input::Loaded) => advance(items, 0, 0, vec![]),
        (
            Parity::At {
                item,
                step,
                wait,
                runs,
                polls,
                last,
            },
            input,
        ) => on_wait(items, item, step, wait, runs, polls, last, input),
        (state, _) => (state, vec![]),
    }
}

#[allow(clippy::too_many_arguments)]
fn on_wait(
    items: &[Item],
    item: usize,
    step: usize,
    wait: Option<Wait>,
    runs: u32,
    polls: u32,
    last: String,
    input: Input,
) -> (Parity, Vec<Action>) {
    let same = |wait, runs, last| Parity::At {
        item,
        step,
        wait,
        runs,
        polls,
        last,
    };
    let fail = |why: String| finish_item(items, item, Err(why));
    match (wait, input) {
        (Some(Wait::Load), Input::Loaded) => advance(items, item, step + 1, vec![]),
        (Some(Wait::Runs(n)), Input::Worked) if n <= 1 => advance(items, item, step + 1, vec![]),
        (Some(Wait::Runs(n)), Input::Worked) => (same(Some(Wait::Runs(n - 1)), runs, last), vec![]),
        (Some(Wait::Gap(n)), Input::Worked) if n <= 1 => ask(items, item, step, polls + 1),
        (Some(Wait::Gap(n)), Input::Worked) => (same(Some(Wait::Gap(n - 1)), runs, last), vec![]),
        (Some(w @ (Wait::Answer(_) | Wait::Menu | Wait::Check(_) | Wait::Load)), Input::Worked) => {
            if runs + 1 >= MAX_WAIT_RUNS {
                let what = match w {
                    Wait::Menu => "no context menu".to_owned(),
                    Wait::Load => "the page did not load again".to_owned(),
                    _ if last.is_empty() => "the page never answered".to_owned(),
                    _ => last,
                };
                let (s, mut a) = fail(what);
                if w == Wait::Menu {
                    a.insert(0, Action::DismissMenu);
                }
                (s, a)
            } else {
                (same(Some(w), runs + 1, last), vec![])
            }
        }
        (Some(Wait::Answer(want)), Input::Title(title)) => match answer(title, want) {
            Some(Ok(())) => advance(items, item, step + 1, vec![]),
            Some(Err(why)) => fail(format!("setup failed: {why}")),
            None => (same(Some(Wait::Answer(want)), runs, last), vec![]),
        },
        (Some(Wait::Check(want)), Input::Title(title)) => match answer(title, want) {
            Some(result) => checked(items, item, step, polls, result),
            None => (same(Some(Wait::Check(want)), runs, last), vec![]),
        },
        (Some(Wait::Menu), Input::Menu(entries)) => match items[item].steps[step] {
            Step::Choose(label) => match entries.iter().position(|(l, on)| l == label && *on) {
                Some(i) => advance(items, item, step + 1, vec![Action::Choose(i)]),
                None => {
                    let (s, mut a) = fail(
                        menu_lists(entries, &[label])
                            .err()
                            .unwrap_or_else(|| format!("{label:?} is disabled")),
                    );
                    a.insert(0, Action::DismissMenu);
                    (s, a)
                }
            },
            Step::MenuHas(want) => match menu_lists(entries, want) {
                Ok(()) => advance(items, item, step + 1, vec![Action::DismissMenu]),
                Err(why) => {
                    let (s, mut a) = fail(why);
                    a.insert(0, Action::DismissMenu);
                    (s, a)
                }
            },
            _ => (same(Some(Wait::Menu), runs, last), vec![]),
        },
        (None, Input::Checked(result)) if checks_at(&items[item], step) => {
            checked(items, item, step, polls, result)
        }
        (wait, _) => (same(wait, runs, last), vec![]),
    }
}

/// A check answered: next expect, or ask again after a gap, or fail.
fn checked(
    items: &[Item],
    item: usize,
    step: usize,
    polls: u32,
    result: Result<(), String>,
) -> (Parity, Vec<Action>) {
    match result {
        Ok(()) => advance(items, item, step + 1, vec![]),
        Err(why) if step < items[item].steps.len() && polls + 1 >= MAX_SETUP_POLLS => {
            finish_item(items, item, Err(format!("setup never held: {why}")))
        }
        Err(why) if step >= items[item].steps.len() && polls + 1 >= MAX_POLLS => {
            finish_item(items, item, Err(why))
        }
        Err(why) => (
            Parity::At {
                item,
                step,
                wait: Some(Wait::Gap(POLL_GAP)),
                runs: 0,
                polls,
                last: why,
            },
            vec![],
        ),
    }
}

/// Whether `step` of `item` is a check: an `Until` step, or an expect past the steps.
fn checks_at(item: &Item, step: usize) -> bool {
    item.steps
        .get(step)
        .is_none_or(|s| matches!(s, Step::Until(_) | Step::Retry(..)))
}

/// The check at `step`: an `Until`'s, or expect `step - steps`.
fn check_at(item: &Item, step: usize) -> Expect {
    match item.steps.get(step) {
        Some(Step::Until(check) | Step::Retry(_, check)) => *check,
        _ => item.expect[step - item.steps.len()],
    }
}

/// Asks the check at `step` of `item` (poll `polls`); a `Retry` step runs its op first, on
/// the first ask and every [`RETRY_EVERY`] after.
fn ask(items: &[Item], item: usize, step: usize, polls: u32) -> (Parity, Vec<Action>) {
    let (state, actions) = ask_check(items, item, step, polls);
    match items[item].steps.get(step) {
        Some(&Step::Retry(op, _)) if polls.is_multiple_of(RETRY_EVERY) => (
            state,
            [vec![Action::Step(Step::Host(op))], actions].concat(),
        ),
        _ => (state, actions),
    }
}

fn ask_check(items: &[Item], item: usize, step: usize, polls: u32) -> (Parity, Vec<Action>) {
    let at = |wait| Parity::At {
        item,
        step,
        wait,
        runs: 0,
        polls,
        last: String::new(),
    };
    match check_at(&items[item], step) {
        Expect::Js(expr) => {
            let serial = serial(item, step, polls);
            (
                at(Some(Wait::Check(serial))),
                vec![Action::Script(format!(
                    "parity.check({serial}, () => ({expr}))"
                ))],
            )
        }
        // The host answers at once with `Input::Checked`.
        Expect::Host(check) => (at(None), vec![Action::Check(check)]),
    }
}

/// Runs the steps of `item` from `step` that need no wait, up to the next wait.
fn advance(
    items: &[Item],
    item: usize,
    mut step: usize,
    mut actions: Vec<Action>,
) -> (Parity, Vec<Action>) {
    let Some(it) = items.get(item) else {
        actions.push(Action::Finish);
        return (Parity::Done, actions);
    };
    let at = |step, wait| Parity::At {
        item,
        step,
        wait: Some(wait),
        runs: 0,
        polls: 0,
        last: String::new(),
    };
    while let Some(&s) = it.steps.get(step) {
        match s {
            Step::Js(script) => {
                let serial = serial(item, step, 0);
                actions.push(Action::Script(format!(
                    "parity.run({serial}, () => {{ {script}; }})"
                )));
                return (at(step, Wait::Answer(serial)), actions);
            }
            Step::Choose(_) | Step::MenuHas(_) => return (at(step, Wait::Menu), actions),
            Step::Settle(n) => return (at(step, Wait::Runs(n.max(1))), actions),
            Step::Until(_) | Step::Retry(..) => {
                let (state, more) = ask(items, item, step, 0);
                actions.extend(more);
                return (state, actions);
            }
            // The host answers with `Input::Loaded`, at once when nothing had to load.
            Step::Restore => {
                actions.push(Action::Step(Step::Restore));
                return (at(step, Wait::Load), actions);
            }
            other => actions.push(Action::Step(other)),
        }
        step += 1;
    }
    if step - it.steps.len() < it.expect.len() {
        let (state, more) = ask(items, item, step, 0);
        actions.extend(more);
        (state, actions)
    } else {
        let (state, more) = finish_item(items, item, Ok(()));
        actions.extend(more);
        (state, actions)
    }
}

/// Reports `item` and starts the next one.
fn finish_item(items: &[Item], item: usize, result: Result<(), String>) -> (Parity, Vec<Action>) {
    let report = Action::Report {
        name: items[item].name,
        result,
        gap: is_gap(&items[item], Platform::current()),
    };
    advance(items, item + 1, 0, vec![report])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_item_s_keys_and_text_can_be_typed() {
        for item in ITEMS {
            for step in item.steps {
                match step {
                    Step::Keys(spec) => {
                        let chords =
                            chords(spec).unwrap_or_else(|| panic!("{}: {spec}", item.name));
                        for c in chords {
                            assert!(key_inputs(c).is_some(), "{}: {c}", item.name);
                        }
                    }
                    Step::Type(text) => {
                        assert!(typed(text).is_some(), "{}: {text}", item.name)
                    }
                    _ => {}
                }
            }
        }
    }

    #[test]
    fn item_names_are_unique() {
        let mut names: Vec<_> = ITEMS.iter().map(|i| i.name).collect();
        let n = names.len();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), n);
    }

    #[test]
    fn ctrl_shift_z_presses_modifiers_around_the_key() {
        let events = key_inputs(Chord::parse("Ctrl+Shift+Z").unwrap()).unwrap();
        let shape: Vec<_> = events
            .iter()
            .map(|e| (e.pressed, e.physical, e.text.clone(), e.modifiers.keys))
            .collect();
        let cs = ModifiersState::CONTROL | ModifiersState::SHIFT;
        assert_eq!(
            shape,
            [
                (
                    true,
                    PhysicalKey::Code(KeyCode::ControlLeft),
                    None,
                    ModifiersState::CONTROL
                ),
                (true, PhysicalKey::Code(KeyCode::ShiftLeft), None, cs),
                (true, PhysicalKey::Code(KeyCode::KeyZ), Some("Z".into()), cs),
                (false, PhysicalKey::Code(KeyCode::KeyZ), None, cs),
                (
                    false,
                    PhysicalKey::Code(KeyCode::ShiftLeft),
                    None,
                    ModifiersState::CONTROL
                ),
                (
                    false,
                    PhysicalKey::Code(KeyCode::ControlLeft),
                    None,
                    ModifiersState::empty()
                ),
            ]
        );
        assert_eq!(events[2].scancode, KeyCode::KeyZ.to_scancode());
        assert!(events[2].scancode.is_some());
    }

    #[test]
    fn typing_shifts_only_shifted_characters() {
        let chords = typed("Hi!").unwrap();
        assert_eq!(
            chords.iter().map(|c| (c.shift, c.key)).collect::<Vec<_>>(),
            [
                (true, Key::Char('h')),
                (false, Key::Char('i')),
                (true, Key::Char('1'))
            ]
        );
        assert!(typed("é").is_none());
    }

    const TWO: &[Item] = &[
        Item {
            name: "one",
            steps: &[Step::Js("x()"), Step::Click(Left, NONE, at::INP)],
            expect: &[Expect::Js("ok()"), Expect::Host(HostCheck::Tabs(1))],
            gap: false,
        },
        Item {
            name: "two",
            steps: &[Step::Click(Right, NONE, at::LINK), Step::Choose("Copy")],
            expect: &[],
            gap: true,
        },
    ];

    fn run(inputs: Vec<Input>) -> (Parity, Vec<Action>) {
        inputs
            .into_iter()
            .fold((Parity::Load, vec![]), |(s, mut all), i| {
                let (s, a) = next_in(TWO, s, i);
                all.extend(a);
                (s, all)
            })
    }

    #[test]
    fn items_run_their_steps_then_poll_their_checks() {
        let s0 = serial(0, 0, 0);
        let c0 = serial(0, 2, 0);
        let c1 = serial(0, 2, 1);
        let t = |s: u32, v: &str| format!("parity:{s}:{v}");
        let (a, b, c, d) = (t(s0, "ok"), t(c0, "fail:not yet"), t(c1, "ok"), t(c1, "ok"));
        let menu = vec![("Cut".to_string(), true), ("Copy".to_string(), true)];
        let mut inputs = vec![Input::Loaded, Input::Title(&a), Input::Title(&b)];
        inputs.extend(std::iter::repeat_n(Input::Worked, POLL_GAP as usize));
        inputs.extend([
            Input::Title(&c),
            Input::Title(&d), // a late duplicate changes nothing
            Input::Checked(Err("one tab short".into())),
        ]);
        let (s, actions) = run(inputs);
        assert!(
            matches!(
                s,
                Parity::At {
                    item: 0,
                    wait: Some(Wait::Gap(_)),
                    ..
                }
            ),
            "{s:?}"
        );
        let mut inputs = vec![];
        inputs.extend(std::iter::repeat_n(Input::Worked, POLL_GAP as usize));
        let (s, more) = inputs.into_iter().fold((s, vec![]), |(s, mut all), i| {
            let (s, a) = next_in(TWO, s, i);
            all.extend(a);
            (s, all)
        });
        assert_eq!(more, [Action::Check(HostCheck::Tabs(1))]);
        let (s, more) = next_in(TWO, s, Input::Checked(Ok(())));
        assert_eq!(
            more,
            [
                Action::Report {
                    name: "one",
                    result: Ok(()),
                    gap: false
                },
                Action::Step(Step::Click(Right, NONE, at::LINK)),
            ]
        );
        let (s, more) = next_in(TWO, s, Input::Menu(&menu));
        assert_eq!(s, Parity::Done);
        assert_eq!(
            more,
            [
                Action::Choose(1),
                Action::Report {
                    name: "two",
                    result: Ok(()),
                    gap: true
                },
                Action::Finish
            ]
        );
        assert_eq!(
            actions[..3],
            [
                Action::Script(format!("parity.run({s0}, () => {{ x(); }})")),
                Action::Step(Step::Click(Left, NONE, at::INP)),
                Action::Script(format!("parity.check({c0}, () => (ok()))")),
            ]
        );
    }

    #[test]
    fn a_missing_menu_entry_fails_the_item_and_dismisses() {
        let s = Parity::At {
            item: 1,
            step: 1,
            wait: Some(Wait::Menu),
            runs: 0,
            polls: 0,
            last: String::new(),
        };
        let menu = vec![("Cut".to_string(), true)];
        let (s, actions) = next_in(TWO, s, Input::Menu(&menu));
        assert_eq!(s, Parity::Done);
        assert_eq!(actions[0], Action::DismissMenu);
        assert!(
            matches!(&actions[1], Action::Report { name: "two", result: Err(e), .. } if e.contains("Copy"))
        );
    }

    #[test]
    fn a_silent_page_fails_after_the_bound() {
        let mut s = next_in(TWO, Parity::Load, Input::Loaded).0;
        let mut all = vec![];
        for _ in 0..MAX_WAIT_RUNS {
            let (n, a) = next_in(TWO, s, Input::Worked);
            s = n;
            all.extend(a);
        }
        assert!(
            matches!(&all[0], Action::Report { name: "one", result: Err(e), .. } if e.contains("never"))
        );
    }

    #[test]
    fn an_until_step_polls_then_goes_on() {
        const ONE: &[Item] = &[Item {
            name: "u",
            steps: &[
                Step::Until(Expect::Host(HostCheck::Tabs(1))),
                Step::MoveTo(at::INP),
            ],
            expect: &[],
            gap: false,
        }];
        let (s, a) = next_in(ONE, Parity::Load, Input::Loaded);
        assert_eq!(a, [Action::Check(HostCheck::Tabs(1))]);
        let (mut s, a) = next_in(ONE, s, Input::Checked(Err("2 tabs".into())));
        assert!(a.is_empty());
        let mut asked = vec![];
        for _ in 0..POLL_GAP {
            let (n, a) = next_in(ONE, s, Input::Worked);
            s = n;
            asked.extend(a);
        }
        assert_eq!(asked, [Action::Check(HostCheck::Tabs(1))]);
        let (s, a) = next_in(ONE, s, Input::Checked(Ok(())));
        assert_eq!(s, Parity::Done);
        assert_eq!(a[0], Action::Step(Step::MoveTo(at::INP)));
        assert!(matches!(a[1], Action::Report { result: Ok(()), .. }));
    }

    #[test]
    fn a_retry_runs_its_op_again_until_the_check_holds() {
        const ONE: &[Item] = &[Item {
            name: "r",
            steps: &[Step::Retry(
                HostOp::Neutral,
                Expect::Host(HostCheck::Tabs(1)),
            )],
            expect: &[],
            gap: false,
        }];
        let (mut s, mut all) = next_in(ONE, Parity::Load, Input::Loaded);
        for _ in 0..RETRY_EVERY {
            let (n, a) = next_in(ONE, s, Input::Checked(Err("2 tabs".into())));
            all.extend(a);
            s = n;
            for _ in 0..POLL_GAP {
                let (n, a) = next_in(ONE, s, Input::Worked);
                all.extend(a);
                s = n;
            }
        }
        let ops = all
            .iter()
            .filter(|a| **a == Action::Step(Step::Host(HostOp::Neutral)))
            .count();
        assert_eq!(ops, 2);
        let (s, _) = next_in(ONE, s, Input::Checked(Ok(())));
        assert_eq!(s, Parity::Done);
    }

    #[test]
    fn an_until_that_never_holds_fails_its_item() {
        const ONE: &[Item] = &[Item {
            name: "u",
            steps: &[Step::Until(Expect::Host(HostCheck::Tabs(1)))],
            expect: &[],
            gap: false,
        }];
        let (mut s, _) = next_in(ONE, Parity::Load, Input::Loaded);
        let mut all = vec![];
        for _ in 0..MAX_SETUP_POLLS {
            let (n, a) = next_in(ONE, s, Input::Checked(Err("2 tabs".into())));
            all.extend(a);
            s = n;
            for _ in 0..POLL_GAP {
                let (n, a) = next_in(ONE, s, Input::Worked);
                all.extend(a);
                s = n;
            }
        }
        assert!(all.iter().any(
            |a| matches!(a, Action::Report { result: Err(e), .. } if e.starts_with("setup never held"))
        ));
    }

    #[test]
    fn the_known_gaps_are_the_listed_ones() {
        let gaps: Vec<_> = ITEMS.iter().filter(|i| i.gap).map(|i| i.name).collect();
        assert!(gaps.is_empty(), "{gaps:?}");
    }

    /// On macOS no item that runs presses a Ctrl chord: each has its Mac chord, or is a gap
    /// there.
    #[test]
    fn every_mac_chord_is_rewritten() {
        let mac = Platform::Mac;
        for item in ITEMS.iter().filter(|i| !is_gap(i, mac)) {
            for step in item.steps {
                if let Step::Keys(spec) = step {
                    for c in chords_on(spec, mac).unwrap() {
                        assert!(!c.ctrl, "{}: {c}", item.name);
                        assert!(key_inputs(c).is_some(), "{}: {c}", item.name);
                    }
                }
            }
        }
        assert_eq!(
            chords_on("Alt+Left Ctrl+BackSpace", mac),
            chords_on("Cmd+[ Alt+Backspace", Platform::Linux)
        );
        assert_eq!(
            chords_on("Ctrl+A", Platform::Linux),
            Chord::parse("Ctrl+A").ok().map(|c| vec![c])
        );
        // every rewrite parses, and names a chord the script uses
        for (linux, mac) in MAC_CHORDS {
            assert!(chords_on(mac, Platform::Linux).is_some(), "{mac}");
            assert!(
                ITEMS.iter().any(|i| i.steps.iter().any(
                    |s| matches!(s, Step::Keys(k) if k.split_whitespace().any(|t| t == *linux))
                )),
                "{linux} is not in the script"
            );
        }
    }

    #[test]
    fn a_mac_click_takes_cmd_for_ctrl() {
        let cmd = Mods { cmd: true, ..NONE };
        assert_eq!(CTRL.for_click(Platform::Mac), cmd);
        assert_eq!(CTRL.for_click(Platform::Linux), CTRL);
        assert_eq!(SHIFT.for_click(Platform::Mac), SHIFT);
    }

    #[test]
    fn platform_gaps_name_items() {
        for (_, name) in PLATFORM_GAPS {
            assert!(ITEMS.iter().any(|i| i.name == *name), "{name}");
        }
        let primary = ITEMS
            .iter()
            .find(|i| i.name == "B.primary-selection")
            .unwrap();
        assert!(is_gap(primary, Platform::Mac));
        assert!(!is_gap(primary, Platform::Linux));
    }

    #[test]
    fn menu_order_is_checked() {
        let menu: Vec<(String, bool)> = ["A", "B", "C"]
            .iter()
            .map(|s| (s.to_string(), true))
            .collect();
        assert!(menu_lists(&menu, &["A", "C"]).is_ok());
        assert!(menu_lists(&menu, &["C", "A"]).is_err());
    }

    #[test]
    fn the_page_has_what_the_table_points_at() {
        let page = include_str!("../assets/parity.html");
        for id in [
            "inp", "inp2", "ta", "ce", "para", "link", "img", "video", "drag", "drop", "filezone",
            "copybtn", "dl", "blank", "mail", "open", "tip",
        ] {
            assert!(page.contains(&format!("id=\"{id}\"")), "{id}");
        }
    }
}
