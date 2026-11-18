//! When the scene draws: one state machine, so idle CPU and GPU drop when nothing moves.
//!
//! The rule (TODO.md "Decided"):
//! - **Running**: a frame every vsync while something moves: the pointer moved within
//!   [`POINTER_HOLD`], a tap wave or ripple is alive, the beam laps, the palette crossfades, a
//!   film rim eases or the glass list changed ([`Needs`]). The shell driver and the pages
//!   request their own frames while they move; the scene renders with them.
//! - **Idle**: nothing of that. The liquid still drifts (the design: "the liquid always drifts"),
//!   so it redraws from a timer at [`IDLE_HZ`], each frame stepping the liquid by the real time
//!   since the last (up to [`IDLE_MAX_DT`]), so it keeps its speed in coarser steps, as under
//!   `Quality::Cached`.
//! - **Paused**: no frames and no timer. Entered when the window is hidden where the platform
//!   says so: winit's `Occluded(true)` (X11 and macOS; winit 0.30 reports nothing on Wayland and
//!   Windows), a minimised window (`is_minimized`, where known) and a 0×0 resize (Windows'
//!   minimise). Left on `Occluded(false)`, focus, or a non-empty resize. On Wayland a hidden
//!   surface gets no frame callbacks, so the compositor already stops presenting; we do not
//!   claim to detect it.
//!
//! Textures are kept while paused: resuming is one frame, no rebuild.

use std::time::Duration;

/// The idle redraw rate.
pub const IDLE_HZ: u32 = 15;
/// The idle timer's period.
pub const IDLE_TICK: Duration = Duration::from_micros(1_000_000 / IDLE_HZ as u64);
/// Longest liquid step of an idle frame, seconds (within `MAX_BACKDROP_DT`, stable).
pub const IDLE_MAX_DT: f32 = 0.07;
/// How long after the last pointer move the scene keeps full rate (the push trails on).
pub const POINTER_HOLD: Duration = Duration::from_secs(2);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RunState {
    #[default]
    Running,
    Idle,
    Paused,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunEvent {
    /// A frame rendered and something in [`Needs`] still moves.
    Moving,
    /// A frame rendered and nothing in [`Needs`] moves.
    Settled,
    /// The pointer moved, a key or a page asked for a frame: back to full rate at once.
    Activity,
    /// The window is hidden (occluded, minimised, 0×0).
    Hidden,
    /// The window shows again.
    Shown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunEffect {
    None,
    /// Ask the window for the next frame now.
    RequestFrame,
    /// Start the idle timer (it asks for a frame every [`IDLE_TICK`]).
    StartIdleTimer,
    /// Stop the idle timer.
    StopIdleTimer,
}

/// What may keep the scene at full rate. All derived each frame from the scene and the host.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Needs {
    pub pointer_recent: bool,
    /// Tap waves, ripples, the beam lap, the palette crossfade, the light lag, a film rim easing.
    pub scene_animating: bool,
    pub glass_changed: bool,
}

impl Needs {
    pub fn moving(&self) -> bool {
        self.pointer_recent || self.scene_animating || self.glass_changed
    }

    pub fn event(&self) -> RunEvent {
        if self.moving() {
            RunEvent::Moving
        } else {
            RunEvent::Settled
        }
    }
}

pub fn transition(s: RunState, e: RunEvent) -> (RunState, RunEffect) {
    use RunEffect::*;
    use RunEvent::*;
    use RunState::*;
    match (s, e) {
        (Paused, Shown) => (Running, RequestFrame),
        (Paused, _) => (Paused, None),
        (_, Hidden) => (Paused, if s == Idle { StopIdleTimer } else { None }),
        (Running, Moving | Activity | Shown) => (Running, RequestFrame),
        (Running, Settled) => (Idle, StartIdleTimer),
        (Idle, Settled | Shown) => (Idle, None),
        (Idle, Moving | Activity) => (Running, StopIdleTimer),
    }
}

/// The longest step a frame in `s` gives the liquid.
pub fn max_dt(s: RunState) -> f32 {
    match s {
        RunState::Idle => IDLE_MAX_DT,
        RunState::Running | RunState::Paused => super::MAX_DT,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use RunEffect as F;
    use RunEvent::*;
    use RunState::*;

    #[test]
    fn every_state_and_event() {
        let cases = [
            (Running, Moving, Running, F::RequestFrame),
            (Running, Activity, Running, F::RequestFrame),
            (Running, Shown, Running, F::RequestFrame),
            (Running, Settled, Idle, F::StartIdleTimer),
            (Running, Hidden, Paused, F::None),
            (Idle, Settled, Idle, F::None),
            (Idle, Shown, Idle, F::None),
            (Idle, Moving, Running, F::StopIdleTimer),
            (Idle, Activity, Running, F::StopIdleTimer),
            (Idle, Hidden, Paused, F::StopIdleTimer),
            (Paused, Moving, Paused, F::None),
            (Paused, Settled, Paused, F::None),
            (Paused, Activity, Paused, F::None),
            (Paused, Hidden, Paused, F::None),
            (Paused, Shown, Running, F::RequestFrame),
        ];
        for (s, e, next, fx) in cases {
            assert_eq!(transition(s, e), (next, fx), "{s:?} + {e:?}");
        }
    }

    #[test]
    fn paused_never_asks_for_frames() {
        for e in [Moving, Settled, Activity, Hidden] {
            let (s, fx) = transition(Paused, e);
            assert_eq!(s, Paused);
            assert!(!matches!(fx, F::RequestFrame | F::StartIdleTimer));
        }
    }

    #[test]
    fn needs_any_keeps_it_running() {
        assert_eq!(Needs::default().event(), Settled);
        for n in [
            Needs {
                pointer_recent: true,
                ..Needs::default()
            },
            Needs {
                scene_animating: true,
                ..Needs::default()
            },
            Needs {
                glass_changed: true,
                ..Needs::default()
            },
        ] {
            assert_eq!(n.event(), Moving);
        }
    }

    #[test]
    fn idle_steps_are_longer_but_stable() {
        assert!(max_dt(Idle) > max_dt(Running));
        assert!(max_dt(Idle) <= super::super::MAX_BACKDROP_DT);
        assert!(IDLE_TICK.as_secs_f32() <= IDLE_MAX_DT);
    }
}
