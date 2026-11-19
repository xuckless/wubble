//! The edge of the scene's run state (`wubble_gfx::scene::run`): maps winit window events to
//! [`RunEvent`]s and executes the [`RunEffect`]s (frame requests, the idle timer).
//! The rule itself, and what each platform reports, is documented in that module.

use std::time::{Duration, Instant};

use slint::ComponentHandle;
use slint::winit_030::winit::event::WindowEvent;
use wubble_gfx::scene::run::{
    self, IDLE_TICK, Needs, POINTER_HOLD, RunEffect, RunEvent, RunState, Timing,
};
use wubble_ui::AppWindow;

/// The run state of one window's scene and its idle timer.
#[derive(Default)]
pub struct Runner {
    state: RunState,
    idle: slint::Timer,
    last_pointer: Option<Instant>,
    /// The state changed since the last [`Runner::take_changed`].
    changed: bool,
}

/// What a window event means to the scene's run state. `minimised`: the platform's answer to
/// `is_minimized`, where it has one (winit: `None` on Wayland).
pub fn event_of(event: &WindowEvent, minimised: impl FnOnce() -> Option<bool>) -> Option<RunEvent> {
    match event {
        WindowEvent::Occluded(true) => Some(RunEvent::Hidden),
        WindowEvent::Occluded(false) => Some(RunEvent::Shown),
        WindowEvent::Resized(size) if size.width == 0 || size.height == 0 => Some(RunEvent::Hidden),
        WindowEvent::Resized(_) => Some(RunEvent::Shown),
        WindowEvent::Focused(true) => Some(RunEvent::Shown),
        WindowEvent::Focused(false) => (minimised() == Some(true)).then_some(RunEvent::Hidden),
        // every way the user touches the window wakes a frozen liquid
        WindowEvent::CursorMoved { .. }
        | WindowEvent::CursorEntered { .. }
        | WindowEvent::MouseInput { .. }
        | WindowEvent::MouseWheel { .. }
        | WindowEvent::KeyboardInput { .. }
        | WindowEvent::Ime(_)
        | WindowEvent::Touch(_)
        | WindowEvent::PinchGesture { .. }
        | WindowEvent::PanGesture { .. }
        | WindowEvent::RotationGesture { .. }
        | WindowEvent::DoubleTapGesture { .. } => Some(RunEvent::Activity),
        _ => None,
    }
}

impl Runner {
    pub fn state(&self) -> RunState {
        self.state
    }

    /// The longest liquid step of the next frame.
    pub fn max_dt(&self) -> f32 {
        run::max_dt(self.state)
    }

    /// How fast the liquid runs in the next frame, 0 to 1 (`run::pace`).
    pub fn pace(&self) -> f32 {
        run::pace(self.state)
    }

    /// The pointer moved at `now` (it keeps full rate for [`POINTER_HOLD`]).
    pub fn pointer_moved(&mut self, now: Instant) {
        self.last_pointer = Some(now);
    }

    /// After a rendered frame: whether the scene still needs frames at full rate. `since` is the
    /// time since the scene's last frame (`None` for its first); it is capped like the scene's
    /// own step, so a frame after a long gap (a wake from Still) moves the clocks no further than
    /// the liquid moved.
    pub fn frame_done(
        &mut self,
        window: &AppWindow,
        now: Instant,
        since: Option<Duration>,
        scene_animating: bool,
        glass_changed: bool,
    ) {
        let needs = Needs {
            pointer_recent: self
                .last_pointer
                .is_some_and(|t| now.saturating_duration_since(t) < POINTER_HOLD),
            scene_animating,
            glass_changed,
        };
        let dt = since
            .unwrap_or_default()
            .min(Duration::from_secs_f32(self.max_dt()));
        self.on(needs.event(dt), window);
    }

    /// Whether the state changed since the last call: the first frame after a change follows a
    /// gap (an idle tick, a pause), so its interval says nothing about lateness.
    pub fn take_changed(&mut self) -> bool {
        std::mem::take(&mut self.changed)
    }

    /// Takes one event: the new state, and the effect to execute.
    fn step(&mut self, event: RunEvent) -> RunEffect {
        let timing = Timing::of(wubble_core::motion::reduced());
        let (next, effect) = run::transition(self.state, event, &timing);
        if next != self.state {
            self.changed = true;
            crate::measure::count(crate::measure::Count::RunChange);
            if std::env::var_os("WUBBLE_TRACE").is_some() {
                eprintln!(
                    "wubble: scene run state {:?} -> {next:?} on {event:?}",
                    self.state
                );
            }
        }
        self.state = next;
        effect
    }

    /// The scene's own motion started (a beam lap, a palette change) while it dozes, and no input
    /// event says so: wakes it for the frame about to be drawn.
    pub fn wake_if_dozing(&mut self, window: &AppWindow) {
        if run::dozing(self.state) {
            self.on(RunEvent::Activity, window);
        }
    }

    /// Runs one event and executes its effect.
    pub fn on(&mut self, event: RunEvent, window: &AppWindow) {
        match self.step(event) {
            RunEffect::None => {}
            RunEffect::RequestFrame => window.window().request_redraw(),
            RunEffect::StartIdleTimer => {
                let weak = window.as_weak();
                self.idle
                    .start(slint::TimerMode::Repeated, IDLE_TICK, move || {
                        crate::measure::count(crate::measure::Count::IdleTick);
                        if let Some(w) = weak.upgrade() {
                            w.window().request_redraw();
                        }
                    });
            }
            RunEffect::StopIdleTimer => {
                self.idle.stop();
                window.window().request_redraw();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use slint::winit_030::winit::dpi::PhysicalSize;
    use slint::winit_030::winit::event::{DeviceId, MouseScrollDelta, TouchPhase};

    #[test]
    fn a_state_change_is_reported_once() {
        let mut r = Runner::default();
        r.step(RunEvent::Moving(Duration::ZERO));
        assert!(!r.take_changed());
        // Running -> Paused -> Running: back where it was, still a change
        r.step(RunEvent::Hidden);
        r.step(RunEvent::Shown);
        assert_eq!(r.state(), RunState::default());
        assert!(r.take_changed());
        assert!(!r.take_changed());
        r.step(RunEvent::Settled(Duration::ZERO));
        r.step(RunEvent::Activity);
        assert!(r.take_changed());
        // the liquid's slow stop and wake are changes too: the frame after follows a gap
        r.step(RunEvent::Settled(Duration::ZERO));
        r.take_changed();
        r.step(RunEvent::Settled(Duration::from_secs(60)));
        assert!(r.take_changed());
        assert_eq!(r.state(), RunState::Easing { level: 1.0 });
        r.step(RunEvent::Settled(Duration::from_secs(60)));
        assert_eq!(r.state(), RunState::Still);
        r.take_changed();
        assert_eq!(r.step(RunEvent::Activity), RunEffect::RequestFrame);
        assert!(r.take_changed());
    }

    #[test]
    fn window_events_map_to_run_events() {
        let none = || None;
        assert_eq!(
            event_of(&WindowEvent::Occluded(true), none),
            Some(RunEvent::Hidden)
        );
        assert_eq!(
            event_of(&WindowEvent::Occluded(false), none),
            Some(RunEvent::Shown)
        );
        assert_eq!(
            event_of(&WindowEvent::Resized(PhysicalSize::new(0, 0)), none),
            Some(RunEvent::Hidden)
        );
        assert_eq!(
            event_of(&WindowEvent::Resized(PhysicalSize::new(800, 600)), none),
            Some(RunEvent::Shown)
        );
        assert_eq!(
            event_of(&WindowEvent::Focused(true), none),
            Some(RunEvent::Shown)
        );
        // unfocused is hidden only when the platform says it is minimised
        assert_eq!(event_of(&WindowEvent::Focused(false), none), None);
        assert_eq!(
            event_of(&WindowEvent::Focused(false), || Some(true)),
            Some(RunEvent::Hidden)
        );
        assert_eq!(event_of(&WindowEvent::Focused(false), || Some(false)), None);
        assert_eq!(event_of(&WindowEvent::RedrawRequested, none), None);
        for touch in [
            WindowEvent::CursorEntered {
                device_id: DeviceId::dummy(),
            },
            WindowEvent::MouseWheel {
                device_id: DeviceId::dummy(),
                delta: MouseScrollDelta::LineDelta(0.0, 1.0),
                phase: TouchPhase::Moved,
            },
            WindowEvent::PinchGesture {
                device_id: DeviceId::dummy(),
                delta: 0.1,
                phase: TouchPhase::Moved,
            },
        ] {
            assert_eq!(
                event_of(&touch, none),
                Some(RunEvent::Activity),
                "{touch:?}"
            );
        }
    }
}
