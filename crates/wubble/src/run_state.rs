//! The edge of the scene's run state (`wubble_gfx::scene::run`): maps winit window events to
//! [`RunEvent`]s and executes the [`RunEffect`]s (frame requests).
//! The rule itself, and what each platform reports, is documented in that module.

use crate::measure::Count;
use slint::ComponentHandle;
use slint::winit_030::winit::event::WindowEvent;
use wubble_gfx::scene::run::{self, Needs, RunEffect, RunEvent, RunState};
use wubble_ui::AppWindow;

/// The run state of one window's scene.
#[derive(Default)]
pub struct Runner {
    state: RunState,
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
        // every way the user touches the window wakes a still scene
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

/// What `--measure` counts when `event` wakes a still or paused scene.
pub fn wake_of(event: &WindowEvent) -> Count {
    match event {
        WindowEvent::CursorEntered { .. } => Count::WakeEntered,
        WindowEvent::KeyboardInput { .. } | WindowEvent::Ime(_) => Count::WakeKey,
        WindowEvent::Touch(_) => Count::WakeTouch,
        WindowEvent::Occluded(_) | WindowEvent::Resized(_) | WindowEvent::Focused(_) => {
            Count::WakeShown
        }
        _ => Count::WakePointer,
    }
}

impl Runner {
    pub fn state(&self) -> RunState {
        self.state
    }

    /// After a rendered frame: whether the scene still needs frames.
    pub fn frame_done(&mut self, window: &AppWindow, scene_animating: bool, glass_changed: bool) {
        let needs = Needs {
            scene_animating,
            glass_changed,
        };
        self.on(needs.event(), Count::WakeChange, window);
    }

    /// Whether the state changed since the last call: the first frame after a change follows a
    /// gap (a still spell, a pause), so its interval says nothing about lateness.
    pub fn take_changed(&mut self) -> bool {
        std::mem::take(&mut self.changed)
    }

    /// Takes one event: the new state, and the effect to execute.
    fn step(&mut self, event: RunEvent) -> RunEffect {
        let (next, effect) = run::transition(self.state, event);
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

    /// The scene's own motion started (a beam lap, a palette change) while it is still, and no
    /// input event says so: wakes it for the frame about to be drawn. `cause` is counted.
    pub fn wake_if_still(&mut self, window: &AppWindow, cause: Count) {
        if self.state == RunState::Still {
            self.on(RunEvent::Activity, cause, window);
        }
    }

    /// Runs one event and executes its effect. A wake (into Running) counts `cause`, so
    /// `--measure idle` names what keeps drawing a still window.
    pub fn on(&mut self, event: RunEvent, cause: Count, window: &AppWindow) {
        let before = self.state;
        let effect = self.step(event);
        if before != RunState::Running && self.state == RunState::Running {
            crate::measure::count(cause);
        }
        match effect {
            RunEffect::None => {}
            RunEffect::RequestFrame => window.window().request_redraw(),
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
        r.step(RunEvent::Moving);
        assert!(!r.take_changed());
        // Running -> Paused -> Running: back where it was, still a change
        r.step(RunEvent::Hidden);
        r.step(RunEvent::Shown);
        assert_eq!(r.state(), RunState::Running);
        assert!(r.take_changed());
        assert!(!r.take_changed());
        // the stop and the wake are changes too: the frame after follows a gap
        r.step(RunEvent::Settled);
        assert_eq!(r.state(), RunState::Still);
        assert!(r.take_changed());
        r.step(RunEvent::Settled);
        assert!(!r.take_changed());
        assert_eq!(r.step(RunEvent::Activity), RunEffect::RequestFrame);
        assert!(r.take_changed());
    }

    #[test]
    fn a_wake_names_its_window_event() {
        let id = DeviceId::dummy();
        assert_eq!(
            wake_of(&WindowEvent::CursorEntered { device_id: id }),
            Count::WakeEntered
        );
        assert_eq!(wake_of(&WindowEvent::Occluded(false)), Count::WakeShown);
        assert_eq!(wake_of(&WindowEvent::Focused(true)), Count::WakeShown);
        assert_eq!(
            wake_of(&WindowEvent::MouseWheel {
                device_id: id,
                delta: MouseScrollDelta::LineDelta(0.0, 1.0),
                phase: TouchPhase::Moved,
            }),
            Count::WakePointer
        );
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
