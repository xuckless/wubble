//! The host's half of the start page (`wubble://start`, the new-tab page; TODO.md Decided "Start
//! page"): one living bubble (`wubble_core::start::living::Living`) per start tab, driven by the
//! page's pointer and the scene's frames, and what it does handed to the scene's start layer
//! (`wubble_gfx::scene::StartFx`, `StartDraw`). The physics are wubble-core's, the passes
//! wubble-gfx's; this file only maps one onto the other, field by field.
//!
//! - Which start tab shows comes from the page layer ([`super::Host::render_pages`] through
//!   `Glass::start_sync`): a `Living` is made on its first show (its random stream seeded from
//!   the tab id and a per-run nonce), told `Active(false)` when another tab shows, and dropped
//!   with its tab. Sleep, discard, session restore and other windows need nothing more: a
//!   start tab is an internal page with no view, and its bubble has no state worth keeping.
//! - The page's pointer (the `StartPage` stage, page-local logical px) becomes `Down / Move /
//!   Up / Leave` with a ms timestamp; the scene's frame becomes `Resize` (the page card's size),
//!   `Reduced` (the shell's motion flag) and `Tick { dt }`.
//! - The effects: `Ripple / Light / LightLeave / Pop` go to the layer's liquid, `Cursor` to the
//!   stage's cursor. `Capture` needs nothing: Slint's `TouchArea` grabs the pointer while
//!   pressed (`input_items.rs`, `InputEventResult::GrabMouse`). `Poked` / `Popped` have no
//!   sound or haptics yet.
//! - The W field (about 28 M iterations) is computed once per window on a thread at the first
//!   start tab, never on the frame path; until it arrives the page shows its liquid without
//!   the bubble.

use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::Instant;

use slint::{ComponentHandle, Model, ModelRc, VecModel};
use wubble_core::action::Action;
use wubble_core::kit_gen::living as K;
use wubble_core::kit_gen::scene as KS;
use wubble_core::pages::InternalPage;
use wubble_core::start::frame::{BubbleUniforms, PopUniforms};
use wubble_core::start::living::{Cursor, Effect, Event, Living, Options, transition};
use wubble_core::start::rand::Rng;
use wubble_core::start::wfield::{W_SIZE, w_field_rgba};
use wubble_core::tab::TabId;
use wubble_gfx::scene::{BubbleDraw, PopDraw, StartCard, StartDraw, StartFx};
use wubble_ui::{AppWindow, StartBridge, StartSite, TabItem};

use super::{BarMode, Host};

/// A press, a move or a release on the page's stage (outside the front's widgets), or the
/// pointer leaving it: page-local logical px.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum StagePointer {
    Down(f64, f64),
    Move(f64, f64),
    Up,
    Leave,
}

/// What one input or frame does outside the `Living`: the layer's liquid, and the stage's
/// cursor when it changes (`Some(true)` grabbing).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Out {
    pub fx: Vec<StartFx>,
    pub grabbing: Option<bool>,
}

/// The start page's step per tick under `--frozen-backdrop`: the fidelity harness's fixed clock
/// (tools/fidelity/README.md, `__wb.step(1000 / 120)`).
pub const FROZEN_STEP: f64 = 1.0 / 120.0;

/// The scene time the start page is shot at under `--frozen-backdrop`: the web capture's
/// (tools/fidelity/scenes.json `"settle": 2000` then `"after": 600` ms on that clock, the
/// bubble stepping through both: capture.mjs freezes only the liquid; checked by a test).
pub const FROZEN_AT: f64 = 2.6;

/// Where the W field is.
enum WField {
    Missing,
    Computing(Receiver<Vec<u8>>),
    /// Handed to the renderer (or lost with its thread): never asked for again.
    Handed,
}

/// The start pages of one window.
pub struct StartHost {
    lives: HashMap<TabId, Living<Rng>>,
    shown: Option<TabId>,
    /// Mixed into every tab's seed: two runs do not repeat each other's bubbles.
    nonce: u64,
    wfield: WField,
    /// The ms clock of the pointer's timestamps (the fling's velocity).
    epoch: Instant,
}

/// A start tab's seed: its id and the run's nonce (`Rng::new` keeps a 0 off).
pub fn seed_of(tab: TabId, nonce: u64) -> u64 {
    (tab.0.wrapping_add(1))
        .wrapping_mul(0x9E37_79B9_7F4A_7C15)
        .rotate_left(17)
        ^ nonce
}

/// The scene's share of a living effect; `None` for the host's own (`Capture`, `Cursor`) and
/// the ones nothing listens to yet (`Poked`, `Popped`).
pub fn fx_of(e: &Effect) -> Option<StartFx> {
    match *e {
        Effect::Ripple {
            x,
            y,
            reach,
            strength,
        } => Some(StartFx::Ripple {
            x: x as f32,
            y: y as f32,
            reach: reach as f32,
            strength: strength as f32,
        }),
        Effect::Light { x, y } => Some(StartFx::Light {
            x: x as f32,
            y: y as f32,
        }),
        Effect::LightLeave => Some(StartFx::LightLeave),
        Effect::Pop(p) => Some(StartFx::Pop {
            x: p.x as f32,
            y: p.y as f32,
            r: p.r as f32,
            strength: KS::pop_push as f32,
            k: KS::pop_push_k as f32,
            min: KS::pop_push_min as f32,
        }),
        Effect::Capture | Effect::Cursor(_) | Effect::Poked | Effect::Popped => None,
    }
}

/// The effects of one transition, split for the scene and the stage.
pub fn out_of(effects: &[Effect]) -> Out {
    Out {
        fx: effects.iter().filter_map(fx_of).collect(),
        grabbing: effects.iter().rev().find_map(|e| match e {
            Effect::Cursor(c) => Some(*c == Cursor::Grabbing),
            _ => None,
        }),
    }
}

/// The bubble pass's draw of one bubble (field by field).
pub fn bubble_draw(u: &BubbleUniforms) -> BubbleDraw {
    BubbleDraw {
        quad: u.quad,
        c: u.c,
        r: u.r,
        ring: u.ring,
        body: u.body,
        alpha: u.alpha,
        swirl: u.swirl,
        seed: u.seed,
        has_w: u.has_w,
        face: u.face,
        wave: u.wave,
        wave_n: u.wave_n,
        w_inv: u.w_inv,
        w_off: u.w_off,
        w_scale: u.w_scale,
    }
}

pub fn pop_draw(u: &PopUniforms) -> PopDraw {
    PopDraw {
        quad: u.quad,
        c: u.c,
        r: u.r,
        k: u.k,
        ang: u.ang,
        seed: u.seed,
    }
}

/// The frame the scene draws for `l` in `card`: its bubbles (the web's skips left out), its
/// pops, its time; the liquid at the web's pace (slowed while reduced, held by `frozen`).
pub fn draw_of(l: &Living<Rng>, card: StartCard, frozen: bool) -> StartDraw {
    let f = l.frame();
    StartDraw {
        card,
        bubbles: f
            .bubbles
            .iter()
            .filter_map(|b| BubbleUniforms::of(b))
            .map(|u| bubble_draw(&u))
            .collect(),
        pops: f
            .pops
            .iter()
            .map(|p| pop_draw(&PopUniforms::of(p)))
            .collect(),
        time: f.time as f32,
        pace: if frozen {
            0.0
        } else if l.still {
            K::still_time as f32
        } else {
            1.0
        },
    }
}

impl StartHost {
    pub fn new(nonce: u64) -> Self {
        StartHost {
            lives: HashMap::new(),
            shown: None,
            nonce,
            wfield: WField::Missing,
            epoch: Instant::now(),
        }
    }

    /// The start tab shown now, if any.
    pub fn shown(&self) -> Option<TabId> {
        self.shown
    }

    /// How many start tabs have a bubble.
    #[cfg(test)]
    pub fn lives(&self) -> usize {
        self.lives.len()
    }

    /// The shown start tab's bubble.
    pub fn living(&self) -> Option<&Living<Rng>> {
        self.lives.get(&self.shown?)
    }

    /// Which start tab the window shows (`None`: none), and which tabs still exist: a bubble
    /// is made on its tab's first show, paused (`Active(false)`) when it hides, dropped when
    /// its tab is gone. The first show also starts the W field.
    pub fn sync(&mut self, shown: Option<TabId>, alive: impl Fn(TabId) -> bool) {
        self.lives.retain(|t, _| alive(*t));
        if shown == self.shown {
            return;
        }
        if let Some(old) = self.shown.take() {
            self.event_for(old, Event::Active(false));
        }
        if let Some(tab) = shown {
            let nonce = self.nonce;
            self.lives.entry(tab).or_insert_with(|| {
                Living::new(Options::start_page(), Rng::new(seed_of(tab, nonce)))
            });
            self.event_for(tab, Event::Active(true));
            if matches!(self.wfield, WField::Missing) {
                let (tx, rx) = mpsc::channel();
                let spawned = std::thread::Builder::new()
                    .name("wubble w field".into())
                    .spawn(move || {
                        let _ = tx.send(w_field_rgba());
                    });
                self.wfield = match spawned {
                    Ok(_) => WField::Computing(rx),
                    Err(e) => {
                        eprintln!("wubble: start page: no W field thread: {e}");
                        WField::Handed
                    }
                };
            }
        }
        self.shown = shown;
    }

    fn event_for(&mut self, tab: TabId, e: Event) -> Out {
        let Some(l) = self.lives.remove(&tab) else {
            return Out::default();
        };
        let (l, fx) = transition(l, e);
        self.lives.insert(tab, l);
        out_of(&fx)
    }

    /// The stage's pointer, for the shown start page.
    pub fn pointer(&mut self, p: StagePointer, now: Instant) -> Out {
        let Some(tab) = self.shown else {
            return Out::default();
        };
        let t_ms = now.saturating_duration_since(self.epoch).as_secs_f64() * 1e3;
        let e = match p {
            StagePointer::Down(x, y) => Event::Down { x, y, t_ms },
            StagePointer::Move(x, y) => Event::Move { x, y, t_ms },
            StagePointer::Up => Event::Up,
            StagePointer::Leave => Event::Leave,
        };
        self.event_for(tab, e)
    }

    /// One scene frame for the shown start page: its size (the page card's, logical px), the
    /// motion flag and the time since the last frame.
    pub fn frame(&mut self, size: (f64, f64), reduced: bool, dt: f64) -> Out {
        let Some(tab) = self.shown else {
            return Out::default();
        };
        let mut out = Out::default();
        let (w, h) = self.lives.get(&tab).map_or((0.0, 0.0), |l| l.size);
        if (w, h) != size {
            out.fx.extend(
                self.event_for(
                    tab,
                    Event::Resize {
                        w: size.0,
                        h: size.1,
                    },
                )
                .fx,
            );
        }
        if self.lives.get(&tab).is_some_and(|l| l.still != reduced) {
            out.fx
                .extend(self.event_for(tab, Event::Reduced(reduced)).fx);
        }
        let tick = self.event_for(tab, Event::Tick { dt });
        out.fx.extend(tick.fx);
        out.grabbing = tick.grabbing;
        out
    }

    /// One scene frame under the fidelity harness's fixed clock: the shown bubble is stepped in
    /// 1/120 s ticks up to the scene time `at`, then held there (a `Tick` of 0), however many
    /// frames the display renders before the shot.
    pub fn frame_frozen(&mut self, size: (f64, f64), reduced: bool, at: f64) -> Out {
        let mut out = self.frame(size, reduced, 0.0);
        let per_tick = FROZEN_STEP * if reduced { K::still_time } else { 1.0 };
        let ticks = self
            .living()
            .map_or(0.0, |l| ((at - l.time) / per_tick).round().max(0.0));
        for _ in 0..ticks as usize {
            let tick = self.frame(size, reduced, FROZEN_STEP);
            out.fx.extend(tick.fx);
            out.grabbing = tick.grabbing.or(out.grabbing);
        }
        out
    }

    /// The W field, once it is computed: given out once.
    pub fn take_wfield(&mut self) -> Option<(Vec<u8>, u32)> {
        let WField::Computing(rx) = &self.wfield else {
            return None;
        };
        let got = rx.try_recv();
        self.received(got)
    }

    /// What the W field's thread gave: the field (handed over), nothing yet, or nothing ever.
    fn received(&mut self, got: Result<Vec<u8>, TryRecvError>) -> Option<(Vec<u8>, u32)> {
        match got {
            Ok(rgba) => {
                self.wfield = WField::Handed;
                Some((rgba, W_SIZE as u32))
            }
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                eprintln!("wubble: start page: the W field thread ended without one");
                self.wfield = WField::Handed;
                None
            }
        }
    }
}

/// The start page's tiles: the window's favourites (its pinned tiles, folders left out), as the
/// sidebar shows them: wubble's data and wording, never the design's sample sites.
pub fn sites_of(pinned: &[TabItem]) -> Vec<StartSite> {
    pinned
        .iter()
        .filter(|t| t.kind == 0 && InternalPage::from_url(&t.url).is_none())
        .map(|t| StartSite {
            title: t.title.clone(),
            url: t.url.clone(),
            favicon: t.favicon.clone(),
        })
        .collect()
}

impl Host {
    /// The start page's half of the page layer ([`Host::render_pages`]): the scene's living
    /// layer follows the shown start tab (or none), the page its tiles and shortcut.
    pub(super) fn render_start(&mut self, window: &AppWindow, page: Option<InternalPage>) {
        let shown = self.shown().filter(|_| page == Some(InternalPage::Start));
        let model = &self.model;
        let start = |t: TabId| {
            model
                .tabs
                .get(&t)
                .is_some_and(|t| InternalPage::from_url(&t.url) == Some(InternalPage::Start))
        };
        if let Ok(mut glass) = self.w.glass.try_borrow_mut() {
            glass.start_sync(window, shown, start);
        }
        if shown.is_none() {
            return;
        }
        let bridge = window.global::<StartBridge>();
        let pinned: Vec<TabItem> = window.get_pinned_tabs().iter().collect();
        let sites = sites_of(&pinned);
        let old: Vec<StartSite> = bridge.get_sites().iter().collect();
        if old != sites {
            bridge.set_sites(ModelRc::new(VecModel::from(sites)));
        }
        // the focus-address binding as the key table has it ("Ctrl+L")
        let shortcut = self
            .keymap
            .shortcut_for(Action::FocusAddress)
            .map(|c| c.to_string())
            .unwrap_or_default();
        bridge.set_shortcut(shortcut.as_str().into());
    }

    /// The start page's stage: a press, move, release or leave.
    pub(crate) fn start_pointer(&mut self, p: StagePointer) {
        let Some(window) = self.w.window.upgrade() else {
            return;
        };
        if let Ok(mut glass) = self.w.glass.try_borrow_mut() {
            glass.start_pointer(&window, p);
        }
    }

    /// The search capsule: the bar (already opening from the capsule, app.slint) goes to the
    /// start tab itself, on an empty query.
    pub(crate) fn start_search(&mut self) {
        let (Some(window), Some(tab)) = (self.w.window.upgrade(), self.shown()) else {
            return;
        };
        self.open_bar(&window, BarMode::Navigate(tab), String::new());
    }
}

impl Host {
    /// One step of `--smoke start` (start_smoke.rs), after a run of CEF work.
    pub(super) fn start_smoke_step(&mut self) {
        use crate::start_smoke::{self, Action as Do, Seen};
        let Some(state) = self.checks.start_smoke.take() else {
            return;
        };
        let Some(window) = self.w.window.upgrade() else {
            self.checks.start_smoke = Some(state);
            return;
        };
        let url = self
            .shown()
            .and_then(|t| self.model.tabs.get(&t))
            .map(|t| t.url.clone())
            .unwrap_or_default();
        let on_start = InternalPage::from_url(&url) == Some(InternalPage::Start);
        let origin = (
            f64::from(window.get_content_x()),
            f64::from(window.get_content_y()),
        );
        let mut seen = Seen {
            ready: self.frame_rate.is_some(),
            pinned: window.get_pinned_tabs().row_count() > 0,
            sites: window.global::<StartBridge>().get_sites().row_count(),
            bar_open: window.get_command_bar_state() != wubble_ui::CommandBarState::Closed,
            navigated: !on_start
                && !url.is_empty()
                && state.stage == start_smoke::Stage::Navigating,
            ..Seen::default()
        };
        if let Ok(glass) = self.w.glass.try_borrow() {
            seen.start = on_start && glass.start_ready();
            if let Some(l) = glass.start_living().filter(|_| on_start) {
                seen.body = Some((l.body.center(), l.body.R));
                seen.waves = l.body.state.waves.len();
                seen.mini = l.minis.list.first().map(|m| (m.x, m.y));
                seen.pops = l.pops.len();
            }
            if state.wants_pixels()
                && let (Some(((cx, cy), r)), Some((w, _, rgba))) = (seen.body, glass.readback())
            {
                let logical = f64::from(window.window().size().width)
                    / f64::from(window.window().scale_factor());
                let scale = f64::from(w) / logical.max(1.0);
                let c = (origin.0 + cx, origin.1 + cy);
                (seen.bright, seen.body_pixels) = start_smoke::body_stats(&rgba, w, scale, c, r);
            }
        }
        let (state, actions) = start_smoke::next(state, &seen);
        self.checks.start_smoke = Some(state);
        for action in actions {
            match action {
                Do::Pin => {
                    if let Some(tab) = self.shown() {
                        self.dispatch(wubble_core::model::Command::PinTab(tab));
                    }
                }
                Do::OpenStart => self.perform(Action::NewTab),
                Do::Shoot(name) => {
                    if crate::shot::has_dir() {
                        let i = ["rest", "poke", "pop", "bar"]
                            .iter()
                            .position(|n| *n == name)
                            .unwrap_or(9);
                        match crate::shot::take_scene(&window, i, name) {
                            Ok(path) => println!("smoke: start shot {}", path.display()),
                            Err(e) => return self.fail(format!("start shot {name}: {e}")),
                        }
                    }
                }
                Do::Click(x, y) => {
                    self.start_click((origin.0 + x) as f32, (origin.1 + y) as f32);
                }
                Do::Search => {
                    let bridge = window.global::<StartBridge>();
                    self.start_click(bridge.get_search_x(), bridge.get_search_y());
                }
                // Esc on the bar, then the tile's own callback as its click sends it: later,
                // outside the host, which both call back into
                Do::Tile => {
                    let _ = self.w.window.upgrade_in_event_loop(|window| {
                        window.invoke_dismiss_command_bar();
                        let bridge = window.global::<StartBridge>();
                        if let Some(site) = bridge.get_sites().row_data(0) {
                            bridge.invoke_site_activated(site.url);
                        }
                    });
                }
                Do::Pass => {
                    self.start_passed = true;
                    crate::quit();
                }
                Do::Fail(reason) => return self.fail(reason),
            }
        }
    }

    /// A left click at a window point, as the pointer sends it: later, outside the host (the
    /// stage's callbacks call back into it).
    fn start_click(&self, x: f32, y: f32) {
        use slint::platform::{PointerEventButton, WindowEvent};
        let position = slint::LogicalPosition::new(x, y);
        let weak = self.w.window.clone();
        let _ = slint::invoke_from_event_loop(move || {
            let Some(window) = weak.upgrade() else { return };
            let button = PointerEventButton::Left;
            for e in [
                WindowEvent::PointerMoved { position },
                WindowEvent::PointerPressed { position, button },
                WindowEvent::PointerReleased { position, button },
            ] {
                window.window().dispatch_event(e);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wubble_core::start::softbody::PopState;

    const DT: f64 = 1.0 / 120.0;

    fn host() -> StartHost {
        let mut h = StartHost::new(7);
        h.sync(Some(TabId(1)), |_| true);
        h.frame((800.0, 600.0), false, DT);
        h
    }

    #[test]
    fn effects_split_between_the_liquid_and_the_cursor() {
        let pop = PopState {
            x: 10.0,
            y: 20.0,
            r: 4.0,
            age: 0.0,
            life: 0.36,
            angle: 0.0,
            seed: 1.0,
        };
        let out = out_of(&[
            Effect::Capture,
            Effect::Cursor(Cursor::Grabbing),
            Effect::Pop(pop),
            Effect::LightLeave,
            Effect::Poked,
        ]);
        assert_eq!(
            out.fx,
            [
                StartFx::Pop {
                    x: 10.0,
                    y: 20.0,
                    r: 4.0,
                    strength: 500.0,
                    k: 0.6,
                    min: 0.002,
                },
                StartFx::LightLeave
            ]
        );
        assert_eq!(out.grabbing, Some(true));
        assert_eq!(
            out_of(&[Effect::Cursor(Cursor::Pointer)]).grabbing,
            Some(false)
        );
        assert_eq!(out_of(&[Effect::Popped]), Out::default());
    }

    #[test]
    fn a_press_on_the_body_grabs_and_the_release_pokes() {
        let mut h = host();
        let c = h.living().unwrap().body.center();
        let now = Instant::now();
        let down = h.pointer(StagePointer::Down(c.0, c.1), now);
        assert_eq!(down.grabbing, Some(true));
        let up = h.pointer(StagePointer::Up, now);
        assert_eq!(up.grabbing, Some(false));
        // the poke ripples the liquid where it was pressed
        assert!(up.fx.iter().any(|f| matches!(f, StartFx::Ripple { .. })));
        // a move moves the light; leaving lets it go
        let mv = h.pointer(StagePointer::Move(5.0, 6.0), now);
        assert_eq!(mv.fx, [StartFx::Light { x: 5.0, y: 6.0 }]);
        assert_eq!(
            h.pointer(StagePointer::Leave, now).fx,
            [StartFx::LightLeave]
        );
    }

    #[test]
    fn each_start_tab_has_its_own_bubble_until_it_closes() {
        assert_ne!(seed_of(TabId(1), 7), seed_of(TabId(2), 7));
        assert_ne!(seed_of(TabId(1), 7), seed_of(TabId(1), 8));
        let mut h = host();
        let first = h.living().unwrap().body.state.seed;
        h.sync(Some(TabId(2)), |_| true);
        assert_eq!(h.lives(), 2);
        assert_ne!(h.living().unwrap().body.state.seed, first);
        // the hidden one is paused
        assert!(!h.lives[&TabId(1)].active && h.lives[&TabId(2)].active);
        // tab 1 closed
        h.sync(Some(TabId(2)), |t| t != TabId(1));
        assert_eq!(h.lives(), 1);
        // no start tab shown: nothing to drive, input goes nowhere
        h.sync(None, |_| true);
        assert!(h.living().is_none());
        assert_eq!(h.pointer(StagePointer::Up, Instant::now()), Out::default());
        assert_eq!(h.frame((1.0, 1.0), false, DT), Out::default());
        assert!(!h.lives[&TabId(2)].active);
    }

    #[test]
    fn a_frame_sizes_slows_and_steps_the_shown_bubble() {
        let mut h = host();
        let l = h.living().unwrap();
        assert_eq!(l.size, (800.0, 600.0));
        assert_eq!(l.time, DT);
        h.frame((900.0, 600.0), true, DT);
        let l = h.living().unwrap();
        assert_eq!(l.size, (900.0, 600.0));
        assert!(l.still);
        let card = StartCard::default();
        let d = draw_of(l, card, false);
        assert_eq!(d.pace, K::still_time as f32);
        assert_eq!(draw_of(l, card, true).pace, 0.0);
        // the body is drawn first, with its W
        assert!(d.bubbles[0].has_w);
        assert_eq!(
            d.bubbles[0].c,
            [l.body.state.x as f32, l.body.state.y as f32]
        );
    }

    #[test]
    fn the_w_field_is_computed_once_off_the_frame() {
        let mut h = host();
        let WField::Computing(rx) = &h.wfield else {
            panic!("the first start tab starts the W field");
        };
        // what the frame's `take_wfield` gets once the thread is done, waited for here
        let got = rx.recv().map_err(|_| TryRecvError::Disconnected);
        let (rgba, n) = h.received(got).expect("the W field");
        assert_eq!((rgba.len(), n), (W_SIZE * W_SIZE * 4, W_SIZE as u32));
        // handed over: never asked for again, and another start tab starts no second thread
        assert!(matches!(h.wfield, WField::Handed));
        assert!(h.take_wfield().is_none());
        h.sync(Some(TabId(5)), |_| true);
        assert!(matches!(h.wfield, WField::Handed));
    }

    #[test]
    fn the_w_field_waits_for_its_thread_and_gives_up_on_a_lost_one() {
        let mut h = StartHost::new(7);
        let (tx, rx) = mpsc::channel::<Vec<u8>>();
        h.wfield = WField::Computing(rx);
        // still computing: asked again next frame
        assert!(h.take_wfield().is_none());
        assert!(matches!(h.wfield, WField::Computing(_)));
        // the thread ended without one: never asked for again
        drop(tx);
        assert!(h.take_wfield().is_none());
        assert!(matches!(h.wfield, WField::Handed));
    }

    #[test]
    fn a_frozen_start_page_is_shot_at_one_scene_time_however_many_frames_run() {
        let at = |frames: usize| {
            let mut h = StartHost::new(7);
            h.sync(Some(TabId(1)), |_| true);
            for _ in 0..frames {
                h.frame_frozen((800.0, 600.0), false, FROZEN_AT);
            }
            let l = h.living().unwrap();
            (l.time, l.body.center(), l.minis.list.len())
        };
        let (t, ..) = at(1);
        assert!((t - FROZEN_AT).abs() < 1e-9, "{t}");
        // a 60 Hz and a 120 Hz settle: the same bubble
        assert_eq!(at(72), at(144));
        assert_eq!(at(1), at(72));
    }

    #[test]
    fn the_frozen_time_is_the_web_capture_s_settle() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tools/fidelity/scenes.json"
        );
        let scenes: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        let start = scenes["scenes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["name"] == "start")
            .unwrap();
        let ms = |k: &str| start.get(k).unwrap_or(&scenes[k]).as_f64().unwrap();
        assert!(
            start.get("steps").is_none(),
            "the web shot's clock has steps too"
        );
        assert!(((ms("settle") + ms("after")) / 1000.0 - FROZEN_AT).abs() < 1e-9);
    }
}
