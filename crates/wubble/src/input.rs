//! Host side of web-content input: which keys go to the page, the state
//! every event carries (modifiers, held buttons, a clock), what the view was
//! told about focus, and where the pointer is in the window.
//!
//! The keyboard comes from raw winit events (`on_winit_window_event`):
//! Slint's own key events carry only text, so they lose the physical key,
//! its location, keys without text and the real Ctrl/Cmd on Apple. The
//! mouse comes from the Slint `TouchArea` over the web frame, which gives
//! hit-testing against the shell, view-relative logical coordinates, the
//! grab and leave for free. See TODO.md, "Web input".

use std::time::{Duration, Instant};

use slint::platform::PointerEventButton;
use slint::winit_030::winit::event::{KeyEvent, MouseButton as WinitButton};
use slint::winit_030::winit::keyboard::{Key, ModifiersState, NamedKey};
use slint::winit_030::winit::platform::modifier_supplement::KeyEventExtModifierSupplement;
use slint::winit_030::winit::platform::scancode::PhysicalKeyExtScancode;
use wubble_core::action::Action;
use wubble_core::keys::{self, Chord, Key as ChordKey, Keymap, Platform, Route};
use wubble_engine::input::{Buttons, KeyId, KeyInput, Modifiers, MouseButton};
use wubble_ui::AppWindow;

/// Where keyboard focus is, as far as routing a key cares.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyFocus {
    /// The web content.
    Page,
    /// A shell text field (the open command bar): it owns editing keys.
    Field,
    /// The start page's search capsule, the bar in the page: a field, but the focus-address keys
    /// still run their action there (they select its text), as the page has no overlay bar.
    Address,
    /// The shell, with nothing that takes text.
    Shell,
}

/// The page that gets keys: the peek after a press in it (`pressed`, while it is still the
/// window's `peek`), else the model's `shown` page.
pub fn key_page(
    pressed: Option<wubble_core::tab::TabId>,
    peek: Option<wubble_core::tab::TabId>,
    shown: Option<wubble_core::tab::TabId>,
) -> Option<wubble_core::tab::TabId> {
    pressed.filter(|&t| Some(t) == peek).or(shown)
}

/// Who takes a pressed Esc before the keymap's routing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EscTo {
    /// The page leaves fullscreen.
    Fullscreen,
    /// The window's peek closes.
    Peek,
    /// The key goes on as any other: to a shell field, sheet or menu holding the focus, or the
    /// page.
    Route,
}

/// Fullscreen first; the peek only when the web content has the focus (no shell overlay took
/// it), as the last taker before the page.
pub fn esc_to(fullscreen: bool, web_focused: bool, peek: bool) -> EscTo {
    if fullscreen {
        EscTo::Fullscreen
    } else if peek && web_focused {
        EscTo::Peek
    } else {
        EscTo::Route
    }
}

/// What to do with a key event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyRoute {
    /// To the page only.
    Page,
    /// To the page, and on to Slint (modifier keys, so Slint's own
    /// tracking stays right).
    PageAndShell,
    /// To Slint only.
    Shell,
    /// Run the action; nobody else sees the key.
    Perform(Action),
    /// To the page; the action runs only if the page leaves the key alone
    /// (see [`PendingKeys`]).
    PageFirst(Action),
}

/// Whether `key` is a modifier on its own.
pub fn is_modifier(key: &Key) -> bool {
    matches!(
        key,
        Key::Named(
            NamedKey::Shift
                | NamedKey::Control
                | NamedKey::Alt
                | NamedKey::AltGraph
                | NamedKey::Super
                | NamedKey::Meta
                | NamedKey::Hyper
        )
    )
}

/// In page fullscreen only `Fullscreen` stays a shell key; every other
/// binding becomes page-first, so a fullscreen game or video player gets
/// its keys (TODO.md Decided "Key routing"; Chromium's
/// `IsReservedCommandOrKey` drops its reserved set in fullscreen too).
pub fn page_fullscreen_binding(binding: Option<(Action, Route)>) -> Option<(Action, Route)> {
    binding.map(|(action, route)| match (action, route) {
        (Action::Fullscreen, _) => (action, route),
        (_, Route::Shell) => (action, Route::PageFirst),
        _ => (action, route),
    })
}

/// Where a key press goes. `binding` is what the keymap binds its chord to.
///
/// - `Route::Shell` bindings always run and never reach the page or Slint.
/// - `Route::PageFirst` means "the page first, the action if the page does
///   not handle it". With the page focused the key goes to the page only; the
///   action waits for the engine's `WebEvent::KeyUnhandled` ([`PendingKeys`]).
///   In a shell text field the field keeps it (Escape closes the bar, it does
///   not stop the page); with nothing focused the action runs, except an edit
///   command ([`Action::is_edit`]): `Shell` focus covers the find bar, dialog
///   and settings fields, which own copy, paste and undo.
/// - Unbound keys go where the focus is; modifier keys also reach Slint.
pub fn key_route(focus: KeyFocus, binding: Option<(Action, Route)>, modifier: bool) -> KeyRoute {
    match (focus, binding, modifier) {
        (KeyFocus::Page, _, true) => KeyRoute::PageAndShell,
        (_, _, true) => KeyRoute::Shell,
        (_, Some((action, Route::Shell)), false) => KeyRoute::Perform(action),
        (KeyFocus::Address, Some((Action::FocusAddress, _)), false) => {
            KeyRoute::Perform(Action::FocusAddress)
        }
        (KeyFocus::Page, Some((action, Route::PageFirst)), false) => KeyRoute::PageFirst(action),
        (KeyFocus::Field | KeyFocus::Address, Some((_, Route::PageFirst)), false) => {
            KeyRoute::Shell
        }
        (KeyFocus::Shell, Some((action, Route::PageFirst)), false) if action.is_edit() => {
            KeyRoute::Shell
        }
        (KeyFocus::Shell, Some((action, Route::PageFirst)), false) => KeyRoute::Perform(action),
        (KeyFocus::Page, None, false) => KeyRoute::Page,
        (KeyFocus::Field | KeyFocus::Address | KeyFocus::Shell, None, false) => KeyRoute::Shell,
    }
}

/// The keymap's chord for a key (without modifiers applied), the held
/// modifiers and the text it typed. Super is `cmd`: on macOS that is Cmd,
/// elsewhere the keymap binds nothing to it. Keys the keymap cannot name are
/// `None`, and so is Ctrl+Alt typing a character other than the key's own:
/// that is AltGr (Windows reports it as Ctrl+Alt), as Chromium treats it.
pub fn chord_of(unmodified: &Key, mods: ModifiersState, text: Option<&str>) -> Option<Chord> {
    // Text other than the key's own: on Linux winit's `text` ignores Ctrl, so
    // a real Ctrl+Alt+Z types "z", the key itself, and stays a chord.
    let own = match unmodified {
        Key::Character(key) => text.is_some_and(|t| t.to_lowercase() == key.to_lowercase()),
        _ => false,
    };
    let printable = text.is_some_and(|t| !t.is_empty() && !t.chars().any(char::is_control));
    if mods.control_key() && mods.alt_key() && printable && !own {
        return None;
    }
    let key = match unmodified {
        Key::Character(key) => {
            let mut chars = key.chars();
            match (chars.next(), chars.next()) {
                (Some(' '), None) => ChordKey::Space,
                (Some(c), None) => ChordKey::Char(shifted_symbol(c, mods, text).unwrap_or(c)),
                _ => return None,
            }
        }
        Key::Named(named) => match named {
            NamedKey::Tab => ChordKey::Tab,
            NamedKey::Enter => ChordKey::Enter,
            NamedKey::Escape => ChordKey::Escape,
            NamedKey::Space => ChordKey::Space,
            NamedKey::Backspace => ChordKey::Backspace,
            NamedKey::Delete => ChordKey::Delete,
            NamedKey::ArrowLeft => ChordKey::Left,
            NamedKey::ArrowRight => ChordKey::Right,
            NamedKey::ArrowUp => ChordKey::Up,
            NamedKey::ArrowDown => ChordKey::Down,
            NamedKey::Home => ChordKey::Home,
            NamedKey::End => ChordKey::End,
            NamedKey::PageUp => ChordKey::PageUp,
            NamedKey::PageDown => ChordKey::PageDown,
            other => ChordKey::F(function_key(*other)?),
        },
        _ => return None,
    };
    Some(
        Chord {
            ctrl: mods.control_key(),
            alt: mods.alt_key(),
            shift: mods.shift_key(),
            cmd: mods.super_key(),
            key,
        }
        .normalize(),
    )
}

/// The symbol Shift makes of a symbol key, which the keymap names instead of the key:
/// Ctrl+Shift+\\ on a US layout is `Ctrl+|`, never `Ctrl+\\` with Shift dropped (QA CRIT-04,
/// which split instead of unsplitting). Letters and digits keep their key and Shift, so
/// Ctrl+Shift+1 stays itself. winit's text ignores Ctrl, so it is the shifted character; without
/// one (no text) the key stands, as before.
fn shifted_symbol(key: char, mods: ModifiersState, text: Option<&str>) -> Option<char> {
    if !mods.shift_key() || key.is_alphanumeric() {
        return None;
    }
    let mut chars = text?.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) if !c.is_alphanumeric() && !c.is_control() && !c.is_whitespace() => Some(c),
        _ => None,
    }
}

/// F1 to F24.
fn function_key(key: NamedKey) -> Option<u8> {
    const KEYS: [NamedKey; 24] = [
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
        NamedKey::F11,
        NamedKey::F12,
        NamedKey::F13,
        NamedKey::F14,
        NamedKey::F15,
        NamedKey::F16,
        NamedKey::F17,
        NamedKey::F18,
        NamedKey::F19,
        NamedKey::F20,
        NamedKey::F21,
        NamedKey::F22,
        NamedKey::F23,
        NamedKey::F24,
    ];
    let i = KEYS.iter().position(|&k| k == key)?;
    u8::try_from(i + 1).ok()
}

/// Page-first actions whose keys went to the page, oldest first, each with
/// the key-down it waits on. The page handles keys in the order they were
/// sent and reports only those it leaves alone, so a report settles its own
/// entry and every older one (those keys were handled).
#[derive(Debug, Default)]
pub struct PendingKeys(Vec<(KeyId, Action)>);

impl PendingKeys {
    /// Keys a page may hold back at once; older entries are dropped.
    const MAX: usize = 16;

    pub fn push(&mut self, key: KeyId, action: Action) {
        if self.0.len() == Self::MAX {
            self.0.remove(0);
        }
        self.0.push((key, action));
    }

    /// The page left `key` alone: the action to run now, if one waits on it.
    pub fn unhandled(&mut self, key: KeyId) -> Option<Action> {
        let i = self.0.iter().position(|(k, _)| *k == key)?;
        let (_, action) = self.0[i];
        self.0.drain(..=i);
        Some(action)
    }

    /// Another view is visible: nothing it reports was sent for these.
    pub fn clear(&mut self) {
        self.0.clear();
    }
}

/// The platform's default keymap with the stored overrides applied. Each
/// stored entry is a `keys::overrides_to_json` list; one that does not parse
/// is skipped with a log line, so a bad setting never blocks startup.
pub fn keymap(platform: Platform, stored: &[String]) -> Keymap {
    let (overrides, bad) = keys::overrides_from_entries(stored);
    if bad > 0 {
        eprintln!("wubble: ignoring {bad} keybinding override entries that do not parse");
    }
    Keymap::defaults(platform).with_overrides(&overrides)
}

/// The CEF buttons; Back/Forward and others are not sent to the page.
pub fn mouse_button(button: PointerEventButton) -> Option<MouseButton> {
    match button {
        PointerEventButton::Left => Some(MouseButton::Left),
        PointerEventButton::Middle => Some(MouseButton::Middle),
        PointerEventButton::Right => Some(MouseButton::Right),
        _ => None,
    }
}

/// The mouse's side buttons are navigation, over the page and the shell
/// alike. Only the press acts.
pub fn button_action(button: WinitButton) -> Option<Action> {
    match button {
        WinitButton::Back => Some(Action::Back),
        WinitButton::Forward => Some(Action::Forward),
        _ => None,
    }
}

/// Whether `p` (logical px) is over the page area (its frames cover the scene there).
pub fn over_content(window: &AppWindow, p: slint::LogicalPosition) -> bool {
    let (x, y) = (window.get_content_x(), window.get_content_y());
    p.x >= x
        && p.y >= y
        && p.x < x + window.get_content_width()
        && p.y < y + window.get_content_height()
}

/// The host-wide "a wubble window has focus" after a window's focus event, or `None` to
/// ignore it. A gain always counts; a loss only from the model's focused window, since some
/// platforms send the old window's loss after the new window's gain.
pub fn window_focus(from_focused_window: bool, gained: bool) -> Option<bool> {
    (gained || from_focused_window).then_some(gained)
}

/// Whether a press of `button` sends a wave through the backdrop: the left button only, not
/// the right, middle or side buttons.
pub fn taps_backdrop(button: WinitButton) -> bool {
    button == WinitButton::Left
}

/// The releases to send when `button` is released while `held` are down:
/// `button`, then every other held button. Slint's `TouchArea` ends its
/// pointer grab on any release (i-slint-core 1.19 `items/input_items.rs`,
/// `MouseEvent::Released`), so the other buttons' releases could land
/// outside the web content and CEF would never hear them.
pub fn releases(held: Buttons, button: MouseButton) -> Vec<MouseButton> {
    let others = [MouseButton::Left, MouseButton::Middle, MouseButton::Right]
        .into_iter()
        .filter(|&other| other != button && held.is_down(other));
    std::iter::once(button).chain(others).collect()
}

/// The input state the host owns.
pub struct InputState {
    /// From winit's `ModifiersChanged`, which always reaches Slint too.
    pub keys: ModifiersState,
    pub buttons: Buttons,
    /// Caps / Num Lock, where the platform tells (`crate::wl` on Wayland).
    pub locks: wubble_engine::input::Locks,
    epoch: Instant,
    pub window_focused: bool,
    /// What the view was last told about keyboard focus.
    view_focused: bool,
    /// The pointer in window logical pixels, while it is over the window.
    pub pointer: Option<slint::LogicalPosition>,
}

impl Default for InputState {
    fn default() -> Self {
        Self {
            keys: ModifiersState::empty(),
            buttons: Buttons::default(),
            locks: Default::default(),
            epoch: Instant::now(),
            window_focused: false,
            view_focused: false,
            pointer: None,
        }
    }
}

impl InputState {
    pub fn modifiers(&self) -> Modifiers {
        Modifiers {
            keys: self.keys,
            buttons: self.buttons,
            locks: self.locks,
        }
    }

    /// The keyboard focus to tell the view, if it differs from what it was
    /// last told. Without an open view nothing is recorded, so the view
    /// hears the focus once it opens.
    pub fn focus_change(&mut self, focused: bool, view_open: bool) -> Option<bool> {
        (view_open && focused != self.view_focused).then(|| {
            self.view_focused = focused;
            focused
        })
    }

    /// Another view is now the visible one: it has been told nothing yet.
    pub fn forget_view_focus(&mut self) {
        self.view_focused = false;
    }

    /// A monotonic time for double-click detection.
    pub fn now(&self) -> Duration {
        self.epoch.elapsed()
    }

    pub fn key_input(&self, event: &KeyEvent) -> KeyInput {
        KeyInput {
            pressed: event.state.is_pressed(),
            repeat: event.repeat,
            physical: event.physical_key,
            scancode: event.physical_key.to_scancode(),
            unmodified: event.key_without_modifiers(),
            text: event.text.as_ref().map(|text| text.to_string()),
            location: event.location,
            modifiers: self.modifiers(),
        }
    }
}

#[cfg(test)]
mod tests {

    #[test]
    fn keys_go_to_the_peek_after_a_press_in_it() {
        use wubble_core::tab::TabId;
        let (opener, peek, other) = (Some(TabId(1)), Some(TabId(2)), Some(TabId(3)));
        assert_eq!(
            key_page(None, peek, opener),
            opener,
            "the opener until a press"
        );
        assert_eq!(key_page(peek, peek, opener), peek);
        assert_eq!(key_page(peek, None, opener), opener, "the peek closed");
        assert_eq!(
            key_page(peek, other, opener),
            opener,
            "another peek needs its own press"
        );
    }

    #[test]
    fn esc_closes_a_peek_only_after_fullscreen_and_shell_overlays() {
        assert_eq!(esc_to(true, true, true), EscTo::Fullscreen);
        assert_eq!(esc_to(false, true, true), EscTo::Peek);
        // the command bar, a sheet or a menu holds the focus: it takes the Esc
        assert_eq!(esc_to(false, false, true), EscTo::Route);
        assert_eq!(esc_to(false, true, false), EscTo::Route);
    }
    use super::*;

    fn ch(c: &str) -> Key {
        Key::Character(c.into())
    }

    fn ctrl() -> ModifiersState {
        ModifiersState::CONTROL
    }

    fn chord(s: &str) -> Option<Chord> {
        Some(Chord::parse(s).unwrap())
    }

    #[test]
    fn page_fullscreen_keeps_only_fullscreen_a_shell_key() {
        assert_eq!(
            page_fullscreen_binding(Some((Action::CloseTab, Route::Shell))),
            Some((Action::CloseTab, Route::PageFirst))
        );
        assert_eq!(
            page_fullscreen_binding(Some((Action::Fullscreen, Route::Shell))),
            Some((Action::Fullscreen, Route::Shell))
        );
        assert_eq!(
            page_fullscreen_binding(Some((Action::Find, Route::PageFirst))),
            Some((Action::Find, Route::PageFirst))
        );
        assert_eq!(page_fullscreen_binding(None), None);
    }

    #[test]
    fn chords_from_winit_keys() {
        let cases = [
            (ch("t"), ctrl(), chord("Ctrl+T")),
            (
                ch("T"),
                ctrl() | ModifiersState::SHIFT,
                chord("Ctrl+Shift+T"),
            ),
            (ch("1"), ModifiersState::ALT, chord("Alt+1")),
            (ch("="), ctrl(), chord("Ctrl+=")),
            (ch(" "), ModifiersState::empty(), chord("Space")),
            (ch("q"), ModifiersState::SUPER, chord("Cmd+Q")),
            (
                Key::Named(NamedKey::F5),
                ModifiersState::empty(),
                chord("F5"),
            ),
            (
                Key::Named(NamedKey::F12),
                ModifiersState::SHIFT,
                chord("Shift+F12"),
            ),
            (Key::Named(NamedKey::Tab), ctrl(), chord("Ctrl+Tab")),
            (
                Key::Named(NamedKey::PageDown),
                ctrl(),
                chord("Ctrl+PageDown"),
            ),
            (
                Key::Named(NamedKey::ArrowLeft),
                ModifiersState::ALT,
                chord("Alt+Left"),
            ),
            (
                Key::Named(NamedKey::Escape),
                ModifiersState::empty(),
                chord("Escape"),
            ),
            (Key::Named(NamedKey::CapsLock), ctrl(), None),
            (ch("ab"), ctrl(), None),
        ];
        for (key, mods, want) in cases {
            assert_eq!(chord_of(&key, mods, None), want, "{key:?} {mods:?}");
        }
    }

    #[test]
    fn default_chords_reach_their_actions() {
        let map = keymap(Platform::Linux, &[]);
        let action = |key: Key, mods| chord_of(&key, mods, None).and_then(|c| map.action_for(c));
        assert_eq!(
            action(ch("t"), ctrl()),
            Some((Action::NewTab, Route::Shell))
        );
        assert_eq!(
            action(ch("l"), ctrl()),
            Some((Action::FocusAddress, Route::PageFirst))
        );
        assert_eq!(action(ch("a"), ModifiersState::empty()), None);
    }

    /// The chord the host makes of a key press the parity smoke builds for `chord` (US layout).
    fn pressed(chord: Chord) -> Option<Chord> {
        let keys = crate::parity::key_inputs(chord)?;
        let key = keys
            .iter()
            .find(|k| k.pressed && !is_modifier(&k.unmodified))?;
        chord_of(&key.unmodified, key.modifiers.keys, key.text.as_deref())
    }

    /// Every default row, on every platform, is reached by the keys a US layout presses for it
    /// and by no other: a row naming a shifted symbol (`Mod+|`, `Mod+Plus`) is reached with
    /// Shift held on its key, and Shift on a symbol key runs the shifted symbol's row (or
    /// nothing), never the key's own (QA CRIT-04: Ctrl+Shift+\\ split instead of unsplitting).
    #[test]
    fn every_default_chord_is_reached_by_its_own_keys_only() {
        for platform in [Platform::Linux, Platform::Windows, Platform::Mac] {
            let map = Keymap::defaults(platform);
            let run = |c: Chord| map.action_for(c).map(|(a, _)| a);
            for (chord, action, _) in keys::default_rows(platform) {
                if let Some(got) = pressed(chord) {
                    // The first row of a chord wins (a duplicate is a conflict, tested apart).
                    assert_eq!(run(got), run(chord), "{platform:?} {chord} ({action:?})");
                }
                let ChordKey::Char(c) = chord.key else {
                    continue;
                };
                if c.is_alphanumeric() || chord.shift {
                    continue;
                }
                let with_shift = Chord {
                    shift: true,
                    ..chord
                };
                // A shifted symbol's own row (`Mod+Plus`) holds Shift already.
                let Some(got) = pressed(with_shift).filter(|&g| Some(g) != pressed(chord)) else {
                    continue;
                };
                let ChordKey::Char(typed) = got.key else {
                    panic!("{got}");
                };
                let shifted_row = Chord {
                    key: ChordKey::Char(typed),
                    ..chord
                };
                assert_eq!(run(got), run(shifted_row), "{platform:?} Shift+{chord}");
            }
        }
    }

    #[test]
    fn unsplit_and_split_chords_differ() {
        let map = keymap(Platform::Linux, &[]);
        let shift = ctrl() | ModifiersState::SHIFT;
        let action = |key: &str, mods, text: &str| {
            chord_of(&ch(key), mods, Some(text)).and_then(|c| map.action_for(c))
        };
        assert_eq!(
            action("\\", shift, "|"),
            Some((Action::Unsplit, Route::Shell))
        );
        assert_eq!(
            action("\\", ctrl(), "\\"),
            Some((Action::SplitNext, Route::Shell))
        );
        // Without text (a platform that gives none with Ctrl) the key stands, as before.
        assert_eq!(chord_of(&ch("\\"), shift, None), chord("Ctrl+\\"));
        // Ctrl+Shift+= types '+': still zoom in.
        assert_eq!(
            action("=", shift, "+"),
            Some((Action::ZoomIn, Route::PageFirst))
        );
    }

    #[test]
    fn stored_overrides_apply_and_bad_ones_are_skipped() {
        let stored = vec![
            r#"[["Ctrl+B","new-tab"],["Ctrl+T",null]]"#.to_string(),
            "not json".to_string(),
        ];
        let map = keymap(Platform::Linux, &stored);
        assert_eq!(
            map.action_for(Chord::parse("Ctrl+B").unwrap()),
            Some((Action::NewTab, Route::Shell))
        );
        assert_eq!(map.action_for(Chord::parse("Ctrl+T").unwrap()), None);
    }

    #[test]
    fn key_routes() {
        use KeyFocus::{Address, Field, Page, Shell};
        let shell = Some((Action::NewTab, Route::Shell));
        let page_first = Some((Action::Reload, Route::PageFirst));
        let edit = Some((Action::Paste, Route::PageFirst));
        let focus_address = Some((Action::FocusAddress, Route::PageFirst));
        let stop = Some((Action::Stop, Route::PageFirst));
        let cases = [
            // Shell-reserved chords run whatever has focus.
            (Page, shell, KeyRoute::Perform(Action::NewTab)),
            (Field, shell, KeyRoute::Perform(Action::NewTab)),
            (Shell, shell, KeyRoute::Perform(Action::NewTab)),
            // Page-first chords: the page (the action only if it leaves the
            // key alone), the field alone, or the action alone.
            (Page, page_first, KeyRoute::PageFirst(Action::Reload)),
            (Field, page_first, KeyRoute::Shell),
            (Shell, page_first, KeyRoute::Perform(Action::Reload)),
            // Edit commands: the page's, or the focused shell field's (find bar, dialog).
            (Page, edit, KeyRoute::PageFirst(Action::Paste)),
            (Field, edit, KeyRoute::Shell),
            (Shell, edit, KeyRoute::Shell),
            // Unbound keys go where the focus is.
            (Page, None, KeyRoute::Page),
            (Field, None, KeyRoute::Shell),
            (Shell, None, KeyRoute::Shell),
            // The start page's capsule: a field (Escape, arrows and edits are its own), but
            // Ctrl+L still runs there and selects its text; the overlay's field keeps it.
            (Address, shell, KeyRoute::Perform(Action::NewTab)),
            (
                Address,
                focus_address,
                KeyRoute::Perform(Action::FocusAddress),
            ),
            (Field, focus_address, KeyRoute::Shell),
            (Address, stop, KeyRoute::Shell),
            (Address, edit, KeyRoute::Shell),
            (Address, None, KeyRoute::Shell),
        ];
        for (focus, binding, want) in cases {
            assert_eq!(
                key_route(focus, binding, false),
                want,
                "{focus:?} {binding:?}"
            );
        }
    }

    #[test]
    fn altgr_text_is_no_ctrl_alt_chord() {
        let ctrl_alt = ModifiersState::CONTROL | ModifiersState::ALT;
        // Polish AltGr+Z types 'ż'; Windows reports Ctrl+Alt.
        assert_eq!(chord_of(&ch("z"), ctrl_alt, Some("ż")), None);
        // A real Ctrl+Alt chord types nothing (or a control character), or,
        // on Linux, where winit's text ignores Ctrl, the key itself.
        assert_eq!(chord_of(&ch("z"), ctrl_alt, None), chord("Ctrl+Alt+Z"));
        assert_eq!(chord_of(&ch("z"), ctrl_alt, Some("z")), chord("Ctrl+Alt+Z"));
        assert_eq!(
            chord_of(&ch("z"), ctrl_alt | ModifiersState::SHIFT, Some("Z")),
            chord("Ctrl+Alt+Shift+Z")
        );
        assert_eq!(
            chord_of(&ch("z"), ctrl_alt, Some("\u{1a}")),
            chord("Ctrl+Alt+Z")
        );
        // Ctrl alone with text, or Alt alone, still make chords.
        assert_eq!(chord_of(&ch("t"), ctrl(), Some("t")), chord("Ctrl+T"));
        assert_eq!(
            chord_of(&ch("1"), ModifiersState::ALT, Some("¡")),
            chord("Alt+1")
        );
    }

    fn key_down(c: &str, mods: ModifiersState) -> KeyId {
        use slint::winit_030::winit::keyboard::{KeyCode, KeyLocation, PhysicalKey};
        let key = KeyInput {
            pressed: true,
            repeat: false,
            physical: PhysicalKey::Code(KeyCode::KeyR),
            scancode: None,
            unmodified: ch(c),
            text: None,
            location: KeyLocation::Standard,
            modifiers: Modifiers {
                keys: mods,
                buttons: Buttons::default(),
                locks: Default::default(),
            },
        };
        KeyId::of(&key).unwrap()
    }

    #[test]
    fn a_page_first_action_runs_only_when_the_page_leaves_its_key() {
        let ctrl_r = key_down("r", ctrl());
        let ctrl_l = key_down("l", ctrl());
        let mut pending = PendingKeys::default();
        // The page handled Ctrl+R (a web terminal): nothing reports it.
        pending.push(ctrl_r, Action::Reload);
        // The page leaves Ctrl+L alone: the bar opens, and the Ctrl+R that
        // was handled before it is settled too.
        pending.push(ctrl_l, Action::FocusAddress);
        assert_eq!(pending.unhandled(ctrl_l), Some(Action::FocusAddress));
        assert_eq!(pending.unhandled(ctrl_r), None);
        // A plain R the page leaves alone runs nothing.
        pending.push(ctrl_r, Action::Reload);
        assert_eq!(
            pending.unhandled(key_down("r", ModifiersState::empty())),
            None
        );
        assert_eq!(pending.unhandled(ctrl_r), Some(Action::Reload));
        // A new visible view reports nothing for the old one's keys.
        pending.push(ctrl_r, Action::Reload);
        pending.clear();
        assert_eq!(pending.unhandled(ctrl_r), None);
    }

    #[test]
    fn pending_keys_stay_bounded() {
        let ctrl_r = key_down("r", ctrl());
        let ctrl_l = key_down("l", ctrl());
        let mut pending = PendingKeys::default();
        pending.push(ctrl_l, Action::FocusAddress);
        for _ in 0..PendingKeys::MAX {
            pending.push(ctrl_r, Action::Reload);
        }
        assert_eq!(pending.0.len(), PendingKeys::MAX);
        assert_eq!(pending.unhandled(ctrl_l), None, "the oldest was dropped");
    }

    #[test]
    fn modifier_keys_reach_both() {
        for named in [
            NamedKey::Shift,
            NamedKey::Control,
            NamedKey::Alt,
            NamedKey::Super,
        ] {
            let key = Key::Named(named);
            assert!(is_modifier(&key), "{named:?}");
            assert_eq!(
                key_route(KeyFocus::Page, None, is_modifier(&key)),
                KeyRoute::PageAndShell
            );
            assert_eq!(
                key_route(KeyFocus::Shell, None, is_modifier(&key)),
                KeyRoute::Shell
            );
        }
        assert!(!is_modifier(&ch("a")));
    }

    #[test]
    fn only_cef_buttons_are_forwarded() {
        assert_eq!(
            mouse_button(PointerEventButton::Left),
            Some(MouseButton::Left)
        );
        assert_eq!(
            mouse_button(PointerEventButton::Middle),
            Some(MouseButton::Middle)
        );
        assert_eq!(
            mouse_button(PointerEventButton::Right),
            Some(MouseButton::Right)
        );
        for other in [
            PointerEventButton::Back,
            PointerEventButton::Forward,
            PointerEventButton::Other,
        ] {
            assert_eq!(mouse_button(other), None);
        }
    }

    #[test]
    fn side_buttons_navigate() {
        assert_eq!(button_action(WinitButton::Back), Some(Action::Back));
        assert_eq!(button_action(WinitButton::Forward), Some(Action::Forward));
        for other in [WinitButton::Left, WinitButton::Middle, WinitButton::Right] {
            assert_eq!(button_action(other), None);
        }
    }

    #[test]
    fn focus_before_the_view_opens_reaches_it_once_open() {
        let mut input = InputState::default();
        assert_eq!(input.focus_change(true, false), None);
        assert_eq!(input.focus_change(true, true), Some(true));
        assert_eq!(input.focus_change(true, true), None);
        assert_eq!(input.focus_change(false, true), Some(false));
    }

    #[test]
    fn a_newly_visible_view_hears_the_focus_again() {
        let mut input = InputState::default();
        assert_eq!(input.focus_change(true, true), Some(true));
        input.forget_view_focus();
        assert_eq!(input.focus_change(true, true), Some(true));
    }

    #[test]
    fn a_release_releases_every_held_button() {
        let held = Buttons {
            left: true,
            middle: false,
            right: true,
        };
        assert_eq!(
            releases(held, MouseButton::Right),
            vec![MouseButton::Right, MouseButton::Left]
        );
        assert_eq!(
            releases(Buttons::default(), MouseButton::Left),
            vec![MouseButton::Left]
        );
    }

    #[test]
    fn only_the_left_button_taps_the_backdrop() {
        let cases = [
            (WinitButton::Left, true),
            (WinitButton::Right, false),
            (WinitButton::Middle, false),
            (WinitButton::Back, false),
            (WinitButton::Forward, false),
            (WinitButton::Other(7), false),
        ];
        for (button, want) in cases {
            assert_eq!(taps_backdrop(button), want, "{button:?}");
        }
        // The side buttons still navigate.
        assert_eq!(button_action(WinitButton::Back), Some(Action::Back));
    }

    #[test]
    fn a_late_loss_from_another_window_keeps_focus() {
        // W2 gains focus first; W1's loss arrives after and must not unfocus W2's page
        assert_eq!(window_focus(false, true), Some(true), "W2 gains");
        assert_eq!(
            window_focus(false, false),
            None,
            "W1's late loss is ignored"
        );
        // the focused window losing focus to another app does count
        assert_eq!(window_focus(true, false), Some(false));
        assert_eq!(window_focus(true, true), Some(true));
    }
}
