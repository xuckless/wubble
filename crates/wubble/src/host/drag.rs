//! A tab dragged out of the sidebar's list: the pure machine is `wubble_core::tabdrag`; this
//! feeds it the pointer and the window's drop targets and runs what it returns. Inside the
//! list the row physics (`wubble_ui::driver`) reorders as before.
//!
//! Drop targets are measured from the window: the sidebar, the Space strip's icons, the
//! content area and, where the platform tells window positions (X11, macOS, Windows; not
//! Wayland), the other wubble windows. A release outside every target makes a new window.

use std::time::Instant;

use slint::ComponentHandle;
use slint::winit_030::WinitWindowAccessor;
use wubble_core::model::{Command, Dest};
use wubble_core::tabdrag::{self, DropTarget, Effect, Input, Point, Rect, Rects, Side};
use wubble_core::window::WindowId;
use wubble_ui::AppWindow;

use super::Host;

/// TabRow `track` kinds.
const DOWN: i32 = 0;
const MOVE: i32 = 1;
const UP: i32 = 2;

impl Host {
    /// The left button on a tab row: down, move, up or cancel, in window px.
    pub fn tab_track(&mut self, id: i32, kind: i32, x: f32, y: f32) {
        let Some(window) = self.w.window.upgrade() else {
            return;
        };
        let at = Point {
            x: f64::from(x),
            y: f64::from(y),
        };
        self.drag_at = at;
        let hit = || tabdrag::hit(at, &self.drop_rects(&window));
        let input = match kind {
            DOWN => match crate::shell::tab_id(id) {
                Some(tab) => Input::Press { tab, at },
                None => return,
            },
            MOVE => Input::Move {
                at,
                hit: hit(),
                now: Instant::now(),
            },
            UP => Input::Release { hit: hit() },
            _ => Input::Cancel,
        };
        self.run_drag(input);
    }

    /// Esc during a drag: nothing moves.
    pub(super) fn cancel_drag(&mut self) -> bool {
        if self.tab_drag == tabdrag::Drag::Idle {
            return false;
        }
        self.run_drag(Input::Cancel);
        true
    }

    /// The hover delay ran out while the pointer rested on a Space's icon.
    pub fn drag_tick(&mut self) {
        self.run_drag(Input::Tick {
            now: Instant::now(),
        });
    }

    fn run_drag(&mut self, input: Input) {
        let (next, effects) = tabdrag::transition(self.tab_drag, input);
        self.tab_drag = next;
        // a rest on a Space's icon still opens it: the machine is asked again once the delay is up
        match next {
            tabdrag::Drag::Dragging {
                over: DropTarget::Space(_),
                opened,
                ..
            } if opened.is_none() => {
                let id = self.w.id;
                self.drag_timer.start(
                    slint::TimerMode::SingleShot,
                    tabdrag::SPACE_HOVER,
                    move || crate::with_host_in(id, Host::drag_tick),
                );
            }
            _ => self.drag_timer.stop(),
        }
        for effect in effects {
            self.drag_effect(effect);
        }
        if let Some(window) = self.w.window.upgrade() {
            self.render_drag(&window);
        }
    }

    fn drag_effect(&mut self, effect: Effect) {
        match effect {
            Effect::Lift(_) | Effect::Land => {}
            Effect::CancelRows => {
                if let Some(window) = self.w.window.upgrade() {
                    window.invoke_tab_rows_cancel();
                }
            }
            Effect::SwitchSpace(space) => self.dispatch(Command::SwitchSpace(space)),
            Effect::MoveToSpace { tab, space } => self.dispatch(Command::MoveTab {
                tab,
                to: Dest::Normal {
                    space,
                    index: usize::MAX,
                },
            }),
            Effect::SplitInto { tab, side } => self.split_into(tab, side),
            Effect::MoveToWindow { tab, to } => {
                self.dispatch(Command::MoveTabToWindow { tab, to });
            }
        }
    }

    /// Puts `tab` beside this window's page, on `side`: into the window's Space first if it
    /// lives in another one.
    fn split_into(&mut self, tab: wubble_core::tab::TabId, side: Side) {
        let Some(space) = self.model.window(self.w.id).and_then(|w| w.space()) else {
            return;
        };
        if self.model.location(tab).map(|(s, _)| s) != Some(space) {
            self.dispatch(Command::MoveTab {
                tab,
                to: Dest::Normal {
                    space,
                    index: usize::MAX,
                },
            });
        }
        self.dispatch(Command::SplitWith(tab));
        if side == Side::Left && self.model.split_of(tab).is_some() {
            self.dispatch(Command::SwapPane { tab, right: false });
        }
    }

    /// The ghost and the lit drop zone, from the drag state.
    fn render_drag(&self, window: &AppWindow) {
        let ghost = self
            .tab_drag
            .ghost()
            .and_then(|t| self.model.tabs.get(&t))
            .map(crate::shell::tab_title)
            .unwrap_or_default();
        window.set_drag_ghost(ghost.into());
        window.set_drag_x(self.drag_at.x as f32);
        window.set_drag_y(self.drag_at.y as f32);
        window.set_drag_zone(match self.tab_drag.over() {
            Some(DropTarget::SplitEdge(Side::Left)) => 1,
            Some(DropTarget::SplitEdge(Side::Right)) => 2,
            _ => 0,
        });
    }

    /// This window's drop targets, in its logical px.
    fn drop_rects(&self, window: &AppWindow) -> Rects {
        let win = window.window();
        let scale = win.scale_factor();
        let size = win.size().to_logical(scale);
        let (w, h) = (f64::from(size.width), f64::from(size.height));
        let sidebar = f64::from(window.get_sidebar_width());
        let tile = f64::from(window.get_space_tile());
        let pitch = f64::from(window.get_space_pitch());
        let (sx, sy) = (
            f64::from(window.get_space_strip_x()),
            f64::from(window.get_space_strip_y()),
        );
        let rail = window.get_sidebar_rail();
        let spaces = (sidebar > 0.5)
            .then(|| self.model.spaces.iter().map(|s| s.id))
            .into_iter()
            .flatten()
            .enumerate()
            .map(|(i, id)| {
                let step = i as f64 * pitch;
                let (x, y) = if rail {
                    (sx, sy + step)
                } else {
                    (sx + step, sy)
                };
                (
                    id,
                    Rect {
                        x,
                        y,
                        w: tile,
                        h: tile,
                    },
                )
            })
            .collect();
        Rects {
            window: Some(Rect {
                x: 0.0,
                y: 0.0,
                w,
                h,
            }),
            list: (sidebar > 0.5).then_some(Rect {
                x: 0.0,
                y: 0.0,
                w: sidebar,
                h,
            }),
            spaces,
            content: Some(Rect {
                x: f64::from(window.get_content_x()),
                y: f64::from(window.get_content_y()),
                w: f64::from(window.get_content_width()),
                h: f64::from(window.get_content_height()),
            }),
            windows: self.other_windows(window),
        }
    }

    /// The other wubble windows relative to this one, where the platform tells positions
    /// (Wayland does not: then none, and a drop outside makes a new window).
    fn other_windows(&self, window: &AppWindow) -> Vec<(WindowId, Rect)> {
        let outer = |w: &AppWindow| {
            w.window()
                .with_winit_window(|w| w.outer_position().ok())
                .flatten()
        };
        let Some(origin) = outer(window) else {
            return vec![];
        };
        let scale = f64::from(window.window().scale_factor());
        self.parked
            .values()
            .filter_map(|other| {
                let at = outer(&other.handle)?;
                let size = other.handle.window().size();
                Some((
                    other.id,
                    Rect {
                        x: f64::from(at.x - origin.x) / scale,
                        y: f64::from(at.y - origin.y) / scale,
                        w: f64::from(size.width) / scale,
                        h: f64::from(size.height) / scale,
                    },
                ))
            })
            .collect()
    }
}
