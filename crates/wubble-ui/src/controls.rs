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
use wubble_core::controls::{Segmented, Slider, Toggle, ToggleSize, confirm, hold};

use crate::{AppWindow, ControlView, Controls as Global};

/// A slider's width before it is laid out.
const DEFAULT_TRAVEL: f64 = 200.0;
/// New slots between sweeps.
const SWEEP_AFTER: usize = 64;

enum Slot {
    Toggle(Toggle),
    Slider(Slider),
    Segmented(Segmented),
    Hold(hold::State),
    Confirm(confirm::State),
}

impl Slot {
    fn step(&mut self, dt: f64) -> bool {
        match self {
            Slot::Toggle(t) => t.step(dt),
            Slot::Slider(s) => s.step(dt),
            Slot::Segmented(s) => s.step(dt),
            // the effects are counted in the state, so the view carries them (`dones`, `pulses`)
            Slot::Hold(s) => {
                *s = hold::transition(*s, hold::Event::Tick(dt)).0;
                s.moving()
            }
            Slot::Confirm(s) => {
                *s = confirm::transition(*s, confirm::Event::Tick(dt)).0;
                s.moving()
            }
        }
    }

    fn view(&self) -> ControlView {
        let rest = ControlView {
            scale_x: 1.0,
            scale_y: 1.0,
            origin: 0.5,
            squash_x: 1.0,
            squash_y: 1.0,
            origin2: 0.5,
            ..ControlView::default()
        };
        match self {
            Slot::Toggle(t) => {
                let v = t.view();
                return ControlView {
                    x: v.x as f32,
                    scale_x: v.scale_x as f32,
                    scale_y: v.scale_y as f32,
                    origin: v.origin as f32,
                    fill: v.fill as f32,
                    value: if v.on { 1.0 } else { 0.0 },
                    held: v.held,
                    squash_x: v.squash_x as f32,
                    squash_y: v.squash_y as f32,
                    origin2: v.squash_origin as f32,
                    ..rest
                };
            }
            Slot::Hold(s) => {
                let v = s.view();
                return ControlView {
                    scale_x: v.scale_x as f32,
                    scale_y: v.scale_y as f32,
                    value: v.p as f32,
                    held: v.held,
                    phase: if v.done { 2 } else { i32::from(v.held) },
                    hit: v.pulses,
                    done: v.dones,
                    ..rest
                };
            }
            Slot::Confirm(s) => {
                return ControlView {
                    phase: s.phase.index(),
                    hit: s.pulses,
                    ..rest
                };
            }
            _ => {}
        }
        // (x, w, scale x, scale y, origin, value, held): the slider's and segmented control's
        let (x, w, sx, sy, origin, value, held) = match self {
            Slot::Slider(s) => {
                let v = s.view();
                (v.x, 0.0, v.scale_x, v.scale_y, v.origin, v.value, v.held)
            }
            Slot::Segmented(s) => {
                let v = s.view();
                let selected = v.selected as f64;
                (v.left, v.width, v.scale_x, v.scale_y, 0.5, selected, v.held)
            }
            _ => unreachable!("matched above"),
        };
        ControlView {
            x: x as f32,
            w: w as f32,
            scale_x: sx as f32,
            scale_y: sy as f32,
            origin: origin as f32,
            value: value as f32,
            held,
            ..rest
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
            Some(Slot::Hold(s)) => *s = hold::transition(*s, hold::Event::Disable(value > 0.5)).0,
            Some(Slot::Confirm(..)) | None => {}
        }
    }

    fn segment(&mut self, id: i32, i: i32, left: f32, width: f32) {
        if let (Some(Slot::Segmented(s)), Ok(i)) = (self.slot(id), usize::try_from(i)) {
            s.set_segment(i, f64::from(left), f64::from(width));
        }
        self.write(id);
    }

    fn resize(&mut self, id: i32, travel: f32) {
        match self.slot(id) {
            Some(Slot::Slider(s)) => s.set_width(f64::from(travel)),
            Some(Slot::Toggle(t)) => t.resize(f64::from(travel)),
            _ => {}
        }
        self.write(id);
    }

    /// Returns whether the press asks the control to run its action (confirm: `Effect::Run`).
    fn press(&mut self, id: i32, px: f32) -> bool {
        let t = self.ms();
        let run = match self.slot(id) {
            Some(Slot::Toggle(tg)) => {
                tg.press(f64::from(px), t);
                false
            }
            Some(Slot::Slider(s)) => {
                s.press(f64::from(px));
                false
            }
            Some(Slot::Segmented(s)) => {
                s.press(f64::from(px), t);
                false
            }
            Some(Slot::Hold(s)) => {
                *s = hold::transition(*s, hold::Event::Start).0;
                false
            }
            Some(Slot::Confirm(s)) => {
                let (n, fx) = confirm::transition(*s, confirm::Event::Click);
                *s = n;
                fx.contains(&confirm::Effect::Run)
            }
            None => false,
        };
        self.write(id);
        run
    }

    fn drag(&mut self, id: i32, px: f32) {
        let t = self.ms();
        match self.slot(id) {
            Some(Slot::Toggle(tg)) => tg.drag(f64::from(px), t),
            Some(Slot::Slider(s)) => s.drag(f64::from(px)),
            Some(Slot::Segmented(s)) => s.drag(f64::from(px), t),
            Some(Slot::Hold(..) | Slot::Confirm(..)) | None => {}
        }
    }

    fn release(&mut self, id: i32) -> f32 {
        match self.slot(id) {
            Some(Slot::Toggle(t)) => t.release().map_or(-1.0, |on| if on { 1.0 } else { 0.0 }),
            Some(Slot::Slider(s)) => s.release().map_or(-1.0, |v| v as f32),
            Some(Slot::Segmented(s)) => s.release().map_or(-1.0, |i| i as f32),
            Some(Slot::Hold(s)) => {
                *s = hold::transition(*s, hold::Event::End).0;
                -1.0
            }
            Some(Slot::Confirm(..)) | None => -1.0,
        }
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
            if let Some(s) = slot {
                moving |= s.step(dt);
                // the step that comes to rest (or ends a countdown) still changes the view
                let v = s.view();
                if self.views.row_data(i).as_ref() != Some(&v) {
                    self.views.set_row_data(i, v);
                }
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
            0 => Slot::Toggle(Toggle::new(value > 0.5, ToggleSize::Lg)),
            2 => Slot::Segmented(Segmented::new(value.max(0.0) as usize)),
            3 => Slot::Hold(
                hold::transition(hold::State::new(), hold::Event::Disable(value > 0.5)).0,
            ),
            4 => Slot::Confirm(confirm::State::new(f64::from(value.max(0.0)))),
            _ => Slot::Slider(Slider::new(f64::from(value), DEFAULT_TRAVEL)),
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
        let run = r.borrow_mut().press(id, px);
        w();
        run
    });
    let (r, w) = (Rc::clone(&reg), wake.clone());
    g.on_drag(move |id, px| {
        r.borrow_mut().drag(id, px);
        w();
    });
    let (r, w) = (Rc::clone(&reg), wake);
    g.on_release(move |id| {
        let v = r.borrow_mut().release(id);
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
            .map(|_| c.acquire(Slot::Toggle(Toggle::new(false, ToggleSize::Lg))))
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
            c.acquire(Slot::Toggle(Toggle::new(true, ToggleSize::Lg))),
            0,
            "a freed slot is reused"
        );
        assert_eq!(c.views.row_count(), SWEEP_AFTER);
    }

    #[test]
    fn a_toggle_tap_flips_and_settles() {
        let mut c = Controls::new();
        let id = c.acquire(Slot::Toggle(Toggle::new(false, ToggleSize::Lg)));
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

    /// Runs frames 8 ms apart until nothing moves; returns how many.
    fn settle(c: &mut Controls) -> usize {
        let mut t = Instant::now();
        let mut n = 0;
        while c.frame(t).0 {
            t += Duration::from_millis(8);
            n += 1;
            assert!(n < 10_000);
        }
        n
    }

    #[test]
    fn a_hold_slot_fills_completes_once_and_resets() {
        let mut c = Controls::new();
        let id = c.acquire(Slot::Hold(hold::State::new()));
        assert!(!c.press(id, 0.0), "a hold runs at the end, not the press");
        settle(&mut c);
        let v = c.views.row_data(0).unwrap();
        assert_eq!(
            (v.done, v.hit, v.phase, v.value),
            (1, 1, 0, 0.0),
            "done once, pulsed once, back to idle"
        );
        // disabled: a press does nothing
        c.set_value(id, 1.0);
        c.press(id, 0.0);
        assert_eq!(settle(&mut c), 0);
        assert_eq!(c.views.row_data(0).unwrap().phase, 0);
    }

    #[test]
    fn a_confirm_slot_runs_busy_then_done() {
        let mut c = Controls::new();
        let id = c.acquire(Slot::Confirm(confirm::State::new(400.0)));
        assert!(c.press(id, 0.0), "the idle click runs the action");
        assert_eq!(c.views.row_data(0).unwrap().phase, 1, "busy at once");
        assert!(!c.press(id, 0.0), "a click while busy does not run it");
        settle(&mut c);
        let v = c.views.row_data(0).unwrap();
        assert_eq!(
            (v.phase, v.hit),
            (0, 1),
            "one pulse; the click while busy was ignored"
        );
    }
}
