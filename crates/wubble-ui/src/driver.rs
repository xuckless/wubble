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
//! - The notifier also carries the host's per-frame hook (the wgpu scene: wubble-ui must not
//!   name wubble-gfx), called after the driver's step with the frame's glass list
//!   ([`crate::glass`]). The panels report their glass from `changed item` handlers, which Slint
//!   runs only when it draws, after this hook: [`Wired::frame`] runs them itself between the
//!   step and the read, so the scene's glass and Slint's content come from the same writes.
//! - The seam starts from the saved width and mode ([`Seam::restored`]), at rest.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

use slint::{ComponentHandle, Model, ModelRc, SetRenderingNotifierError, VecModel, Weak};
use wubble_core::beam::{self, Beam};
use wubble_core::bow::{BOWED, EdgeTable};
use wubble_core::chrome::{
    self, CommandBar, CommandEffect, CommandEvent, Peek, PeekEffect, PeekEvent,
};
use wubble_core::motion::{DT_CAP, OVERLAP_IN, ease, overlap_in, tween};

/// The outgoing page's exit, s: the design's `wb-out` (`webview.tsx`, 190 ms in-out).
const PAGE_OUT: f64 = 0.19;
use wubble_core::scroller::{DeltaMode, Scroller};
use wubble_core::seam::{self, INSET, SEAM, Seam};
use wubble_core::tabs::{self, SpaceSwitcher, TabRows, TabUp};

use crate::glass::{GlassFrame, GlassRegistry};
use crate::{
    AppWindow, BeamShape, Chrome, CommandBarState, CommandMode, CommandPlacement, GlassItem,
    GlassLayer, InternalPageKind, LeavingRow, Origin, PageScroll, PeekCue, Radius, Shell,
    SidebarMode, TabItem, sync,
};

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
    /// The open native page's scroller (`PageScroll`): the design's pages scroll in its
    /// `Scroller` too.
    page_scroll: Scroller,
    /// The address pill's border beam (wubble_core::beam), and what turns the shown tab and its
    /// loading flag into the beam's events; `laps` counts laps started, for their hues.
    beam: Beam,
    watch: beam::Watch,
    laps: u64,
    /// The shown tab, and the seconds since another one was shown: its page enters on the
    /// Overlap entrance (webview.tsx gives the shown view `wb-in`).
    page_tab: Option<i64>,
    page_t: f64,
    /// Seconds since the outgoing page started leaving ([`PAGE_OUT`] or more at rest).
    exit_t: f64,
    /// Seconds since another Space was shown: its name enters (`wb-in`).
    space_t: f64,
    /// The command bar's beam while typing, and the query it last saw.
    thinking: beam::Thinking,
    query: slint::SharedString,
    row_y: Rc<VecModel<f32>>,
    row_scale: Rc<VecModel<f32>>,
    row_enter: Rc<VecModel<f32>>,
    /// The rows' items as last read, and those of rows on their way out (what they showed).
    items: Vec<TabItem>,
    gone: Vec<TabItem>,
    leaving: Rc<VecModel<LeavingRow>>,
    /// The seam's edge table (`seam-edge`): how far the bowed string is displaced at each height.
    edge: Rc<VecModel<f32>>,
    /// The scroller's millisecond clock: real time from this origin once installed; without
    /// one (tests) it is the sum of the frame deltas passed to [`Driver::advance`].
    origin: Option<Instant>,
    clock_ms: f64,
    /// Time of the previous frame while moving; `None` while idle, so the first frame after a
    /// wake steps with dt 0.
    last: Option<Instant>,
    /// The hidden sidebar's peek (wubble_core::chrome::Peek); `peek_timer` runs its `Arm`.
    peek: Peek,
    /// The command bar's life (wubble_core::chrome::CommandBar); `exit_timer` runs its exit.
    command: CommandBar,
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

/// A Slint row id as a core row id. Injective over `i32`: header rows (folders, day labels) have
/// negative ids, which must not collide.
fn row_id(id: i32) -> u64 {
    u64::from(id as u32)
}

fn row_ids(ids: &[i32]) -> Vec<u64> {
    ids.iter().map(|&id| row_id(id)).collect()
}

/// Whether the Slint row `t` is core row `id` (`row_ids`).
fn same(t: &TabItem, id: u64) -> bool {
    row_id(t.id) == id
}

fn index(i: i32) -> usize {
    usize::try_from(i).unwrap_or(usize::MAX)
}

fn slint_index(i: Option<usize>) -> i32 {
    i.and_then(|i| i32::try_from(i).ok()).unwrap_or(-1)
}

impl Driver {
    fn new(window: &AppWindow, seam: Seam) -> Self {
        let row_y = Rc::new(VecModel::<f32>::default());
        let row_scale = Rc::new(VecModel::<f32>::default());
        let row_enter = Rc::new(VecModel::<f32>::default());
        let leaving = Rc::new(VecModel::<LeavingRow>::default());
        let edge = Rc::new(VecModel::<f32>::default());
        window.set_row_y(ModelRc::from(row_y.clone()));
        window.set_row_scale(ModelRc::from(row_scale.clone()));
        window.set_row_enter(ModelRc::from(row_enter.clone()));
        window.set_leaving_rows(ModelRc::from(leaving.clone()));
        window.set_seam_edge(ModelRc::from(edge.clone()));
        let (ids, row) = model_rows(window);
        let space = active_space(window);
        window.set_selected_row(slint_index(row));
        window.set_space_index(slint_index(space));
        window.set_sidebar_mode(seam.mode.into());
        let mut driver = Self {
            tabs: TabRows::new(&row_ids(&ids), row),
            ids,
            spaces: SpaceSwitcher::new(space.unwrap_or(0)),
            seam,
            scroll: Scroller::new(),
            page_scroll: Scroller::new(),
            beam: Beam::Idle,
            watch: beam::Watch::default(),
            laps: 0,
            page_tab: None,
            page_t: OVERLAP_IN,
            exit_t: PAGE_OUT,
            space_t: OVERLAP_IN,
            thinking: beam::Thinking::Idle,
            query: Default::default(),
            row_y,
            row_scale,
            row_enter,
            items: window.get_tabs().iter().collect(),
            gone: vec![],
            leaving,
            edge,
            origin: None,
            clock_ms: 0.0,
            last: None,
            peek: Peek::Idle,
            command: CommandBar::Closed,
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
        // keep what a dropped row showed while it leaves
        let items: Vec<TabItem> = window.get_tabs().iter().collect();
        let old = std::mem::replace(&mut self.items, items);
        for l in &self.tabs.leaving {
            if !self.gone.iter().any(|g| same(g, l.id))
                && let Some(item) = old.iter().find(|t| same(t, l.id))
            {
                self.gone.push(item.clone());
            }
        }
        self.ids = ids;
        window.set_selected_row(slint_index(row));
        let space = active_space(window);
        if let Some(i) = space {
            if i != self.spaces.active {
                self.space_t = 0.0;
            }
            self.spaces.active = i;
        }
        window.set_space_index(slint_index(space));
    }

    /// The sidebar's compact look (chrome::compact) from the seam's live width, its mode and the
    /// peek.
    fn set_compact(&self, window: &AppWindow) {
        let w = self.seam.w.x.max(0.0);
        window.set_sidebar_rail(chrome::compact(w, self.seam.mode, self.peek.shown()));
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

        // tab rows and the indicator (tabs.rs), at the pitch the chrome's row size gives
        // (touch sizing: `Shell.row`)
        let shell = window.global::<Shell>();
        let pitch = f64::from(shell.get_row() + window.global::<Chrome>().get_rowGap());
        if pitch != self.tabs.pitch {
            self.tabs.set_pitch(pitch);
        }
        moving |= self.tabs.step(dt);
        sync(
            &self.row_y,
            self.tabs.rows.iter().map(|r| r.y.x as f32).collect(),
        );
        sync(
            &self.row_scale,
            self.tabs.rows.iter().map(|r| r.s.x as f32).collect(),
        );
        sync(
            &self.row_enter,
            self.tabs.rows.iter().map(|r| r.entrance() as f32).collect(),
        );
        window.set_list_enter(self.tabs.list_entrance() as f32);
        let tabs = &self.tabs;
        let dragged = tabs.rows.iter().position(|r| tabs.is_dragging(r.id));
        window.set_dragged_row(slint_index(dragged));
        let leaving = &self.tabs.leaving;
        self.gone.retain(|g| leaving.iter().any(|l| same(g, l.id)));
        let rows = leaving
            .iter()
            .filter_map(|l| {
                let tab = self.gone.iter().find(|g| same(g, l.id))?;
                Some(LeavingRow {
                    tab: tab.clone(),
                    y: l.y as f32,
                    e: l.progress() as f32,
                })
            })
            .collect();
        sync(&self.leaving, rows);
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
        self.set_compact(window);
        window.set_sidebar_saved(self.seam.saved as f32);
        let bowed = self.seam.held || self.seam.bend.x.abs() > BOWED;
        window.set_seam_bowed(bowed);
        let r = f64::from(window.global::<Radius>().get_md());
        let (lw, lh) = (w_win - INSET * 2.0, h_win - INSET * 2.0);
        let edge = if bowed {
            self.seam.edge(h_win, r)
        } else {
            EdgeTable::default()
        };
        if bowed {
            // The string runs along the web view's left edge, in the web view's frame (inset on
            // every side); app.slint decides where that edge is, from the width set above.
            let a = f64::from(window.get_content_x()) - INSET;
            let (_path, line) = self.seam.clip(a, lw, lh, r, self.seam.gy - INSET);
            window.set_seam_path(line.into());
        } else {
            window.set_seam_path(Default::default());
        }
        // The sidebar's sections ride the string (design browser/chrome.tsx `[data-ride]`) and
        // the page's edge follows it (the page mask: the host reads the table).
        window.set_seam_edge_step(edge.step as f32);
        sync(&self.edge, edge.offset);

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

        // the open native page's scroller
        let page = window.global::<PageScroll>();
        self.page_scroll.measure(
            f64::from(page.get_viewport()),
            f64::from(page.get_content()),
        );
        moving |= self.page_scroll.step(dt, now);
        let r = self.page_scroll.render(now);
        page.set_y(r.shown as f32);
        if let Some((top, h)) = r.thumb {
            page.set_thumb_top(top as f32);
            page.set_thumb_height(h as f32);
        }
        page.set_thumb_visible(r.thumb_visible);

        // the address pill's border beam: a lap when another tab is shown, lapping while the
        // page loads, fading once it has loaded (beam.rs)
        moving |= self.step_beam(window, dt);
        moving |= self.step_page(window, dt);
        self.space_t = (self.space_t + dt).min(OVERLAP_IN);
        window.set_space_enter(overlap_in(self.space_t) as f32);
        moving |= self.space_t < OVERLAP_IN;
        moving
    }

    /// A tab switch: the newly shown page enters (one page only: a split's panes keep their
    /// places). Not on the first page.
    fn step_page(&mut self, window: &AppWindow, dt: f64) -> bool {
        let tab = Some(i64::from(window.get_active_tab_id())).filter(|&id| id >= 0);
        let was = std::mem::replace(&mut self.page_tab, tab);
        if was.is_some() && tab.is_some() && was != tab && window.get_panes().row_count() <= 1 {
            self.page_t = 0.0;
            self.exit_t = 0.0;
        } else {
            self.page_t = (self.page_t + dt).min(OVERLAP_IN);
            self.exit_t = (self.exit_t + dt).min(PAGE_OUT);
        }
        window.set_page_enter(overlap_in(self.page_t) as f32);
        // The outgoing page (the host's copy of its last frame, `exit-page`) leaves under the
        // incoming one; once gone, or when no switch started it, the copy is dropped.
        window.set_exit_t(tween(ease::inOut, self.exit_t, PAGE_OUT) as f32);
        if self.exit_t >= PAGE_OUT && window.get_exit_page().size().width > 0 {
            window.set_exit_page(slint::Image::default());
        }
        self.page_t < OVERLAP_IN || self.exit_t < PAGE_OUT
    }

    fn step_beam(&mut self, window: &AppWindow, dt: f64) -> bool {
        let tab = Some(i64::from(window.get_active_tab_id())).filter(|&id| id >= 0);
        let hue = beam::hue(self.laps);
        if let Some(mut event) = self.watch.observe(tab, window.get_address_loading(), hue) {
            // under reduced motion the beam never starts (the design's `chrome.tsx` `RM.on`)
            if wubble_core::motion::reduced()
                && matches!(
                    event,
                    beam::Event::Shown { .. } | beam::Event::LoadStarted { .. }
                )
            {
                event = beam::Event::Reduce;
            }
            if matches!(
                event,
                beam::Event::Shown { .. } | beam::Event::LoadStarted { .. }
            ) {
                self.laps += 1;
            }
            self.beam = beam::transition(self.beam, event);
        }
        self.beam = beam::transition(self.beam, beam::Event::Tick(dt));
        window.set_address_beam(shape_of(self.beam.shape()));

        // the command bar thinks while you type (command.tsx)
        let query = window.get_command_query();
        if query != self.query {
            self.thinking = beam::Thinking::typed(query.trim().is_empty());
            self.query = query;
        }
        self.thinking = self.thinking.tick(dt);
        window.set_command_beam(shape_of(self.thinking.shape()));
        self.beam.running() || self.thinking != beam::Thinking::Idle
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

/// A beam shape for Slint; none draws nothing.
fn shape_of(shape: Option<beam::Shape>) -> BeamShape {
    shape.map_or(BeamShape::default(), |s| BeamShape {
        head: s.head as f32,
        length: s.length as f32,
        strength: s.strength as f32,
        hue: s.hue as f32,
    })
}

type Shared = Rc<RefCell<Driver>>;

/// The driver and the glass registry of one window.
pub struct Wired {
    pub driver: Shared,
    pub glass: Rc<RefCell<GlassRegistry>>,
    /// The component sheet's springs (toggle, slider), one slot per control.
    pub controls: Rc<RefCell<crate::controls::Controls>>,
}

impl Wired {
    /// One frame at `now`, before Slint draws it: step the driver, let the panels report what
    /// the step moved, and take the frame's glass. Returns whether another frame is needed
    /// (something still moves, or a sweep asked the panels to report again) and the glass.
    ///
    /// Slint runs `changed` handlers while it draws (`WindowInner::draw_contents`), after the
    /// rendering notifier: reading the registry before them gave the scene the previous
    /// frame's glass, one frame behind Slint (the sidebar's glass trailing its width). The same
    /// handlers run here a moment earlier, so nothing runs that the draw would not run.
    pub fn frame(&self, window: &AppWindow, now: Instant) -> (bool, GlassFrame) {
        let mut moving = self.driver.borrow_mut().frame(window, now);
        let (controls, bump) = self.controls.borrow_mut().frame(now);
        moving |= controls;
        if bump {
            crate::controls::bump(window);
        }
        slint::private_unstable_api::re_exports::ChangeTracker::run_change_handlers();
        let frame = self.glass.borrow_mut().frame();
        if frame.bump {
            let layer = window.global::<GlassLayer>();
            layer.set_generation(layer.get_generation().wrapping_add(1));
        }
        (moving || frame.bump, frame)
    }
}

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

/// One step of the peek (wubble_core::chrome::peek), its effects run: the one timer armed or
/// stopped, `peek` written, and the sidebar's compact look worked out again (a peek is never
/// compact).
fn peek_step(driver: &Shared, window: &AppWindow, timer: &Rc<slint::Timer>, event: PeekEvent) {
    let state = driver.borrow().peek;
    let (state, effects) = chrome::peek(state, event);
    driver.borrow_mut().peek = state;
    for effect in effects {
        match effect {
            PeekEffect::Arm(ms) => {
                let (d, w, t) = (
                    Rc::downgrade(driver),
                    window.as_weak(),
                    Rc::downgrade(timer),
                );
                timer.start(
                    slint::TimerMode::SingleShot,
                    std::time::Duration::from_millis(ms.into()),
                    move || {
                        if let (Some(d), Some(w), Some(t)) = (d.upgrade(), w.upgrade(), t.upgrade())
                        {
                            peek_step(&d, &w, &t, PeekEvent::TimerFired);
                        }
                    },
                );
            }
            PeekEffect::Disarm => timer.stop(),
        }
    }
    window.set_peek(state.shown());
    driver.borrow().set_compact(window);
    window.window().request_redraw();
}

/// One step of the command bar (wubble_core::chrome::command), its effects run: the state, mode
/// and origin written, the exit timed (the dropdown's out duration), the keys moved (taken once
/// the opener's prefill has landed).
fn command_step(
    driver: &Shared,
    window: &AppWindow,
    timer: &Rc<slint::Timer>,
    event: CommandEvent,
) {
    let state = driver.borrow().command;
    let (state, effects) = chrome::command(state, event);
    driver.borrow_mut().command = state;
    window.set_command_bar_state(match state {
        CommandBar::Closed => CommandBarState::Closed,
        CommandBar::Open { .. } => CommandBarState::Open,
        CommandBar::Closing { .. } => CommandBarState::Closing,
    });
    if let Some(mode) = state.mode() {
        window.set_command_mode(match mode {
            chrome::CommandMode::Here => CommandMode::Here,
            chrome::CommandMode::New => CommandMode::New,
            chrome::CommandMode::Split => CommandMode::Split,
        });
    }
    if let Some(at) = state.placement() {
        window.set_command_placement(match at {
            chrome::Placement::Overlay => CommandPlacement::Overlay,
            chrome::Placement::InPage => CommandPlacement::InPage,
        });
    }
    if let Some(from) = state.from() {
        window.set_command_origin(Origin {
            x: from.x as f32,
            y: from.y as f32,
        });
    }
    for effect in effects {
        match effect {
            CommandEffect::Disarm => timer.stop(),
            // Deferred to the event loop: an opener prefills the query right after the open
            // (`request-address-focus` then the host's `open_bar`), and the input must select
            // that text, not the one it held. Not if the bar has gone by then.
            CommandEffect::FocusInput => {
                let (d, w) = (Rc::downgrade(driver), window.as_weak());
                slint::Timer::single_shot(std::time::Duration::ZERO, move || {
                    if let (Some(d), Some(w)) = (d.upgrade(), w.upgrade())
                        && matches!(d.borrow().command, CommandBar::Open { .. })
                    {
                        w.invoke_command_focus_input();
                    }
                });
            }
            CommandEffect::RestoreFocus => window.invoke_command_release_keys(),
            CommandEffect::StartExit => {
                let (d, w, t) = (
                    Rc::downgrade(driver),
                    window.as_weak(),
                    Rc::downgrade(timer),
                );
                timer.start(
                    slint::TimerMode::SingleShot,
                    std::time::Duration::from_millis(
                        u64::try_from(window.get_command_exit()).unwrap_or(0),
                    ),
                    move || {
                        if let (Some(d), Some(w), Some(t)) = (d.upgrade(), w.upgrade(), t.upgrade())
                        {
                            command_step(&d, &w, &t, CommandEvent::Settled);
                        }
                    },
                );
            }
        }
    }
    window.window().request_redraw();
}

/// Connect the Slint callbacks to a new driver whose seam starts as `seam`, and the panels'
/// reports to a new glass registry. Separate from [`install`] so tests can drive it without a
/// renderer.
pub fn wire(window: &AppWindow, seam: Seam) -> Wired {
    let driver = Rc::new(RefCell::new(Driver::new(window, seam)));
    let glass = Rc::new(RefCell::new(GlassRegistry::default()));
    let hook = || (Rc::clone(&driver), window.as_weak());

    // A report outside a frame (a layout the host changed) still reaches the scene: a changed
    // registry asks for a frame.
    let g = Rc::clone(&glass);
    let w = window.as_weak();
    window.global::<GlassLayer>().on_report(move |item| {
        if g.borrow_mut().report(item)
            && let Some(w) = w.upgrade()
        {
            w.window().request_redraw();
        }
    });

    let (d, w) = hook();
    let g = Rc::clone(&glass);
    window.on_shell_model_changed(move || {
        g.borrow_mut().request_sweep();
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
    // the host took a drag that left the list: the row settles back, nothing is reordered
    let (d, w) = hook();
    window.on_tab_rows_cancel(move || {
        run(&d, &w, |s, w| {
            s.tabs.cancel();
            s.read_model(w);
            None
        })
    });

    // Up / Down in the tab list: the nearest tab row (folder headers are not stops)
    let w = window.as_weak();
    window.on_tab_step(move |i, down| {
        let Some(w) = w.upgrade() else { return i };
        let is_tab: Vec<bool> = w.get_tabs().iter().map(|t| t.kind == 0).collect();
        slint_index(Some(tabs::step_focus(&is_tab, index(i), down)))
    });

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
    window.on_sidebar_mode_requested(move |mode| {
        run(&d, &w, |s, w| {
            if s.seam.held {
                s.seam.release();
            }
            s.set_mode(w, mode.into());
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

    // the command bar: opened, closed and picked, into its machine
    let exit_timer = Rc::new(slint::Timer::default());
    let (d, w, t) = (Rc::clone(&driver), window.as_weak(), Rc::clone(&exit_timer));
    window.on_command_open(move |mode, from| {
        let Some(w) = w.upgrade() else { return };
        let mode = match mode {
            CommandMode::Here => chrome::CommandMode::Here,
            CommandMode::New => chrome::CommandMode::New,
            CommandMode::Split => chrome::CommandMode::Split,
        };
        let from = chrome::Point {
            x: f64::from(from.x),
            y: f64::from(from.y),
        };
        // the start page keeps the bar's field in its capsule
        let at = chrome::Placement::for_page(w.get_page() == InternalPageKind::Start);
        command_step(&d, &w, &t, CommandEvent::Open { mode, from, at });
    });
    let (d, w, t) = (Rc::clone(&driver), window.as_weak(), Rc::clone(&exit_timer));
    window.on_command_close(move || {
        if let Some(w) = w.upgrade() {
            command_step(&d, &w, &t, CommandEvent::Close);
        }
    });
    let (d, w, t) = (Rc::clone(&driver), window.as_weak(), exit_timer);
    window.on_command_pick(move || {
        if let Some(w) = w.upgrade() {
            command_step(&d, &w, &t, CommandEvent::Pick);
        }
    });

    // the hidden sidebar's peek: the pointer's cues and the mode, into the machine
    let peek_timer = Rc::new(slint::Timer::default());
    let (d, w, t) = (Rc::clone(&driver), window.as_weak(), Rc::clone(&peek_timer));
    window.on_peek_cue(move |cue| {
        let Some(w) = w.upgrade() else { return };
        let event = match cue {
            PeekCue::EdgeEntered => PeekEvent::EdgeEntered,
            PeekCue::StripMoved => PeekEvent::StripMoved,
            PeekCue::PanelEntered => PeekEvent::PanelEntered,
            PeekCue::PanelLeft => PeekEvent::PanelLeft,
        };
        peek_step(&d, &w, &t, event);
    });
    let (d, w, t) = (Rc::clone(&driver), window.as_weak(), peek_timer);
    window.on_sidebar_mode_changed(move |mode| {
        if let Some(w) = w.upgrade() {
            peek_step(&d, &w, &t, PeekEvent::ModeChanged(mode.into()));
        }
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
    wire_page_scroll(window, &driver);
    let controls = crate::controls::wire(window);
    Wired {
        driver,
        glass,
        controls,
    }
}

/// The open native page's scroller: wheel, touch and the thumb from `PageScroll`.
fn wire_page_scroll(window: &AppWindow, driver: &Shared) {
    let page = window.global::<PageScroll>();
    let hook = || (Rc::clone(driver), window.as_weak());
    let (d, w) = hook();
    page.on_wheel(move |dy| {
        run(&d, &w, |s, _| {
            let now = s.now_ms();
            s.page_scroll.wheel(f64::from(dy), DeltaMode::Pixel, now);
            None
        })
    });
    let (d, w) = hook();
    page.on_touch_down(move |y| {
        run(&d, &w, |s, _| {
            let now = s.now_ms();
            s.page_scroll.touch_down(f64::from(y), now);
            None
        })
    });
    let (d, w) = hook();
    page.on_touch_move(move |y| {
        run(&d, &w, |s, _| {
            let now = s.now_ms();
            s.page_scroll.touch_move(f64::from(y), now);
            None
        })
    });
    let (d, w) = hook();
    page.on_touch_up(move || {
        run(&d, &w, |s, _| {
            s.page_scroll.touch_up();
            None
        })
    });
    let (d, w) = hook();
    page.on_thumb_down(move |y| {
        run(&d, &w, |s, _| {
            s.page_scroll.thumb_down(f64::from(y));
            None
        })
    });
    let (d, w) = hook();
    page.on_thumb_move(move |y| {
        run(&d, &w, |s, _| {
            s.page_scroll.thumb_move(f64::from(y));
            None
        })
    });
    let (d, w) = hook();
    page.on_thumb_up(move || {
        run(&d, &w, |s, _| {
            let now = s.now_ms();
            s.page_scroll.thumb_up(now);
            None
        })
    });
    let (d, w) = hook();
    page.on_to(move |y| {
        run(&d, &w, |s, _| {
            let now = s.now_ms();
            s.page_scroll.to(f64::from(y), now);
            None
        })
    });
    let (d, w) = hook();
    page.on_reset(move || {
        run(&d, &w, |s, _| {
            s.page_scroll.reset();
            None
        })
    });
}

/// Wire the shell driver, its seam starting as `seam`, and register the window's one
/// rendering notifier. On every frame, after the driver's step, `hook` gets the window, the
/// frame time and the glass list when it changed (back to front); `after_rendering` runs after
/// each render (the host's frame measurement and begin-frame pacing). Returns the registry,
/// the list's one owner, for readers outside the frame (the pixel smoke). The notifier also tells
/// [`crate::window`] when the native window exists.
pub fn install(
    window: &AppWindow,
    seam: Seam,
    mut hook: impl FnMut(&AppWindow, Instant, Option<&[GlassItem]>) + 'static,
    mut after_rendering: impl FnMut() + 'static,
) -> Result<Rc<RefCell<GlassRegistry>>, SetRenderingNotifierError> {
    let wired = wire(window, seam);
    let registry = Rc::clone(&wired.glass);
    wired.driver.borrow_mut().origin = Some(Instant::now());
    let weak = window.as_weak();
    window
        .window()
        .set_rendering_notifier(move |state, _api| match state {
            // The first frame: the native window exists now.
            slint::RenderingState::RenderingSetup => {
                if let Some(window) = weak.upgrade() {
                    crate::window::native_window_ready(&window);
                }
            }
            slint::RenderingState::BeforeRendering => {
                let Some(window) = weak.upgrade() else {
                    return;
                };
                let now = Instant::now();
                let (again, frame) = wired.frame(&window, now);
                if again {
                    window.window().request_redraw();
                }
                hook(&window, now, frame.glass.as_deref());
            }
            slint::RenderingState::AfterRendering => after_rendering(),
            _ => {}
        })?;
    Ok(registry)
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
