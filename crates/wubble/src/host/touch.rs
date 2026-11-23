//! A touchscreen over the window (`wubble_core::touch::Fingers`). Slint sees every finger, as
//! before, and hit-tests the first: when its press lands on a page's pane (not on the shell
//! drawn over it), the page has the finger, and every finger that goes down while it is, as
//! touch (touch and pointer events, `pointerType` touch), so Blink scrolls, taps,
//! long-presses and pinches as in Chrome. The mouse input Slint makes of a finger never
//! reaches a page (`InputState::touching`); on the shell it works as it did.
//!
//! A pen is a finger here: winit 0.30 reports a stylus as a touch at most (Windows, with its
//! pressure), never as a pen (Wayland and X11 give the pointer the compositor emulates), so no
//! page gets `pointerType` pen yet.

use slint::ComponentHandle;
use slint::winit_030::winit::event::{Touch, TouchPhase};
use wubble_core::touch::{self, FingerInput, PageTouch, TouchKind, UNKNOWN_PRESSURE};
use wubble_engine::input::Point;
use wubble_gfx::pacing::PaceEvent;
use wubble_ui::AppWindow;

use super::Host;

impl Host {
    /// One finger's event from winit, before Slint dispatches it.
    pub(super) fn touch(&mut self, window: &AppWindow, t: &Touch) {
        let at = t
            .location
            .to_logical::<f32>(window.window().scale_factor().into());
        let at = (at.x, at.y);
        let pressure = t.force.map_or(UNKNOWN_PRESSURE, |f| f.normalized() as f32);
        let id = t.id;
        let input = match t.phase {
            TouchPhase::Started => FingerInput::Down { id, at, pressure },
            TouchPhase::Moved => FingerInput::Moved { id, at, pressure },
            TouchPhase::Ended => FingerInput::Up { id, at },
            TouchPhase::Cancelled => FingerInput::Cancelled { id },
        };
        // A page that left the screen since (closed, switched away) loses its fingers first.
        let shown = self.shown_tabs();
        self.fingers(FingerInput::OnScreen(shown));
        self.fingers(input);
        // Over once Slint has dispatched the event, for input that comes with no raw event of
        // its own after it (a screen reader's click, a smoke's).
        let _ =
            slint::invoke_from_event_loop(|| crate::with_host(|host| host.input.touching = false));
    }

    /// Feeds the fingers one input and sends the page its touch events.
    pub(super) fn fingers(&mut self, input: FingerInput) {
        let (next, effects) = touch::transition(std::mem::take(&mut self.w.fingers), input);
        self.w.fingers = next;
        let modifiers = self.input.modifiers();
        for PageTouch {
            pane,
            slot,
            kind,
            point,
            pressure,
        } in effects
        {
            if kind == TouchKind::Press {
                self.page_pressed(Some(pane));
            }
            self.pace(PaceEvent::Wake);
            let point = Point {
                x: point.0,
                y: point.1,
            };
            if let Some(view) = self.tab_view(pane) {
                view.touch(slot, kind, point, pressure, &modifiers);
            }
        }
    }
}
