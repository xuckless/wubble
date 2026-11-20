//! A tab dragged out of the sidebar's list: the pure machine is `wubble_core::tabdrag`; this
//! feeds it the pointer and the window's drop targets and runs what it returns. Inside the
//! list the row physics (`wubble_ui::driver`) reorders as before.
//!
//! Drop targets are measured from the window: the sidebar, the footer's Spaces switcher and,
//! while it is open, the Spaces popup's rows, the content area and, where the
//! platform tells window positions (X11, macOS, Windows; not Wayland), the other wubble windows.
//! A release outside every target makes a new window.

use std::time::Instant;

use slint::ComponentHandle;
use slint::winit_030::WinitWindowAccessor;
use wubble_core::model::{Command, Dest};
use wubble_core::tabdrag::{self, DropTarget, Effect, Input, Point, Rect, Rects, Side};
use wubble_core::window::WindowId;
use wubble_ui::AppWindow;

use super::Host;

/// TabRow `track` kinds: the row reports the press and a cancel; the pointer after the press
/// comes from the window (`tab_drag_pointer`), since the pressed row may be gone mid-drag (a Space
/// the hover opened re-lists the rows).
const DOWN: i32 = 0;

impl Host {
    /// The left button on a tab row: down or cancel, in window px.
    pub fn tab_track(&mut self, id: i32, kind: i32, x: f32, y: f32) {
        self.hover_dismiss();
        let input = match kind {
            DOWN => match crate::shell::tab_id(id) {
                Some(tab) => {
                    let at = Point {
                        x: f64::from(x),
                        y: f64::from(y),
                    };
                    self.drag_at = at;
                    self.drag_window = self.w.id;
                    Input::Press { tab, at }
                }
                None => return,
            },
            _ => Input::Cancel,
        };
        self.run_tab_drag(input);
    }

    /// The window's pointer while a press on a row is held: moved, or the left button let go,
    /// in this window's logical px.
    pub(super) fn tab_drag_pointer(&mut self, x: f32, y: f32, released: bool) {
        if self.tab_drag == tabdrag::Drag::Idle || self.w.id != self.drag_window {
            return;
        }
        let Some(window) = self.w.window.upgrade() else {
            return;
        };
        let at = Point {
            x: f64::from(x),
            y: f64::from(y),
        };
        self.drag_at = at;
        let hit = tabdrag::hit(at, &self.drop_rects(&window));
        self.run_tab_drag(if released {
            Input::Release { hit }
        } else {
            Input::Move {
                at,
                hit,
                now: Instant::now(),
            }
        });
    }

    /// `--smoke windows`: drags `tab` from the window showing its Space onto window `to`, as
    /// a pointer would where the platform tells window positions.
    pub(super) fn smoke_drag(&mut self, tab: wubble_core::tab::TabId, to: WindowId) {
        let Some(from) = self
            .model
            .location(tab)
            .and_then(|(space, _)| self.model.windows.iter().find(|w| w.space() == Some(space)))
            .map(|w| w.id)
        else {
            return;
        };
        if !self.enter(from) {
            return;
        }
        self.drag_window = from;
        let at = Point { x: 10.0, y: 10.0 };
        self.run_tab_drag(Input::Press { tab, at });
        let hit = DropTarget::Window(to);
        let at = Point { x: 200.0, y: 10.0 };
        self.drag_at = at;
        self.run_tab_drag(Input::Move {
            at,
            hit,
            now: Instant::now(),
        });
        self.run_tab_drag(Input::Release { hit });
    }

    /// `--smoke gallery`'s "spaces-popup-drag": the visible tab dragged onto the switcher, held
    /// there past the hover delay (the popup opens), then over the popup's first drop row.
    pub(super) fn gallery_drag_to_spaces(&mut self) {
        let (Some(window), Some(tab)) = (self.w.window.upgrade(), self.shown()) else {
            return;
        };
        self.drag_window = self.w.id;
        let g = window.get_spaces_menu_geometry();
        let at = Point {
            x: f64::from(g.switcher_x + g.switcher_w / 2.0),
            y: f64::from(g.switcher_y + g.switcher_h / 2.0),
        };
        self.run_tab_drag(Input::Press {
            tab,
            at: Point { x: 0.0, y: 0.0 },
        });
        let now = Instant::now();
        let hit = tabdrag::hit(at, &self.drop_rects(&window));
        self.drag_at = at;
        self.run_tab_drag(Input::Move { at, hit, now });
        self.run_tab_drag(Input::Tick {
            now: now + tabdrag::SPACE_HOVER,
        });
        let rows = self
            .spaces_popup()
            .drop_rows(&spaces_geometry(&window.get_spaces_menu_geometry()));
        if let Some((_, _, r)) = rows.first() {
            let at = Point {
                x: r.x + r.w / 2.0,
                y: r.y + r.h / 2.0,
            };
            let hit = tabdrag::hit(at, &self.drop_rects(&window));
            self.drag_at = at;
            self.run_tab_drag(Input::Move {
                at,
                hit,
                now: now + tabdrag::SPACE_HOVER,
            });
        }
    }

    /// Esc during a drag: nothing moves.
    pub(super) fn cancel_tab_drag(&mut self) -> bool {
        if self.tab_drag == tabdrag::Drag::Idle {
            return false;
        }
        self.run_tab_drag(Input::Cancel);
        true
    }

    /// A hover delay ran out: on the Spaces switcher (the popup opens) or off the popup (it
    /// closes).
    pub fn drag_tick(&mut self) {
        self.run_tab_drag(Input::Tick {
            now: Instant::now(),
        });
    }

    /// Runs the machine, then draws the ghost in the window the drag started in. Effects may
    /// move focus to another window (a tear-out, a drop on another window, a hovered Space
    /// another window shows); the ghost is never drawn there, and the source's goes once idle.
    fn run_tab_drag(&mut self, input: Input) {
        let (next, effects) = tabdrag::transition(self.tab_drag, input);
        self.tab_drag = next;
        let source = self.drag_window;
        // a rest on the switcher opens the popup, a while off it closes it: the machine is asked
        // again once the delay is up
        if next.awaits_dwell() {
            self.drag_timer.start(
                slint::TimerMode::SingleShot,
                tabdrag::SPACE_HOVER,
                move || crate::with_host_in(source, Host::drag_tick),
            );
        } else {
            self.drag_timer.stop();
        }
        for effect in effects {
            self.tab_drag_effect(effect);
        }
        let here = self.w.id;
        if self.enter(source)
            && let Some(window) = self.w.window.upgrade()
        {
            self.render_tab_drag(&window);
        }
        self.enter(here);
    }

    fn tab_drag_effect(&mut self, effect: Effect) {
        match effect {
            Effect::CancelRows => {
                if self.enter(self.drag_window)
                    && let Some(window) = self.w.window.upgrade()
                {
                    window.invoke_tab_rows_cancel();
                }
            }
            Effect::OpenSpaces => self.spaces_menu_requested(),
            Effect::CloseSpaces => {
                if self.enter(self.drag_window)
                    && let Some(window) = self.w.window.upgrade()
                {
                    window.set_spaces_open(false);
                }
            }
            Effect::MoveToNewSpace { tab } => {
                let command = self.model.new_space();
                self.dispatch(command);
                if let Some(space) = self.model.spaces.last().map(|s| s.id) {
                    self.dispatch(Command::MoveTab {
                        tab,
                        to: Dest::Normal {
                            space,
                            index: usize::MAX,
                        },
                    });
                }
            }
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
    fn render_tab_drag(&self, window: &AppWindow) {
        let ghost = self
            .tab_drag
            .ghost()
            .and_then(|t| self.model.tabs.get(&t))
            .map(crate::shell::tab_title)
            .unwrap_or_default();
        window.set_drag_ghost(ghost.into());
        window.set_drag_x(self.drag_at.x as f32);
        window.set_drag_y(self.drag_at.y as f32);
        // the popup row a release would drop on, lit as the pointer lights one
        let (line, top) = match self.tab_drag.over() {
            Some(DropTarget::Space(space)) => {
                let g = spaces_geometry(&window.get_spaces_menu_geometry());
                let popup = self.spaces_popup();
                popup
                    .drop_rows(&g)
                    .into_iter()
                    .find(|(_, s, _)| *s == space)
                    .map_or((-1, 0.0), |(i, _, _)| {
                        let line = i32::try_from(i).unwrap_or(-1);
                        (line, popup.line_top(i, &g) as f32)
                    })
            }
            _ => (-1, 0.0),
        };
        window.set_spaces_drop_line(line);
        window.set_spaces_drop_top(top);
        window.set_spaces_drop_sticky(self.tab_drag.over() == Some(DropTarget::NewSpace));
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
        let g = window.get_spaces_menu_geometry();
        let rect = |x: f32, y: f32, w: f32, h: f32| Rect {
            x: f64::from(x),
            y: f64::from(y),
            w: f64::from(w),
            h: f64::from(h),
        };
        let trigger =
            (sidebar > 0.5).then(|| rect(g.switcher_x, g.switcher_y, g.switcher_w, g.switcher_h));
        let open = g.shown && sidebar > 0.5;
        let spaces = if open {
            self.spaces_popup()
                .drop_rows(&spaces_geometry(&g))
                .into_iter()
                .map(|(_, space, r)| (space, r))
                .collect()
        } else {
            vec![]
        };
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
            trigger,
            menu: open.then(|| rect(g.panel_x, g.panel_y, g.panel_w, g.panel_h)),
            spaces,
            new_space: open.then(|| rect(g.rows_x, g.sticky_top, g.rows_w, g.line)),
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

/// The open popup's lines in window px, as the shell reports them.
fn spaces_geometry(g: &wubble_ui::SpacesMenuGeometry) -> wubble_core::spacemenu::Geometry {
    wubble_core::spacemenu::Geometry {
        x: f64::from(g.rows_x),
        w: f64::from(g.rows_w),
        list_top: f64::from(g.list_top),
        list_height: f64::from(g.list_h),
        scroll: f64::from(g.scroll),
        row: f64::from(g.line),
        rule: f64::from(g.rule),
    }
}
