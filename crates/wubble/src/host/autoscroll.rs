//! Middle-click autoscroll's pointer: the page's panning cursors drive
//! `wubble_core::autoscroll`, the pointer moves the arrow, and each shell
//! frame steps the arrow's stretch with the scroll's speed.

use std::time::Instant;

use slint::ComponentHandle;

use wubble_core::autoscroll::{self, At, Autoscroll, Input};
use wubble_core::motion::{K, Spring, spring, squash_of, step_k};
use wubble_core::tab::TabId;
use wubble_engine::input::Point;
use wubble_ui::AppWindow;

use super::Host;

/// The arrow's stretch while autoscroll runs.
pub(super) struct AutoscrollPointer {
    state: Autoscroll,
    /// Window logical px.
    pointer: At,
    /// The arrow's length factor, following `autoscroll::stretch_target` of the scroll speed.
    stretch: Spring,
    last: Option<Instant>,
}

impl Default for AutoscrollPointer {
    fn default() -> Self {
        Self {
            state: Autoscroll::Off,
            pointer: At::default(),
            stretch: spring(1.0),
            last: None,
        }
    }
}

impl Host {
    /// CSS px per logical px of the shown page.
    fn autoscroll_zoom(&mut self) -> f64 {
        self.model
            .visible_tab()
            .and_then(|t| self.tab_view(t))
            .map_or(1.0, |v| f64::from(v.zoom_percent()) / 100.0)
    }

    /// The pointer moved over `tab`'s pane to `point` (relative to the pane).
    pub(super) fn autoscroll_pointer(&mut self, window: &AppWindow, tab: TabId, point: Point) {
        let x = self
            .pane_rects(window)
            .into_iter()
            .find(|r| r.tab == tab)
            .map_or(0.0, |r| r.x as f32);
        self.autoscroll.pointer = At {
            x: f64::from(window.get_content_x() + x + point.x),
            y: f64::from(window.get_content_y() + point.y),
        };
        if self.autoscroll.state != Autoscroll::Off {
            self.render_autoscroll(window);
        }
    }

    /// The shown page asked for a cursor (autoscroll starts, turns or ends), or
    /// a button or key went down (it ends).
    pub(super) fn autoscroll_input(&mut self, window: &AppWindow, input: Input) {
        let a = &mut self.autoscroll;
        let next = autoscroll::transition(a.state, input, a.pointer);
        if next == a.state {
            return;
        }
        // Ended by a press: CEF asks for the page's cursor again only on the
        // next move, so show the default one until then.
        if next == Autoscroll::Off && input == Input::Pressed {
            window.set_web_cursor(wubble_engine::nav::Cursor::Default as i32);
            for tab in self.shown_tabs() {
                if let Some(view) = self.tab_view(tab) {
                    view.reset_cursor();
                }
            }
        }
        let a = &mut self.autoscroll;
        if a.state == Autoscroll::Off {
            *a = AutoscrollPointer {
                pointer: a.pointer,
                ..AutoscrollPointer::default()
            };
        }
        a.state = next;
        self.render_autoscroll(window);
        window.window().request_redraw();
    }

    /// One shell frame: steps the stretch; true while autoscroll runs.
    pub(super) fn autoscroll_frame(&mut self, window: &AppWindow, now: Instant) -> bool {
        if self.autoscroll.state == Autoscroll::Off {
            self.autoscroll.last = None;
            return false;
        }
        let zoom = self.autoscroll_zoom();
        let a = &mut self.autoscroll;
        let dt = a
            .last
            .replace(now)
            .map_or(0.0, |prev| now.duration_since(prev).as_secs_f64().min(0.05));
        let target = autoscroll::stretch_target(autoscroll::scroll_speed(a.state, a.pointer, zoom));
        step_k(&mut a.stretch, target, K::stretch, dt);
        self.render_autoscroll(window);
        true
    }

    fn render_autoscroll(&mut self, window: &AppWindow) {
        let zoom = self.autoscroll_zoom();
        let a = &self.autoscroll;
        let on = a.state != Autoscroll::Off;
        window.set_autoscroll_on(on);
        let Some(anchor) = a.state.anchor() else {
            return;
        };
        window.set_autoscroll_anchor_x(anchor.x as f32);
        window.set_autoscroll_anchor_y(anchor.y as f32);
        window.set_autoscroll_x(a.pointer.x as f32);
        window.set_autoscroll_y(a.pointer.y as f32);
        let moving = match a.state {
            Autoscroll::Moving { anchor, direction } => {
                let offset = At {
                    x: (a.pointer.x - anchor.x) / zoom,
                    y: (a.pointer.y - anchor.y) / zoom,
                };
                // the pointer's own direction turns smoothly; the cursor's eight only
                // when the pointer is inside the dead zone
                window.set_autoscroll_heading(
                    autoscroll::heading(offset).unwrap_or_else(|| direction.degrees()),
                );
                true
            }
            _ => false,
        };
        window.set_autoscroll_moving(moving);
        // The arrow's length stretches; its width keeps the area.
        let stretch = if moving { a.stretch.x } else { 1.0 };
        window.set_autoscroll_stretch(stretch as f32);
        window.set_autoscroll_squash(squash_of(stretch) as f32);
    }

    /// Autoscroll's state now, for `--smoke parity`.
    pub(super) fn autoscroll_state(&self) -> Autoscroll {
        self.autoscroll.state
    }
}
