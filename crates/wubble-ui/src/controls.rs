//! The component sheet's springs for Slint: one slot per control instance (`Controls` in
//! `ui/components/controls/registry.slint`), stepped once per frame with the rest of the shell
//! and written to `Controls.views`. The physics is `wubble_core::controls`.
//!
//! Slint has no "destroyed" callback: a control a `for` or `if` removes cannot give its slot
//! back. The registry sweeps like the glass one (`glass.rs`): after enough new slots it bumps
//! `Controls.generation`, every live control answers `alive`, and the next frame frees the
//! rest for reuse.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

use slint::{ComponentHandle, Model, ModelRc, VecModel};
use wubble_core::controls::slider::{Cfg, Key};
use wubble_core::controls::{Ring, Segmented, Slider, Toggle};

use crate::{AppWindow, ControlView, Controls as Global};

/// A slider's width before it is laid out.
const DEFAULT_TRAVEL: f64 = 200.0;
/// New slots between sweeps.
const SWEEP_AFTER: usize = 64;

enum Slot {
    Toggle(Toggle),
    Slider(Slider),
    Segmented(Segmented),
    Swatches(Ring),
}

impl Slot {
    fn step(&mut self, dt: f64) -> bool {
        match self {
            Slot::Toggle(t) => t.step(dt),
            Slot::Slider(s) => s.step(dt),
            Slot::Segmented(s) => s.step(dt),
            Slot::Swatches(r) => r.step(dt),
        }
    }

    fn view(&self) -> ControlView {
        // (x, w, scale x, scale y, origin, fill, value, held): one mapping for every control
        let (mut bubble_x, mut lean) = (0.0, 0.0);
        let (x, w, sx, sy, origin, fill, value, held) = match self {
            Slot::Toggle(t) => {
                let v = t.view();
                let on = if v.on { 1.0 } else { 0.0 };
                (v.x, 0.0, v.scale_x, v.scale_y, v.origin, v.fill, on, false)
            }
            Slot::Slider(s) => {
                let v = s.view();
                (bubble_x, lean) = (v.bubble_x, v.lean);
                (
                    v.x, 0.0, v.scale_x, v.scale_y, v.origin, 0.0, v.value, v.held,
                )
            }
            Slot::Segmented(s) => {
                let v = s.view();
                let selected = v.selected as f64;
                (
                    v.left, v.width, v.scale_x, v.scale_y, 0.5, 0.0, selected, v.held,
                )
            }
            Slot::Swatches(r) => {
                let v = r.view();
                let idx = r.idx() as f64;
                (v.x, 0.0, v.scale_x, v.scale_y, v.origin, 0.0, idx, false)
            }
        };
        ControlView {
            x: x as f32,
            w: w as f32,
            scale_x: sx as f32,
            scale_y: sy as f32,
            origin: origin as f32,
            fill: fill as f32,
            value: value as f32,
            held,
            bubble_x: bubble_x as f32,
            lean: lean as f32,
        }
    }
}

/// Where the registry is in a sweep (the glass registry's states).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Sweep {
    Idle,
    /// Generation bumped this frame; slots answer `alive` before the next.
    Asking,
}

pub struct Controls {
    slots: Vec<Option<Slot>>,
    /// Slots answered `alive` since the last bump.
    alive: Vec<bool>,
    since_sweep: usize,
    sweep: Sweep,
    views: Rc<VecModel<ControlView>>,
    /// The time base of the toggle's fling velocity.
    epoch: Instant,
    /// The previous frame while something moves.
    last: Option<Instant>,
}

impl Controls {
    fn new() -> Self {
        Self {
            slots: vec![],
            alive: vec![],
            since_sweep: 0,
            sweep: Sweep::Idle,
            views: Rc::new(VecModel::default()),
            epoch: Instant::now(),
            last: None,
        }
    }

    fn ms(&self) -> f64 {
        self.epoch.elapsed().as_secs_f64() * 1000.0
    }

    fn acquire(&mut self, slot: Slot) -> i32 {
        let view = slot.view();
        self.since_sweep += 1;
        let i = match self.slots.iter().position(Option::is_none) {
            Some(i) => {
                self.slots[i] = Some(slot);
                self.alive[i] = true;
                self.views.set_row_data(i, view);
                i
            }
            None => {
                self.slots.push(Some(slot));
                self.alive.push(true);
                self.views.push(view);
                self.slots.len() - 1
            }
        };
        i32::try_from(i).unwrap_or(-1)
    }

    fn slot(&mut self, id: i32) -> Option<&mut Slot> {
        usize::try_from(id)
            .ok()
            .and_then(|i| self.slots.get_mut(i))
            .and_then(Option::as_mut)
    }

    fn set_value(&mut self, id: i32, value: f32) {
        match self.slot(id) {
            Some(Slot::Toggle(t)) => t.set(value > 0.5),
            Some(Slot::Slider(s)) => s.set(f64::from(value)),
            Some(Slot::Segmented(s)) => s.set(value.max(0.0) as usize),
            Some(Slot::Swatches(r)) => r.set(value.max(0.0) as usize),
            None => {}
        }
    }

    fn configure(&mut self, id: i32, min: f32, max: f32, step: f32) {
        match self.slot(id) {
            Some(Slot::Slider(s)) => {
                s.configure(Cfg::new(f64::from(min), f64::from(max), f64::from(step)))
            }
            // the swatches' count is their `max`
            Some(Slot::Swatches(r)) => r.set_count(max.max(0.0) as usize),
            _ => {}
        }
        self.write(id);
    }

    /// A key on a control: the value it now reports (see `Controls.key`).
    fn key(&mut self, id: i32, code: i32, shift: bool) -> f32 {
        let dir = match Key::from_code(code) {
            Some(Key::Left) => -1,
            Some(Key::Right) => 1,
            _ => 0,
        };
        let v = match (self.slot(id), Key::from_code(code)) {
            (Some(Slot::Slider(s)), Some(k)) => {
                s.key(k, shift);
                s.view().value
            }
            (Some(Slot::Segmented(s)), _) => {
                if dir != 0 {
                    s.key(dir);
                }
                s.view().selected as f64
            }
            (Some(Slot::Swatches(r)), _) => {
                if dir != 0 {
                    r.key(dir);
                }
                r.idx() as f64
            }
            (Some(Slot::Slider(s)), None) => s.view().value,
            _ => -1.0,
        };
        self.write(id);
        v as f32
    }

    fn segment(&mut self, id: i32, i: i32, left: f32, width: f32) {
        if let (Some(Slot::Segmented(s)), Ok(i)) = (self.slot(id), usize::try_from(i)) {
            s.set_segment(i, f64::from(left), f64::from(width));
        }
        self.write(id);
    }

    fn resize(&mut self, id: i32, travel: f32) {
        if let Some(Slot::Slider(s)) = self.slot(id) {
            s.set_width(f64::from(travel));
        }
        self.write(id);
    }

    fn press(&mut self, id: i32, px: f32) {
        let t = self.ms();
        match self.slot(id) {
            Some(Slot::Toggle(tg)) => tg.press(f64::from(px), t),
            Some(Slot::Slider(s)) => s.press(f64::from(px)),
            Some(Slot::Segmented(s)) => s.press(f64::from(px), t),
            _ => {}
        }
        self.write(id);
    }

    fn drag(&mut self, id: i32, px: f32) {
        let t = self.ms();
        match self.slot(id) {
            Some(Slot::Toggle(tg)) => tg.drag(f64::from(px), t),
            Some(Slot::Slider(s)) => s.drag(f64::from(px)),
            Some(Slot::Segmented(s)) => s.drag(f64::from(px), t),
            _ => {}
        }
    }

    fn release(&mut self, id: i32) -> f32 {
        let v = match self.slot(id) {
            Some(Slot::Toggle(t)) => t.release().map_or(-1.0, |on| if on { 1.0 } else { 0.0 }),
            Some(Slot::Slider(s)) => s.release().map_or(-1.0, |v| v as f32),
            Some(Slot::Segmented(s)) => s.release().map_or(-1.0, |i| i as f32),
            _ => -1.0,
        };
        self.write(id);
        v
    }

    fn write(&mut self, id: i32) {
        if let Ok(i) = usize::try_from(id)
            && let Some(Some(s)) = self.slots.get(i)
        {
            self.views.set_row_data(i, s.view());
        }
    }

    /// One frame: frees the slots a sweep found dead, steps every live control and writes the
    /// views. Returns (another frame needed, bump the generation).
    pub fn frame(&mut self, now: Instant) -> (bool, bool) {
        if self.sweep == Sweep::Asking {
            for (slot, alive) in self.slots.iter_mut().zip(&self.alive) {
                if !alive {
                    *slot = None;
                }
            }
            self.sweep = Sweep::Idle;
        }
        let bump = self.since_sweep >= SWEEP_AFTER;
        if bump {
            self.since_sweep = 0;
            self.alive.iter_mut().for_each(|a| *a = false);
            self.sweep = Sweep::Asking;
        }
        let dt = self
            .last
            .replace(now)
            .map_or(0.0, |p| now.duration_since(p).as_secs_f64())
            .min(wubble_core::motion::DT_CAP);
        let mut moving = false;
        for (i, slot) in self.slots.iter_mut().enumerate() {
            if let Some(s) = slot
                && s.step(dt)
            {
                moving = true;
                self.views.set_row_data(i, s.view());
            }
        }
        if !moving {
            self.last = None;
        }
        (moving || bump, bump)
    }

    #[cfg(test)]
    fn live(&self) -> usize {
        self.slots.iter().filter(|s| s.is_some()).count()
    }
}

/// Connects the `Controls` global of `window` to a new registry.
pub fn wire(window: &AppWindow) -> Rc<RefCell<Controls>> {
    let reg = Rc::new(RefCell::new(Controls::new()));
    let g = window.global::<Global>();
    g.set_views(ModelRc::from(reg.borrow().views.clone()));
    let wake = {
        let w = window.as_weak();
        move || {
            if let Some(w) = w.upgrade() {
                w.window().request_redraw();
            }
        }
    };
    let r = Rc::clone(&reg);
    g.on_acquire(move |kind, value| {
        let slot = match kind {
            0 => Slot::Toggle(Toggle::new(value > 0.5)),
            2 => Slot::Segmented(Segmented::new(value.max(0.0) as usize)),
            3 => Slot::Swatches(Ring::new(value.max(0.0) as usize)),
            _ => Slot::Slider(Slider::new(
                f64::from(value),
                DEFAULT_TRAVEL,
                Cfg::default(),
            )),
        };
        r.borrow_mut().acquire(slot)
    });
    let (r, w) = (Rc::clone(&reg), wake.clone());
    g.on_set_value(move |id, v| {
        r.borrow_mut().set_value(id, v);
        w();
    });
    let r = Rc::clone(&reg);
    g.on_resize(move |id, travel| r.borrow_mut().resize(id, travel));
    let r = Rc::clone(&reg);
    g.on_segment(move |id, i, left, width| r.borrow_mut().segment(id, i, left, width));
    let (r, w) = (Rc::clone(&reg), wake.clone());
    g.on_press(move |id, px| {
        r.borrow_mut().press(id, px);
        w();
    });
    let (r, w) = (Rc::clone(&reg), wake.clone());
    g.on_drag(move |id, px| {
        r.borrow_mut().drag(id, px);
        w();
    });
    let (r, w) = (Rc::clone(&reg), wake.clone());
    g.on_release(move |id| {
        let v = r.borrow_mut().release(id);
        w();
        v
    });
    let (r, w) = (Rc::clone(&reg), wake.clone());
    g.on_configure(move |id, min, max, step| {
        r.borrow_mut().configure(id, min, max, step);
        w();
    });
    let (r, w) = (Rc::clone(&reg), wake.clone());
    g.on_key(move |id, code, shift| {
        let v = r.borrow_mut().key(id, code, shift);
        w();
        v
    });
    let r = Rc::clone(&reg);
    g.on_alive(move |id| {
        if let Ok(i) = usize::try_from(id)
            && let Some(a) = r.borrow_mut().alive.get_mut(i)
        {
            *a = true;
        }
    });
    reg
}

/// Bumps `Controls.generation` (live controls answer `alive`).
pub fn bump(window: &AppWindow) {
    let g = window.global::<Global>();
    g.set_generation(g.get_generation().wrapping_add(1));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn slots_are_reused_after_a_sweep_frees_the_dead() {
        let mut c = Controls::new();
        let ids: Vec<i32> = (0..SWEEP_AFTER)
            .map(|_| c.acquire(Slot::Toggle(Toggle::new(false))))
            .collect();
        assert_eq!(ids[3], 3);
        let t0 = Instant::now();
        let (_, bump) = c.frame(t0);
        assert!(bump, "enough new slots: sweep");
        // only slot 3 answers
        c.alive[3] = true;
        c.frame(t0 + Duration::from_millis(16));
        assert_eq!(c.live(), 1);
        assert_eq!(
            c.acquire(Slot::Toggle(Toggle::new(true))),
            0,
            "a freed slot is reused"
        );
        assert_eq!(c.views.row_count(), SWEEP_AFTER);
    }

    #[test]
    fn a_toggle_tap_flips_and_settles() {
        let mut c = Controls::new();
        let id = c.acquire(Slot::Toggle(Toggle::new(false)));
        c.press(id, 5.0);
        assert_eq!(c.release(id), 1.0);
        assert_eq!(c.release(id), -1.0, "released once");
        let mut t = Instant::now();
        let mut n = 0;
        loop {
            t += Duration::from_millis(8);
            if !c.frame(t).0 {
                break;
            }
            n += 1;
            assert!(n < 10_000);
        }
        let v = c.views.row_data(0).unwrap();
        assert_eq!((v.x, v.fill, v.value), (40.0, 1.0, 1.0));
    }
}
