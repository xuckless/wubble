//! The edge of the scene's run state (`wubble_gfx::scene::run`): maps winit window events to
//! [`RunEvent`]s and executes the [`RunEffect`]s (frame requests, the idle timer).
//! The rule itself, and what each platform reports, is documented in that module.

use std::time::Instant;

use slint::ComponentHandle;
use slint::winit_030::winit::event::WindowEvent;
use wubble_gfx::scene::run::{self, IDLE_TICK, Needs, POINTER_HOLD, RunEffect, RunEvent, RunState};
use wubble_ui::AppWindow;

/// The run state of one window's scene and its idle timer.
pub struct Runner {
    state: RunState,
    idle: slint::Timer,
    last_pointer: Option<Instant>,
}

impl Default for Runner {
    fn default() -> Self {
        Self {
            state: RunState::Running,
            idle: slint::Timer::default(),
            last_pointer: None,
        }
    }
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
        WindowEvent::CursorMoved { .. }
        | WindowEvent::MouseInput { .. }
        | WindowEvent::MouseWheel { .. }
        | WindowEvent::KeyboardInput { .. }
        | WindowEvent::Touch(_) => Some(RunEvent::Activity),
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

    /// The pointer moved at `now` (it keeps full rate for [`POINTER_HOLD`]).
    pub fn pointer_moved(&mut self, now: Instant) {
        self.last_pointer = Some(now);
    }

    /// After a rendered frame: whether the scene still needs frames at full rate.
    pub fn frame_done(
        &mut self,
        window: &AppWindow,
        now: Instant,
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
        self.on(needs.event(), window);
    }

    /// Runs one event and executes its effect.
    pub fn on(&mut self, event: RunEvent, window: &AppWindow) {
        let (next, effect) = run::transition(self.state, event);
        self.state = next;
        match effect {
            RunEffect::None => {}
            RunEffect::RequestFrame => window.window().request_redraw(),
            RunEffect::StartIdleTimer => {
                let weak = window.as_weak();
                self.idle
                    .start(slint::TimerMode::Repeated, IDLE_TICK, move || {
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
    }
}
