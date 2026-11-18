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
use slint::winit_030::winit::event::KeyEvent;
use slint::winit_030::winit::keyboard::{Key, ModifiersState, NamedKey};
use slint::winit_030::winit::platform::modifier_supplement::KeyEventExtModifierSupplement;
use slint::winit_030::winit::platform::scancode::PhysicalKeyExtScancode;
use wubble_engine::input::{Buttons, KeyInput, Modifiers, MouseButton};

/// The modifier the shell's own shortcuts use (Slint maps `Control` in
/// `@keys` to Cmd on Apple).
const PRIMARY: ModifiersState = if cfg!(target_os = "macos") {
    ModifiersState::SUPER
} else {
    ModifiersState::CONTROL
};

/// Shortcuts the shell keeps even while the page has focus (`PRIMARY` +
/// key): new tab, address, close tab. Matches the `KeyBinding`s in `app.slint`.
const RESERVED_CHORDS: &[&str] = &["t", "l", "w"];

/// What to do with a key event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyRoute {
    /// To the page only.
    Page,
    /// To the page, and on to Slint (modifier keys, so Slint's own
    /// tracking stays right).
    PageAndShell,
    Shell,
}

/// `unmodified` is the key without modifiers, `mods` those held.
pub fn route(page_focused: bool, unmodified: &Key, mods: ModifiersState) -> KeyRoute {
    let modifier_key = matches!(
        unmodified,
        Key::Named(
            NamedKey::Shift
                | NamedKey::Control
                | NamedKey::Alt
                | NamedKey::AltGraph
                | NamedKey::Super
                | NamedKey::Meta
                | NamedKey::Hyper
        )
    );
    let reserved = mods == PRIMARY
        && matches!(unmodified, Key::Character(c)
            if RESERVED_CHORDS.iter().any(|chord| c.eq_ignore_ascii_case(chord)));
    match (page_focused, modifier_key, reserved) {
        (false, _, _) | (true, _, true) => KeyRoute::Shell,
        (true, true, false) => KeyRoute::PageAndShell,
        (true, false, false) => KeyRoute::Page,
    }
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
    use super::*;

    fn ch(c: &str) -> Key {
        Key::Character(c.into())
    }

    #[test]
    fn keys_go_to_the_page_only_while_it_has_focus() {
        let none = ModifiersState::empty();
        assert_eq!(route(false, &ch("a"), none), KeyRoute::Shell);
        assert_eq!(route(true, &ch("a"), none), KeyRoute::Page);
        assert_eq!(
            route(true, &Key::Named(NamedKey::Escape), none),
            KeyRoute::Page
        );
    }

    #[test]
    fn modifier_keys_reach_both() {
        for named in [
            NamedKey::Shift,
            NamedKey::Control,
            NamedKey::Alt,
            NamedKey::Super,
        ] {
            assert_eq!(
                route(true, &Key::Named(named), ModifiersState::empty()),
                KeyRoute::PageAndShell,
                "{named:?}"
            );
            assert_eq!(
                route(false, &Key::Named(named), ModifiersState::empty()),
                KeyRoute::Shell
            );
        }
    }

    #[test]
    fn reserved_chords_stay_with_the_shell() {
        for key in ["t", "l", "w", "T", "W"] {
            assert_eq!(route(true, &ch(key), PRIMARY), KeyRoute::Shell, "{key}");
        }
        // Other chords, and the reserved keys with other modifiers, go to the page.
        assert_eq!(route(true, &ch("c"), PRIMARY), KeyRoute::Page);
        assert_eq!(
            route(true, &ch("t"), PRIMARY | ModifiersState::SHIFT),
            KeyRoute::Page
        );
        assert_eq!(
            route(true, &ch("t"), ModifiersState::empty()),
            KeyRoute::Page
        );
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
}
