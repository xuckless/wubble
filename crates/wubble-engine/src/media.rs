//! Fullscreen and mute.
//!
//! Fullscreen: CEF reports the page entering and leaving it
//! (`OnFullscreenModeChange`). Windowless rendering is Alloy style, so the
//! host makes the transition itself (grows the view) and must call
//! `View::exit_fullscreen` on Esc (CEF `ExitFullscreen`).
//!
//! Audio: only mute. CEF 156 has no audible-changed callback, and its audio
//! capture (`AudioHandler`) silences the page's own playback while it runs,
//! so it cannot stand in for one (see TODO.md).

use crate::page::PageEvent;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Fullscreen {
    #[default]
    Off,
    On,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FullscreenInput {
    /// From CEF.
    Changed(bool),
    /// From the host (Esc, the shell's own exit).
    Exit,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MediaEffect {
    Emit(PageEvent),
    ExitFullscreen,
    SetMuted(bool),
}

pub fn fullscreen(state: Fullscreen, input: FullscreenInput) -> (Fullscreen, Vec<MediaEffect>) {
    use {Fullscreen::*, FullscreenInput::*};
    match (state, input) {
        (Off, Changed(true)) => (
            On,
            vec![MediaEffect::Emit(PageEvent::FullscreenChanged(true))],
        ),
        (On, Changed(false)) => (
            Off,
            vec![MediaEffect::Emit(PageEvent::FullscreenChanged(false))],
        ),
        // CEF reports the exit as Changed(false).
        (On, Exit) => (On, vec![MediaEffect::ExitFullscreen]),
        _ => (state, vec![]),
    }
}

/// The host's mute: (now muted, effects) for a request to be `muted`.
pub fn mute(state: bool, muted: bool) -> (bool, Vec<MediaEffect>) {
    if muted == state {
        return (state, vec![]);
    }
    (
        muted,
        vec![
            MediaEffect::SetMuted(muted),
            MediaEffect::Emit(PageEvent::MutedChanged(muted)),
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::{FullscreenInput::*, MediaEffect::*, *};

    #[test]
    fn fullscreen_reports_changes_only() {
        assert_eq!(
            fullscreen(Fullscreen::Off, Changed(false)),
            (Fullscreen::Off, vec![])
        );
        assert_eq!(
            fullscreen(Fullscreen::Off, Changed(true)),
            (
                Fullscreen::On,
                vec![Emit(PageEvent::FullscreenChanged(true))]
            )
        );
        assert_eq!(
            fullscreen(Fullscreen::On, Changed(true)),
            (Fullscreen::On, vec![])
        );
        assert_eq!(
            fullscreen(Fullscreen::On, Changed(false)),
            (
                Fullscreen::Off,
                vec![Emit(PageEvent::FullscreenChanged(false))]
            )
        );
    }

    #[test]
    fn exit_is_asked_only_while_fullscreen() {
        assert_eq!(fullscreen(Fullscreen::Off, Exit), (Fullscreen::Off, vec![]));
        assert_eq!(
            fullscreen(Fullscreen::On, Exit),
            (Fullscreen::On, vec![ExitFullscreen])
        );
    }

    #[test]
    fn mute_reports_and_applies_changes_only() {
        assert_eq!(mute(false, false), (false, vec![]));
        assert_eq!(
            mute(false, true),
            (
                true,
                vec![
                    MediaEffect::SetMuted(true),
                    Emit(PageEvent::MutedChanged(true))
                ]
            )
        );
        assert_eq!(mute(true, true), (true, vec![]));
    }
}
