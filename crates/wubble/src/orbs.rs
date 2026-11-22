//! The thinking orbs a window shows (wubble_core::orb), and their frames: each orb is a cell
//! with its look, its life on screen ([`Life`], the core's machine) and its own texture
//! (wubble-gfx `orb.rs`). The shell driver's per-frame hook ([`frame`]) draws the orbs whose life
//! says they move, at the shared clock, and nothing else: an orb at rest was drawn once when it
//! came on screen, an orb off screen is not a cell at all. Today only the gallery's specimen puts
//! orbs on a window (`AgentWidgets`, `kit/specimen/agent-widgets.slint`); the shell's own places
//! for them are later wiring (TODO.md).
//!
//! One stage per window, owned here (a window's GPU context is the app's one device).

use std::cell::RefCell;
use std::collections::HashMap;
use std::mem::ManuallyDrop;
use std::rc::Rc;
use std::time::Instant;

use slint::{ComponentHandle, Image, Model, ModelRc, VecModel};
use wubble_core::orb::{DPR_MAX, Life, LifeEffect, LifeEvent, Orb, transition};
use wubble_core::window::WindowId;
use wubble_gfx::GpuContext;
use wubble_gfx::orb::{OrbCanvas, OrbMark, OrbPainter};
use wubble_ui::{AgentWidgets, AppWindow};

/// One orb on the window: what it shows, the side it is shown at (logical px), its life and its
/// texture.
struct Cell {
    orb: Orb,
    side: f32,
    life: Life,
    canvas: Option<OrbCanvas>,
}

/// A window's orbs.
struct Stage {
    painter: OrbPainter,
    cells: Vec<Cell>,
    frames: Rc<VecModel<Image>>,
    /// The shared clock's zero (the web's `performance.now()` origin).
    origin: Instant,
    /// Reused per-frame mark buffer so `paint` does not allocate on every orb frame.
    scratch: Vec<OrbMark>,
}

thread_local! {
    // Never dropped by the thread's exit: a stage's textures are Slint images, and Slint's own
    // thread-locals may be gone by then. A closed window's stage goes with it ([`remove`]).
    static STAGES: ManuallyDrop<RefCell<HashMap<WindowId, Stage>>> =
        ManuallyDrop::new(RefCell::new(HashMap::new()));
}

/// Gives a new window its stage (no orbs yet).
pub fn install(id: WindowId, gpu: &GpuContext, window: &AppWindow) {
    let frames = Rc::new(VecModel::<Image>::default());
    window
        .global::<AgentWidgets>()
        .set_frames(ModelRc::from(frames.clone()));
    STAGES.with(|s| {
        s.borrow_mut().insert(
            id,
            Stage {
                painter: OrbPainter::try_new(gpu),
                cells: vec![],
                frames,
                origin: Instant::now(),
                scratch: Vec::new(),
            },
        )
    });
}

/// The window's stage gone with it.
pub fn remove(id: WindowId) {
    STAGES.with(|s| s.borrow_mut().remove(&id));
}

/// An orb to show: its look, its side in logical px, and whether it is paused.
#[derive(Clone, Copy, Debug)]
pub struct Shown {
    pub orb: Orb,
    pub side: f32,
    pub paused: bool,
}

/// Puts `orbs` on the window in slot order, replacing what it showed (those leave the screen).
/// Each is drawn once now; the moving ones then draw in [`frame`].
pub fn show(id: WindowId, window: &AppWindow, orbs: &[Shown]) {
    let reduced = window.global::<wubble_ui::Calm>().get_reduced();
    let scale = pixel_ratio(window);
    STAGES.with(|s| {
        let mut s = s.borrow_mut();
        let Some(stage) = s.get_mut(&id) else {
            return;
        };
        stage.cells.clear();
        stage.frames.set_vec(vec![Image::default(); orbs.len()]);
        let now = stage.clock(Instant::now());
        for (slot, o) in orbs.iter().enumerate() {
            let mut cell = Cell {
                orb: o.orb,
                side: o.side,
                life: Life::Off,
                canvas: None,
            };
            let event = LifeEvent::Shown {
                t: o.orb.time(now),
                paused: o.paused,
                reduced,
            };
            stage.apply(&mut cell, slot, event, scale);
            stage.cells.push(cell);
        }
    });
}

/// Every orb off the window.
pub fn clear(id: WindowId) {
    STAGES.with(|s| {
        let mut s = s.borrow_mut();
        let Some(stage) = s.get_mut(&id) else {
            return;
        };
        let mut cells = std::mem::take(&mut stage.cells);
        for (slot, cell) in cells.iter_mut().enumerate() {
            stage.apply(cell, slot, LifeEvent::Hidden, 1.0);
        }
        stage.frames.set_vec(vec![]);
    });
}

/// One shell frame: draws every moving orb at the shared clock. Returns whether one moves (the
/// hook asks for the next frame then).
pub fn frame(id: WindowId, window: &AppWindow, now: Instant) -> bool {
    let scale = pixel_ratio(window);
    STAGES.with(|s| {
        let mut s = s.borrow_mut();
        let Some(stage) = s.get_mut(&id) else {
            return false;
        };
        let clock = stage.clock(now);
        let mut cells = std::mem::take(&mut stage.cells);
        let mut moving = false;
        for (slot, cell) in cells.iter_mut().enumerate() {
            if !cell.life.animating() {
                continue;
            }
            moving = true;
            let t = cell.orb.time(clock);
            stage.paint(cell, slot, t, scale);
        }
        stage.cells = cells;
        moving
    })
}

/// The canvas's pixel ratio: the window's, capped as the design caps its canvas (ThinkingOrb.tsx
/// `Math.min(2, devicePixelRatio)`); Slint scales the texture to the side shown above it.
fn pixel_ratio(window: &AppWindow) -> f32 {
    window.window().scale_factor().min(DPR_MAX as f32)
}

/// How many orbs the window shows and how many of them move (`--measure`, the gallery).
pub fn counts(id: WindowId) -> (usize, usize) {
    STAGES.with(|s| {
        s.borrow().get(&id).map_or((0, 0), |st| {
            let moving = st.cells.iter().filter(|c| c.life.animating()).count();
            (st.cells.len(), moving)
        })
    })
}

/// The shared clock under `--frozen-backdrop` (the fidelity runs): every orb at this many
/// seconds, so a shot is the same picture each run and the web harness can draw the same instant.
const FROZEN_CLOCK: f64 = 2.0;

impl Stage {
    /// The shared clock, seconds.
    fn clock(&self, now: Instant) -> f64 {
        if crate::shot::frozen_backdrop() {
            FROZEN_CLOCK
        } else {
            now.saturating_duration_since(self.origin).as_secs_f64()
        }
    }

    /// Steps a cell's life and carries out what the step says.
    fn apply(&mut self, cell: &mut Cell, slot: usize, event: LifeEvent, scale: f32) {
        let (life, effects) = transition(cell.life, event);
        cell.life = life;
        for effect in effects {
            match effect {
                LifeEffect::Paint { t } => self.paint(cell, slot, t, scale),
                // the hook reads the life each frame (`Life::animating`)
                LifeEffect::Animate(_) => {}
            }
        }
        if !cell.life.shown() {
            cell.canvas = None;
        }
    }

    /// Draws the cell's frame at orb time `t` into its texture and shows it at its slot.
    fn paint(&mut self, cell: &mut Cell, slot: usize, t: f64, scale: f32) {
        let side_px = (cell.side * scale).round().max(1.0) as u32;
        if cell.canvas.as_ref().is_none_or(|c| c.size() != side_px) {
            match self.painter.canvas(side_px) {
                Ok(c) => cell.canvas = Some(c),
                Err(e) => {
                    eprintln!("wubble: orb: {e}");
                    return;
                }
            }
        }
        let Some(canvas) = cell.canvas.as_mut() else {
            return;
        };
        let frame = cell.orb.frame(t);
        let marks = cell.orb.marks(&frame);
        let ink = |i: [f64; 4]| i.map(|c| c as f32);
        self.scratch.clear();
        self.scratch.extend(
            marks
                .lines
                .iter()
                .map(|(l, i)| OrbMark::Line {
                    x1: l.x1 as f32,
                    y1: l.y1 as f32,
                    x2: l.x2 as f32,
                    y2: l.y2 as f32,
                    w: l.w as f32,
                    ink: ink(*i),
                })
                .chain(marks.dots.iter().map(|(d, i)| OrbMark::Dot {
                    x: d.x as f32,
                    y: d.y as f32,
                    r: d.r as f32,
                    ink: ink(*i),
                })),
        );
        // physical px per frame unit: the tuned size's CSS px drawn over the shown side
        let per_unit = side_px as f32 / cell.orb.size.px() as f32;
        let image = self.painter.paint(canvas, per_unit, &self.scratch);
        if slot < self.frames.row_count() {
            self.frames.set_row_data(slot, image);
        }
    }
}
