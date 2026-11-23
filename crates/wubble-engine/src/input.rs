//! Mouse, wheel and keyboard from the host, translated into CEF events.
//!
//! One pure translator per event kind; [`crate::view::View`] sends what they
//! return to the view's `BrowserHost`. The host speaks winit's key vocabulary
//! (physical key, unmodified logical key, produced text) and Slint's view-
//! relative logical pixels, which equal CEF's DIP coordinates.
//!
//! Ported from chromiumembedded/cef `tests/cefclient/browser/browser_window_osr_gtk.cc`
//! (branch 7339: `KeyEvent`, `GetCefStateModifiers`, `KeyboardCodeFromXKeysym`,
//! `GetWindowsKeyCodeWithoutLocation`, `GetControlCharacter`, `ScrollEvent`)
//! and `osr_window_win.cc` (click counting, Shift+wheel), BSD-3-Clause, and
//! Chromium `ui/events/keycodes/dom_us_layout_data.h` (`kDomCodeToKeyboardCodeMap`)
//! with values from `keyboard_codes_posix.h`, BSD-3-Clause. Flag constants and
//! the builder shape follow `vendor/welding/src/cef_input.rs` (MPL-2.0).
//! See THIRD_PARTY_NOTICES.md.
//!
//! Caps Lock / Num Lock state (`EVENTFLAG_CAPS_LOCK_ON`, `NUM_LOCK_ON`) is not
//! winit's: the host reads it where it can ([`Locks`]). What winit cannot tell
//! us and is therefore lost: AltGr as a modifier
//! (`EVENTFLAG_ALTGR_DOWN`; the text it produces does arrive), and the
//! layout's remapping of keys without text (they are identified by their
//! physical position, so a Caps/Ctrl swap reports the physical key, and a
//! keypad key with Num Lock off reports `VKEY_NUMPAD*`). IME composition is
//! out of scope.

use std::time::Duration;

use crate::app::{Current, Platform};
use winit::keyboard::{Key, KeyCode, KeyLocation, ModifiersState, PhysicalKey};
use wubble_core::touch::TouchKind;

/// `cef_event_flags_t` (cef-dll-sys 156).
mod flag {
    pub const CAPS_LOCK: u32 = 1 << 0;
    pub const SHIFT: u32 = 1 << 1;
    pub const CONTROL: u32 = 1 << 2;
    pub const ALT: u32 = 1 << 3;
    pub const LEFT_MOUSE: u32 = 1 << 4;
    pub const MIDDLE_MOUSE: u32 = 1 << 5;
    pub const RIGHT_MOUSE: u32 = 1 << 6;
    pub const COMMAND: u32 = 1 << 7;
    pub const NUM_LOCK: u32 = 1 << 8;
    pub const KEY_PAD: u32 = 1 << 9;
    pub const LEFT: u32 = 1 << 10;
    pub const RIGHT: u32 = 1 << 11;
    pub const REPEAT: u32 = 1 << 13;
    pub const PRECISION_SCROLLING_DELTA: u32 = 1 << 14;
}

/// Mouse buttons held down.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Buttons {
    pub left: bool,
    pub middle: bool,
    pub right: bool,
}

impl Buttons {
    pub fn with(mut self, button: MouseButton, down: bool) -> Self {
        match button {
            MouseButton::Left => self.left = down,
            MouseButton::Middle => self.middle = down,
            MouseButton::Right => self.right = down,
        }
        self
    }

    pub fn is_down(self, button: MouseButton) -> bool {
        match button {
            MouseButton::Left => self.left,
            MouseButton::Middle => self.middle,
            MouseButton::Right => self.right,
        }
    }
}

/// Keyboard modifiers (winit's, not Slint's: Slint swaps Ctrl and Cmd on
/// Apple) and held mouse buttons, as every CEF event carries them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Modifiers {
    pub keys: ModifiersState,
    pub buttons: Buttons,
    pub locks: Locks,
}

/// The lock keys that are on. winit does not report them; the host reads
/// them from the platform where it can (Wayland's keyboard), else they stay off.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Locks {
    pub caps: bool,
    pub num: bool,
}

/// The buttons CEF knows. Back/Forward mouse buttons have no CEF button.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
}

/// View-relative logical pixels (= CEF DIP).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

/// One winit `KeyboardInput`, reduced to what the translation needs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyInput {
    pub pressed: bool,
    pub repeat: bool,
    pub physical: PhysicalKey,
    /// `PhysicalKeyExtScancode::to_scancode` (Linux: evdev code).
    pub scancode: Option<u32>,
    /// `KeyEventExtModifierSupplement::key_without_modifiers`.
    pub unmodified: Key,
    /// The text the key produced (with Shift/AltGr applied), if any.
    pub text: Option<String>,
    pub location: KeyLocation,
    pub modifiers: Modifiers,
}

pub fn cef_modifiers(m: &Modifiers) -> u32 {
    [
        (m.keys.shift_key(), flag::SHIFT),
        (m.keys.control_key(), flag::CONTROL),
        (m.keys.alt_key(), flag::ALT),
        (m.keys.super_key(), flag::COMMAND),
        (m.buttons.left, flag::LEFT_MOUSE),
        (m.buttons.middle, flag::MIDDLE_MOUSE),
        (m.buttons.right, flag::RIGHT_MOUSE),
        (m.locks.caps, flag::CAPS_LOCK),
        (m.locks.num, flag::NUM_LOCK),
    ]
    .into_iter()
    .filter_map(|(on, bit)| on.then_some(bit))
    .fold(0, |all, bit| all | bit)
}

/// The DIP pixel the point lies in. Slint's logical pixels and CEF's DIP
/// agree (the view is `round(physical / scale)` DIP), so this only floors,
/// as cefclient's `DeviceToLogical` truncates.
pub fn mouse_event(point: Point, m: &Modifiers) -> cef::MouseEvent {
    cef::MouseEvent {
        x: point.x.floor() as i32,
        y: point.y.floor() as i32,
        modifiers: cef_modifiers(m),
    }
}

/// A Ctrl+wheel at `point` with precise (pixel) deltas, as a trackpad pinch's wheel is
/// (`pinch.rs`), whatever keys are held.
pub fn ctrl_wheel_event(point: Point) -> cef::MouseEvent {
    cef::MouseEvent {
        modifiers: flag::CONTROL | flag::PRECISION_SCROLLING_DELTA,
        ..mouse_event(point, &Modifiers::default())
    }
}

/// One finger's touch event: finger `id` of the view at `point` (DIP, unrounded: touch keeps
/// fractions), as a touchscreen's (`pointer_type` touch, no contact size or angle).
pub fn touch_event(
    id: i32,
    kind: TouchKind,
    point: Point,
    pressure: f32,
    m: &Modifiers,
) -> cef::TouchEvent {
    cef::TouchEvent {
        id,
        x: point.x,
        y: point.y,
        radius_x: 0.0,
        radius_y: 0.0,
        rotation_angle: 0.0,
        pressure,
        type_: match kind {
            TouchKind::Press => cef::TouchEventType::PRESSED,
            TouchKind::Move => cef::TouchEventType::MOVED,
            TouchKind::Release => cef::TouchEventType::RELEASED,
            TouchKind::Cancel => cef::TouchEventType::CANCELLED,
        },
        modifiers: cef_modifiers(m),
        pointer_type: cef::PointerType::TOUCH,
    }
}

pub fn cef_button(button: MouseButton) -> cef::MouseButtonType {
    match button {
        MouseButton::Left => cef::MouseButtonType::LEFT,
        MouseButton::Middle => cef::MouseButtonType::MIDDLE,
        MouseButton::Right => cef::MouseButtonType::RIGHT,
    }
}

/// Double-click time and half the double-click rectangle: Windows' defaults
/// (`GetDoubleClickTime`, `SM_CXDOUBLECLK / 2`), which cefclient asks for.
const CLICK_TIME: Duration = Duration::from_millis(500);
const CLICK_DISTANCE: f32 = 2.0;
/// GTK reports at most a triple press; Blink needs no more.
const MAX_CLICKS: i32 = 3;

#[derive(Clone, Copy, Debug, PartialEq)]
struct Press {
    at: Duration,
    point: Point,
    button: MouseButton,
    count: i32,
}

/// Click counting, ported from cefclient `OsrWindowWin::OnMouseEvent`
/// (winit and Slint report no count). Time is passed in.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ClickTracker {
    last: Option<Press>,
}

impl ClickTracker {
    fn continues(last: &Press, now: Duration, point: Point) -> bool {
        (last.point.x - point.x).abs() <= CLICK_DISTANCE
            && (last.point.y - point.y).abs() <= CLICK_DISTANCE
            && now.saturating_sub(last.at) <= CLICK_TIME
    }

    /// The click count of a press.
    pub fn press(&mut self, now: Duration, point: Point, button: MouseButton) -> i32 {
        let (count, point) = match self.last {
            // As cefclient: the rectangle stays where the series started.
            Some(last) if last.button == button && Self::continues(&last, now, point) => {
                ((last.count + 1).min(MAX_CLICKS), last.point)
            }
            _ => (1, point),
        };
        self.last = Some(Press {
            at: now,
            point,
            button,
            count,
        });
        count
    }

    /// The count a release reports: that of the press it ends.
    pub fn count(&self) -> i32 {
        self.last.map_or(1, |last| last.count)
    }

    /// A move or leave beyond the limits ends the series.
    pub fn moved(&mut self, now: Duration, point: Point) {
        if self
            .last
            .is_some_and(|last| !Self::continues(&last, now, point))
        {
            self.last = None;
        }
    }
}

/// Wheel motion not yet sent, per axis. CEF takes whole DIP and touchpads
/// send fractions of one, so the fraction is carried to the next event
/// instead of rounded away; a reversal drops what was carried.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WheelRemainder {
    x: f64,
    y: f64,
}

impl WheelRemainder {
    fn axis(rest: &mut f64, delta: f32) -> i32 {
        let delta = f64::from(delta);
        if delta * *rest < 0.0 {
            *rest = 0.0;
        }
        *rest += delta;
        let whole = rest.trunc();
        *rest -= whole;
        whole as i32
    }
}

/// Wheel deltas in DIP (positive = up / left, as both winit and CEF use) and
/// the event flags. Shift turns a vertical-only scroll horizontal
/// (cefclient win); sub-pixel motion is carried in `rest`. `precise` marks
/// pixel deltas (a touchpad, cefclient GTK smooth scroll), which Blink scrolls
/// at once; a notched wheel is not precise, so Blink animates it as Chrome does.
pub fn wheel_deltas(
    dx: f32,
    dy: f32,
    m: &Modifiers,
    precise: bool,
    rest: &mut WheelRemainder,
) -> (i32, i32, u32) {
    let (dx, dy) = if m.keys.shift_key() && dx == 0.0 {
        (dy, 0.0)
    } else {
        (dx, dy)
    };
    (
        WheelRemainder::axis(&mut rest.x, dx),
        WheelRemainder::axis(&mut rest.y, dy),
        cef_modifiers(m) | (flag::PRECISION_SCROLLING_DELTA * u32::from(precise)),
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyKind {
    RawKeyDown,
    KeyUp,
    Char,
}

/// A `cef::KeyEvent` as plain data (comparable in tests).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CefKey {
    pub kind: KeyKind,
    pub modifiers: u32,
    pub windows_key_code: i32,
    pub native_key_code: i32,
    pub is_system_key: bool,
    pub character: u16,
    pub unmodified_character: u16,
}

impl From<CefKey> for cef::KeyEvent {
    fn from(key: CefKey) -> Self {
        Self {
            type_: match key.kind {
                KeyKind::RawKeyDown => cef::KeyEventType::RAWKEYDOWN,
                KeyKind::KeyUp => cef::KeyEventType::KEYUP,
                KeyKind::Char => cef::KeyEventType::CHAR,
            },
            modifiers: key.modifiers,
            windows_key_code: key.windows_key_code,
            native_key_code: key.native_key_code,
            is_system_key: key.is_system_key.into(),
            character: key.character,
            unmodified_character: key.unmodified_character,
            ..Self::default()
        }
    }
}

/// A key-down sent to the page, as CEF's unhandled-key report names it
/// (`WebEvent::KeyUnhandled`): its Windows key code and the Shift, Ctrl, Alt
/// and Cmd flags. Side, keypad, repeat and lock flags are left out, since CEF
/// need not report them as they were sent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyId {
    windows_key_code: i32,
    modifiers: u32,
}

impl KeyId {
    const MODIFIERS: u32 = flag::SHIFT | flag::CONTROL | flag::ALT | flag::COMMAND;

    /// The key-down `key` sends; `None` for a release.
    pub fn of(key: &KeyInput) -> Option<Self> {
        let down = key_events(key)
            .into_iter()
            .find(|e| e.kind == KeyKind::RawKeyDown)?;
        Some(Self::reported(down.windows_key_code, down.modifiers))
    }

    /// From CEF's report (welding `CefSurfaceEvent::KeyUnhandled`).
    pub(crate) fn reported(windows_key_code: i32, modifiers: u32) -> Self {
        Self {
            windows_key_code,
            modifiers: modifiers & Self::MODIFIERS,
        }
    }
}

/// The CEF events for one key: RAWKEYDOWN then one CHAR per UTF-16 unit of
/// the text on press (none for keys without text), KEYUP on release. As
/// cefclient GTK: Enter types `\r`, Ctrl types the control character, Alt
/// marks a system key. Command/Super chords type nothing.
pub fn key_events(key: &KeyInput) -> Vec<CefKey> {
    key_events_with(key, Current::named_key_character)
}

/// [`key_events`] with a platform's named-key characters (`Platform::named_key_character`),
/// so one OS's translation is testable on another.
fn key_events_with(key: &KeyInput, named_key_character: fn(i32) -> Option<u16>) -> Vec<CefKey> {
    let windows_key_code = without_location(vkey(key));
    let mods = &key.modifiers.keys;
    let modifiers = cef_modifiers(&key.modifiers)
        | match key.location {
            KeyLocation::Numpad => flag::KEY_PAD,
            KeyLocation::Left => flag::LEFT,
            KeyLocation::Right => flag::RIGHT,
            _ => 0,
        }
        | if key.repeat { flag::REPEAT } else { 0 };
    let named = named_key_character(windows_key_code);
    let unmodified = match &key.unmodified {
        _ if windows_key_code == vk::RETURN => Some('\r' as u16),
        Key::Character(text) => text.encode_utf16().next(),
        _ => named,
    };
    let event = |kind, character: u16, unmodified_character: u16| CefKey {
        kind,
        modifiers,
        windows_key_code,
        native_key_code: native_key_code(key.scancode),
        is_system_key: mods.alt_key(),
        character,
        unmodified_character,
    };
    if !key.pressed {
        let c = unmodified.unwrap_or(0);
        return vec![event(KeyKind::KeyUp, c, c)];
    }
    let chars: Vec<u16> = if mods.super_key() {
        vec![]
    } else if mods.control_key() && !mods.alt_key() {
        control_character(windows_key_code, mods.shift_key())
            .into_iter()
            .collect()
    } else if windows_key_code == vk::RETURN {
        vec!['\r' as u16]
    } else {
        key.text.as_deref().unwrap_or("").encode_utf16().collect()
    };
    // An unmodified character only describes a single produced unit.
    let single = chars.len() == 1;
    let unmodified_of = |c: u16| if single { unmodified.unwrap_or(c) } else { c };
    let first = chars.first().copied();
    let mut events = vec![event(
        KeyKind::RawKeyDown,
        first.or(named).unwrap_or(0),
        first.map_or(unmodified.unwrap_or(0), unmodified_of),
    )];
    events.extend(
        chars
            .iter()
            .map(|&c| event(KeyKind::Char, c, unmodified_of(c))),
    );
    events
}

/// CEF's `native_key_code`: on Linux the X11/xkb keycode (evdev + 8), which
/// CEF turns into the DOM `code` (`NativeKeycodeToDomCode`). Elsewhere what
/// winit reports (macOS keycode; Windows has no windowless view yet).
fn native_key_code(scancode: Option<u32>) -> i32 {
    let offset = if cfg!(target_os = "linux") { 8 } else { 0 };
    scancode.map_or(0, |code| (code + offset) as i32)
}

/// The character AppKit gives a named key on macOS (`NSEvent.characters`: the function-key
/// constants of `NSEvent.h`, ASCII for the rest); `None` for other keys. macOS's
/// `Platform::named_key_character`.
/// CEF's macOS key events are NSEvents built from `character` / `unmodified_character`
/// (`key_event_util_mac.mm` `GetLegacyWebKeyEvent`), and both zero makes a flags-changed
/// event: Backspace and the arrows then acted twice (a key-up read as a second press), and
/// Space's key-up (winit names Space, so its release carries no text) reached the page as a
/// second keydown and never as a keyup: YouTube's Space play/pause and hold-for-2x never fired.
/// The keys winit's macOS `code_to_key` names (so no release text) are here, but for the
/// modifiers (flags-changed is right for them), Num Lock (keypad Clear) and the volume keys.
#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
pub(crate) fn appkit_character(windows_key_code: i32) -> Option<u16> {
    let f = |n: i32| windows_key_code == vk::F1 + n;
    Some(match windows_key_code {
        vk::BACK => 0x7F,
        vk::TAB => 0x09,
        vk::ESCAPE => 0x1B,
        vk::SPACE => 0x20,
        vk::UP => 0xF700,
        vk::DOWN => 0xF701,
        vk::LEFT => 0xF702,
        vk::RIGHT => 0xF703,
        vk::INSERT => 0xF727,
        vk::DELETE => 0xF728,
        vk::HOME => 0xF729,
        vk::END => 0xF72B,
        vk::PRIOR => 0xF72C,
        vk::NEXT => 0xF72D,
        // NSF1FunctionKey .. NSF24FunctionKey
        code if (0..24).any(f) => 0xF704 + (code - vk::F1) as u16,
        _ => return None,
    })
}

/// `KeyboardCode` values from Chromium `keyboard_codes_posix.h` (= Windows `VK_*`).
mod vk {
    pub const CANCEL: i32 = 0x03;
    pub const BACK: i32 = 0x08;
    pub const TAB: i32 = 0x09;
    pub const CLEAR: i32 = 0x0C;
    pub const RETURN: i32 = 0x0D;
    pub const SHIFT: i32 = 0x10;
    pub const CONTROL: i32 = 0x11;
    pub const MENU: i32 = 0x12;
    pub const PAUSE: i32 = 0x13;
    pub const CAPITAL: i32 = 0x14;
    pub const KANA: i32 = 0x15;
    pub const KANJI: i32 = 0x19;
    pub const ESCAPE: i32 = 0x1B;
    pub const CONVERT: i32 = 0x1C;
    pub const NONCONVERT: i32 = 0x1D;
    pub const SPACE: i32 = 0x20;
    pub const PRIOR: i32 = 0x21;
    pub const NEXT: i32 = 0x22;
    pub const END: i32 = 0x23;
    pub const HOME: i32 = 0x24;
    pub const LEFT: i32 = 0x25;
    pub const UP: i32 = 0x26;
    pub const RIGHT: i32 = 0x27;
    pub const DOWN: i32 = 0x28;
    pub const SELECT: i32 = 0x29;
    pub const EXECUTE: i32 = 0x2B;
    pub const SNAPSHOT: i32 = 0x2C;
    pub const INSERT: i32 = 0x2D;
    pub const DELETE: i32 = 0x2E;
    pub const HELP: i32 = 0x2F;
    /// `VKEY_0`; `VKEY_1..9` follow.
    pub const K0: i32 = 0x30;
    pub const K9: i32 = 0x39;
    /// `VKEY_A`; `VKEY_B..Z` follow.
    pub const A: i32 = 0x41;
    pub const Z: i32 = 0x5A;
    pub const LWIN: i32 = 0x5B;
    pub const RWIN: i32 = 0x5C;
    pub const APPS: i32 = 0x5D;
    pub const SLEEP: i32 = 0x5F;
    /// `VKEY_NUMPAD0`; `VKEY_NUMPAD1..9` follow.
    pub const NUMPAD0: i32 = 0x60;
    pub const MULTIPLY: i32 = 0x6A;
    pub const ADD: i32 = 0x6B;
    pub const SUBTRACT: i32 = 0x6D;
    pub const DECIMAL: i32 = 0x6E;
    pub const DIVIDE: i32 = 0x6F;
    /// `VKEY_F1`; `VKEY_F2..F24` follow.
    pub const F1: i32 = 0x70;
    pub const NUMLOCK: i32 = 0x90;
    pub const SCROLL: i32 = 0x91;
    pub const POWER: i32 = 0x98;
    pub const LSHIFT: i32 = 0xA0;
    pub const RSHIFT: i32 = 0xA1;
    pub const LCONTROL: i32 = 0xA2;
    pub const RCONTROL: i32 = 0xA3;
    pub const LMENU: i32 = 0xA4;
    pub const RMENU: i32 = 0xA5;
    pub const BROWSER_BACK: i32 = 0xA6;
    pub const BROWSER_FORWARD: i32 = 0xA7;
    pub const BROWSER_REFRESH: i32 = 0xA8;
    pub const BROWSER_STOP: i32 = 0xA9;
    pub const BROWSER_SEARCH: i32 = 0xAA;
    pub const BROWSER_FAVORITES: i32 = 0xAB;
    pub const BROWSER_HOME: i32 = 0xAC;
    pub const VOLUME_MUTE: i32 = 0xAD;
    pub const VOLUME_DOWN: i32 = 0xAE;
    pub const VOLUME_UP: i32 = 0xAF;
    pub const MEDIA_NEXT_TRACK: i32 = 0xB0;
    pub const MEDIA_PREV_TRACK: i32 = 0xB1;
    pub const MEDIA_STOP: i32 = 0xB2;
    pub const MEDIA_PLAY_PAUSE: i32 = 0xB3;
    pub const MEDIA_LAUNCH_MAIL: i32 = 0xB4;
    pub const MEDIA_LAUNCH_MEDIA_SELECT: i32 = 0xB5;
    pub const MEDIA_LAUNCH_APP1: i32 = 0xB6;
    pub const MEDIA_LAUNCH_APP2: i32 = 0xB7;
    pub const OEM_1: i32 = 0xBA;
    pub const OEM_PLUS: i32 = 0xBB;
    pub const OEM_COMMA: i32 = 0xBC;
    pub const OEM_MINUS: i32 = 0xBD;
    pub const OEM_PERIOD: i32 = 0xBE;
    pub const OEM_2: i32 = 0xBF;
    pub const OEM_3: i32 = 0xC0;
    pub const OEM_4: i32 = 0xDB;
    pub const OEM_5: i32 = 0xDC;
    pub const OEM_6: i32 = 0xDD;
    pub const OEM_7: i32 = 0xDE;
    pub const OEM_102: i32 = 0xE2;
}

/// The located Windows key code. Policy as cefclient's
/// `GdkEventToWindowsKeyCode`: the layout's character first (so AZERTY's `a`
/// on the US `Q` key is `VKEY_A`), the physical US position for keys whose
/// character is not ASCII (Cyrillic `ф` on `A` is `VKEY_A`) and for keys
/// without text. Two deviations, matching Windows: the digit row and the
/// keypad are positional (AZERTY's `&` key is `VKEY_1`, keypad `5` is
/// `VKEY_NUMPAD5`), and cefclient's shifted-digit rows (`!` → `VKEY_1`, ...)
/// are left out since only non-digit keys could still reach them.
fn vkey(key: &KeyInput) -> i32 {
    let positional = match key.physical {
        PhysicalKey::Code(code) => dom_code_vkey(code),
        PhysicalKey::Unidentified(_) => 0,
    };
    let digit_row = (vk::K0..=vk::K9).contains(&positional);
    if (digit_row || key.location == KeyLocation::Numpad) && positional != 0 {
        return positional;
    }
    let from_char = match &key.unmodified {
        Key::Character(text) => {
            let mut chars = text.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) => char_vkey(c),
                _ => 0,
            }
        }
        _ => 0,
    };
    if from_char != 0 {
        from_char
    } else {
        positional
    }
}

/// `KeyboardCodeFromXKeysym` (cefclient GTK), the rows for characters.
fn char_vkey(c: char) -> i32 {
    match c.to_ascii_lowercase() {
        c @ 'a'..='z' => vk::A + (c as i32 - 'a' as i32),
        c @ '0'..='9' => vk::K0 + (c as i32 - '0' as i32),
        ' ' => vk::SPACE,
        '=' | '+' => vk::OEM_PLUS,
        ',' | '<' => vk::OEM_COMMA,
        '-' | '_' => vk::OEM_MINUS,
        '.' | '>' => vk::OEM_PERIOD,
        ';' | ':' => vk::OEM_1,
        '/' | '?' => vk::OEM_2,
        '`' | '~' => vk::OEM_3,
        '[' | '{' => vk::OEM_4,
        '\\' | '|' => vk::OEM_5,
        ']' | '}' => vk::OEM_6,
        '\'' | '"' => vk::OEM_7,
        '×' => vk::MULTIPLY,
        // The international backslash key of 102-key keyboards.
        '«' | '»' | '°' | 'ù' | 'Ù' | '¦' => vk::OEM_102,
        _ => 0,
    }
}

/// Chromium `kDomCodeToKeyboardCodeMap` (US positions), the rows for keys
/// winit's `KeyCode` names. 0 for keys Chromium has no code for.
#[rustfmt::skip]
fn dom_code_vkey(code: KeyCode) -> i32 {
    use KeyCode as K;
    match code {
        K::Sleep => vk::SLEEP,
        K::KeyA => vk::A, K::KeyB => vk::A + 1, K::KeyC => vk::A + 2, K::KeyD => vk::A + 3,
        K::KeyE => vk::A + 4, K::KeyF => vk::A + 5, K::KeyG => vk::A + 6, K::KeyH => vk::A + 7,
        K::KeyI => vk::A + 8, K::KeyJ => vk::A + 9, K::KeyK => vk::A + 10, K::KeyL => vk::A + 11,
        K::KeyM => vk::A + 12, K::KeyN => vk::A + 13, K::KeyO => vk::A + 14, K::KeyP => vk::A + 15,
        K::KeyQ => vk::A + 16, K::KeyR => vk::A + 17, K::KeyS => vk::A + 18, K::KeyT => vk::A + 19,
        K::KeyU => vk::A + 20, K::KeyV => vk::A + 21, K::KeyW => vk::A + 22, K::KeyX => vk::A + 23,
        K::KeyY => vk::A + 24, K::KeyZ => vk::Z,
        K::Digit1 => vk::K0 + 1, K::Digit2 => vk::K0 + 2, K::Digit3 => vk::K0 + 3,
        K::Digit4 => vk::K0 + 4, K::Digit5 => vk::K0 + 5, K::Digit6 => vk::K0 + 6,
        K::Digit7 => vk::K0 + 7, K::Digit8 => vk::K0 + 8, K::Digit9 => vk::K9,
        K::Digit0 => vk::K0,
        K::Enter => vk::RETURN,
        K::Escape => vk::ESCAPE,
        K::Backspace => vk::BACK,
        K::Tab => vk::TAB,
        K::Space => vk::SPACE,
        K::Minus => vk::OEM_MINUS,
        K::Equal => vk::OEM_PLUS,
        K::BracketLeft => vk::OEM_4,
        K::BracketRight => vk::OEM_6,
        K::Backslash => vk::OEM_5,
        K::Semicolon => vk::OEM_1,
        K::Quote => vk::OEM_7,
        K::Backquote => vk::OEM_3,
        K::Comma => vk::OEM_COMMA,
        K::Period => vk::OEM_PERIOD,
        K::Slash => vk::OEM_2,
        K::CapsLock => vk::CAPITAL,
        K::F1 => vk::F1, K::F2 => vk::F1 + 1, K::F3 => vk::F1 + 2, K::F4 => vk::F1 + 3,
        K::F5 => vk::F1 + 4, K::F6 => vk::F1 + 5, K::F7 => vk::F1 + 6, K::F8 => vk::F1 + 7,
        K::F9 => vk::F1 + 8, K::F10 => vk::F1 + 9, K::F11 => vk::F1 + 10, K::F12 => vk::F1 + 11,
        K::PrintScreen => vk::SNAPSHOT,
        K::ScrollLock => vk::SCROLL,
        K::Pause => vk::PAUSE,
        K::Insert => vk::INSERT,
        K::Home => vk::HOME,
        K::PageUp => vk::PRIOR,
        K::Delete => vk::DELETE,
        K::End => vk::END,
        K::PageDown => vk::NEXT,
        K::ArrowRight => vk::RIGHT,
        K::ArrowLeft => vk::LEFT,
        K::ArrowDown => vk::DOWN,
        K::ArrowUp => vk::UP,
        K::NumLock => vk::NUMLOCK,
        K::NumpadDivide => vk::DIVIDE,
        K::NumpadMultiply => vk::MULTIPLY,
        K::NumpadSubtract => vk::SUBTRACT,
        K::NumpadAdd => vk::ADD,
        K::NumpadEnter => vk::RETURN,
        K::Numpad1 => vk::NUMPAD0 + 1, K::Numpad2 => vk::NUMPAD0 + 2, K::Numpad3 => vk::NUMPAD0 + 3,
        K::Numpad4 => vk::NUMPAD0 + 4, K::Numpad5 => vk::NUMPAD0 + 5, K::Numpad6 => vk::NUMPAD0 + 6,
        K::Numpad7 => vk::NUMPAD0 + 7, K::Numpad8 => vk::NUMPAD0 + 8, K::Numpad9 => vk::NUMPAD0 + 9,
        K::Numpad0 => vk::NUMPAD0,
        K::NumpadDecimal => vk::DECIMAL,
        K::IntlBackslash => vk::OEM_102,
        K::ContextMenu => vk::APPS,
        K::Power => vk::POWER,
        K::F13 => vk::F1 + 12, K::F14 => vk::F1 + 13, K::F15 => vk::F1 + 14, K::F16 => vk::F1 + 15,
        K::F17 => vk::F1 + 16, K::F18 => vk::F1 + 17, K::F19 => vk::F1 + 18, K::F20 => vk::F1 + 19,
        K::F21 => vk::F1 + 20, K::F22 => vk::F1 + 21, K::F23 => vk::F1 + 22, K::F24 => vk::F1 + 23,
        K::Open => vk::EXECUTE,
        K::Help => vk::HELP,
        K::Select => vk::SELECT,
        K::AudioVolumeMute => vk::VOLUME_MUTE,
        K::AudioVolumeUp => vk::VOLUME_UP,
        K::AudioVolumeDown => vk::VOLUME_DOWN,
        K::NumpadComma => vk::OEM_COMMA,
        K::IntlRo => vk::OEM_102,
        K::KanaMode => vk::KANA,
        K::IntlYen => vk::OEM_5,
        K::Convert => vk::CONVERT,
        K::NonConvert => vk::NONCONVERT,
        K::Lang1 => vk::KANA,
        K::Lang2 => vk::KANJI,
        K::Abort => vk::CANCEL,
        K::NumpadBackspace => vk::BACK,
        K::NumpadClear | K::NumpadClearEntry => vk::CLEAR,
        K::ControlLeft => vk::LCONTROL,
        K::ShiftLeft => vk::LSHIFT,
        K::AltLeft => vk::LMENU,
        K::SuperLeft => vk::LWIN,
        K::ControlRight => vk::RCONTROL,
        K::ShiftRight => vk::RSHIFT,
        K::AltRight => vk::RMENU,
        K::SuperRight => vk::RWIN,
        K::MediaTrackNext => vk::MEDIA_NEXT_TRACK,
        K::MediaTrackPrevious => vk::MEDIA_PREV_TRACK,
        K::MediaStop => vk::MEDIA_STOP,
        K::MediaPlayPause => vk::MEDIA_PLAY_PAUSE,
        K::MediaSelect => vk::MEDIA_LAUNCH_MEDIA_SELECT,
        K::LaunchMail => vk::MEDIA_LAUNCH_MAIL,
        K::LaunchApp2 => vk::MEDIA_LAUNCH_APP2,
        K::LaunchApp1 => vk::MEDIA_LAUNCH_APP1,
        K::BrowserSearch => vk::BROWSER_SEARCH,
        K::BrowserHome => vk::BROWSER_HOME,
        K::BrowserBack => vk::BROWSER_BACK,
        K::BrowserForward => vk::BROWSER_FORWARD,
        K::BrowserStop => vk::BROWSER_STOP,
        K::BrowserRefresh => vk::BROWSER_REFRESH,
        K::BrowserFavorites => vk::BROWSER_FAVORITES,
        _ => 0,
    }
}

/// `GetWindowsKeyCodeWithoutLocation` (cefclient GTK).
fn without_location(code: i32) -> i32 {
    match code {
        vk::LCONTROL | vk::RCONTROL => vk::CONTROL,
        vk::LSHIFT | vk::RSHIFT => vk::SHIFT,
        vk::LMENU | vk::RMENU => vk::MENU,
        _ => code,
    }
}

/// `GetControlCharacter` (cefclient GTK): what Ctrl+key types, as Windows does.
fn control_character(code: i32, shift: bool) -> Option<u16> {
    const K2: i32 = vk::K0 + 2;
    const K6: i32 = vk::K0 + 6;
    match (code, shift) {
        // Ctrl+A..Z are 0x01..0x1A.
        (vk::A..=vk::Z, _) => Some((code - vk::A + 1) as u16),
        // Ctrl+@ is NUL, Ctrl+^ RS, Ctrl+_ US.
        (K2, true) => Some(0),
        (K6, true) => Some(0x1E),
        (vk::OEM_MINUS, true) => Some(0x1F),
        // Ctrl+[ ESC, Ctrl+\ FS, Ctrl+] GS, Ctrl+Enter LF.
        (vk::OEM_4, false) => Some(0x1B),
        (vk::OEM_5, false) => Some(0x1C),
        (vk::OEM_6, false) => Some(0x1D),
        (vk::RETURN, false) => Some(0x0A),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use winit::keyboard::{NamedKey, NativeKeyCode};

    fn mods(keys: ModifiersState) -> Modifiers {
        Modifiers {
            keys,
            buttons: Buttons::default(),
            locks: Locks::default(),
        }
    }

    #[test]
    fn lock_keys_set_cef_s_lock_flags() {
        let mut m = mods(ModifiersState::empty());
        assert_eq!(cef_modifiers(&m), 0);
        m.locks.caps = true;
        assert_eq!(cef_modifiers(&m), 1 << 0);
        m.locks = Locks {
            caps: false,
            num: true,
        };
        assert_eq!(cef_modifiers(&m), 1 << 8);
    }

    #[test]
    fn every_modifier_combination_maps_bit_for_bit() {
        let keys = [
            (ModifiersState::SHIFT, flag::SHIFT),
            (ModifiersState::CONTROL, flag::CONTROL),
            (ModifiersState::ALT, flag::ALT),
            (ModifiersState::SUPER, flag::COMMAND),
        ];
        let buttons = [
            (MouseButton::Left, flag::LEFT_MOUSE),
            (MouseButton::Middle, flag::MIDDLE_MOUSE),
            (MouseButton::Right, flag::RIGHT_MOUSE),
        ];
        for bits in 0..128u32 {
            let mut m = Modifiers::default();
            let mut want = 0;
            for (i, (state, f)) in keys.iter().enumerate() {
                if bits & (1 << i) != 0 {
                    m.keys |= *state;
                    want |= f;
                }
            }
            for (i, (button, f)) in buttons.iter().enumerate() {
                if bits & (1 << (4 + i)) != 0 {
                    m.buttons = m.buttons.with(*button, true);
                    want |= f;
                }
            }
            assert_eq!(cef_modifiers(&m), want, "{m:?}");
        }
    }

    #[test]
    fn flag_values_match_cef() {
        use cef::sys::cef_event_flags_t as F;
        let pairs = [
            (flag::CAPS_LOCK, F::EVENTFLAG_CAPS_LOCK_ON),
            (flag::NUM_LOCK, F::EVENTFLAG_NUM_LOCK_ON),
            (flag::SHIFT, F::EVENTFLAG_SHIFT_DOWN),
            (flag::CONTROL, F::EVENTFLAG_CONTROL_DOWN),
            (flag::ALT, F::EVENTFLAG_ALT_DOWN),
            (flag::LEFT_MOUSE, F::EVENTFLAG_LEFT_MOUSE_BUTTON),
            (flag::MIDDLE_MOUSE, F::EVENTFLAG_MIDDLE_MOUSE_BUTTON),
            (flag::RIGHT_MOUSE, F::EVENTFLAG_RIGHT_MOUSE_BUTTON),
            (flag::COMMAND, F::EVENTFLAG_COMMAND_DOWN),
            (flag::KEY_PAD, F::EVENTFLAG_IS_KEY_PAD),
            (flag::LEFT, F::EVENTFLAG_IS_LEFT),
            (flag::RIGHT, F::EVENTFLAG_IS_RIGHT),
            (flag::REPEAT, F::EVENTFLAG_IS_REPEAT),
            (
                flag::PRECISION_SCROLLING_DELTA,
                F::EVENTFLAG_PRECISION_SCROLLING_DELTA,
            ),
        ];
        for (ours, cef) in pairs {
            assert_eq!(ours, crate::cef_flags(cef.0), "{cef:?}");
        }
    }

    #[test]
    fn buttons_map_to_cef() {
        assert_eq!(cef_button(MouseButton::Left), cef::MouseButtonType::LEFT);
        assert_eq!(
            cef_button(MouseButton::Middle),
            cef::MouseButtonType::MIDDLE
        );
        assert_eq!(cef_button(MouseButton::Right), cef::MouseButtonType::RIGHT);
        let held = Buttons::default().with(MouseButton::Right, true);
        assert_eq!(held.with(MouseButton::Right, false), Buttons::default());
    }

    /// Logical px from a physical position at a scale, floored to the DIP pixel.
    #[test]
    fn mouse_coordinates_floor_logical_pixels() {
        let cases = [
            // (physical, scale) -> DIP
            ((101.0, 51.0), 1.0, (101, 51)),
            ((101.0, 51.0), 1.25, (80, 40)),
            ((101.0, 51.0), 2.0, (50, 25)),
            ((-3.0, -1.0), 2.0, (-2, -1)),
            ((-0.5, 0.0), 1.25, (-1, 0)),
        ];
        for ((px, py), scale, (x, y)) in cases {
            let point = Point {
                x: px / scale,
                y: py / scale,
            };
            let event = mouse_event(point, &mods(ModifiersState::SHIFT));
            assert_eq!(
                (event.x, event.y, event.modifiers),
                (x, y, flag::SHIFT),
                "{point:?}"
            );
        }
    }

    const MS: Duration = Duration::from_millis(1);
    const AT: Point = Point { x: 10.0, y: 10.0 };

    #[test]
    fn clicks_count_up_to_three() {
        let mut t = ClickTracker::default();
        let counts: Vec<_> = (0..5)
            .map(|i| t.press(MS * (100 * i), AT, MouseButton::Left))
            .collect();
        assert_eq!(counts, [1, 2, 3, 3, 3]);
        assert_eq!(t.count(), 3);
    }

    #[test]
    fn a_click_series_breaks() {
        let near = Point { x: 12.0, y: 8.0 };
        let far = Point { x: 12.5, y: 10.0 };
        let cases = [
            ("continues", MS * 500, near, MouseButton::Left, 2),
            ("other button", MS * 10, AT, MouseButton::Right, 1),
            ("too far", MS * 10, far, MouseButton::Left, 1),
            ("too late", MS * 501, AT, MouseButton::Left, 1),
        ];
        for (name, after, point, button, want) in cases {
            let mut t = ClickTracker::default();
            t.press(Duration::ZERO, AT, MouseButton::Left);
            assert_eq!(t.press(after, point, button), want, "{name}");
        }
    }

    #[test]
    fn moving_away_ends_the_series() {
        let mut t = ClickTracker::default();
        t.press(Duration::ZERO, AT, MouseButton::Left);
        t.moved(MS, Point { x: 11.0, y: 11.0 });
        assert_eq!(t.press(MS * 2, AT, MouseButton::Left), 2);
        t.moved(MS * 3, Point { x: 30.0, y: 10.0 });
        assert_eq!(t.count(), 1);
        assert_eq!(t.press(MS * 4, AT, MouseButton::Left), 1);
    }

    #[test]
    fn wheel_deltas_swap_and_flag() {
        let none = mods(ModifiersState::empty());
        let shift = mods(ModifiersState::SHIFT);
        let p = flag::PRECISION_SCROLLING_DELTA;
        let mut rest = WheelRemainder::default();
        assert_eq!(wheel_deltas(0.0, 60.0, &none, true, &mut rest), (0, 60, p));
        assert_eq!(
            wheel_deltas(0.0, -60.0, &shift, true, &mut rest),
            (-60, 0, p | flag::SHIFT)
        );
        // A real horizontal delta is left alone.
        assert_eq!(
            wheel_deltas(5.0, 7.0, &shift, true, &mut rest),
            (5, 7, p | flag::SHIFT)
        );
        // A notched wheel is not precise.
        assert_eq!(wheel_deltas(0.0, 60.0, &none, false, &mut rest), (0, 60, 0));
        assert_eq!(
            wheel_deltas(0.0, 60.0, &shift, false, &mut rest),
            (60, 0, flag::SHIFT)
        );
    }

    #[test]
    fn sub_pixel_wheel_motion_adds_up() {
        let none = mods(ModifiersState::empty());
        let mut rest = WheelRemainder::default();
        let (mut x, mut y) = (0, 0);
        for _ in 0..10 {
            let (dx, dy, _) = wheel_deltas(-0.3, 0.3, &none, true, &mut rest);
            (x, y) = (x + dx, y + dy);
        }
        assert_eq!((x, y), (-3, 3));
    }

    #[test]
    fn a_wheel_reversal_drops_the_carried_fraction() {
        let none = mods(ModifiersState::empty());
        let mut rest = WheelRemainder::default();
        assert_eq!(wheel_deltas(0.0, 0.9, &none, true, &mut rest).1, 0);
        assert_eq!(wheel_deltas(0.0, -0.5, &none, true, &mut rest).1, 0);
        assert_eq!(wheel_deltas(0.0, -0.6, &none, true, &mut rest).1, -1);
    }

    fn key(code: KeyCode, unmodified: Key, text: Option<&str>) -> KeyInput {
        KeyInput {
            pressed: true,
            repeat: false,
            physical: PhysicalKey::Code(code),
            scancode: Some(30),
            unmodified,
            text: text.map(String::from),
            location: KeyLocation::Standard,
            modifiers: Modifiers::default(),
        }
    }

    fn ch(text: &str) -> Key {
        Key::Character(text.into())
    }

    fn kinds(events: &[CefKey]) -> Vec<(KeyKind, i32, u16)> {
        events
            .iter()
            .map(|e| (e.kind, e.windows_key_code, e.character))
            .collect()
    }

    const VK_A: i32 = 0x41;

    #[test]
    fn letters_follow_the_layout_and_fall_back_to_position() {
        use KeyKind::*;
        let cases = [
            ("qwerty a", KeyCode::KeyA, "a", VK_A),
            ("azerty a on KeyQ", KeyCode::KeyQ, "a", VK_A),
            ("cyrillic on KeyA", KeyCode::KeyA, "ф", VK_A),
            ("german - on Slash", KeyCode::Slash, "-", 0xBD),
            ("azerty & on Digit1", KeyCode::Digit1, "&", 0x31),
        ];
        for (name, code, text, vk) in cases {
            let c = text.encode_utf16().next().expect("one unit");
            assert_eq!(
                kinds(&key_events(&key(code, ch(text), Some(text)))),
                [(RawKeyDown, vk, c), (Char, vk, c)],
                "{name}"
            );
        }
    }

    #[test]
    fn a_reported_key_down_matches_the_one_sent() {
        let ctrl_r = KeyInput {
            modifiers: mods(ModifiersState::CONTROL),
            location: KeyLocation::Left,
            ..key(KeyCode::KeyR, ch("r"), None)
        };
        let id = KeyId::of(&ctrl_r).expect("a press");
        // CEF may add side and lock flags; they do not matter.
        assert_eq!(
            KeyId::reported(0x52, flag::CONTROL | flag::LEFT | flag::REPEAT),
            id
        );
        // Plain R or Ctrl+Shift+R is another key-down.
        assert_ne!(KeyId::reported(0x52, 0), id);
        assert_ne!(KeyId::reported(0x52, flag::CONTROL | flag::SHIFT), id);
        let release = KeyInput {
            pressed: false,
            ..ctrl_r
        };
        assert_eq!(KeyId::of(&release), None);
    }

    #[test]
    fn enter_types_carriage_return() {
        let events = key_events(&key(
            KeyCode::Enter,
            Key::Named(NamedKey::Enter),
            Some("\r"),
        ));
        assert_eq!(
            kinds(&events),
            [(KeyKind::RawKeyDown, 0x0D, 13), (KeyKind::Char, 0x0D, 13)]
        );
        assert_eq!(events[1].unmodified_character, 13);
    }

    #[test]
    fn ctrl_types_control_characters() {
        let mut ctrl_a = key(KeyCode::KeyA, ch("a"), Some("a"));
        ctrl_a.modifiers = mods(ModifiersState::CONTROL);
        let events = key_events(&ctrl_a);
        assert_eq!(
            kinds(&events),
            [(KeyKind::RawKeyDown, VK_A, 1), (KeyKind::Char, VK_A, 1)]
        );
        assert_eq!(events[1].unmodified_character, 'a' as u16);
        assert_eq!(events[1].modifiers, flag::CONTROL);
        // Ctrl+1 types nothing.
        let mut ctrl_1 = key(KeyCode::Digit1, ch("1"), Some("1"));
        ctrl_1.modifiers = mods(ModifiersState::CONTROL);
        assert_eq!(
            kinds(&key_events(&ctrl_1)),
            [(KeyKind::RawKeyDown, 0x31, 0)]
        );
        assert_eq!(control_character(vk::OEM_MINUS, true), Some(0x1F));
        assert_eq!(control_character(vk::K0 + 2, true), Some(0));
        assert_eq!(control_character(vk::RETURN, false), Some(0x0A));
        assert_eq!(control_character(vk::RETURN, true), None);
    }

    #[test]
    fn shifted_text_is_what_the_key_produced() {
        let mut bang = key(KeyCode::Digit1, ch("1"), Some("!"));
        bang.modifiers = mods(ModifiersState::SHIFT);
        let events = key_events(&bang);
        assert_eq!(kinds(&events)[1], (KeyKind::Char, 0x31, '!' as u16));
        assert_eq!(events[1].unmodified_character, '1' as u16);
    }

    #[test]
    fn keypad_and_sided_keys_carry_location_flags() {
        let mut five = key(KeyCode::Numpad5, ch("5"), Some("5"));
        five.location = KeyLocation::Numpad;
        let events = key_events(&five);
        assert_eq!(events[0].windows_key_code, 0x65);
        assert_eq!(events[0].modifiers, flag::KEY_PAD);
        for (code, location, side) in [
            (KeyCode::ShiftLeft, KeyLocation::Left, flag::LEFT),
            (KeyCode::ShiftRight, KeyLocation::Right, flag::RIGHT),
        ] {
            let mut shift = key(code, Key::Named(NamedKey::Shift), None);
            shift.location = location;
            shift.modifiers = mods(ModifiersState::SHIFT);
            let events = key_events(&shift);
            assert_eq!(kinds(&events), [(KeyKind::RawKeyDown, 0x10, 0)]);
            assert_eq!(events[0].modifiers, flag::SHIFT | side);
        }
    }

    #[test]
    fn alt_marks_a_system_key() {
        let mut alt_f = key(KeyCode::KeyF, ch("f"), Some("f"));
        alt_f.modifiers = mods(ModifiersState::ALT);
        assert!(key_events(&alt_f).iter().all(|e| e.is_system_key));
        assert!(!key_events(&key(KeyCode::KeyF, ch("f"), Some("f")))[0].is_system_key);
    }

    #[test]
    fn keys_without_text_send_no_char() {
        for (code, named, vk) in [
            (KeyCode::F5, NamedKey::F5, 0x74),
            (KeyCode::ArrowLeft, NamedKey::ArrowLeft, 0x25),
            (KeyCode::ArrowDown, NamedKey::ArrowDown, 0x28),
        ] {
            let events = key_events(&key(code, Key::Named(named), None));
            let c = Current::named_key_character(vk).unwrap_or(0);
            assert_eq!(kinds(&events), [(KeyKind::RawKeyDown, vk, c)], "{code:?}");
        }
    }

    /// AppKit's characters for named keys; letters have none.
    #[test]
    fn appkit_characters() {
        assert_eq!(appkit_character(vk::LEFT), Some(0xF702));
        assert_eq!(appkit_character(vk::BACK), Some(0x7F));
        assert_eq!(appkit_character(vk::F1 + 11), Some(0xF70F));
        assert_eq!(appkit_character(vk::A), None);
    }

    /// A named key's events carry this OS's character for it; on macOS never both characters
    /// zero (CEF would build a flags-changed NSEvent).
    #[test]
    fn named_keys_carry_the_platform_character() {
        let mut up = key(KeyCode::Backspace, Key::Named(NamedKey::Backspace), None);
        up.pressed = false;
        let up = key_events(&up);
        let want = Current::named_key_character(vk::BACK).unwrap_or(0);
        assert_eq!(up[0].unmodified_character, want);
    }

    /// Whether CEF's macOS `TranslateWebKeyEvent` builds a flags-changed NSEvent from this
    /// event (`browser_platform_delegate_native_mac.mm`): both characters zero. Chromium's
    /// `IsKeyUpEvent` reads a flags-changed non-modifier key as a press.
    fn mac_flags_changed(e: &CefKey) -> bool {
        e.character == 0 && e.unmodified_character == 0
    }

    /// Space as winit reports it on macOS and Linux: a named key, its press typing " ".
    fn space(pressed: bool, repeat: bool) -> KeyInput {
        KeyInput {
            pressed,
            repeat,
            text: pressed.then(|| " ".into()),
            ..key(KeyCode::Space, Key::Named(NamedKey::Space), None)
        }
    }

    /// A Space tap on macOS: RAWKEYDOWN and CHAR " ", then a KEYUP the page sees as a keyup
    /// (YouTube toggles play on it), not a flags-changed event read as a second keydown.
    #[test]
    fn mac_space_tap_is_keydown_then_keyup() {
        use KeyKind::*;
        let down = key_events_with(&space(true, false), appkit_character);
        let up = key_events_with(&space(false, false), appkit_character);
        assert_eq!(
            kinds(&down),
            [(RawKeyDown, vk::SPACE, 0x20), (Char, vk::SPACE, 0x20)]
        );
        assert_eq!(kinds(&up), [(KeyUp, vk::SPACE, 0x20)]);
        assert!(!down.iter().chain(&up).any(mac_flags_changed));
    }

    /// Space held (YouTube's hold-for-2x): repeats carry the repeat flag, the release is a
    /// KEYUP; on both platforms' tables.
    #[test]
    fn space_hold_repeats_then_keyup() {
        use KeyKind::*;
        let tables: [fn(i32) -> Option<u16>; 2] = [appkit_character, |_| None];
        for table in tables {
            let first = key_events_with(&space(true, false), table);
            let again = key_events_with(&space(true, true), table);
            let up = key_events_with(&space(false, false), table);
            assert_eq!(first[0].modifiers & flag::REPEAT, 0);
            assert!(again.iter().all(|e| e.modifiers & flag::REPEAT != 0));
            assert_eq!(kinds(&again)[0], (RawKeyDown, vk::SPACE, 0x20));
            assert_eq!(up.len(), 1);
            assert_eq!((up[0].kind, up[0].windows_key_code), (KeyUp, vk::SPACE));
        }
    }

    /// No key winit's macOS `code_to_key` names is released as a flags-changed event, but for
    /// the modifiers (for which it is right), the keypad's Clear and the volume keys.
    #[test]
    fn mac_named_key_releases_are_key_ups() {
        use NamedKey as N;
        let named = [
            (KeyCode::Enter, N::Enter),
            (KeyCode::Tab, N::Tab),
            (KeyCode::Space, N::Space),
            (KeyCode::Backspace, N::Backspace),
            (KeyCode::Escape, N::Escape),
            (KeyCode::Insert, N::Insert),
            (KeyCode::Home, N::Home),
            (KeyCode::PageUp, N::PageUp),
            (KeyCode::Delete, N::Delete),
            (KeyCode::End, N::End),
            (KeyCode::PageDown, N::PageDown),
            (KeyCode::ArrowLeft, N::ArrowLeft),
            (KeyCode::ArrowRight, N::ArrowRight),
            (KeyCode::ArrowDown, N::ArrowDown),
            (KeyCode::ArrowUp, N::ArrowUp),
            (KeyCode::F1, N::F1),
            (KeyCode::F12, N::F12),
            (KeyCode::F20, N::F20),
        ];
        for (code, n) in named {
            let mut up = key(code, Key::Named(n), None);
            up.pressed = false;
            let up = key_events_with(&up, appkit_character);
            assert!(!mac_flags_changed(&up[0]), "{code:?}");
        }
        let mut shift = key(KeyCode::ShiftLeft, Key::Named(N::Shift), None);
        shift.pressed = false;
        assert!(mac_flags_changed(
            &key_events_with(&shift, appkit_character)[0]
        ));
    }

    #[test]
    fn non_bmp_text_is_one_char_per_utf16_unit() {
        let emoji = key(KeyCode::KeyE, ch("🫧"), Some("🫧"));
        let units: Vec<u16> = "🫧".encode_utf16().collect();
        let events = key_events(&emoji);
        assert_eq!(events.len(), 3);
        assert_eq!(
            events[1..].iter().map(|e| e.character).collect::<Vec<_>>(),
            units
        );
        // No single unmodified unit to report, so each unit stands for itself.
        assert_eq!(events[2].unmodified_character, units[1]);
    }

    #[test]
    fn release_sends_only_keyup_and_repeat_is_flagged() {
        let mut up = key(KeyCode::KeyA, ch("a"), None);
        up.pressed = false;
        assert_eq!(
            kinds(&key_events(&up)),
            [(KeyKind::KeyUp, VK_A, 'a' as u16)]
        );
        let mut again = key(KeyCode::KeyA, ch("a"), Some("a"));
        again.repeat = true;
        assert!(
            key_events(&again)
                .iter()
                .all(|e| e.modifiers & flag::REPEAT != 0)
        );
    }

    #[test]
    fn command_chords_type_nothing() {
        let mut cmd_c = key(KeyCode::KeyC, ch("c"), Some("c"));
        cmd_c.modifiers = mods(ModifiersState::SUPER);
        assert_eq!(kinds(&key_events(&cmd_c)), [(KeyKind::RawKeyDown, 0x43, 0)]);
    }

    #[test]
    fn unidentified_keys_use_the_character_only() {
        let mut odd = key(KeyCode::KeyA, ch("x"), Some("x"));
        odd.physical = PhysicalKey::Unidentified(NativeKeyCode::Xkb(300));
        assert_eq!(key_events(&odd)[0].windows_key_code, 0x58);
        odd.unmodified = ch("ж");
        assert_eq!(key_events(&odd)[0].windows_key_code, 0);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_native_key_code_is_the_xkb_keycode() {
        assert_eq!(native_key_code(Some(30)), 38);
        assert_eq!(native_key_code(None), 0);
        assert_eq!(
            key_events(&key(KeyCode::KeyA, ch("a"), None))[0].native_key_code,
            38
        );
    }

    /// Every winit code with a Windows equivalent maps to one; spot-check
    /// the table against Chromium's values.
    #[rustfmt::skip]
    #[test]
    fn dom_code_table_has_no_holes() {
        use KeyCode as K;
        let named = [
            K::KeyA, K::KeyZ, K::Digit0, K::Digit9, K::Enter, K::Escape, K::Backspace, K::Tab,
            K::Space, K::Minus, K::Equal, K::BracketLeft, K::BracketRight, K::Backslash,
            K::Semicolon, K::Quote, K::Backquote, K::Comma, K::Period, K::Slash, K::CapsLock,
            K::F1, K::F12, K::F13, K::F24, K::PrintScreen, K::ScrollLock, K::Pause, K::Insert,
            K::Home, K::PageUp, K::Delete, K::End, K::PageDown, K::ArrowRight, K::ArrowLeft,
            K::ArrowDown, K::ArrowUp, K::NumLock, K::NumpadDivide, K::NumpadMultiply,
            K::NumpadSubtract, K::NumpadAdd, K::NumpadEnter, K::Numpad0, K::Numpad9,
            K::NumpadDecimal, K::IntlBackslash, K::ContextMenu, K::ControlLeft, K::ShiftLeft,
            K::AltLeft, K::SuperLeft, K::ControlRight, K::ShiftRight, K::AltRight, K::SuperRight,
            K::AudioVolumeMute, K::MediaPlayPause, K::BrowserBack,
        ];
        for code in named {
            assert_ne!(dom_code_vkey(code), 0, "{code:?}");
        }
        let spot = [
            (K::KeyZ, 0x5A), (K::Digit0, 0x30), (K::Backquote, 0xC0), (K::F24, 0x87),
            (K::Numpad9, 0x69), (K::IntlBackslash, 0xE2), (K::SuperRight, 0x5C),
            (K::PageDown, 0x22), (K::BrowserFavorites, 0xAB), (K::Lang2, 0x19),
        ];
        for (code, value) in spot {
            assert_eq!(dom_code_vkey(code), value, "{code:?}");
        }
        assert_eq!(dom_code_vkey(K::F25), 0);
        assert_eq!(without_location(vk::RMENU), vk::MENU);
        assert_eq!(without_location(vk::LWIN), vk::LWIN);
    }
}
