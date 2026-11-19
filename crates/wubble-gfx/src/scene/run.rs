//! When the scene draws: one state machine, so idle CPU and GPU drop when nothing moves.
//!
//! The rule (TODO.md "Decided", owner 2026-11-19: "the liquid slowly comes to a freeze, and it's
//! woken up on touch/interactions"):
//! - **Running**: a frame every vsync while something moves: the pointer moved within
//!   [`POINTER_HOLD`], a tap wave or ripple is alive, the beam laps, the palette crossfades, a
//!   film rim eases or the glass list changed ([`Needs`]). The shell driver and the pages
//!   request their own frames while they move; the scene renders with them. After a wake the
//!   liquid's pace climbs from where it was back to 1 over [`Timing::ease_in`] (`level`).
//! - **Idle**: nothing of that. The liquid still drifts, so it redraws from a timer at
//!   [`IDLE_HZ`], each frame stepping the liquid by the real time since the last (up to
//!   [`IDLE_MAX_DT`]), so it keeps its speed in coarser steps, as under `Quality::Cached`. After
//!   [`Timing::still_after`] of this the liquid eases out.
//! - **Easing**: still the idle timer, but the liquid's pace falls to 0 over [`Timing::ease_out`]
//!   (a smoothstep of `level`, so neither end snaps).
//! - **Still**: the liquid has stopped. No scene frames and no timer: only a change of the glass
//!   list or the window's size renders one (frozen), and any [`RunEvent::Activity`] or moving
//!   frame wakes it. The scene's clock continues where it stopped: a step is the real time since
//!   the last frame capped at the state's `max_dt`, scaled by the pace, so the time spent frozen
//!   is not skipped.
//! - **Paused**: no frames and no timer. Entered when the window is hidden where the platform
//!   says so: winit's `Occluded(true)` (X11 and macOS; winit 0.30 reports nothing on Wayland and
//!   Windows), a minimised window (`is_minimized`, where known) and a 0×0 resize (Windows'
//!   minimise). Left on `Occluded(false)`, focus, or a non-empty resize, at full pace. On Wayland
//!   a hidden surface gets no frame callbacks, so the compositor already stops presenting; we do
//!   not claim to detect it.
//!
//! Reduced motion ([`Timing::REDUCED`]) skips the wait and eases out quickly.
//! Textures are kept while paused or still: waking is one frame, no rebuild.

use std::time::Duration;

/// The idle redraw rate.
pub const IDLE_HZ: u32 = 15;
/// The idle timer's period.
pub const IDLE_TICK: Duration = Duration::from_micros(1_000_000 / IDLE_HZ as u64);
/// Longest liquid step of an idle frame, seconds (within `MAX_BACKDROP_DT`, stable).
pub const IDLE_MAX_DT: f32 = 0.07;
/// How long after the last pointer move the scene keeps full rate (the push trails on).
pub const POINTER_HOLD: Duration = Duration::from_secs(2);

/// How long the scene spends in [`RunState::Idle`], and how fast it eases out and back in. Inputs
/// of [`transition`], so the machine reads no clock.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Timing {
    /// Idle time before the liquid starts to ease out.
    pub still_after: Duration,
    /// From full pace to a stop.
    pub ease_out: Duration,
    /// From a stop to full pace, after a wake.
    pub ease_in: Duration,
}

impl Timing {
    /// Five idle seconds, two to stop, 0.6 to come back (owner: "slowly comes to a freeze").
    pub const NORMAL: Self = Self {
        still_after: Duration::from_secs(5),
        ease_out: Duration::from_secs(2),
        ease_in: Duration::from_millis(600),
    };
    /// Reduced motion: no wait, half a second out, a quarter back.
    pub const REDUCED: Self = Self {
        still_after: Duration::ZERO,
        ease_out: Duration::from_millis(500),
        ease_in: Duration::from_millis(250),
    };

    pub const fn of(reduced: bool) -> Self {
        if reduced { Self::REDUCED } else { Self::NORMAL }
    }
}

/// `level`: how far the liquid is up to its pace, 0 (stopped) to 1; [`pace`] shapes it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RunState {
    Running {
        level: f32,
    },
    /// Nothing moves but the drift; `calm` is how long it has been so.
    Idle {
        calm: Duration,
    },
    Easing {
        level: f32,
    },
    Still,
    Paused,
}

impl Default for RunState {
    fn default() -> Self {
        Self::Running { level: 1.0 }
    }
}

/// Each event that carries `dt` is a rendered frame: the time it stepped the scene, capped by
/// [`max_dt`] of the state before it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunEvent {
    /// A frame rendered and something in [`Needs`] still moves.
    Moving(Duration),
    /// A frame rendered and nothing in [`Needs`] moves.
    Settled(Duration),
    /// The pointer moved, a key, a touch or a scroll came, or something started to move: back to
    /// full rate at once.
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

    /// The event of a frame that stepped the scene by `dt`.
    pub fn event(&self, dt: Duration) -> RunEvent {
        if self.moving() {
            RunEvent::Moving(dt)
        } else {
            RunEvent::Settled(dt)
        }
    }
}

/// The fraction of a ramp of length `over` that `dt` covers (a zero ramp is done at once).
fn fraction(dt: Duration, over: Duration) -> f32 {
    if over.is_zero() {
        1.0
    } else {
        dt.as_secs_f32() / over.as_secs_f32()
    }
}

pub fn transition(s: RunState, e: RunEvent, t: &Timing) -> (RunState, RunEffect) {
    use RunEffect::*;
    use RunEvent::*;
    use RunState::*;
    let up = |level: f32, dt| (level + fraction(dt, t.ease_in)).min(1.0);
    match (s, e) {
        (Paused, Shown) => (RunState::default(), RequestFrame),
        (Paused, _) => (Paused, None),
        (Idle { .. } | Easing { .. }, Hidden) => (Paused, StopIdleTimer),
        (_, Hidden) => (Paused, None),

        (Running { level }, Moving(dt)) => (
            Running {
                level: up(level, dt),
            },
            RequestFrame,
        ),
        (Running { level }, Activity | Shown) => (Running { level }, RequestFrame),
        // quiet, but the liquid is still coming up to its pace: frames until it is
        (Running { level }, Settled(dt)) => match up(level, dt) {
            l if l < 1.0 => (Running { level: l }, RequestFrame),
            _ => (
                Idle {
                    calm: Duration::ZERO,
                },
                StartIdleTimer,
            ),
        },

        (Idle { calm }, Settled(dt)) => match calm + dt {
            c if c < t.still_after => (Idle { calm: c }, None),
            _ => (Easing { level: 1.0 }, None),
        },
        (Idle { .. }, Shown) => (s, None),
        (Idle { .. }, Moving(_) | Activity) => (RunState::default(), StopIdleTimer),

        (Easing { level }, Settled(dt)) => match level - fraction(dt, t.ease_out) {
            l if l > 0.0 => (Easing { level: l }, None),
            _ => (Still, StopIdleTimer),
        },
        (Easing { .. }, Shown) => (s, None),
        (Easing { level }, Moving(_) | Activity) => (Running { level }, StopIdleTimer),

        // a frame rendered in Still was a change (see `scene_due`), so it is Moving; a Settled
        // one changes nothing
        (Still, Settled(_) | Shown) => (Still, None),
        (Still, Moving(_) | Activity) => (Running { level: 0.0 }, RequestFrame),
    }
}

/// How fast the liquid runs in `s`, 0 to 1: a smoothstep of its level, so a freeze and a wake
/// have no snap. The scene scales the time of its drift and its fluid step by it.
pub fn pace(s: RunState) -> f32 {
    match s {
        RunState::Running { level } | RunState::Easing { level } => {
            let l = level.clamp(0.0, 1.0);
            l * l * (3.0 - 2.0 * l)
        }
        RunState::Idle { .. } | RunState::Paused => 1.0,
        RunState::Still => 0.0,
    }
}

/// Whether the scene may be drifting on its own and needs an interaction or a start of motion
/// to be at full rate again.
pub fn dozing(s: RunState) -> bool {
    matches!(
        s,
        RunState::Idle { .. } | RunState::Easing { .. } | RunState::Still
    )
}

/// Whether a window render also renders the scene. The window renders for more than the scene
/// (a page's new frame, the shell's springs), and the glass never samples the page, so a render
/// in [`RunState::Idle`] or [`RunState::Easing`] steps the scene only when its idle frame is
/// due: `since_last` (since the scene's last frame, `None` before the first) reached half an
/// [`IDLE_TICK`], so the idle timer's own render is never skipped and page frames cannot drive
/// the liquid faster than twice the idle rate. `changed` (the glass list, the window's size)
/// always renders, also in [`RunState::Still`], where nothing else does: a page's frames (a
/// video) do not wake the liquid. `covered` (the page fills the window: page fullscreen) and
/// [`RunState::Paused`] never do; the last scene image stays.
pub fn scene_due(
    state: RunState,
    covered: bool,
    changed: bool,
    since_last: Option<Duration>,
) -> bool {
    match state {
        _ if covered => false,
        RunState::Paused => false,
        RunState::Running { .. } => true,
        RunState::Idle { .. } | RunState::Easing { .. } => {
            changed || since_last.is_none_or(|d| d >= IDLE_TICK / 2)
        }
        RunState::Still => changed,
    }
}

/// The longest step a frame in `s` gives the liquid.
pub fn max_dt(s: RunState) -> f32 {
    match s {
        RunState::Idle { .. } | RunState::Easing { .. } => IDLE_MAX_DT,
        RunState::Running { .. } | RunState::Still | RunState::Paused => super::MAX_DT,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use RunEffect as F;
    use RunEvent::*;
    use RunState::*;

    const T: Timing = Timing {
        still_after: Duration::from_secs(4),
        ease_out: Duration::from_secs(2),
        ease_in: Duration::from_secs(1),
    };
    const SEC: Duration = Duration::from_secs(1);
    const HALF: Duration = Duration::from_millis(500);
    const UP: RunState = Running { level: 1.0 };

    fn calm(secs: u64) -> RunState {
        Idle {
            calm: Duration::from_secs(secs),
        }
    }

    fn go(s: RunState, e: RunEvent) -> (RunState, RunEffect) {
        transition(s, e, &T)
    }

    #[test]
    fn every_state_and_event() {
        let cases = [
            (UP, Moving(SEC), UP, F::RequestFrame),
            (UP, Activity, UP, F::RequestFrame),
            (UP, Shown, UP, F::RequestFrame),
            (UP, Settled(SEC), calm(0), F::StartIdleTimer),
            (UP, Hidden, Paused, F::None),
            (calm(1), Settled(SEC), calm(2), F::None),
            (calm(1), Shown, calm(1), F::None),
            (calm(1), Moving(SEC), UP, F::StopIdleTimer),
            (calm(1), Activity, UP, F::StopIdleTimer),
            (calm(1), Hidden, Paused, F::StopIdleTimer),
            (
                Easing { level: 0.75 },
                Settled(HALF),
                Easing { level: 0.5 },
                F::None,
            ),
            (Easing { level: 0.5 }, Shown, Easing { level: 0.5 }, F::None),
            (
                Easing { level: 0.5 },
                Moving(SEC),
                Running { level: 0.5 },
                F::StopIdleTimer,
            ),
            (
                Easing { level: 0.5 },
                Activity,
                Running { level: 0.5 },
                F::StopIdleTimer,
            ),
            (Easing { level: 0.5 }, Hidden, Paused, F::StopIdleTimer),
            (Still, Settled(SEC), Still, F::None),
            (Still, Shown, Still, F::None),
            (Still, Moving(SEC), Running { level: 0.0 }, F::RequestFrame),
            (Still, Activity, Running { level: 0.0 }, F::RequestFrame),
            (Still, Hidden, Paused, F::None),
            (Paused, Moving(SEC), Paused, F::None),
            (Paused, Settled(SEC), Paused, F::None),
            (Paused, Activity, Paused, F::None),
            (Paused, Hidden, Paused, F::None),
            (Paused, Shown, UP, F::RequestFrame),
        ];
        for (s, e, next, fx) in cases {
            assert_eq!(go(s, e), (next, fx), "{s:?} + {e:?}");
        }
    }

    #[test]
    fn idle_eases_out_after_the_wait_and_then_stops() {
        // not before `still_after`, then the ease starts at full level
        let (s, fx) = go(calm(3), Settled(HALF));
        assert_eq!(fx, F::None);
        assert_eq!(
            s,
            Idle {
                calm: Duration::from_millis(3500)
            }
        );
        assert_eq!(go(calm(3), Settled(SEC)), (Easing { level: 1.0 }, F::None));
        assert_eq!(
            go(calm(3), Settled(2 * SEC)),
            (Easing { level: 1.0 }, F::None)
        );
        // the ease runs its length, then the timer goes
        let mut s = Easing { level: 1.0 };
        for _ in 0..3 {
            let (next, fx) = go(s, Settled(HALF));
            assert!(matches!(next, Easing { .. }) && fx == F::None, "{next:?}");
            s = next;
        }
        assert_eq!(s, Easing { level: 0.25 });
        assert_eq!(go(s, Settled(HALF)), (Still, F::StopIdleTimer));
        // a long frame (a suspended machine) ends it, never overshoots
        assert_eq!(
            go(Easing { level: 0.5 }, Settled(60 * SEC)),
            (Still, F::StopIdleTimer)
        );
    }

    #[test]
    fn a_wake_ramps_up_and_keeps_asking_for_frames_until_it_is_there() {
        // from a stop: one frame at pace 0, then up over `ease_in`, even with nothing moving
        let (mut s, fx) = go(Still, Activity);
        assert_eq!((s, fx), (Running { level: 0.0 }, F::RequestFrame));
        assert_eq!(pace(s), 0.0);
        s = go(s, Moving(HALF)).0;
        assert_eq!(s, Running { level: 0.5 });
        // quiet, and not there yet: frames go on
        assert_eq!(
            go(s, Settled(HALF / 2)),
            (Running { level: 0.75 }, F::RequestFrame)
        );
        // there: the idle wait starts from nothing
        assert_eq!(go(s, Settled(HALF)), (calm(0), F::StartIdleTimer));
    }

    #[test]
    fn a_wake_mid_ease_continues_from_the_level_it_had() {
        let (s, _) = go(Easing { level: 0.5 }, Activity);
        assert_eq!(s, Running { level: 0.5 });
        assert_eq!(pace(s), 0.5);
        // the ramp is the same one: half a second more is half way to full
        assert_eq!(go(s, Moving(HALF)).0, UP);
    }

    #[test]
    fn a_zero_ramp_is_done_at_once() {
        let t = Timing {
            still_after: Duration::ZERO,
            ease_out: Duration::ZERO,
            ease_in: Duration::ZERO,
        };
        assert_eq!(
            transition(calm(0), Settled(Duration::ZERO), &t).0,
            Easing { level: 1.0 }
        );
        assert_eq!(
            transition(Easing { level: 1.0 }, Settled(Duration::ZERO), &t),
            (Still, F::StopIdleTimer)
        );
        assert_eq!(
            transition(Running { level: 0.0 }, Moving(Duration::ZERO), &t).0,
            UP
        );
    }

    #[test]
    fn reduced_motion_goes_still_at_the_first_quiet_frame_and_in_a_blink() {
        let t = Timing::of(true);
        assert!(t.still_after.is_zero() && t.ease_out <= SEC);
        let (s, _) = transition(calm(0), Settled(IDLE_TICK), &t);
        assert_eq!(s, Easing { level: 1.0 });
        let mut s = s;
        let mut frames = 0;
        while s != Still {
            s = transition(s, Settled(IDLE_TICK), &t).0;
            frames += 1;
            assert!(frames < 100);
        }
        assert_eq!(Timing::of(false), Timing::NORMAL);
    }

    #[test]
    fn the_real_timing_reaches_still_after_seven_quiet_seconds() {
        // 15 Hz ticks of real time: the sum is the wait plus the ease (a count, not a clock)
        let mut s = UP;
        let mut ticks = 0u32;
        while s != Still {
            s = transition(s, Settled(IDLE_TICK), &Timing::NORMAL).0;
            ticks += 1;
            assert!(ticks < 10_000, "{s:?}");
        }
        let total = IDLE_TICK * ticks;
        let want = Timing::NORMAL.still_after + Timing::NORMAL.ease_out;
        assert!(total >= want && total < want + 3 * IDLE_TICK, "{total:?}");
    }

    #[test]
    fn pace_has_no_snap_at_either_end() {
        assert_eq!(pace(Running { level: 1.0 }), 1.0);
        assert_eq!(pace(Running { level: 0.0 }), 0.0);
        assert_eq!(pace(Easing { level: 0.5 }), 0.5);
        assert_eq!(pace(calm(2)), 1.0);
        assert_eq!(pace(Still), 0.0);
        // monotone, and flat at the ends (the first and last steps are small)
        let steps: Vec<f32> = (0..=10)
            .map(|i| {
                pace(Easing {
                    level: i as f32 / 10.0,
                })
            })
            .collect();
        assert!(steps.windows(2).all(|w| w[0] < w[1]));
        assert!(steps[1] < 0.03 && steps[10] - steps[9] < 0.03);
        // out of range stays in range
        assert_eq!(pace(Running { level: 3.0 }), 1.0);
    }

    #[test]
    fn only_dozing_states_need_a_wake() {
        assert!(dozing(calm(0)) && dozing(Easing { level: 0.3 }) && dozing(Still));
        assert!(!dozing(UP) && !dozing(Paused));
    }

    #[test]
    fn paused_never_asks_for_frames() {
        for e in [Moving(SEC), Settled(SEC), Activity, Hidden] {
            let (s, fx) = go(Paused, e);
            assert_eq!(s, Paused);
            assert!(!matches!(fx, F::RequestFrame | F::StartIdleTimer));
        }
    }

    #[test]
    fn still_runs_no_timer_and_asks_for_nothing_by_itself() {
        for e in [Settled(SEC), Shown] {
            assert_eq!(go(Still, e), (Still, F::None));
        }
    }

    #[test]
    fn needs_any_keeps_it_running() {
        assert_eq!(Needs::default().event(SEC), Settled(SEC));
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
            assert_eq!(n.event(SEC), Moving(SEC));
        }
    }

    #[test]
    fn the_scene_renders_only_when_due() {
        let ms = |n| Some(Duration::from_millis(n));
        let half = IDLE_TICK / 2;
        let easing = Easing { level: 0.5 };
        let cases = [
            // (state, covered, changed, since_last, due)
            (UP, false, false, ms(1), true),
            (UP, false, true, ms(1), true),
            (UP, false, false, None, true),
            (calm(0), false, false, None, true),
            (calm(0), false, false, ms(1), false),
            (
                calm(0),
                false,
                false,
                Some(half - Duration::from_micros(1)),
                false,
            ),
            (calm(0), false, false, Some(half), true),
            (calm(0), false, false, Some(IDLE_TICK), true),
            (calm(0), false, true, ms(1), true),
            (easing, false, false, ms(1), false),
            (easing, false, false, Some(IDLE_TICK), true),
            (easing, false, true, ms(1), true),
            // Still: only a change; the page's frames and the time never
            (Still, false, false, None, false),
            (Still, false, false, ms(1), false),
            (Still, false, false, Some(60 * SEC), false),
            (Still, false, true, Some(60 * SEC), true),
            (Still, true, true, None, false),
            (Paused, false, false, None, false),
            (Paused, false, true, Some(IDLE_TICK), false),
            (UP, true, false, ms(1), false),
            (calm(0), true, true, Some(IDLE_TICK), false),
            (UP, true, true, None, false),
        ];
        for (state, covered, changed, since, due) in cases {
            assert_eq!(
                scene_due(state, covered, changed, since),
                due,
                "{state:?} covered={covered} changed={changed} {since:?}"
            );
        }
    }

    #[test]
    fn idle_steps_are_longer_but_stable() {
        assert!(max_dt(calm(0)) > max_dt(UP));
        assert_eq!(max_dt(calm(0)), max_dt(Easing { level: 0.5 }));
        assert!(max_dt(calm(0)) <= super::super::MAX_BACKDROP_DT);
        assert!(IDLE_TICK.as_secs_f32() <= IDLE_MAX_DT);
    }
}
