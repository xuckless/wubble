//! Effect side of the spring-driven shell: steps the physics in `wubble_core` (tab rows, the
//! Space capsule, the seam and the tab list's scroller) once per rendered frame and writes the
//! values into Slint `in` properties.
//!
//! The frame body is the design repo's port driver (`src/wubble/slint/driver.rs`,
//! `MotionState::frame`, xuckless/the-wallpaper-project @ 6c840da) without the split view.
//! Differences, on purpose:
//! - The per-frame hook is Slint's rendering notifier instead of the port's 16 ms
//!   `slint::Timer`, so frames stay vsync-aligned (the port's README suggests this when a
//!   notifier exists). The idiom is copied from `examples/wgpu_texture/lib.rs` (slint-ui/slint
//!   v1.19.0, MIT): set properties in `BeforeRendering`, then `request_redraw()`. A window accepts
//!   only one notifier, so [`install`] is the single owner. While everything is at rest nothing
//!   requests a redraw, so an idle shell costs nothing.
//! - The driver never edits the Slint models: the host's core model owns tab order. It reads the
//!   ids back on `shell-model-changed` and reports a click (`tab-activated`) or a reorder
//!   (`tab-moved`) as an intent.
//! - No `FrameClock`: dt comes from the notifier's frame times, capped at `DT_CAP`.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

use slint::{ComponentHandle, Model, ModelRc, SetRenderingNotifierError, VecModel, Weak};
use wubble_core::motion::DT_CAP;
use wubble_core::scroller::{DeltaMode, Scroller};
use wubble_core::seam::{self, INSET, SEAM, Seam};
use wubble_core::tabs::{SpaceSwitcher, TabRows, TabUp};

use crate::{AppWindow, Radius, SidebarMode, sync};

impl From<seam::SidebarMode> for SidebarMode {
    fn from(m: seam::SidebarMode) -> Self {
        match m {
            seam::SidebarMode::Extended => Self::Extended,
            seam::SidebarMode::Rail => Self::Rail,
            seam::SidebarMode::Hidden => Self::Hidden,
        }
    }
}

impl From<SidebarMode> for seam::SidebarMode {
    fn from(m: SidebarMode) -> Self {
        match m {
            SidebarMode::Extended => Self::Extended,
            SidebarMode::Rail => Self::Rail,
            SidebarMode::Hidden => Self::Hidden,
        }
    }
}

/// An intent to report once the driver is no longer borrowed (the host may re-render the
/// models from it, which comes back here as `shell-model-changed`).
enum Emit {
    Activated(i32),
    Moved(i32, i32),
}

pub struct Driver {
    tabs: TabRows,
    /// Tab ids of the rows, in the Slint model's order.
    ids: Vec<i32>,
    spaces: SpaceSwitcher,
    seam: Seam,
    scroll: Scroller,
    row_y: Rc<VecModel<f32>>,
    row_scale: Rc<VecModel<f32>>,
    /// The scroller's millisecond clock: real time from this origin once installed; without
    /// one (tests) it is the sum of the frame deltas passed to [`Driver::advance`].
    origin: Option<Instant>,
    clock_ms: f64,
    /// Time of the previous frame while moving; `None` while idle, so the first frame after a
    /// wake steps with dt 0.
    last: Option<Instant>,
}

/// The rows' tab ids and the active tab's row.
fn model_rows(window: &AppWindow) -> (Vec<i32>, Option<usize>) {
    let ids: Vec<i32> = window.get_tabs().iter().map(|t| t.id).collect();
    let active = window.get_active_tab_id();
    let row = ids.iter().position(|&id| id == active);
    (ids, row)
}

fn active_space(window: &AppWindow) -> Option<usize> {
    let active = window.get_active_space_id();
    window.get_spaces().iter().position(|s| s.id == active)
}

fn row_ids(ids: &[i32]) -> Vec<u64> {
    ids.iter()
        .map(|&id| u64::try_from(id).unwrap_or(u64::MAX))
        .collect()
}

fn index(i: i32) -> usize {
    usize::try_from(i).unwrap_or(usize::MAX)
}

fn slint_index(i: Option<usize>) -> i32 {
    i.and_then(|i| i32::try_from(i).ok()).unwrap_or(-1)
}

impl Driver {
    fn new(window: &AppWindow) -> Self {
        let row_y = Rc::new(VecModel::<f32>::default());
        let row_scale = Rc::new(VecModel::<f32>::default());
        window.set_row_y(ModelRc::from(row_y.clone()));
        window.set_row_scale(ModelRc::from(row_scale.clone()));
        let (ids, row) = model_rows(window);
        let space = active_space(window);
        window.set_selected_row(slint_index(row));
        window.set_space_index(slint_index(space));
        let seam = Seam::new();
        window.set_sidebar_mode(seam.mode.into());
        let mut driver = Self {
            tabs: TabRows::new(&row_ids(&ids), row),
            ids,
            spaces: SpaceSwitcher::new(space.unwrap_or(0)),
            seam,
            scroll: Scroller::new(),
            row_y,
            row_scale,
            origin: None,
            clock_ms: 0.0,
            last: None,
        };
        driver.advance(window, 0.0);
        driver
    }

    fn now_ms(&self) -> f64 {
        self.origin
            .map_or(self.clock_ms, |o| o.elapsed().as_secs_f64() * 1000.0)
    }

    /// Re-read the tab ids and the active tab and Space from the models.
    fn read_model(&mut self, window: &AppWindow) {
        let (ids, row) = model_rows(window);
        self.tabs.set_ids(&row_ids(&ids), row);
        self.ids = ids;
        window.set_selected_row(slint_index(row));
        let space = active_space(window);
        if let Some(i) = space {
            self.spaces.active = i;
        }
        window.set_space_index(slint_index(space));
    }

    fn set_mode(&mut self, window: &AppWindow, mode: seam::SidebarMode) {
        self.seam.set_mode(mode, true);
        window.set_sidebar_mode(self.seam.mode.into());
    }

    fn tab_up(&mut self, window: &AppWindow) -> Option<Emit> {
        match self.tabs.up() {
            TabUp::Pick(i) => self.ids.get(i).copied().map(Emit::Activated),
            TabUp::Moved(from, to) => {
                let id = *self.ids.get(from)?;
                // Back to the model's order, springs kept: the row settles where the host puts it.
                self.read_model(window);
                Some(Emit::Moved(id, i32::try_from(to).ok()?))
            }
            TabUp::None => None,
        }
    }

    /// Step everything by `dt` seconds (capped at `DT_CAP`) and write the values.
    /// Returns true while another frame is needed.
    pub fn advance(&mut self, window: &AppWindow, dt: f32) -> bool {
        let dt = f64::from(dt).min(DT_CAP);
        self.clock_ms += dt * 1000.0;
        let now = self.now_ms();
        let win = window.window();
        let size = win.size().to_logical(win.scale_factor());
        let (w_win, h_win) = (f64::from(size.width), f64::from(size.height));
        let mut moving = false;

        // tab rows and the indicator (tabs.rs)
        moving |= self.tabs.step(dt);
        sync(
            &self.row_y,
            self.tabs.rows.iter().map(|r| r.y.x as f32).collect(),
        );
        sync(
            &self.row_scale,
            self.tabs.rows.iter().map(|r| r.s.x as f32).collect(),
        );
        let ind = self.tabs.indicator();
        window.set_indicator_y(ind.y as f32);
        window.set_indicator_stretch(ind.stretch as f32);
        window.set_indicator_origin(ind.origin as f32);

        // the Space capsule (tabs.rs SpaceSwitcher)
        moving |= self.spaces.step(dt);
        let cap = self.spaces.indicator();
        window.set_capsule_offset(cap.y as f32);
        window.set_capsule_stretch(cap.stretch as f32);
        window.set_capsule_origin(cap.origin as f32);

        // the seam (seam.rs): width, contents a beat behind, the bowed string
        moving |= self.seam.step(dt, h_win);
        let w = self.seam.w.x.max(0.0);
        window.set_sidebar_width(w as f32);
        window.set_sidebar_contents(self.seam.contents.x.max(if w > 100.0 {
            SEAM::min
        } else {
            SEAM::rail
        }) as f32);
        window.set_sidebar_opacity(1f64.min(w / SEAM::rail).powf(2.0) as f32);
        window.set_sidebar_rail(w < 140.0);
        window.set_sidebar_saved(self.seam.saved as f32);
        let bowed = self.seam.held || self.seam.bend.x.abs() > 0.05;
        window.set_seam_bowed(bowed);
        if bowed {
            // The string runs along the web view's left edge, in the web view's frame (inset on
            // every side); app.slint decides where that edge is, from the width set above.
            let a = f64::from(window.get_content_x()) - INSET;
            let r = f64::from(window.global::<Radius>().get_md());
            let (lw, lh) = (w_win - INSET * 2.0, h_win - INSET * 2.0);
            let (_path, line) = self.seam.clip(a, lw, lh, r, self.seam.gy - INSET);
            window.set_seam_path(line.into());
        } else {
            window.set_seam_path(Default::default());
        }

        // the tab list's scroller (scroller.rs)
        self.scroll.measure(
            f64::from(window.get_scroll_viewport()),
            f64::from(window.get_scroll_content()),
        );
        moving |= self.scroll.step(dt, now);
        let r = self.scroll.render(now);
        window.set_scroll_y(r.shown as f32);
        if let Some((top, h)) = r.thumb {
            window.set_thumb_top(top as f32);
            window.set_thumb_height(h as f32);
        }
        window.set_thumb_visible(r.thumb_visible);
        moving
    }

    /// One rendered frame at `now`. Returns true while another frame is needed; once idle it
    /// forgets the last frame time, so the next wake steps with dt 0.
    pub fn frame(&mut self, window: &AppWindow, now: Instant) -> bool {
        let dt = self
            .last
            .replace(now)
            .map_or(0.0, |prev| now.duration_since(prev).as_secs_f32());
        let moving = self.advance(window, dt);
        if !moving {
            self.last = None;
        }
        moving
    }
}

type Shared = Rc<RefCell<Driver>>;

/// Run `f` on the driver, wake the frame loop, then report what `f` asked for.
fn run(
    driver: &RefCell<Driver>,
    window: &Weak<AppWindow>,
    f: impl FnOnce(&mut Driver, &AppWindow) -> Option<Emit>,
) {
    let Some(window) = window.upgrade() else {
        return;
    };
    let emit = f(&mut driver.borrow_mut(), &window);
    window.window().request_redraw();
    match emit {
        Some(Emit::Activated(id)) => window.invoke_tab_activated(id),
        Some(Emit::Moved(id, to)) => window.invoke_tab_moved(id, to),
        None => {}
    }
}

/// Connect the Slint callbacks to a new driver. Separate from [`install`] so tests can drive
/// it without a renderer.
pub fn connect(window: &AppWindow) -> Shared {
    let driver = Rc::new(RefCell::new(Driver::new(window)));
    let hook = || (Rc::clone(&driver), window.as_weak());

    let (d, w) = hook();
    window.on_shell_model_changed(move || {
        run(&d, &w, |s, w| {
            s.read_model(w);
            None
        })
    });

    // tabs: press → maybe drag (6 px) → release picks or reorders
    let (d, w) = hook();
    window.on_tab_press(move |i, y| {
        run(&d, &w, |s, _| {
            let i = index(i);
            if i < s.tabs.rows.len() {
                s.tabs.down(i, f64::from(y));
            }
            None
        })
    });
    let (d, w) = hook();
    window.on_tab_drag(move |y| {
        run(&d, &w, |s, _| {
            s.tabs.r#move(f64::from(y));
            None
        })
    });
    let (d, w) = hook();
    window.on_tab_release(move |_| run(&d, &w, |s, w| s.tab_up(w)));

    // seam: one gesture for resize, rail and hidden
    let (d, w) = hook();
    window.on_seam_grab(move |x, y| {
        run(&d, &w, |s, w| {
            let win = w.window();
            let h = win.size().to_logical(win.scale_factor()).height;
            s.seam.grab(f64::from(x), f64::from(y), f64::from(h));
            None
        })
    });
    let (d, w) = hook();
    window.on_seam_drag(move |x, y| {
        run(&d, &w, |s, _| {
            s.seam.drag(f64::from(x), f64::from(y));
            None
        })
    });
    let (d, w) = hook();
    window.on_seam_release(move || {
        run(&d, &w, |s, w| {
            let mode = s.seam.release();
            w.set_sidebar_mode(mode.into());
            None
        })
    });
    let (d, w) = hook();
    window.on_seam_double_clicked(move || {
        run(&d, &w, |s, w| {
            let mode = if s.seam.mode == seam::SidebarMode::Rail {
                seam::SidebarMode::Extended
            } else {
                seam::SidebarMode::Rail
            };
            s.set_mode(w, mode);
            None
        })
    });
    let (d, w) = hook();
    window.on_sidebar_toggle_requested(move || {
        run(&d, &w, |s, w| {
            if s.seam.held {
                s.seam.release();
            }
            let mode = s.seam.mode.toggled();
            s.set_mode(w, mode);
            None
        })
    });

    // the tab list's scroller: wheel, touch and the thumb
    let (d, w) = hook();
    window.on_scroll_wheel(move |dy| {
        run(&d, &w, |s, _| {
            let now = s.now_ms();
            s.scroll.wheel(f64::from(dy), DeltaMode::Pixel, now);
            None
        })
    });
    let (d, w) = hook();
    window.on_scroll_touch_down(move |y| {
        run(&d, &w, |s, _| {
            let now = s.now_ms();
            s.scroll.touch_down(f64::from(y), now);
            None
        })
    });
    let (d, w) = hook();
    window.on_scroll_touch_move(move |y| {
        run(&d, &w, |s, _| {
            let now = s.now_ms();
            s.scroll.touch_move(f64::from(y), now);
            None
        })
    });
    let (d, w) = hook();
    window.on_scroll_touch_up(move || {
        run(&d, &w, |s, _| {
            s.scroll.touch_up();
            None
        })
    });
    let (d, w) = hook();
    window.on_scroll_thumb_down(move |y| {
        run(&d, &w, |s, _| {
            s.scroll.thumb_down(f64::from(y));
            None
        })
    });
    let (d, w) = hook();
    window.on_scroll_thumb_move(move |y| {
        run(&d, &w, |s, _| {
            s.scroll.thumb_move(f64::from(y));
            None
        })
    });
    let (d, w) = hook();
    window.on_scroll_thumb_up(move || {
        run(&d, &w, |s, _| {
            let now = s.now_ms();
            s.scroll.thumb_up(now);
            None
        })
    });
    driver
}

/// Wire the shell driver and register the window's one rendering notifier.
pub fn install(window: &AppWindow) -> Result<(), SetRenderingNotifierError> {
    let driver = connect(window);
    driver.borrow_mut().origin = Some(Instant::now());
    let weak = window.as_weak();
    window.window().set_rendering_notifier(move |state, _api| {
        if let slint::RenderingState::BeforeRendering = state
            && let Some(window) = weak.upgrade()
            && driver.borrow_mut().frame(&window, Instant::now())
        {
            window.window().request_redraw();
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The Slint `SidebarMode` mirrors `wubble_core::seam::SidebarMode`; the two `From` impls
    /// are exhaustive matches, so a variant added on either side fails to compile.
    #[test]
    fn sidebar_mode_enum_matches_core() {
        use seam::SidebarMode::*;
        for mode in [Extended, Rail, Hidden] {
            assert_eq!(seam::SidebarMode::from(SidebarMode::from(mode)), mode);
        }
        assert_eq!(SidebarMode::from(Rail), SidebarMode::Rail);
    }
}
