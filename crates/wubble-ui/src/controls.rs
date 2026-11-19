//! The component sheet's springs for Slint: one slot per control instance (`Controls` in
//! `ui/components/controls/registry.slint`), stepped once per frame with the rest of the shell
//! and written to `Controls.views`. The physics is `wubble_core::controls`.
//!
//! Slint has no "destroyed" callback: a control a `for` or `if` removes cannot give its slot
//! back. The registry sweeps like the glass one (`glass.rs`): after enough new slots it bumps
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
use wubble_core::controls::{Segmented, Slider, Toggle};

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
        }
    }

    fn view(&self) -> ControlView {
        if let Slot::Menu(m) = self {
            return menu_view(m);
        }
        // (x, w, scale x, scale y, origin, fill, value, held): one mapping for every control
        let (x, w, sx, sy, origin, fill, value, held) = match self {
            Slot::Menu(_) => unreachable!("handled above"),
            Slot::Toggle(t) => {
                let v = t.view();
                let on = if v.on { 1.0 } else { 0.0 };
                (v.x, 0.0, v.scale_x, v.scale_y, v.origin, v.fill, on, false)
            }
            Slot::Slider(s) => {
                let v = s.view();
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
            ..rest()
        }
    }
}

/// The fields only the menu uses, at rest.
fn rest() -> ControlView {
    ControlView {
        scale_x: 1.0,
        scale_y: 1.0,
        origin: 0.5,
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
            Some(Slot::Menu(_)) | None => {}
        }
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
            Some(Slot::Menu(_)) | None => {}
        }
    }

    fn drag(&mut self, id: i32, px: f32) {
        let t = self.ms();
        match self.slot(id) {
            Some(Slot::Toggle(tg)) => tg.drag(f64::from(px), t),
            Some(Slot::Slider(s)) => s.drag(f64::from(px)),
            Some(Slot::Segmented(s)) => s.drag(f64::from(px), t),
            Some(Slot::Menu(_)) | None => {}
        }
    }

    fn release(&mut self, id: i32) -> f32 {
        match self.slot(id) {
            Some(Slot::Toggle(t)) => t.release().map_or(-1.0, |on| if on { 1.0 } else { 0.0 }),
            Some(Slot::Slider(s)) => s.release().map_or(-1.0, |v| v as f32),
            Some(Slot::Segmented(s)) => s.release().map_or(-1.0, |i| i as f32),
            Some(Slot::Menu(_)) | None => -1.0,
        }
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
            3 => Slot::Menu(MenuState::default()),
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
}
