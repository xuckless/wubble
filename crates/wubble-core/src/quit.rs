//! "Warn before quitting" (`settings::QUIT_WARN`): the quit keys quit only when pressed twice or
//! held. One enum with a pure [`transition`].
//!
//! Chrome on macOS, behaviour ported, no code copied: `ConfirmQuitPanelController`
//! (chrome/browser/ui/cocoa/confirm_quit_panel_controller.mm, BSD-3-Clause) shows "Hold ⌘Q to
//! Quit" on the first Cmd+Q and quits when the keys are held or pressed again soon after
//! (`kTimeDeltaFuzzFactor`). Holding the keys arrives here as key repeats, a second press each.
//! Only the keys are guarded: the menu's Quit, the Dock's, logout and the like quit at once.

use std::time::{Duration, Instant};

/// A second press this soon after the first quits.
pub const AGAIN_WITHIN: Duration = Duration::from_millis(2_000);

/// A press closer than this to the first is the same press seen twice (the key and the app
/// menu's key equivalent), not a second one. Key repeats come later than this.
pub const SAME_PRESS: Duration = Duration::from_millis(100);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum QuitGuard {
    #[default]
    Idle,
    /// The keys were pressed once, at this time: the warning shows.
    Armed(Instant),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuitEffect {
    Quit,
    /// Show the warning ([`warning`]).
    Warn,
    Nothing,
}

/// The quit keys were pressed at `now`; `warn` is the setting.
pub fn transition(guard: QuitGuard, now: Instant, warn: bool) -> (QuitGuard, QuitEffect) {
    if !warn {
        return (QuitGuard::Idle, QuitEffect::Quit);
    }
    match guard {
        QuitGuard::Armed(at) if now.saturating_duration_since(at) < SAME_PRESS => {
            (guard, QuitEffect::Nothing)
        }
        QuitGuard::Armed(at) if now.saturating_duration_since(at) <= AGAIN_WITHIN => {
            (QuitGuard::Idle, QuitEffect::Quit)
        }
        QuitGuard::Armed(_) | QuitGuard::Idle => (QuitGuard::Armed(now), QuitEffect::Warn),
    }
}

/// The warning for the quit chord as shown (`Cmd+Q`, `Ctrl+Shift+Q`).
pub fn warning(chord: &str) -> String {
    format!("Press {chord} again or hold it to quit")
}
