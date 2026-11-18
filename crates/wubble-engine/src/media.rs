//! Fullscreen and mute.
//!
//! Fullscreen: CEF reports the page entering and leaving it
//! (`OnFullscreenModeChange`). Windowless rendering is Alloy style, so the
//! host makes the transition itself (grows the view) and must call
//! `View::exit_fullscreen` on Esc (CEF `ExitFullscreen`).
//!
//! Audio: mute, plus a probe for "audible". CEF 156 has no audible-changed
//! callback, and its audio capture (`AudioHandler`) silences the page's own
//! playback while it runs, so it cannot stand in for one (see TODO.md). The
//! probe asks the page's main frame through a script ([`AUDIBLE_PROBE`]);
//! it sees `<audio>` and `<video>` in the main document only, not WebAudio,
//! shadow DOM or iframes.

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

/// True while a media element of the main document plays with sound.
pub const AUDIBLE_PROBE: &str = "[...document.querySelectorAll('audio,video')].some(m => \
    !m.paused && !m.ended && !m.muted && m.volume > 0 && m.readyState > 2)";

/// The audible probe: at most one script in flight per view, so a slow page
/// is never asked twice; `audible` is the last answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AudioProbe {
    Idle { audible: bool },
    Waiting { id: u64, audible: bool },
}

impl Default for AudioProbe {
    fn default() -> Self {
        Self::Idle { audible: false }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProbeInput {
    /// The script `id` was sent.
    Sent(u64),
    /// A script finished: `Some(audible)`, or `None` when it threw or its
    /// result was not a boolean.
    Answered { id: u64, audible: Option<bool> },
    /// The main frame's address changed: a reply in flight may have been
    /// dropped with the old document (welding fails pending scripts only
    /// when the browser closes or the renderer exits), so stop waiting for
    /// it. A late reply is then ignored by its id.
    Navigated,
}

impl AudioProbe {
    /// Whether a new probe may be sent.
    pub fn idle(self) -> bool {
        matches!(self, Self::Idle { .. })
    }
}

/// Emits `PageEvent::Audible` only when the answer changes. Answers to
/// other scripts (a different id) are ignored.
pub fn probe(state: AudioProbe, input: ProbeInput) -> (AudioProbe, Option<PageEvent>) {
    use AudioProbe::*;
    match (state, input) {
        (Idle { audible }, ProbeInput::Sent(id)) => (Waiting { id, audible }, None),
        (
            Waiting { id, audible },
            ProbeInput::Answered {
                id: got,
                audible: answer,
            },
        ) if id == got => {
            let now = answer.unwrap_or(audible);
            let event = (now != audible).then_some(PageEvent::Audible(now));
            (Idle { audible: now }, event)
        }
        (Waiting { audible, .. }, ProbeInput::Navigated) => (Idle { audible }, None),
        (state, _) => (state, None),
    }
}

#[cfg(test)]
mod tests {
    use super::{FullscreenInput::*, MediaEffect::*, *};

    #[test]
    fn the_probe_never_overlaps_and_reports_changes_only() {
        let s = AudioProbe::default();
        let (s, e) = probe(s, ProbeInput::Sent(1));
        assert_eq!(
            (s, e),
            (
                AudioProbe::Waiting {
                    id: 1,
                    audible: false
                },
                None
            )
        );
        assert!(!s.idle());
        // A second send while waiting changes nothing.
        assert_eq!(probe(s, ProbeInput::Sent(2)).0, s);
        // Another script's answer is not ours.
        let other = ProbeInput::Answered {
            id: 7,
            audible: Some(true),
        };
        assert_eq!(probe(s, other), (s, None));
        let (s, e) = probe(
            s,
            ProbeInput::Answered {
                id: 1,
                audible: Some(true),
            },
        );
        assert_eq!(
            (s, e),
            (
                AudioProbe::Idle { audible: true },
                Some(PageEvent::Audible(true))
            )
        );
        let (s, _) = probe(s, ProbeInput::Sent(3));
        let (s, e) = probe(
            s,
            ProbeInput::Answered {
                id: 3,
                audible: Some(true),
            },
        );
        assert_eq!((s, e), (AudioProbe::Idle { audible: true }, None));
        // A failed script keeps the last answer.
        let (s, _) = probe(s, ProbeInput::Sent(4));
        let (s, e) = probe(
            s,
            ProbeInput::Answered {
                id: 4,
                audible: None,
            },
        );
        assert_eq!((s, e), (AudioProbe::Idle { audible: true }, None));
        // An answer arriving while idle is stale.
        let late = ProbeInput::Answered {
            id: 4,
            audible: Some(false),
        };
        assert_eq!(probe(s, late), (s, None));
    }

    #[test]
    fn navigating_while_waiting_frees_the_probe() {
        let (s, _) = probe(AudioProbe::Idle { audible: true }, ProbeInput::Sent(1));
        // The reply was dropped with the old document: the probe is free
        // again and keeps the last answer.
        let (s, e) = probe(s, ProbeInput::Navigated);
        assert_eq!((s, e), (AudioProbe::Idle { audible: true }, None));
        assert!(s.idle());
        // The next probe is answered; the orphan's late reply is ignored.
        let (s, _) = probe(s, ProbeInput::Sent(2));
        let orphan = ProbeInput::Answered {
            id: 1,
            audible: Some(true),
        };
        assert_eq!(probe(s, orphan), (s, None));
        let (s, e) = probe(
            s,
            ProbeInput::Answered {
                id: 2,
                audible: Some(false),
            },
        );
        assert_eq!(
            (s, e),
            (
                AudioProbe::Idle { audible: false },
                Some(PageEvent::Audible(false))
            )
        );
        // Navigating while idle changes nothing.
        assert_eq!(probe(s, ProbeInput::Navigated), (s, None));
    }

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
