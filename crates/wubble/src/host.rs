//! The windowed browser: one [`Model`], one view per live tab, the pump.
//!
//! Every change to tabs is a [`Command`] passed to [`Host::dispatch`], which
//! runs `model::apply` and executes the returned effects in [`Host::execute`]
//! (through the pure [`call`] table). UI callbacks and engine events only
//! ever become commands. The sidebar is re-derived from the model after each
//! change (`shell::view_model`). Only the visible tab's view takes frames,
//! layout and input; the others are hidden (`was_hidden`).

use std::collections::{BTreeMap, VecDeque};
use std::time::{Duration, Instant};

use slint::ComponentHandle;
use slint::platform::PointerEventButton;
use slint::winit_030::winit::event::WindowEvent;
use slint::winit_030::{EventResult, WinitWindowAccessor};
use wubble_core::lifecycle::Effect as Life;
use wubble_core::model::{self, Command, Effect, Model};
use wubble_core::sleep::Pressure;
use wubble_core::tab::TabId;
use wubble_engine::app::{Engine, PumpOutcome, PumpTimer, ViewId};
use wubble_engine::input::Point;
use wubble_engine::nav::{Cursor, WebEvent};
use wubble_engine::view::{View, ViewConfig};
use wubble_gfx::FramePresenter;
use wubble_gfx::pacing::{self, FrameStats};
use wubble_ui::AppWindow;

use crate::bar::{self, BarMode, BarRow};
use crate::input::{self, InputState, KeyRoute};
use crate::nav_smoke::{self, NavSmoke, SmokeAction, SmokeInput};
use crate::pixels;
use crate::tabs_smoke::{self, Seen, TabsSmoke};
use crate::{
    NAV_PAGE, SMOKE_MAX_IDLE_RUNS, TAB3_PAGE, Wake, content_layout, deliver, display_refresh_mhz,
    page_url, physical, quit, slint_int,
};

/// How far ahead `--smoke tabs` ticks the lifecycle: past the discard delay.
const SMOKE_DISCARD_AFTER: Duration = Duration::from_secs(20 * 60);

/// What executing one effect asks of the engine or the shell.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Call {
    /// Open a view for the tab unless it has one.
    OpenView {
        tab: TabId,
        url: String,
    },
    /// Close the tab's view if it has one.
    CloseView(TabId),
    Load {
        tab: TabId,
        url: String,
    },
    SetVisible {
        tab: TabId,
        visible: bool,
    },
    ShowCommandBar,
    Nothing,
}

/// The engine work an effect stands for. Freeze has no CEF call (CEF 156
/// has no freeze API; hiding already throttles) and snapshots have no UI
/// yet, so those are recorded no-ops (TODO.md "Tabs in the host").
pub fn call(effect: &Effect, model: &Model) -> Call {
    match effect {
        Effect::CreateTab { tab, url, .. } => Call::OpenView {
            tab: *tab,
            url: url.clone(),
        },
        Effect::CloseTab(tab) => Call::CloseView(*tab),
        Effect::Navigate { tab, url } => Call::Load {
            tab: *tab,
            url: url.clone(),
        },
        Effect::ShowCommandBar => Call::ShowCommandBar,
        // Which view is shown follows the lifecycle effects; focus and
        // frames follow `Model::visible_tab`.
        Effect::ActivateTab(_) | Effect::SpaceChanged(_) => Call::Nothing,
        Effect::Lifecycle { tab, effect } => match effect {
            Life::Show => Call::SetVisible {
                tab: *tab,
                visible: true,
            },
            Life::Hide => Call::SetVisible {
                tab: *tab,
                visible: false,
            },
            Life::Discard | Life::ReleaseTexture => Call::CloseView(*tab),
            // A live view reloads itself after a crash (`nav::transition`).
            Life::Restore => model
                .tabs
                .get(tab)
                .map_or(Call::Nothing, |t| Call::OpenView {
                    tab: *tab,
                    url: t.url.clone(),
                }),
            Life::Freeze | Life::Unfreeze | Life::CaptureSnapshot | Life::ReleaseSnapshot => {
                Call::Nothing
            }
        },
    }
}

/// What the command bar was opened for and what it lists.
struct Bar {
    mode: BarMode,
    rows: Vec<BarRow>,
}

/// What a windowed run is for, as far as the host cares.
pub struct Checks {
    /// Accelerated frames after which the loop quits (`--smoke frames`).
    pub quit_after: Option<u64>,
    /// Snapshot the window before quitting and require the content area to
    /// show the page. Only for the bundled page: its colours are known.
    pub check_pixels: bool,
    pub nav_smoke: Option<NavSmoke>,
    pub tabs_smoke: Option<TabsSmoke>,
}

/// Everything the pump drives. Owned by `HOST` in `main.rs`.
pub struct Host {
    pub engine: Engine,
    window: slint::Weak<AppWindow>,
    /// The engine's single-shot pump timer.
    pub timer: slint::Timer,
    presenter: FramePresenter,
    /// The one owner of tabs and Spaces.
    model: Model,
    /// The view of every live tab.
    views: BTreeMap<TabId, ViewId>,
    /// Effects not yet executed: they wait until the window can size a view.
    pending: VecDeque<Effect>,
    /// CEF's frame rate, known once winit has created the window.
    frame_rate: Option<u32>,
    bar: Bar,
    /// The view last told about keyboard focus.
    focus_view: Option<ViewId>,
    /// The tab whose last frame `web-frame` shows, if any.
    presented: Option<TabId>,
    pub stats: FrameStats,
    last_present: Option<Instant>,
    idle_runs: u32,
    checks: Checks,
    pub nav_title: Option<String>,
    pub tabs_passed: bool,
    input: InputState,
    /// Why the loop was quit early, if it was.
    pub failure: Option<String>,
}

impl Host {
    /// A host with one tab on `url`; its view opens once the window exists.
    pub fn new(
        engine: Engine,
        window: &AppWindow,
        presenter: FramePresenter,
        url: String,
        checks: Checks,
    ) -> Self {
        let mut host = Self {
            engine,
            window: window.as_weak(),
            timer: slint::Timer::default(),
            presenter,
            model: Model::new(Instant::now()),
            views: BTreeMap::new(),
            pending: VecDeque::new(),
            frame_rate: None,
            bar: Bar {
                mode: BarMode::NewTab,
                rows: vec![],
            },
            focus_view: None,
            presented: None,
            stats: FrameStats::default(),
            last_present: None,
            idle_runs: 0,
            checks,
            nav_title: None,
            tabs_passed: false,
            input: InputState::default(),
            failure: None,
        };
        host.dispatch(new_tab(url, None));
        host
    }

    /// The only way tabs change: apply, then execute the effects.
    pub fn dispatch(&mut self, command: Command) {
        let mut queue = VecDeque::from([command]);
        while let Some(command) = queue.pop_front() {
            // `apply` takes the model by value; a fresh one holds its place meanwhile.
            let placeholder = Model::new(Instant::now());
            let current = std::mem::replace(&mut self.model, placeholder);
            let (model, effects) = model::apply(current, command);
            self.model = model;
            self.pending.extend(effects);
            queue.extend(self.flush());
        }
        self.after_change();
    }

    /// Executes queued effects once views can be opened. Returns commands
    /// that must follow (a tab whose view could not be opened is closed).
    fn flush(&mut self) -> Vec<Command> {
        let Some(frame_rate) = self.frame_rate else {
            return vec![];
        };
        let mut follow = vec![];
        while let Some(effect) = self.pending.pop_front() {
            let c = call(&effect, &self.model);
            follow.extend(self.execute(c, frame_rate));
        }
        follow
    }

    /// The only place effects touch the engine or open the bar.
    fn execute(&mut self, c: Call, frame_rate: u32) -> Option<Command> {
        match c {
            Call::OpenView { tab, url } => {
                let window = self.window.upgrade()?;
                if self.views.contains_key(&tab) {
                    return None;
                }
                let config = ViewConfig {
                    url,
                    layout: content_layout(&window),
                    frame_rate,
                    gpu: self.presenter.context().clone(),
                };
                match self.engine.open_view(config) {
                    Ok(id) => {
                        self.views.insert(tab, id);
                    }
                    Err(e) => {
                        eprintln!("wubble: tab {}: {e}; closing it", tab.0);
                        return Some(Command::CloseTab(tab));
                    }
                }
            }
            Call::CloseView(tab) => {
                if let Some(id) = self.views.remove(&tab) {
                    self.engine.close_view(id);
                }
            }
            Call::Load { tab, url } => {
                if let Some(view) = self.tab_view(tab) {
                    view.load(&url);
                }
            }
            Call::SetVisible { tab, visible } => {
                if let Some(view) = self.tab_view(tab) {
                    view.set_visible(visible);
                }
            }
            // Later, outside the host: opening the bar calls back into it.
            Call::ShowCommandBar => {
                let _ = self
                    .window
                    .upgrade_in_event_loop(|window| window.invoke_request_new_tab());
            }
            Call::Nothing => {}
        }
        None
    }

    /// Re-derives the shell from the model.
    fn after_change(&mut self) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        // Another tab's frame (and cursor) must not stay on screen while
        // input already goes to the visible tab: blank until it paints.
        if self.presented.is_some() && self.presented != self.model.visible_tab() {
            self.presented = None;
            window.set_web_frame(slint::Image::default());
            window.set_web_frame_clip_width(0);
            window.set_web_frame_clip_height(0);
            window.set_web_cursor(Cursor::Default as i32);
        }
        self.sync_focus(&window);
        crate::bind::render(&window, &crate::shell::view_model(&self.model));
    }

    fn tab_view(&mut self, tab: TabId) -> Option<&mut View> {
        let id = *self.views.get(&tab)?;
        self.engine.view(id)
    }

    /// The visible tab's view, the only one that gets frames and input.
    fn view(&mut self) -> Option<&mut View> {
        self.tab_view(self.model.visible_tab()?)
    }

    fn tab_of(&self, view: ViewId) -> Option<TabId> {
        self.views
            .iter()
            .find_map(|(&tab, &id)| (id == view).then_some(tab))
    }

    pub fn pump(&mut self, wake: Wake) {
        let outcome = match wake {
            Wake::Schedule(delay_ms) => self.engine.schedule_pump(delay_ms),
            Wake::TimerFired => self.engine.pump_timer_fired(),
        };
        let PumpOutcome {
            timer,
            worked,
            events,
        } = outcome;
        match timer {
            Some(PumpTimer::Start(delay)) => {
                self.timer.start(slint::TimerMode::SingleShot, delay, || {
                    deliver(Wake::TimerFired)
                });
            }
            Some(PumpTimer::Cancel) => self.timer.stop(),
            None => {}
        }
        if !worked {
            return;
        }
        let Some(window) = self.window.upgrade() else {
            return;
        };
        for (view, event) in events {
            self.on_web_event(&window, view, event);
        }
        self.close_tabs_of_gone_views();
        self.smoke_step(SmokeInput::Worked);
        self.tabs_smoke_step();
        if let Err(e) = self.show_frame(&window) {
            self.fail(e);
        }
        if self.checks.quit_after.is_some() {
            self.idle_runs += 1;
            if self.idle_runs > SMOKE_MAX_IDLE_RUNS {
                self.fail(format!(
                    "no new frame after {SMOKE_MAX_IDLE_RUNS} runs of CEF work ({} accelerated so far)",
                    self.stats.accelerated
                ));
            }
        }
    }

    /// A page that closed itself (`window.close()`) takes its tab with it.
    fn close_tabs_of_gone_views(&mut self) {
        let gone: Vec<TabId> = self
            .views
            .iter()
            .filter(|&(_, &id)| !self.engine.has_view(id))
            .map(|(&tab, _)| tab)
            .collect();
        for tab in gone {
            self.views.remove(&tab);
            self.dispatch(Command::CloseTab(tab));
        }
    }

    fn show_frame(&mut self, window: &AppWindow) -> Result<(), String> {
        if self.frame_rate.is_none() {
            // Before winit has created the window there is no monitor to
            // ask; CEF's next pump (at most `MAX_TIMER_DELAY` away) retries.
            let Some(refresh) = window.window().with_winit_window(display_refresh_mhz) else {
                return Ok(());
            };
            let frame_rate = pacing::frame_rate_for(refresh);
            eprintln!("wubble: CEF frame rate {frame_rate} (display {refresh:?} mHz)");
            self.frame_rate = Some(frame_rate);
            let follow = self.flush();
            for command in follow {
                self.dispatch(command);
            }
            self.after_change();
        }
        let new_cpu = self
            .stats
            .record_cpu_fallback(self.engine.cpu_fallback_frames());
        if new_cpu > 0 {
            eprintln!("wubble: {new_cpu} CPU fallback frame(s) (OnPaint), not shown");
        }
        let layout = content_layout(window);
        let Some(view) = self.view() else {
            return Ok(());
        };
        view.set_layout(layout);
        let Some(frame) = view.take_frame() else {
            return Ok(());
        };
        let frame_size = frame.size();
        let clip = frame_size.cropped_to(layout.size);
        let image = self.presenter.present(frame).map_err(|e| e.to_string())?;
        window.set_web_frame(image);
        self.presented = self.model.visible_tab();
        window.set_web_frame_clip_width(slint_int(clip.width));
        window.set_web_frame_clip_height(slint_int(clip.height));
        let now = Instant::now();
        self.stats
            .record_accelerated(self.last_present.map(|last| now - last));
        self.last_present = Some(now);
        self.idle_runs = 0;
        if self
            .checks
            .quit_after
            .is_some_and(|n| self.stats.accelerated >= n)
        {
            eprintln!(
                "smoke: content {}x{} @{} frame {}x{} clip {}x{}",
                layout.size.width,
                layout.size.height,
                layout.scale,
                frame_size.width,
                frame_size.height,
                clip.width,
                clip.height
            );
            if self.checks.check_pixels {
                let sample = check_pixels(window)?;
                println!(
                    "smoke: pixels ok distinct={} mean_luma={}",
                    sample.distinct, sample.mean_luma
                );
            }
            quit();
        }
        Ok(())
    }

    fn on_web_event(&mut self, window: &AppWindow, view: ViewId, event: WebEvent) {
        // Events of a view whose tab is already closed are dropped.
        let Some(tab) = self.tab_of(view) else {
            return;
        };
        self.smoke_step(SmokeInput::Event(&event));
        match event {
            WebEvent::CursorChanged(cursor) => {
                if Some(tab) != self.model.visible_tab() {
                    return;
                }
                window.set_web_cursor(cursor as i32);
                // Slint applies a TouchArea's cursor only while it handles a
                // mouse event (i-slint-core 1.19 `window.rs`, after
                // `process_mouse_input`), and CEF reports the shape after
                // the move that caused it. Replay the pointer where it is,
                // once the host is free again, so the shape shows without
                // waiting for the mouse to move.
                if let Some(position) = self.input.pointer {
                    let _ = self.window.upgrade_in_event_loop(move |window| {
                        window
                            .window()
                            .dispatch_event(slint::platform::WindowEvent::PointerMoved { position })
                    });
                }
            }
            WebEvent::UrlChanged(url) => self.dispatch(Command::UrlChanged { tab, url }),
            WebEvent::TitleChanged(title) => self.dispatch(Command::TitleChanged { tab, title }),
            WebEvent::PopupRequested { url } => self.dispatch(new_tab(url, Some(tab))),
            WebEvent::LoadingChanged { loading, .. } => {
                self.dispatch(Command::LoadingChanged { tab, loading });
            }
            WebEvent::RendererGone => self.dispatch(Command::RendererGone(tab)),
            // No favicons in the shell yet.
            WebEvent::FaviconChanged(_) => {}
        }
    }

    // ----- command bar

    /// Ctrl+T or the new-tab row: the bar is opening on an empty query.
    pub fn bar_for_new_tab(&mut self, window: &AppWindow) {
        self.open_bar(window, BarMode::NewTab, String::new());
    }

    /// Ctrl+L or the address pill: the bar opens on the visible tab's URL.
    pub fn bar_for_address(&mut self, window: &AppWindow) {
        let visible = self.model.visible_tab();
        match visible.and_then(|t| self.model.tabs.get(&t)) {
            Some(tab) => {
                let url = tab.url.clone();
                self.open_bar(window, BarMode::Navigate(tab.id), url);
            }
            None => self.bar_for_new_tab(window),
        }
    }

    fn open_bar(&mut self, window: &AppWindow, mode: BarMode, query: String) {
        self.bar.mode = mode;
        window.set_command_query(query.as_str().into());
        self.bar_query(window, &query);
    }

    pub fn bar_query(&mut self, window: &AppWindow, query: &str) {
        self.bar.rows = bar::bar_rows(&self.model, query, self.bar.mode);
        crate::bind::render_bar(window, &self.bar.rows);
    }

    pub fn bar_chosen(&mut self, index: i32) {
        let row = usize::try_from(index)
            .ok()
            .and_then(|i| self.bar.rows.get(i));
        if let Some(row) = row {
            let command = bar::action_command(row.action.clone());
            self.dispatch(command);
        }
    }

    /// Ctrl+W.
    pub fn close_visible_tab(&mut self) {
        if let Some(tab) = self.model.visible_tab() {
            self.dispatch(Command::CloseTab(tab));
        }
    }

    /// The coarse lifecycle tick.
    pub fn tick(&mut self, now: Instant) {
        self.dispatch(Command::Tick {
            now,
            pressure: Pressure::None,
        });
    }

    // ----- smokes

    fn smoke_step(&mut self, input: SmokeInput) {
        let Some(state) = self.checks.nav_smoke.take() else {
            return;
        };
        let want = nav_smoke::title_of(NAV_PAGE).unwrap_or_default();
        let (state, action) = nav_smoke::next(state, input, want);
        self.checks.nav_smoke = Some(state);
        match action {
            Some(SmokeAction::LoadSecondPage) => {
                if let Some(tab) = self.model.visible_tab() {
                    self.dispatch(Command::Navigate {
                        tab,
                        url: page_url(NAV_PAGE),
                    });
                }
            }
            Some(SmokeAction::Pass(title)) => {
                self.nav_title = Some(title);
                quit();
            }
            Some(SmokeAction::Fail(reason)) => self.fail(reason),
            None => {}
        }
    }

    fn seen(&self) -> Seen {
        let order = self.model.order(self.model.active_space);
        let shown = self
            .model
            .visible_tab()
            .and_then(|t| self.views.get(&t))
            .is_some_and(|&id| self.engine.is_visible(id));
        Seen {
            titles: order
                .iter()
                .filter_map(|t| self.model.tabs.get(t))
                .map(|t| t.title.clone())
                .collect(),
            views: self.engine.view_count(),
            live_tabs: self.model.live_tabs(),
            visible_views: self.engine.visible_views(),
            shown_is_visible: shown,
            stale_frame: self
                .presented
                .is_some_and(|p| Some(p) != self.model.visible_tab()),
        }
    }

    fn tabs_smoke_step(&mut self) {
        let Some(state) = self.checks.tabs_smoke.take() else {
            return;
        };
        let want =
            crate::TABS_SMOKE_PAGES.map(|page| nav_smoke::title_of(page).unwrap_or_default());
        let (state, action) = tabs_smoke::next(state, &self.seen(), &want);
        self.checks.tabs_smoke = Some(state);
        let nth = |host: &Self, i: usize| host.model.order(host.model.active_space).get(i).copied();
        match action {
            Some(tabs_smoke::Action::OpenPages) => {
                for page in [NAV_PAGE, TAB3_PAGE] {
                    self.dispatch(new_tab(page_url(page), None));
                }
            }
            Some(tabs_smoke::Action::Activate(i)) => {
                if let Some(tab) = nth(self, i) {
                    self.dispatch(Command::ActivateTab(tab));
                }
            }
            Some(tabs_smoke::Action::DiscardHidden) => {
                self.tick(Instant::now() + SMOKE_DISCARD_AFTER);
            }
            Some(tabs_smoke::Action::Close(i)) => {
                if let Some(tab) = nth(self, i) {
                    self.dispatch(Command::CloseTab(tab));
                }
            }
            Some(tabs_smoke::Action::Pass) => {
                self.tabs_passed = true;
                quit();
            }
            Some(tabs_smoke::Action::Fail(reason)) => self.fail(reason),
            None => {}
        }
    }

    // ----- input

    /// Tells the visible view about keyboard focus: it has it while the
    /// window has focus and the web content is the shell's focused item.
    /// A view that stops being visible loses focus first.
    pub fn sync_focus(&mut self, window: &AppWindow) {
        let target = self
            .model
            .visible_tab()
            .and_then(|t| self.views.get(&t))
            .copied();
        if target != self.focus_view {
            if let Some(view) = self.focus_view.and_then(|id| self.engine.view(id)) {
                view.focus(false);
            }
            self.input.forget_view_focus();
            self.focus_view = target;
        }
        let focused = self.input.window_focused && window.get_web_focused();
        let view_open = self.view().is_some();
        if let Some(focused) = self.input.focus_change(focused, view_open)
            && let Some(view) = self.view()
        {
            view.focus(focused);
        }
    }

    /// Raw winit events, before Slint sees them: keys for the page,
    /// modifiers and window focus.
    pub fn window_event(&mut self, event: &WindowEvent) -> EventResult {
        let Some(window) = self.window.upgrade() else {
            return EventResult::Propagate;
        };
        match event {
            WindowEvent::ModifiersChanged(modifiers) => self.input.keys = modifiers.state(),
            WindowEvent::CursorMoved { position, .. } => {
                let position = position.to_logical(window.window().scale_factor().into());
                self.input.pointer = Some(slint::LogicalPosition::new(position.x, position.y));
            }
            WindowEvent::CursorLeft { .. } => self.input.pointer = None,
            WindowEvent::Focused(focused) => {
                self.input.window_focused = *focused;
                self.sync_focus(&window);
            }
            // Synthetic presses (keys already held when the window gained
            // focus, X11) never reach the page.
            WindowEvent::KeyboardInput {
                event,
                is_synthetic: false,
                ..
            } => {
                let key = self.input.key_input(event);
                let route = input::route(
                    window.get_web_focused(),
                    &key.unmodified,
                    key.modifiers.keys,
                );
                if route != KeyRoute::Shell
                    && let Some(view) = self.view()
                {
                    view.key(&key);
                }
                if route == KeyRoute::Page {
                    return EventResult::PreventDefault;
                }
            }
            _ => {}
        }
        EventResult::Propagate
    }

    pub fn pointer_moved(&mut self, point: Point) {
        let (modifiers, now) = (self.input.modifiers(), self.input.now());
        if let Some(view) = self.view() {
            view.mouse_move(point, &modifiers, now);
        }
    }

    pub fn pointer_left(&mut self, point: Point) {
        let (modifiers, now) = (self.input.modifiers(), self.input.now());
        if let Some(view) = self.view() {
            view.mouse_leave(point, &modifiers, now);
        }
    }

    pub fn pointer_button(&mut self, button: PointerEventButton, pressed: bool, point: Point) {
        let Some(button) = input::mouse_button(button) else {
            return;
        };
        let sent = if pressed {
            vec![button]
        } else {
            input::releases(self.input.buttons, button)
        };
        for button in sent {
            self.input.buttons = self.input.buttons.with(button, pressed);
            let (modifiers, now) = (self.input.modifiers(), self.input.now());
            if let Some(view) = self.view() {
                view.mouse_button(button, pressed, point, &modifiers, now);
            }
        }
    }

    pub fn pointer_cancelled(&mut self) {
        self.input.buttons = Default::default();
        if let Some(view) = self.view() {
            view.capture_lost();
        }
    }

    pub fn scrolled(&mut self, point: Point, dx: f32, dy: f32) {
        let modifiers = self.input.modifiers();
        if let Some(view) = self.view() {
            view.wheel(point, dx, dy, &modifiers);
        }
    }

    pub fn fail(&mut self, reason: String) {
        if self.failure.is_none() {
            self.failure = Some(reason);
        }
        quit();
    }
}

/// Snapshots the window and checks that the content rectangle shows the page.
fn check_pixels(window: &AppWindow) -> Result<pixels::Sample, String> {
    let snapshot = window
        .window()
        .take_snapshot()
        .map_err(|e| format!("cannot snapshot the window: {e}"))?;
    let scale = window.window().scale_factor();
    let region = pixels::Region {
        x: physical(window.get_content_x(), scale),
        y: physical(window.get_content_y(), scale),
        width: physical(window.get_content_width(), scale),
        height: physical(window.get_content_height(), scale),
    };
    pixels::check(&snapshot, region)
}

fn new_tab(url: String, opener: Option<TabId>) -> Command {
    Command::NewTab {
        url,
        container: None,
        opener,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wubble_core::space::{ContainerId, SpaceId};

    fn one_tab() -> (Model, TabId) {
        let (m, _) = model::apply(
            Model::new(Instant::now()),
            new_tab("https://a.test/".into(), None),
        );
        let t = m.visible_tab().unwrap();
        (m, t)
    }

    #[test]
    fn model_effects_map_to_engine_calls() {
        let (m, t) = one_tab();
        let url = || "https://a.test/".to_string();
        let cases = [
            (
                Effect::CreateTab {
                    tab: t,
                    url: url(),
                    container: ContainerId::DEFAULT,
                },
                Call::OpenView { tab: t, url: url() },
            ),
            (Effect::CloseTab(t), Call::CloseView(t)),
            (
                Effect::Navigate { tab: t, url: url() },
                Call::Load { tab: t, url: url() },
            ),
            (Effect::ShowCommandBar, Call::ShowCommandBar),
            (Effect::ActivateTab(t), Call::Nothing),
            (Effect::SpaceChanged(SpaceId(0)), Call::Nothing),
        ];
        for (effect, want) in cases {
            assert_eq!(call(&effect, &m), want, "{effect:?}");
        }
    }

    #[test]
    fn lifecycle_effects_map_to_engine_calls() {
        let (m, t) = one_tab();
        let cases = [
            (
                Life::Show,
                Call::SetVisible {
                    tab: t,
                    visible: true,
                },
            ),
            (
                Life::Hide,
                Call::SetVisible {
                    tab: t,
                    visible: false,
                },
            ),
            (Life::Discard, Call::CloseView(t)),
            (Life::ReleaseTexture, Call::CloseView(t)),
            (
                Life::Restore,
                Call::OpenView {
                    tab: t,
                    url: "https://a.test/".into(),
                },
            ),
            (Life::Freeze, Call::Nothing),
            (Life::Unfreeze, Call::Nothing),
            (Life::CaptureSnapshot, Call::Nothing),
            (Life::ReleaseSnapshot, Call::Nothing),
        ];
        for (effect, want) in cases {
            let effect = Effect::Lifecycle { tab: t, effect };
            assert_eq!(call(&effect, &m), want, "{effect:?}");
        }
    }

    /// Executes `effects` against a fake engine: the set of tabs with a
    /// view and which of them are visible, as `Host::execute` would.
    fn simulate(model: &Model, effects: &[Effect], views: &mut BTreeMap<TabId, bool>) {
        for effect in effects {
            match call(effect, model) {
                Call::OpenView { tab, .. } => {
                    views.entry(tab).or_insert(true);
                }
                Call::CloseView(tab) => {
                    views.remove(&tab);
                }
                Call::SetVisible { tab, visible } => {
                    if let Some(v) = views.get_mut(&tab) {
                        *v = visible;
                    }
                }
                Call::Load { .. } | Call::ShowCommandBar | Call::Nothing => {}
            }
        }
    }

    #[test]
    fn views_follow_the_live_tabs_through_a_session() {
        let mut m = Model::new(Instant::now());
        let clock = Instant::now();
        let mut views = BTreeMap::new();
        let later = |mins: u64| Command::Tick {
            now: clock + Duration::from_secs(mins * 60),
            pressure: Pressure::None,
        };
        let script = [
            new_tab("a".into(), None),
            new_tab("b".into(), None),
            new_tab("c".into(), Some(TabId(1))),
            Command::ActivateTab(TabId(1)),
            later(20),
            Command::ActivateTab(TabId(2)),
            Command::RendererGone(TabId(1)),
            Command::NewSpace {
                name: "W".into(),
                tint: wubble_core::space::Rgb(0, 0, 0),
                container: ContainerId::DEFAULT,
            },
            Command::SwitchSpace(SpaceId(1)),
            new_tab("w".into(), None),
            Command::SwitchSpace(SpaceId(0)),
            Command::CloseTab(TabId(2)),
            later(60),
            Command::ActivateTab(TabId(1)),
            Command::CloseTab(TabId(1)),
        ];
        for command in script {
            let label = format!("{command:?}");
            let (next, effects) = model::apply(m, command);
            m = next;
            simulate(&m, &effects, &mut views);
            assert_eq!(views.len(), m.live_tabs(), "{label}: view count");
            let visible: Vec<TabId> = views.iter().filter(|(_, v)| **v).map(|(t, _)| *t).collect();
            assert_eq!(
                visible,
                m.visible_tab().into_iter().collect::<Vec<_>>(),
                "{label}: visible views"
            );
        }
    }

    #[test]
    fn restoring_a_tab_the_model_forgot_does_nothing() {
        let effect = Effect::Lifecycle {
            tab: TabId(9),
            effect: Life::Restore,
        };
        assert_eq!(call(&effect, &Model::new(Instant::now())), Call::Nothing);
    }

    #[test]
    fn restore_reloads_the_url_the_model_last_heard() {
        let (m, t) = one_tab();
        let (m, _) = model::apply(
            m,
            Command::UrlChanged {
                tab: t,
                url: "https://a.test/later".into(),
            },
        );
        let effect = Effect::Lifecycle {
            tab: t,
            effect: Life::Restore,
        };
        assert_eq!(
            call(&effect, &m),
            Call::OpenView {
                tab: t,
                url: "https://a.test/later".into()
            }
        );
    }
}
