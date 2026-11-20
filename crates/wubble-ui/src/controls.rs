//! The component sheet's springs for Slint: one slot per control instance (`Controls` in
//! `ui/components/controls/registry.slint`), stepped once per frame with the rest of the shell
//! and written to `Controls.views`. The physics is `wubble_core::controls`.
//!
//! Slint has no "destroyed" callback: a control a `for` or `if` removes cannot give its slot
//! back. The registry sweeps: after enough new slots it bumps
//! `Controls.generation`, every live control answers `alive`, and the next frame frees the
//! rest for reuse.
//!
//! The kit menu takes a slot too (`wubble_core::controls::menu`): its events go through the
//! pure transition at once and come back to Slint as the owner's intents (`MenuEffect`); the
//! frame step times its exit and confirm flash.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

use slint::{ComponentHandle, Model, ModelRc, VecModel};
use wubble_core::controls::menu::{self, Effect, Event, Key, MenuState, Phase, RowKind};
use wubble_core::controls::slider::{Cfg, Key as SliderKey};
use wubble_core::controls::{Ring, Segmented, Slider, Toggle, ToggleSize, confirm, hold};

use crate::{AppWindow, ControlView, Controls as Global, MenuEffect, MenuEntry};

/// A slider's width before it is laid out.
const DEFAULT_TRAVEL: f64 = 200.0;
/// New slots between sweeps.
const SWEEP_AFTER: usize = 64;

enum Slot {
    Toggle(Toggle),
    Slider(Slider),
    Segmented(Segmented),
    Menu(MenuState),
    Hold(hold::State),
    Confirm(confirm::State),
    Swatches(Ring),
}

impl Slot {
    fn step(&mut self, dt: f64) -> bool {
        match self {
            Slot::Toggle(t) => t.step(dt),
            Slot::Slider(s) => s.step(dt),
            Slot::Segmented(s) => s.step(dt),
            Slot::Menu(m) => {
                // the frame that ends the exit or the flash still writes its view
                let was = m.moving();
                if was {
                    *m = menu::transition(m, Event::Tick(dt * 1000.0)).0;
                }
                was
            }
            // the effects are counted in the state, so the view carries them (`dones`, `pulses`)
            Slot::Hold(s) => {
                *s = hold::transition(*s, hold::Event::Tick(dt)).0;
                s.moving()
            }
            Slot::Confirm(s) => {
                *s = confirm::transition(*s, confirm::Event::Tick(dt)).0;
                s.moving()
            }
            Slot::Swatches(r) => r.step(dt),
        }
    }

    fn view(&self) -> ControlView {
        let rest = rest();
        match self {
            Slot::Menu(m) => return menu_view(m),
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
                    dones: v.dones,
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
        // (x, w, scale x, scale y, origin, value, held): the slider's, segmented control's and
        // swatches' ring; the slider's bubble too
        let (mut bubble_x, mut lean) = (0.0, 0.0);
        let (x, w, sx, sy, origin, value, held) = match self {
            Slot::Slider(s) => {
                let v = s.view();
                (bubble_x, lean) = (v.bubble_x, v.lean);
                (v.x, 0.0, v.scale_x, v.scale_y, v.origin, v.value, v.held)
            }
            Slot::Segmented(s) => {
                let v = s.view();
                let selected = v.selected as f64;
                (v.left, v.width, v.scale_x, v.scale_y, 0.5, selected, v.held)
            }
            Slot::Swatches(r) => {
                let v = r.view();
                let idx = r.idx() as f64;
                (v.x, 0.0, v.scale_x, v.scale_y, v.origin, idx, false)
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
            bubble_x: bubble_x as f32,
            lean: lean as f32,
            ..rest
        }
    }
}

/// Every control's view at rest: unit scales and centred origins (the toggle's squash too),
/// and the menu's rows at none.
fn rest() -> ControlView {
    ControlView {
        scale_x: 1.0,
        scale_y: 1.0,
        origin: 0.5,
        squash_x: 1.0,
        squash_y: 1.0,
        origin2: 0.5,
        hl: -1,
        jump: true,
        focus: -1,
        done: -1,
        ..ControlView::default()
    }
}

fn index(i: Option<usize>) -> i32 {
    i.and_then(|i| i32::try_from(i).ok()).unwrap_or(-1)
}

fn menu_view(m: &MenuState) -> ControlView {
    let above = &m.rows[..m.hl.unwrap_or(0).min(m.rows.len())];
    let rules = above.iter().filter(|k| **k == RowKind::Separator).count();
    ControlView {
        hl: index(m.hl),
        hl_rows: index(Some(above.len() - rules)),
        hl_rules: index(Some(rules)),
        jump: m.jump,
        phase: match m.phase {
            Phase::Closed => 0,
            Phase::Open => 1,
            Phase::Closing { .. } => 2,
        },
        focus: index(m.focus),
        done: index(m.done.map(|d| d.0)),
        ..rest()
    }
}

/// A menu event from Slint (`Controls.menu-event`'s kind and arg), `None` for an unknown kind.
fn menu_event(kind: i32, arg: i32) -> Option<Event> {
    let row = usize::try_from(arg).ok();
    Some(match kind {
        0 => Event::Open {
            via_key: false,
            start: None,
        },
        1 => Event::Open {
            via_key: true,
            start: row,
        },
        2 => Event::Close,
        3 => Event::Enter(row?),
        4 => Event::Leave,
        5 => Event::Key(match arg {
            0 => Key::Up,
            1 => Key::Down,
            2 => Key::Escape,
            3 => Key::Tab,
            4 => Key::Activate,
            _ => return None,
        }),
        6 => Event::Choose(row?),
        7 => Event::Outside,
        _ => return None,
    })
}

/// The rows' kinds: `count` rows, the given entries first, plain items past them.
fn row_kinds(count: i32, entries: &ModelRc<MenuEntry>) -> Vec<RowKind> {
    (0..usize::try_from(count).unwrap_or(0))
        .map(|i| match entries.row_data(i) {
            Some(e) if e.separator => RowKind::Separator,
            Some(e) if !e.enabled => RowKind::Disabled,
            _ => RowKind::Item,
        })
        .collect()
}

/// The owner's intents, as Slint reads them.
fn effect_of(fx: &[Effect]) -> MenuEffect {
    let mut out = MenuEffect::default();
    for e in fx {
        match *e {
            Effect::Chosen(i) => out.pick = index(Some(i)) + 1,
            Effect::Dismiss { refocus } => {
                out.dismiss = true;
                out.refocus = refocus;
            }
        }
    }
    out
}

/// Where the registry is in a sweep.
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
            Some(Slot::Hold(s)) => *s = hold::transition(*s, hold::Event::Disable(value > 0.5)).0,
            Some(Slot::Menu(_) | Slot::Confirm(..)) | None => {}
        }
    }

    fn configure(&mut self, id: i32, min: f32, max: f32, step: f32) {
        if let Some(Slot::Slider(s)) = self.slot(id) {
            s.configure(Cfg::new(f64::from(min), f64::from(max), f64::from(step)));
        }
        self.write(id);
    }

    /// A key on a control: the value it reports (see `Controls.key`): the slider's snapped
    /// value, the segment a segmented control asks for (its value stays the caller's).
    fn key(&mut self, id: i32, code: i32, shift: bool) -> f32 {
        let key = SliderKey::from_code(code);
        let v = match self.slot(id) {
            Some(Slot::Slider(s)) => {
                if let Some(k) = key {
                    s.key(k, shift);
                }
                s.view().value
            }
            Some(Slot::Segmented(s)) => s.key(key.and_then(SliderKey::dir).unwrap_or(0)) as f64,
            _ => -1.0,
        };
        self.write(id);
        v as f32
    }

    fn segment(&mut self, id: i32, i: i32, left: f32, width: f32) {
        match (self.slot(id), usize::try_from(i)) {
            (Some(Slot::Segmented(s)), Ok(i)) => {
                s.set_segment(i, f64::from(left), f64::from(width))
            }
            // the chosen swatch laid out: the ring starts on it
            (Some(Slot::Swatches(r)), Ok(i)) => r.place(i),
            _ => {}
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
            Some(Slot::Menu(_) | Slot::Swatches(..)) | None => false,
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
            Some(Slot::Menu(_) | Slot::Swatches(..) | Slot::Hold(..) | Slot::Confirm(..))
            | None => {}
        }
    }

    fn release(&mut self, id: i32) -> f32 {
        let v = match self.slot(id) {
            Some(Slot::Toggle(t)) => t.release().map_or(-1.0, |on| if on { 1.0 } else { 0.0 }),
            Some(Slot::Slider(s)) => s.release().map_or(-1.0, |v| v as f32),
            Some(Slot::Segmented(s)) => s.release().map_or(-1.0, |i| i as f32),
            Some(Slot::Hold(s)) => {
                *s = hold::transition(*s, hold::Event::End).0;
                -1.0
            }
            Some(Slot::Menu(_) | Slot::Swatches(..) | Slot::Confirm(..)) | None => -1.0,
        };
        self.write(id);
        v
    }

    /// One menu event: the rows, then the event, through the pure transition; the view is
    /// written at once and the owner's intents returned.
    fn menu(&mut self, id: i32, event: Option<Event>, rows: Vec<RowKind>) -> MenuEffect {
        let mut fx = vec![];
        if let Some(Slot::Menu(m)) = self.slot(id) {
            let (next, _) = menu::transition(m, Event::Rows(rows));
            *m = next;
            if let Some(event) = event {
                let (next, out) = menu::transition(m, event);
                *m = next;
                fx = out;
            }
        }
        self.write(id);
        effect_of(&fx)
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
            3 => Slot::Menu(MenuState::default()),
            4 => Slot::Hold(
                hold::transition(hold::State::new(), hold::Event::Disable(value > 0.5)).0,
            ),
            5 => Slot::Confirm(confirm::State::new(f64::from(value.max(0.0)))),
            6 => Slot::Swatches(Ring::new(value.max(0.0) as usize)),
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
        let run = r.borrow_mut().press(id, px);
        w();
        run
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
    let (r, w) = (Rc::clone(&reg), wake);
    g.on_menu_event(move |id, kind, arg, count, entries| {
        let fx = r
            .borrow_mut()
            .menu(id, menu_event(kind, arg), row_kinds(count, &entries));
        w();
        fx
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
    use wubble_core::kit_gen::settings::SWATCH;

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
    fn keys_reach_the_slider_in_its_domain_and_the_segmented_only_asks() {
        let mut c = Controls::new();
        let id = c.acquire(Slot::Slider(Slider::new(110.0, 240.0, Cfg::default())));
        c.configure(id, 50.0, 200.0, 10.0);
        c.set_value(id, 110.0);
        assert_eq!(c.key(id, 1, false), 120.0, "ArrowRight: one step");
        assert_eq!(c.key(id, 1, true), 200.0, "Shift: ten steps, clamped");
        assert_eq!(c.key(id, 6, false), 50.0, "Home");
        assert_eq!(c.views.row_data(0).unwrap().value, 50.0);
        let seg = c.acquire(Slot::Segmented(Segmented::new(0)));
        for i in 0..3 {
            c.segment(seg, i, i as f32 * 60.0, 60.0);
        }
        assert_eq!(c.key(seg, 0, false), 0.0, "clamped at the first");
        assert_eq!(c.key(seg, 1, false), 1.0, "ArrowRight asks for the next");
        assert_eq!(
            c.key(seg, 1, false),
            1.0,
            "and the value stays the caller's"
        );
        assert_eq!(c.views.row_data(1).unwrap().value, 0.0);
        c.set_value(seg, 2.0);
        assert_eq!(c.key(seg, 1, false), 2.0, "clamped at the last");
        assert_eq!(c.key(seg, 4, false), 2.0, "PageUp moves no segment");
        let sw = c.acquire(Slot::Swatches(Ring::new(0)));
        c.segment(sw, 2, 0.0, 0.0);
        let v = c.views.row_data(2).unwrap();
        assert_eq!(
            (v.value, v.x),
            (2.0, (2.0 * SWATCH::pitch) as f32),
            "placed, not travelling"
        );
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

    #[test]
    fn a_menu_slot_returns_intents_and_times_its_exit() {
        let mut c = Controls::new();
        let id = c.acquire(Slot::Menu(MenuState::default()));
        let rows = || vec![RowKind::Item, RowKind::Separator, RowKind::Item];
        c.menu(id, menu_event(0, -1), rows());
        assert_eq!(c.views.row_data(0).unwrap().phase, 1);
        c.menu(id, menu_event(5, 1), rows());
        c.menu(id, menu_event(5, 1), rows());
        let v = c.views.row_data(0).unwrap();
        assert_eq!((v.focus, v.hl, v.hl_rows, v.hl_rules), (2, 2, 1, 1));
        let fx = c.menu(id, menu_event(5, 4), rows());
        assert_eq!((fx.pick, fx.dismiss, fx.refocus), (3, true, true));
        c.menu(id, menu_event(2, 0), rows());
        assert_eq!(c.views.row_data(0).unwrap().phase, 2);
        let mut t = Instant::now();
        c.frame(t);
        for _ in 0..8 {
            t += Duration::from_millis(30);
            c.frame(t);
        }
        let v = c.views.row_data(0).unwrap();
        assert_eq!((v.phase, v.done), (0, 2), "closed; still confirming");
        assert!(menu_event(9, 0).is_none());
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
            (v.dones, v.hit, v.phase, v.value),
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
