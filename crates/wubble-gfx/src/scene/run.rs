//! When the scene draws: one state machine, so idle CPU and GPU drop when nothing moves.
//!
//! The rule (TODO.md "Decided", owner 2026-11-19: the browser window's backdrop is still, "we
//! dont have to have a water wave effect in the navbar"):
//! - **Running**: a frame every vsync while something moves: a tap wave or ripple, the beam's
//!   lap, the palette crossfade, the light catching up with the pointer, a film rim easing or a
//!   changed glass list ([`Needs`]). The shell driver and the pages request their own frames
//!   while they move; the scene renders with them.
//! - **Still**: nothing moves. No scene frames and no timer: only a change of the glass list or
//!   of the window's size renders one, and any [`RunEvent::Activity`] (input, or the scene's own
//!   motion starting) or moving frame wakes it.
//! - **Paused**: no frames. Entered when the window is hidden where the platform says so:
//!   winit's `Occluded(true)` (X11 and macOS; winit 0.30 reports nothing on Wayland and
//!   Windows), a minimised window (`is_minimized`, where known) and a 0×0 resize (Windows'
//!   minimise). Left on `Occluded(false)`, focus, or a non-empty resize. On Wayland a hidden
//!   surface gets no frame callbacks, so the compositor already stops presenting; we do not
//!   claim to detect it.
//!
//! Textures are kept while paused or still: waking is one frame, no rebuild. The design's living
//! liquid (`Backdrop::Liquid`) is not run by this machine; the window's backdrop is still.

/// The scene's run state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RunState {
    #[default]
    Running,
    Still,
    Paused,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunEvent {
    /// A frame rendered and something in [`Needs`] still moves.
    Moving,
    /// A frame rendered and nothing in [`Needs`] moves.
    Settled,
    /// The pointer moved, a key, a touch or a scroll came, or the scene's own motion started
    /// (a beam lap, a Space's palette): frames again at once.
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
}

/// What keeps the scene running. Derived each frame from the scene and the host.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Needs {
    /// Tap waves, ripples, the beam lap, the palette crossfade, the light lag, a film rim easing.
    pub scene_animating: bool,
    pub glass_changed: bool,
}

impl Needs {
    pub fn moving(&self) -> bool {
        self.scene_animating || self.glass_changed
    }

    /// The event of a rendered frame.
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
        (Paused, _) | (_, Hidden) => (Paused, None),
        (Running, Settled) => (Still, None),
        (Running | Still, Moving | Activity) | (Running, Shown) => (Running, RequestFrame),
        // a frame rendered in Still was a change (see `scene_due`) that left nothing moving
        (Still, Settled | Shown) => (Still, None),
    }
}

/// Whether a window render also renders the scene. The window renders for more than the scene
/// (a page's new frame, the shell's springs), and the glass never samples the page: in
/// [`RunState::Still`] only `changed` (the glass list, the window's size) renders it, so a page's
/// frames (a video) do not. `covered` (the page fills the window: page fullscreen) and
/// [`RunState::Paused`] never do; the last scene image stays.
pub fn scene_due(state: RunState, covered: bool, changed: bool) -> bool {
    match state {
        _ if covered => false,
        RunState::Paused => false,
        RunState::Running => true,
        RunState::Still => changed,
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
            (Running, Settled, Still, F::None),
            (Running, Hidden, Paused, F::None),
            (Still, Moving, Running, F::RequestFrame),
            (Still, Activity, Running, F::RequestFrame),
            (Still, Settled, Still, F::None),
            (Still, Shown, Still, F::None),
            (Still, Hidden, Paused, F::None),
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
            assert_eq!(transition(Paused, e), (Paused, F::None));
        }
    }

    #[test]
    fn still_asks_for_nothing_by_itself() {
        for e in [Settled, Shown] {
            assert_eq!(transition(Still, e), (Still, F::None));
        }
    }

    #[test]
    fn a_quiet_frame_stops_and_input_wakes() {
        let (s, fx) = transition(Running, Needs::default().event());
        assert_eq!((s, fx), (Still, F::None));
        assert_eq!(transition(s, Activity), (Running, F::RequestFrame));
    }

    #[test]
    fn needs_any_keeps_it_running() {
        assert_eq!(Needs::default().event(), Settled);
        for n in [
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
    fn the_scene_renders_only_when_due() {
        let cases = [
            // (state, covered, changed, due)
            (Running, false, false, true),
            (Running, false, true, true),
            // Still: only a change; the page's frames never
            (Still, false, false, false),
            (Still, false, true, true),
            (Paused, false, false, false),
            (Paused, false, true, false),
            // covered: never
            (Running, true, false, false),
            (Running, true, true, false),
            (Still, true, true, false),
        ];
        for (state, covered, changed, due) in cases {
            assert_eq!(
                scene_due(state, covered, changed),
                due,
                "{state:?} covered={covered} changed={changed}"
            );
        }
    }
}
