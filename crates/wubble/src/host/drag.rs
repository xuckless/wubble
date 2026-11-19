//! The host's half of drags: a page's own drag run inside the window
//! (`page_drag`), handed to the OS when it leaves the window where the
//! platform can (`crate::wl`), and drags from other apps dropped on the page
//! under the pointer (winit's files; on Wayland, `crate::wl`'s files, links,
//! HTML and text).

use slint::ComponentHandle;

use crate::clipboard::Bridge as _;
use slint::winit_030::winit::event::WindowEvent;
use wubble_core::tab::TabId;
use wubble_engine::input::Point;
use wubble_engine::nav::Cursor;
use wubble_engine::page::{DragData, DragOps};
use wubble_engine::view::DragPhase;
use wubble_ui::AppWindow;

use super::{Host, new_tab};
use crate::page_drag::{self, DragEffect, DragInput, Over, PageDrag};

/// Every operation: what a drop from another app offers the page.
const ANY: DragOps = DragOps(DragOps::COPY.0 | DragOps::LINK.0 | DragOps::MOVE.0);

/// Where a drag from another app is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum External {
    Enter,
    Over,
    Leave,
    Drop,
}

/// A drag from another app over the window: what it carries and the page it is over.
#[derive(Clone, Debug, Default)]
pub(super) struct ExternalDrag {
    data: DragData,
    over: Option<TabId>,
}

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

    /// The page in `tab` says what a drop at the pointer would do: its own drag's
    /// operation, or the answer to a drag from another app.
    pub(super) fn drag_cursor(&mut self, tab: TabId, op: DragOps) {
        match &self.page_drag {
            PageDrag::Dragging { source, .. } if *source == tab => {
                let point = self.drag_point;
                self.apply_drag(DragInput::Cursor(op), point);
            }
            _ if self.external_drag.as_ref().and_then(|d| d.over) == Some(tab) => {
                self.aux().drop_action(op)
            }
            _ => {}
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
        let Some(window) = self.w.window.upgrade() else {
            return false;
        };
        let input = match (released, self.drag_over(&window, point)) {
            (true, over) => {
                self.input.buttons = Default::default();
                DragInput::Released(over.unwrap_or(Over::Elsewhere))
            }
            (false, Some(over)) => DragInput::Moved(over),
            (false, None) => DragInput::LeftWindow,
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

    /// The OS drag a page drag became ended with `op` done.
    fn os_drag_ended(&mut self, op: DragOps) {
        let point = self.drag_point;
        self.apply_drag(DragInput::OsEnded(op), point);
    }

    /// Where `point` (relative to the drag's page) is in the window; `None`
    /// outside it when the platform can take the drag over (the button is
    /// held, so the window still sees the pointer there).
    fn drag_over(&self, window: &AppWindow, point: Point) -> Option<Over> {
        let PageDrag::Dragging { source, .. } = &self.page_drag else {
            return Some(Over::Elsewhere);
        };
        let rect = self
            .pane_rects(window)
            .into_iter()
            .find(|r| r.tab == *source);
        let (width, height) = (
            rect.map_or(0.0, |r| r.width as f32),
            window.get_content_height(),
        );
        let (origin_x, origin_y) = self.view_origin(window, *source);
        let (at_x, at_y) = (origin_x + point.x, origin_y + point.y);
        let size = window
            .window()
            .size()
            .to_logical(window.window().scale_factor());
        let inside = (0.0..size.width).contains(&at_x) && (0.0..size.height).contains(&at_y);
        if (0.0..width).contains(&point.x) && (0.0..height).contains(&point.y) {
            Some(Over::Page)
        } else if !inside && self.aux().drags_out() {
            None
        } else if at_x < window.get_content_x() {
            Some(Over::Sidebar)
        } else {
            Some(Over::Elsewhere)
        }
    }

    fn apply_drag(&mut self, input: DragInput, point: Point) {
        let source = match (&self.page_drag, &input) {
            (PageDrag::Dragging { source, .. } | PageDrag::Outside { source }, _)
            | (_, DragInput::Started { source, .. }) => *source,
            _ => return,
        };
        let (next, effects) = page_drag::transition(std::mem::take(&mut self.page_drag), input);
        self.page_drag = next;
        let modifiers = self.input.modifiers();
        let window = self.w.window.upgrade();
        for effect in effects {
            match effect {
                DragEffect::OpenTab(url) => {
                    self.dispatch(new_tab(url, Some(source)));
                    continue;
                }
                DragEffect::Feedback(op) => {
                    if let Some(window) = &window {
                        window.set_web_cursor(page_drag::cursor(op) as i32);
                    }
                    continue;
                }
                DragEffect::StartOs(data, allowed) => {
                    if !self.aux().start_drag(self.w.id, &data, allowed) {
                        self.os_drag_ended(DragOps::NONE);
                    }
                    continue;
                }
                DragEffect::End(_) => {
                    if let Some(window) = &window {
                        window.set_web_cursor(Cursor::Default as i32);
                    }
                }
                _ => {}
            }
            let Some(view) = self.tab_view(source) else {
                continue;
            };
            match effect {
                DragEffect::Enter(allowed) => {
                    view.drag(DragPhase::EnterSource, point, &modifiers, allowed)
                }
                DragEffect::Over(allowed) => view.drag(DragPhase::Over, point, &modifiers, allowed),
                DragEffect::Leave => view.drag(DragPhase::Leave, point, &modifiers, DragOps::NONE),
                DragEffect::Drop => view.drag(DragPhase::Drop, point, &modifiers, DragOps::NONE),
                DragEffect::End(op) => view.drag_source_ended(point, op),
                DragEffect::OpenTab(_) | DragEffect::Feedback(_) | DragEffect::StartOs(..) => {}
            }
        }
    }

    /// A file from another app over the window (winit's `HoveredFile`,
    /// `DroppedFile`, `HoveredFileCancelled`; X11, macOS and Windows: winit
    /// 0.30 has no Wayland drops, which come through `crate::wl`).
    pub(super) fn file_drop(&mut self, window: &AppWindow, event: &WindowEvent) {
        let at = self.input.pointer.map_or((0.0, 0.0), |p| (p.x, p.y));
        let phase = match event {
            WindowEvent::HoveredFile(path) => {
                // winit reports each file of a drop on its own: they add up.
                let mut files = self
                    .external_drag
                    .as_ref()
                    .map(|d| d.data.files.clone())
                    .unwrap_or_default();
                files.push(path.clone());
                self.external_drag(window, External::Leave, at);
                let data = DragData {
                    files,
                    ..DragData::default()
                };
                self.external_drag = Some(ExternalDrag { data, over: None });
                External::Enter
            }
            WindowEvent::DroppedFile(_) => External::Drop,
            WindowEvent::HoveredFileCancelled => External::Leave,
            _ => return,
        };
        self.external_drag(window, phase, at);
    }

    /// A drag from another app at `at` (window logical px). `Enter` starts it
    /// with the data already in [`Host::external_drag`]; the page under the
    /// pointer gets it, and another page when the pointer crosses into its pane.
    pub(super) fn external_drag(&mut self, window: &AppWindow, phase: External, at: (f32, f32)) {
        let Some(mut drag) = self.external_drag.take() else {
            return;
        };
        let modifiers = self.input.modifiers();
        let under = self.pane_under(window, at);
        let point_in = |host: &Self, tab: TabId| {
            let (x, y) = host.view_origin(window, tab);
            Point {
                x: at.0 - x,
                y: at.1 - y,
            }
        };
        let (leave, enter) = match phase {
            External::Leave => (drag.over, None),
            _ if drag.over != under || phase == External::Enter => (drag.over, under),
            _ => (None, None),
        };
        if let Some(tab) = leave {
            let p = point_in(self, tab);
            if let Some(view) = self.tab_view(tab) {
                view.drag(DragPhase::Leave, p, &modifiers, DragOps::NONE);
            }
        }
        if let Some(tab) = enter {
            let p = point_in(self, tab);
            if let Some(view) = self.tab_view(tab) {
                view.drag(DragPhase::Enter(drag.data.clone()), p, &modifiers, ANY);
            }
        }
        if phase == External::Leave {
            return;
        }
        drag.over = under;
        match under {
            None => self.aux().drop_action(DragOps::NONE),
            Some(tab) => {
                let p = point_in(self, tab);
                if let Some(view) = self.tab_view(tab) {
                    view.drag(DragPhase::Over, p, &modifiers, ANY);
                    if phase == External::Drop {
                        view.drag(DragPhase::Drop, p, &modifiers, DragOps::NONE);
                    }
                }
            }
        }
        if phase != External::Drop {
            self.external_drag = Some(drag);
        }
    }

    /// The page on screen under `at` (window logical px).
    fn pane_under(&self, window: &AppWindow, at: (f32, f32)) -> Option<TabId> {
        let height = window.get_content_height();
        self.pane_rects(window).into_iter().find_map(|r| {
            let (x, y) = self.view_origin(window, r.tab);
            let (px, py) = (at.0 - x, at.1 - y);
            ((0.0..r.width as f32).contains(&px) && (0.0..height).contains(&py)).then_some(r.tab)
        })
    }

    /// The platform's extras (`crate::wl`), on the clipboard's connection.
    pub(super) fn aux(&self) -> &dyn crate::wl::Aux {
        self.clipboard.extras().unwrap_or(&crate::wl::Null)
    }

    /// Tells the platform's extras about this window once its platform window exists.
    pub(super) fn add_aux_window(&mut self, window: &AppWindow) {
        if !self.w.aux_added {
            self.w.aux_added = true;
            self.aux().add_window(self.w.id, window.window());
        }
    }

    /// What the platform reported for this window (`crate::wl`).
    pub(crate) fn aux_event(&mut self, event: crate::wl::AuxEvent) {
        use crate::wl::AuxEvent as A;
        let Some(window) = self.w.window.upgrade() else {
            return;
        };
        match event {
            A::DropEnter { x, y, data } => {
                self.external_drag(&window, External::Leave, (x, y));
                self.external_drag = Some(ExternalDrag { data, over: None });
                self.external_drag(&window, External::Enter, (x, y));
            }
            A::DropMove { x, y } => self.external_drag(&window, External::Over, (x, y)),
            A::DropLeave => self.external_drag(&window, External::Leave, (0.0, 0.0)),
            A::Dropped { x, y } => self.external_drag(&window, External::Drop, (x, y)),
            A::DragEnded(op) => self.os_drag_ended(op),
            A::Pinch(input) => self.pinch(input),
            A::Swipe(input) => self.history_swipe(&window, input),
            A::Locks(locks) => self.input.locks = locks,
        }
    }
}
