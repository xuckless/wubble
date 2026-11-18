//! Fullscreen and audio.
//!
//! Fullscreen: CEF reports the page entering and leaving it
//! (`OnFullscreenModeChange`). Windowless rendering is Alloy style, so the
//! host makes the transition itself (grows the view) and must call
//! `View::exit_fullscreen` on Esc (CEF `ExitFullscreen`).
//!
//! Audio: CEF has no audible-changed callback. welding returns true from
//! `AudioHandler::GetAudioParameters`, which makes Alloy start a loopback
//! capture while the page is audible and stop it after a stretch of silence
//! (`alloy_browser_host_impl.cc`, `kRecentlyAudibleTimeout`); the stream
//! start/stop is the debounced audible signal, so no second debounce here.

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

/// Audio streams CEF is capturing, and the host's mute.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Audio {
    pub streams: u32,
    pub muted: bool,
}

impl Audio {
    pub fn audible(self) -> bool {
        self.streams > 0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AudioInput {
    StreamStarted,
    StreamStopped,
    SetMuted(bool),
}

pub fn audio(state: Audio, input: AudioInput) -> (Audio, Vec<MediaEffect>) {
    use AudioInput::*;
    match input {
        StreamStarted => {
            let next = Audio {
                streams: state.streams.saturating_add(1),
                ..state
            };
            let effects = if state.audible() {
                vec![]
            } else {
                vec![MediaEffect::Emit(PageEvent::AudibleChanged(true))]
            };
            (next, effects)
        }
        StreamStopped => {
            let next = Audio {
                streams: state.streams.saturating_sub(1),
                ..state
            };
            let effects = if state.audible() && !next.audible() {
                vec![MediaEffect::Emit(PageEvent::AudibleChanged(false))]
            } else {
                vec![]
            };
            (next, effects)
        }
        SetMuted(muted) if muted != state.muted => (
            Audio { muted, ..state },
            vec![
                MediaEffect::SetMuted(muted),
                MediaEffect::Emit(PageEvent::MutedChanged(muted)),
            ],
        ),
        SetMuted(_) => (state, vec![]),
    }
}

#[cfg(test)]
mod tests {
    use super::{AudioInput::*, FullscreenInput::*, MediaEffect::*, *};

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

    fn run(inputs: &[AudioInput]) -> (Audio, Vec<MediaEffect>) {
        inputs
            .iter()
            .fold((Audio::default(), vec![]), |(s, mut all), &i| {
                let (n, e) = audio(s, i);
                all.extend(e);
                (n, all)
            })
    }

    #[test]
    fn audible_follows_the_stream_count() {
        let (state, effects) = run(&[StreamStarted, StreamStarted, StreamStopped, StreamStopped]);
        assert_eq!(state, Audio::default());
        assert_eq!(
            effects,
            vec![
                Emit(PageEvent::AudibleChanged(true)),
                Emit(PageEvent::AudibleChanged(false))
            ]
        );
    }

    #[test]
    fn a_stop_without_a_start_is_ignored() {
        let (state, effects) = run(&[StreamStopped, StreamStarted]);
        assert_eq!(state.streams, 1);
        assert_eq!(effects, vec![Emit(PageEvent::AudibleChanged(true))]);
    }

    #[test]
    fn mute_reports_and_applies_changes_only() {
        let (state, effects) = run(&[
            AudioInput::SetMuted(false),
            AudioInput::SetMuted(true),
            AudioInput::SetMuted(true),
        ]);
        assert!(state.muted);
        assert_eq!(
            effects,
            vec![
                MediaEffect::SetMuted(true),
                Emit(PageEvent::MutedChanged(true))
            ]
        );
    }
}
