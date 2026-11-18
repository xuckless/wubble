//! The host's half of drags: a page's own drag run inside the window
//! (`page_drag`), and files dropped on the window from other apps.

use slint::winit_030::winit::event::WindowEvent;
use wubble_core::tab::TabId;
use wubble_engine::input::Point;
use wubble_engine::page::{DragData, DragOps};
use wubble_engine::view::DragPhase;
use wubble_ui::AppWindow;

use super::{Host, new_tab};
use crate::page_drag::{self, DragEffect, DragInput, Over, PageDrag};

impl Host {
    /// The page in `tab` started a drag (CEF `StartDragging`). CEF asks a
    /// little after the press and moves that began it; if the button was
    /// already released by then, the drag ends at once with nothing dropped,
    /// as an OS drag started after the release does. Otherwise it would hold
    /// every later pointer move as a drag-over.
    pub(super) fn drag_started(&mut self, tab: TabId, data: DragData, allowed: DragOps) {
        let point = self.drag_point;
        self.apply_drag(
            DragInput::Started {
                source: tab,
                data,
                allowed,
            },
            point,
        );
        if !self
            .input
            .buttons
            .is_down(wubble_engine::input::MouseButton::Left)
        {
            self.drag_cancelled();
        }
    }

    /// No page drag runs (`--smoke parity`'s reset).
    pub(super) fn drag_idle(&self) -> bool {
        self.page_drag == PageDrag::Idle
    }

    /// Pointer input during a page drag goes to the drag, not to the page as
    /// mouse input; false when no drag runs.
    pub(super) fn drag_pointer(&mut self, point: Point, released: bool) -> bool {
        self.drag_point = point;
        if self.page_drag == PageDrag::Idle {
            return false;
        }
        let Some(window) = self.window.upgrade() else {
            return false;
        };
        let over = self.drag_over(&window, point);
        let input = if released {
            self.input.buttons = Default::default();
            DragInput::Released(over)
        } else {
            DragInput::Moved(over)
        };
        self.apply_drag(input, point);
        true
    }

    /// Escape ends a page drag with nothing dropped; false when no drag runs.
    pub(super) fn drag_cancelled(&mut self) -> bool {
        if self.page_drag == PageDrag::Idle {
            return false;
        }
        let point = self.drag_point;
        self.apply_drag(DragInput::Cancelled, point);
        true
    }

    /// Where `point` (relative to the drag's page) is in the window.
    fn drag_over(&self, window: &AppWindow, point: Point) -> Over {
        let PageDrag::Dragging { source, .. } = &self.page_drag else {
            return Over::Elsewhere;
        };
        let rect = self
            .pane_rects(window)
            .into_iter()
            .find(|r| r.tab == *source);
        let (width, height) = (
            rect.map_or(0.0, |r| r.width as f32),
            window.get_content_height(),
        );
        let page_x = window.get_content_x() + rect.map_or(0.0, |r| r.x as f32);
        if (0.0..width).contains(&point.x) && (0.0..height).contains(&point.y) {
            Over::Page
        } else if page_x + point.x < window.get_content_x() {
            Over::Sidebar
        } else {
            Over::Elsewhere
        }
    }

    fn apply_drag(&mut self, input: DragInput, point: Point) {
        let source = match (&self.page_drag, &input) {
            (PageDrag::Dragging { source, .. }, _) | (_, DragInput::Started { source, .. }) => {
                *source
            }
            _ => return,
        };
        let (next, effects) = page_drag::transition(std::mem::take(&mut self.page_drag), input);
        self.page_drag = next;
        let modifiers = self.input.modifiers();
        for effect in effects {
            if let DragEffect::OpenTab(url) = effect {
                self.dispatch(new_tab(url, Some(source)));
                continue;
            }
            let Some(view) = self.tab_view(source) else {
                continue;
            };
            match effect {
                DragEffect::Enter(data, allowed) => {
                    view.drag(DragPhase::Enter(data), point, &modifiers, allowed)
                }
                DragEffect::Over(allowed) => view.drag(DragPhase::Over, point, &modifiers, allowed),
                DragEffect::Leave => view.drag(DragPhase::Leave, point, &modifiers, DragOps::NONE),
                DragEffect::Drop => view.drag(DragPhase::Drop, point, &modifiers, DragOps::NONE),
                DragEffect::End(op) => view.drag_source_ended(point, op),
                DragEffect::OpenTab(_) => {}
            }
        }
    }

    /// A file from another app over the window (winit's `HoveredFile`,
    /// `DroppedFile`, `HoveredFileCancelled`): dropped on the visible page at
    /// the pointer. winit 0.30 reports these on X11, macOS and Windows, not
    /// on Wayland (TODO.md).
    pub(super) fn file_drop(&mut self, window: &AppWindow, event: &WindowEvent) {
        let Some(tab) = self.model.visible_tab() else {
            return;
        };
        let (x, y) = self.focus_origin(window);
        let at = self
            .input
            .pointer
            .map_or(Point { x: 0.0, y: 0.0 }, |p| Point {
                x: p.x - x,
                y: p.y - y,
            });
        let modifiers = self.input.modifiers();
        let any = DragOps(DragOps::COPY.0 | DragOps::LINK.0 | DragOps::MOVE.0);
        let Some(view) = self.tab_view(tab) else {
            return;
        };
        match event {
            WindowEvent::HoveredFile(path) => {
                let data = DragData {
                    files: vec![path.clone()],
                    ..DragData::default()
                };
                view.drag(DragPhase::Enter(data), at, &modifiers, any);
                view.drag(DragPhase::Over, at, &modifiers, any);
            }
            WindowEvent::DroppedFile(_) => view.drag(DragPhase::Drop, at, &modifiers, any),
            WindowEvent::HoveredFileCancelled => {
                view.drag(DragPhase::Leave, at, &modifiers, DragOps::NONE)
            }
            _ => {}
        }
    }
}
