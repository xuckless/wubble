//! `--smoke quit`: Inspect on a selection, then quit with DevTools windows open.
//!
//! Two bugs behind one crash: Inspect showed no DevTools window (CEF's external
//! pump never ran GLib's default context, where Chromium watches its own
//! windows' Wayland connection, so the window was never configured), and
//! quitting with that window alive ran `cef::shutdown` under it (SIGSEGV in
//! `CefBrowserInfo::RemoveFrame`). Three views, pumped the way the host pumps
//! (CEF's schedule requests, a single-shot timer):
//! 1. View one: select its text, right-click the selection, dismiss the menu
//!    and Inspect there: the detached DevTools window revealing that node, as
//!    the context menu's Inspect opens it (`DockEffect::OpenDetached`).
//! 2. View two: a detached DevTools window without a node.
//! 3. Both DevTools windows are counted by the engine and report themselves
//!    visible at a non-zero size; the page also tries a `window.open` (welding
//!    denies it: no browser).
//! 4. View three asks for DevTools just before the quit, so that one reports
//!    itself created during the shutdown; the quit must still be clean.
//!
//! No window of wubble's; bounded by a count of pump turns, not by time.

use std::error::Error;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use slint::winit_030::winit::keyboard::ModifiersState;
use wubble_engine::app::{
    Engine, EngineConfig, EngineError, PumpOutcome, PumpScheduler, PumpTimer, RequestMode, ViewId,
};
use wubble_engine::input::{Buttons, Modifiers, MouseButton, Point};
use wubble_engine::page::PageEvent;
use wubble_engine::view::{Layout, View, ViewConfig};
use wubble_gfx::{FrameSize, GpuContext};

/// Pump turns each step waits for what it needs.
const MAX_TURNS: u32 = 20_000;

/// Pump turns the first DevTools windows get to load before the quit: one only
/// just created may close with its view, which hid the crash.
const SETTLE_TURNS: u32 = 2_000;

/// Turns between tries at the selection and right-click, while the page loads.
const RETRY_EVERY: u32 = 200;

/// Each turn sleeps this long, so CEF's other threads get to work. Pacing only:
/// nothing measures it.
const TURN_PAUSE: Duration = Duration::from_millis(1);

/// On the bundled page's paragraph.
const TEXT_AT: Point = Point { x: 40.0, y: 26.0 };

const SELECT: &str = "getSelection().selectAllChildren(document.querySelector('p'))";

/// Run in each DevTools window: its title reports what its page sees.
const VISIBLE: &str = "document.title = 'wubble-visible ' + document.visibilityState + ' ' \
                       + innerWidth + 'x' + innerHeight";

/// CEF's pump requests and the host's single-shot timer, as `main.rs` runs them.
struct Pump {
    requests: Arc<Mutex<Vec<i64>>>,
    timer: Option<Instant>,
}

impl Pump {
    fn new() -> (Self, PumpScheduler) {
        let requests: Arc<Mutex<Vec<i64>>> = Arc::default();
        let sink = requests.clone();
        let scheduler = PumpScheduler::new(move |delay| {
            sink.lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(delay)
        });
        let pump = Self {
            requests,
            timer: None,
        };
        (pump, scheduler)
    }

    /// Runs what CEF asked for and the timer if due, then pauses; the page
    /// events reported meanwhile.
    fn turn(&mut self, engine: &mut Engine) -> Vec<(ViewId, PageEvent)> {
        let requests =
            std::mem::take(&mut *self.requests.lock().unwrap_or_else(PoisonError::into_inner));
        let mut outcomes: Vec<PumpOutcome> = requests
            .into_iter()
            .map(|delay| engine.schedule_pump(delay))
            .collect();
        if self.timer.is_some_and(|at| Instant::now() >= at) {
            self.timer = None;
            outcomes.push(engine.pump_timer_fired());
        }
        for outcome in outcomes {
            match outcome.timer {
                Some(PumpTimer::Start(delay)) => self.timer = Some(Instant::now() + delay),
                Some(PumpTimer::Cancel) => self.timer = None,
                None => {}
            }
        }
        std::thread::sleep(TURN_PAUSE);
        engine.take_page_events()
    }

    /// Turns until `done` holds, at most [`MAX_TURNS`]; `step` names it.
    fn until(
        &mut self,
        engine: &mut Engine,
        step: &str,
        mut done: impl FnMut(&mut Engine, u32, Vec<(ViewId, PageEvent)>) -> bool,
    ) -> Result<(), String> {
        for turns in 0..MAX_TURNS {
            let events = self.turn(engine);
            if done(engine, turns, events) {
                return Ok(());
            }
        }
        Err(format!("{step}: not after {MAX_TURNS} pump turns"))
    }
}

fn view(engine: &mut Engine, id: ViewId) -> Result<&mut View, &'static str> {
    engine.view(id).ok_or("a view closed by itself")
}

fn right_click(view: &mut View, at: Point) {
    let up = Modifiers {
        keys: ModifiersState::empty(),
        buttons: Buttons::default(),
    };
    let down = Modifiers {
        buttons: Buttons::default().with(MouseButton::Right, true),
        ..up
    };
    view.focus(true);
    view.mouse_move(at, &up, Duration::ZERO);
    view.mouse_button(MouseButton::Right, true, at, &down, Duration::ZERO);
    view.mouse_button(MouseButton::Right, false, at, &up, Duration::ZERO);
}

/// `(visibility, width, height)` from a DevTools title [`VISIBLE`] set.
fn visibility(title: &str) -> Option<(&str, u32, u32)> {
    let mut words = title.strip_prefix("wubble-visible ")?.split(' ');
    let state = words.next()?;
    let (w, h) = words.next()?.split_once('x')?;
    Some((state, w.parse().ok()?, h.parse().ok()?))
}

/// Passes with the DevTools windows the engine counted at the quit.
pub fn run(config: Result<EngineConfig, EngineError>) -> Result<usize, Box<dyn Error>> {
    let gpu = GpuContext::new()?;
    let (mut pump, scheduler) = Pump::new();
    let mut engine = Engine::start(EngineConfig {
        pump_scheduler: Some(scheduler),
        ..config?
    })?;
    engine.set_request_mode(RequestMode::AskHost);
    // The host's first pump, as `main.rs` starts it.
    pump.requests
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push(0);
    let open = |engine: &mut Engine| {
        engine.open_view(ViewConfig {
            url: crate::page_url(crate::NAV_PAGE),
            layout: Layout {
                size: FrameSize::new(800, 600),
                scale: 1.0,
                area: None,
            },
            frame_rate: 30,
            begin_frames: false,
            gpu: gpu.presenter().context().clone(),
        })
    };
    let [first, second, third] = [open(&mut engine)?, open(&mut engine)?, open(&mut engine)?];

    // 1. Inspect on a selection, through the context menu's path.
    let mut inspected = false;
    pump.until(
        &mut engine,
        "a context menu on the selection",
        |engine, turns, events| {
            for (id, event) in events {
                if let PageEvent::ContextMenuRequested { id: menu, menu: at } = event
                    && id == first
                    && let Some(view) = engine.view(first)
                {
                    view.answer_context_menu(menu, None);
                    if !at.selection_text.is_empty() {
                        view.open_devtools_at(Some((at.x, at.y)));
                        inspected = true;
                    }
                }
            }
            if !inspected
                && turns % RETRY_EVERY == RETRY_EVERY - 1
                && let Some(view) = engine.view(first)
            {
                view.run_script(SELECT);
                right_click(view, TEXT_AT);
            }
            inspected
        },
    )?;
    // 2. Dev tools.
    view(&mut engine, second)?.open_devtools();
    pump.until(&mut engine, "two DevTools windows", |engine, _, _| {
        engine.extra_browsers() == 2
            && engine
                .extra_titles()
                .iter()
                .all(|t| t.starts_with("DevTools"))
    })?;
    for id in [first, second] {
        if !view(&mut engine, id)?.has_devtools() {
            return Err("a view has no DevTools after all".into());
        }
    }
    // 3. Shown, at a size.
    engine.run_script_in_extras(VISIBLE);
    pump.until(
        &mut engine,
        "DevTools windows reporting visibility",
        |engine, _, _| {
            engine
                .extra_titles()
                .iter()
                .all(|t| visibility(t).is_some())
        },
    )?;
    for title in engine.extra_titles() {
        match visibility(&title) {
            Some(("visible", w, h)) if w > 0 && h > 0 => {}
            _ => return Err(format!("a DevTools window is not shown: {title:?}").into()),
        }
    }
    for _ in 0..SETTLE_TURNS {
        pump.turn(&mut engine);
    }
    view(&mut engine, first)?.run_script("window.open('about:blank')");
    // 4. One more, asked for just before the quit.
    view(&mut engine, third)?.open_devtools();
    let counted = engine.extra_browsers();
    crate::drain(&mut engine);
    if engine.abandoned() {
        return Err("the shutdown gave up on browsers that never closed".into());
    }
    Ok(counted)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visibility_titles_parse() {
        assert_eq!(
            visibility("wubble-visible visible 616x615"),
            Some(("visible", 616, 615))
        );
        assert_eq!(
            visibility("wubble-visible hidden 0x0"),
            Some(("hidden", 0, 0))
        );
        assert_eq!(visibility("DevTools - data:"), None);
        assert_eq!(visibility("wubble-visible visible 1x"), None);
    }
}
