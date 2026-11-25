//! Caret browsing (F7): a movable text caret in every page, Chromium's
//! `IDC_CARET_BROWSING_TOGGLE`. The state is Chromium's own profile preferences, read and written
//! by the engine (`View::caret_browsing`): `settings.a11y.caret_browsing.enabled` and
//! `settings.a11y.caretbrowsing.show_dialog`, which Chrome registers in `browser_ui_prefs.cc`
//! and pushes to open pages (`PrefWatcher`, `WebPreferences::caret_browsing_enabled`).
//!
//! As Chrome's `chrome::ToggleCaretBrowsing` (chrome/browser/ui/browser_commands.cc,
//! BSD-3-Clause; behaviour only): turning it off is immediate; turning it on asks first
//! ("Turn on caret browsing?") unless the user said not to ask again. The question is the
//! shell's menu: choosing a row is the confirmation, dismissing it changes nothing.

/// What the preferences say now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Caret {
    pub enabled: bool,
    /// Chrome's `show_dialog`: ask before turning it on.
    pub ask: bool,
}

/// What the user did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    /// F7.
    Toggle,
    /// A row of the question: turn it on, and stop asking when `dont_ask`.
    Confirm { dont_ask: bool },
}

/// What the host does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    /// Write the preferences: caret browsing `enabled`, and `ask` from now on.
    Set { enabled: bool, ask: bool },
    /// Show the question (`menu::caret_entries`).
    Ask,
}

/// The one transition: the preferences after `event`, as the effect that writes them.
pub fn transition(now: Caret, event: Event) -> Option<Effect> {
    match event {
        Event::Toggle if now.enabled => Some(Effect::Set {
            enabled: false,
            ask: now.ask,
        }),
        Event::Toggle if now.ask => Some(Effect::Ask),
        Event::Toggle => Some(Effect::Set {
            enabled: true,
            ask: false,
        }),
        // a late answer once it is on already changes nothing
        Event::Confirm { .. } if now.enabled => None,
        Event::Confirm { dont_ask } => Some(Effect::Set {
            enabled: true,
            ask: now.ask && !dont_ask,
        }),
    }
}
