//! CEF initialisation, message-loop integration, process-wide switches.
//!
//! [`Engine`] owns the CEF runtime (via `welding::CefRuntime`, which sets
//! windowless rendering, the external message pump and `cef::shutdown` on drop).
//! CEF schedules its own pumping: [`EngineConfig::pump_scheduler`] receives
//! each request on any thread, the host hands it to [`Engine::schedule_pump`]
//! on the thread that started the engine and runs the one timer the engine
//! asks for ([`PumpOutcome`]). The scheduling rules are the pure
//! [`wubble_core::pump::transition`]; every lifecycle change goes through
//! [`state::transition`]. Their effects run in `Engine::pump` and
//! `Engine::execute`.

pub mod state;

use std::ffi::OsString;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use state::{Effect, Event, State};
pub use welding::PumpScheduler;
use welding::{CefRuntime, CefRuntimeConfig, CefSandboxMode};
use wubble_core::pump::{self, PumpEffect, PumpEvent, PumpState};

use crate::filter::ContentFilter;
use crate::nav::WebEvent;
use crate::page::{PageEvent, PageQueue};
use crate::view::{View, ViewConfig};

/// Sandbox off on every platform for now (TODO.md, M1 "Sandbox enabled").
/// wubble-helper passes the same mode to its `execute_process`; keep them
/// equal (the helper must not depend on this crate; see the TODO.md row).
const SANDBOX: CefSandboxMode = CefSandboxMode::UnsandboxedTrustedContent;

/// What the engine needs to start.
#[derive(Clone, Debug)]
pub struct EngineConfig {
    /// Directory holding the main binary, `wubble-helper` and the CEF files.
    pub exe_dir: PathBuf,
    /// CEF's `root_cache_path`: profile data and the process-singleton lock.
    /// Never left to CEF's shared default (`~/.config/cef_user_data`), which
    /// other CEF apps and concurrent wubble processes would also lock.
    pub cache_dir: PathBuf,
    /// Receives CEF's pump requests (delay in ms, `<= 0` = now) on any thread;
    /// it must only forward them to [`Engine::schedule_pump`] on the engine's
    /// thread. `None` without a host event loop: the caller drives CEF with
    /// [`Engine::tick`] instead.
    pub pump_scheduler: Option<PumpScheduler>,
    /// Checks every request each view sends and injects cosmetic filters
    /// ([`crate::filter`]). `None`: nothing is filtered.
    pub content_filter: Option<Arc<dyn ContentFilter>>,
    /// Chromium features chosen by the user's settings; CEF takes them only at start.
    pub features: PageFeatures,
}

/// Page features that are Chromium start switches (the settings' restart rows).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PageFeatures {
    /// Scrollbars drawn over the page, thin and fading, instead of a classic gutter that takes
    /// layout width (Chromium `features::kOverlayScrollbar`, the ChromeOS style). Measured on
    /// CEF 156 windowless, Linux: a tall page's `innerWidth - documentElement.clientWidth` is 15
    /// without it and 0 with it. `FluentOverlayScrollbar` alone changes nothing there.
    pub overlay_scrollbars: bool,
    /// Middle-click autoscroll (press the wheel, then move to scroll), which Chromium turns on
    /// by default only on Windows (`features::kMiddleClickAutoscroll`).
    pub middle_click_autoscroll: bool,
    /// `settings::MEMORY_BACK_FORWARD_CACHE`.
    pub back_forward_cache: BackForwardCache,
    /// `settings::MEMORY_MAX_DECODED_IMAGE_MB`; 0 = Chromium's default.
    pub max_decoded_image_mb: u32,
    /// `settings::PROCESS_SITE_ISOLATION`.
    pub site_isolation: bool,
    /// `settings::PROCESS_RENDERER_LIMIT`; 0 = Chromium decides.
    pub renderer_limit: u32,
    /// `settings::PROCESS_SPARE_RENDERER`.
    pub spare_renderer: bool,
}

pub use wubble_core::settings::BackForwardCache;

impl Default for PageFeatures {
    fn default() -> Self {
        Self {
            overlay_scrollbars: true,
            middle_click_autoscroll: true,
            back_forward_cache: BackForwardCache::Default,
            max_decoded_image_mb: 0,
            site_isolation: true,
            renderer_limit: 0,
            spare_renderer: true,
        }
    }
}

/// One thing a page feature asks of Chromium's command line.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Knob {
    /// Goes into the one `enable-features`.
    Enable(&'static str),
    /// Goes into the one `disable-features`.
    Disable(&'static str),
    Switch(&'static str, Option<String>),
}

impl PageFeatures {
    /// The table: each knob, whether these features ask for it, and why. What prebuilt CEF 156
    /// honours: every switch name below is a string in libcef (checked); the feature names
    /// `BackForwardCache` (with `cache_size`) and `SpareRendererForSitePerProcess` are not
    /// (Chromium strips them), so those two come from Chromium's `content_features.cc` and are
    /// unverified until observed.
    fn knobs(self) -> Vec<(bool, Knob, &'static str)> {
        use Knob::*;
        let bf = self.back_forward_cache;
        vec![
            (
                self.overlay_scrollbars,
                Enable("OverlayScrollbar"),
                "thin scrollbars over the page",
            ),
            (
                self.middle_click_autoscroll,
                Enable("MiddleClickAutoscroll"),
                "middle-click autoscroll",
            ),
            (
                self.middle_click_autoscroll,
                Switch(
                    "enable-blink-features",
                    Some("MiddleClickAutoscroll".into()),
                ),
                "the Blink side of the same feature, which older builds read alone",
            ),
            (
                bf == BackForwardCache::Small,
                Enable("BackForwardCache:cache_size/2"),
                "keeps at most two left pages alive per tab",
            ),
            (
                bf == BackForwardCache::Off,
                Switch("disable-back-forward-cache", None),
                "left pages are not kept: wubble already sleeps tabs",
            ),
            (
                self.max_decoded_image_mb > 0,
                Switch(
                    "max-decoded-image-size-mb",
                    Some(self.max_decoded_image_mb.to_string()),
                ),
                "caps one decoded image (Chromium has no switch for the cache total)",
            ),
            (
                !self.site_isolation,
                Switch("disable-site-isolation-trials", None),
                "fewer renderers; weakens the defence against Spectre-style attacks",
            ),
            (
                self.renderer_limit > 0,
                Switch(
                    "renderer-process-limit",
                    Some(self.renderer_limit.to_string()),
                ),
                "renderers are reused past the limit",
            ),
            (
                !self.spare_renderer,
                Disable("SpareRendererForSitePerProcess"),
                "no renderer started ahead of the next navigation",
            ),
        ]
    }

    /// The switches: one `enable-features` and one `disable-features` (a switch given twice
    /// keeps one value, so every feature goes in these), then the plain switches in table order.
    fn switches(self) -> Vec<(String, Option<String>)> {
        let knobs: Vec<Knob> = self
            .knobs()
            .into_iter()
            .filter_map(|(on, knob, _)| on.then_some(knob))
            .collect();
        let list = |pick: fn(&Knob) -> Option<&'static str>| -> Vec<&str> {
            knobs.iter().filter_map(pick).collect()
        };
        let enable = list(|k| match k {
            Knob::Enable(f) => Some(f),
            _ => None,
        });
        let disable = list(|k| match k {
            Knob::Disable(f) => Some(f),
            _ => None,
        });
        let mut out = vec![];
        for (name, features) in [("enable-features", enable), ("disable-features", disable)] {
            if !features.is_empty() {
                out.push((name.into(), Some(features.join(","))));
            }
        }
        out.extend(knobs.into_iter().filter_map(|k| match k {
            Knob::Switch(name, value) => Some((name.to_string(), value)),
            _ => None,
        }));
        out
    }
}

impl EngineConfig {
    /// Config for the running executable's directory and the per-user cache.
    pub fn from_current_exe() -> Result<Self, EngineError> {
        let exe = std::env::current_exe().map_err(|e| EngineError::ExeDir(e.to_string()))?;
        let exe_dir = exe
            .parent()
            .ok_or_else(|| EngineError::ExeDir(format!("{} has no parent", exe.display())))?;
        let cache_dir = user_data_dir()
            .map(|dir| dir.join(CEF_CACHE))
            .ok_or(EngineError::CacheDir)?;
        Ok(Self {
            exe_dir: exe_dir.to_path_buf(),
            cache_dir,
            pump_scheduler: None,
            content_filter: None,
            features: PageFeatures::default(),
        })
    }
}

#[derive(Debug)]
pub enum EngineError {
    /// The running executable's directory could not be determined.
    ExeDir(String),
    /// No per-user directory for CEF's cache could be determined.
    CacheDir,
    /// The CEF subprocess executable is not next to the main binary.
    HelperMissing { path: PathBuf },
    /// CEF refused to initialise.
    Init(String),
    /// A browser view could not be created.
    View(String),
    /// The engine is shutting down or stopped; no new views.
    NotRunning,
}

impl fmt::Display for EngineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ExeDir(e) => write!(f, "cannot locate the wubble executable: {e}"),
            Self::CacheDir => write!(
                f,
                "cannot pick a cache directory: set XDG_DATA_HOME or HOME to an absolute path"
            ),
            Self::HelperMissing { path } => write!(
                f,
                "CEF helper not found at {}; run `cargo build --workspace` first",
                path.display()
            ),
            Self::Init(e) => write!(f, "CEF initialisation failed: {e}"),
            Self::View(e) => write!(f, "cannot create a browser view: {e}"),
            Self::NotRunning => write!(f, "the engine is not running"),
        }
    }
}

impl std::error::Error for EngineError {}

/// CEF's cache inside the per-user data directory.
pub const CEF_CACHE: &str = "cef";

/// The per-user data directory (profile): wubble's database and, under
/// [`CEF_CACHE`], CEF's cache. `None` when the environment names none.
pub fn user_data_dir() -> Option<PathBuf> {
    Current::user_data_dir(&|key| std::env::var_os(key))
}

/// What differs per OS. One impl per OS, chosen by cfg into [`Current`].
trait Platform {
    /// Per-user data directory, given an environment lookup.
    fn user_data_dir(env: &dyn Fn(&str) -> Option<OsString>) -> Option<PathBuf>;

    /// Where the CEF binaries are, given the main executable's directory:
    /// next to it by default.
    fn framework_dir(exe_dir: &Path) -> PathBuf {
        exe_dir.to_path_buf()
    }

    /// The CEF subprocess executable, or `None` when CEF finds it itself:
    /// next to the main one by default.
    fn helper_path(exe_dir: &Path) -> Option<PathBuf> {
        Some(exe_dir.join(format!("wubble-helper{}", std::env::consts::EXE_SUFFIX)))
    }

    /// Chromium switches every wubble process gets on this OS, after
    /// [`CHROMIUM_SWITCHES`].
    const CHROMIUM_SWITCHES: &[(&str, Option<&str>)];
}

/// Chromium switches every wubble process gets on every OS. One entry per
/// switch, each with the reason. A switch given twice keeps one value, so a
/// platform table must not add its own `enable-features`: merge it here.
/// The `enable-features` switch is not here: [`PageFeatures::switches`] writes it, from the
/// user's settings.
const CHROMIUM_SWITCHES: &[(&str, Option<&str>)] = &[];

#[cfg_attr(not(any(target_os = "linux", test)), allow(dead_code))]
struct Linux;

impl Platform for Linux {
    /// No entries: CEF 156.0.30's defaults hand over real, tiled dmabufs on
    /// NVIDIA's proprietary driver (CEF #4237, #4313). `enable-features=Vulkan`,
    /// which CEF 154 needed there, makes every 156 frame empty, so it is gone.
    /// Each entry added here gets a comment saying which bug it works around.
    const CHROMIUM_SWITCHES: &[(&str, Option<&str>)] = &[];

    fn user_data_dir(env: &dyn Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
        xdg_data_dir(env)
    }
}

/// macOS runs from `wubble.app/Contents/MacOS` (built by `wubble-bundle`).
#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
struct MacOs;

impl Platform for MacOs {
    const CHROMIUM_SWITCHES: &[(&str, Option<&str>)] = &[];

    fn user_data_dir(env: &dyn Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
        mac_data_dir(env)
    }

    /// `Contents/Frameworks`, holding `Chromium Embedded Framework.framework`.
    fn framework_dir(exe_dir: &Path) -> PathBuf {
        exe_dir.join("../Frameworks")
    }

    /// Chromium derives the helper apps (`wubble Helper (GPU).app`, ...) in
    /// `Contents/Frameworks` from the main bundle's name, which is how
    /// `cef::build_util::mac::bundle` lays them out.
    fn helper_path(_exe_dir: &Path) -> Option<PathBuf> {
        None
    }
}

/// Windows: no windowless views yet (see `view.rs`), but CEF boots; CEF
/// and the helper sit next to the binary (the trait's defaults).
#[cfg_attr(not(windows), allow(dead_code))]
struct Windows;

impl Platform for Windows {
    const CHROMIUM_SWITCHES: &[(&str, Option<&str>)] = &[];

    fn user_data_dir(env: &dyn Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
        windows_data_dir(env)
    }
}

/// An environment variable holding an absolute path; relative values are
/// ignored, as the XDG Base Directory spec requires.
fn absolute_var(env: &dyn Fn(&str) -> Option<OsString>, key: &str) -> Option<PathBuf> {
    env(key).map(PathBuf::from).filter(|p| p.is_absolute())
}

/// Linux: `$XDG_DATA_HOME/wubble`, else `$HOME/.local/share/wubble`.
#[cfg_attr(not(any(target_os = "linux", test)), allow(dead_code))]
fn xdg_data_dir(env: &dyn Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    let data_home = absolute_var(env, "XDG_DATA_HOME")
        .or_else(|| absolute_var(env, "HOME").map(|home| home.join(".local/share")))?;
    Some(data_home.join("wubble"))
}

/// macOS: `$HOME/Library/Application Support/wubble`.
#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
fn mac_data_dir(env: &dyn Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    absolute_var(env, "HOME").map(|home| home.join("Library/Application Support/wubble"))
}

/// Windows: `%APPDATA%\wubble` (the roaming application data folder).
#[cfg_attr(not(any(windows, test)), allow(dead_code))]
fn windows_data_dir(env: &dyn Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    absolute_var(env, "APPDATA").map(|appdata| appdata.join("wubble"))
}

#[cfg(target_os = "linux")]
type Current = Linux;
#[cfg(target_os = "macos")]
type Current = MacOs;
#[cfg(windows)]
type Current = Windows;

/// The welding config for `config`; pure so the mapping is testable.
fn cef_config(config: &EngineConfig) -> CefRuntimeConfig {
    let mut cef = CefRuntimeConfig::new(Current::framework_dir(&config.exe_dir), SANDBOX);
    cef.browser_subprocess_path = Current::helper_path(&config.exe_dir);
    cef.cache_path = Some(config.cache_dir.clone());
    cef.command_line_switches = config.features.switches();
    cef.command_line_switches.extend(
        CHROMIUM_SWITCHES
            .iter()
            .chain(Current::CHROMIUM_SWITCHES)
            .map(|(name, value)| ((*name).into(), value.map(Into::into))),
    );
    cef.pump_scheduler = config.pump_scheduler.clone();
    cef
}

/// Extra Chromium switches for experiments, from `WUBBLE_PROBE_SWITCHES`
/// (`name=value;name;...`; unset is empty, so the shipped command line does not
/// change). They come last, so one overrides a table entry of the same name
/// (`enable-features` replaces the table's value, it does not extend it).
/// Used by the HDR probe (`examples/hdr_probe.rs`) to try colour-profile and
/// HDR feature switches without a rebuild.
fn probe_switches(spec: Option<&str>) -> Vec<(String, Option<String>)> {
    spec.unwrap_or_default()
        .split(';')
        .filter(|s| !s.is_empty())
        .map(|s| match s.split_once('=') {
            Some((name, value)) => (name.to_string(), Some(value.to_string())),
            None => (s.to_string(), None),
        })
        .collect()
}

/// The CEF version wubble is built against (the bundled libcef comes from the
/// same `cef-dll-sys` release).
pub fn version() -> &'static str {
    nul_terminated(cef::sys::CEF_VERSION)
}

/// CEF's own version and the Chromium version it wraps, from [`version`]
/// (`156.0.30+g6028375+chromium-156.0.8078.96`): for the About section.
pub fn cef_and_chromium() -> (&'static str, &'static str) {
    split_version(version())
}

fn split_version(full: &str) -> (&str, &str) {
    let cef = full.split('+').next().unwrap_or(full);
    let chromium = full.rsplit_once("chromium-").map_or("unknown", |(_, v)| v);
    (cef, chromium)
}

fn nul_terminated(bytes: &'static [u8]) -> &'static str {
    std::ffi::CStr::from_bytes_until_nul(bytes)
        .ok()
        .and_then(|c| c.to_str().ok())
        .unwrap_or("unknown")
}

/// Identifies one view owned by the [`Engine`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ViewId(usize);

/// What the host must do with its single-shot pump timer after a pump call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PumpTimer {
    /// (Re)arm it; call [`Engine::pump_timer_fired`] when it fires.
    Start(Duration),
    Cancel,
}

/// The result of one pump call.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PumpOutcome {
    /// The last timer request (a later one replaces an earlier one).
    pub timer: Option<PumpTimer>,
    /// CEF did work, so views may have new frames.
    pub worked: bool,
    /// What the views reported during that work, in order.
    pub events: Vec<(ViewId, WebEvent)>,
}

/// What the engine does with a page's requests (dialogs, menus,
/// permissions, file choosers, auth, certificate errors, download paths).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RequestMode {
    /// Answer each with its default at once (cancel, deny, decline) and do
    /// not report it: for a host that draws no prompts yet, so no page waits
    /// on one.
    #[default]
    DefaultNow,
    /// Report each as a [`PageEvent`] and wait for the host's answer (or the
    /// view closing, which answers the default).
    AskHost,
}

/// The running CEF process. Drive it only from the thread that started it.
pub struct Engine {
    /// Open views, each counted in `state` until it reports itself closed.
    /// Declared before `runtime` so views drop before CEF shuts down.
    views: Vec<(ViewId, View)>,
    next_view: usize,
    /// CPU fallback frames of views already dropped, so the total never shrinks.
    closed_cpu_fallback: u64,
    /// `Some` exactly while `state` is Running or ShuttingDown: an Engine only
    /// exists after CEF initialised, and `Effect::ShutdownCef` takes it.
    runtime: Option<CefRuntime>,
    state: State,
    pump: PumpState,
    requests: RequestMode,
    /// What the views reported beyond navigation, until the host takes it.
    page_events: PageQueue<ViewId>,
    content_filter: Option<Arc<dyn ContentFilter>>,
}

impl Engine {
    /// Initialises CEF. Call once per process, on the UI thread.
    pub fn start(config: EngineConfig) -> Result<Self, EngineError> {
        let mut cef = cef_config(&config);
        // Read here, not in `cef_config`, so that stays pure and its tests
        // do not depend on the caller's environment.
        cef.command_line_switches.extend(probe_switches(
            std::env::var("WUBBLE_PROBE_SWITCHES").ok().as_deref(),
        ));
        if let Some(helper) = cef
            .browser_subprocess_path
            .as_ref()
            .filter(|p| !p.is_file())
        {
            return Err(EngineError::HelperMissing {
                path: helper.clone(),
            });
        }
        let runtime = CefRuntime::initialize(cef).map_err(|e| EngineError::Init(e.to_string()))?;
        let mut engine = Self {
            views: Vec::new(),
            next_view: 0,
            closed_cpu_fallback: 0,
            runtime: Some(runtime),
            state: State::Uninitialised,
            pump: PumpState::default(),
            requests: RequestMode::default(),
            page_events: PageQueue::default(),
            content_filter: config.content_filter,
        };
        engine.apply(Event::Initialised);
        Ok(engine)
    }

    /// CEF asked (through [`EngineConfig::pump_scheduler`]) for work after
    /// `delay_ms`. The host should also call it once with 0 to start pumping.
    pub fn schedule_pump(&mut self, delay_ms: i64) -> PumpOutcome {
        self.pump(PumpEvent::Schedule(delay_ms))
    }

    /// The timer from the last [`PumpTimer::Start`] fired.
    pub fn pump_timer_fired(&mut self) -> PumpOutcome {
        self.pump(PumpEvent::TimerFired)
    }

    /// Runs CEF's pending work now, for callers without an event loop
    /// (`--smoke boot`, the shutdown drain). Timer requests are dropped: the
    /// caller simply ticks again.
    pub fn tick(&mut self) {
        self.apply(Event::Tick);
    }

    /// Starts an orderly shutdown; keep calling [`Self::tick`] until [`Self::is_stopped`].
    pub fn request_shutdown(&mut self) {
        self.apply(Event::ShutdownRequested);
    }

    pub fn is_stopped(&self) -> bool {
        self.state == State::Stopped
    }

    /// Creates a windowless browser. The engine owns it, closes it on
    /// shutdown and waits for CEF to drop it before shutting CEF down.
    pub fn open_view(&mut self, config: ViewConfig) -> Result<ViewId, EngineError> {
        let (State::Running { .. }, Some(runtime)) = (self.state, &self.runtime) else {
            return Err(EngineError::NotRunning);
        };
        let view = View::open(runtime, config, self.content_filter.clone())?;
        let id = ViewId(self.next_view);
        self.next_view += 1;
        self.views.push((id, view));
        self.apply(Event::BrowserOpened);
        Ok(id)
    }

    /// How page requests are answered from now on. Requests already
    /// reported keep waiting for the host.
    pub fn set_request_mode(&mut self, mode: RequestMode) {
        self.requests = mode;
    }

    /// The page events every view reported since the last call, in order.
    /// Bounded: the oldest droppable notices (console output, intermediate
    /// progress, find, IME, popup moves) are dropped when the host falls
    /// behind ([`Self::page_events_dropped`]); requests and events that
    /// start or end something never are.
    pub fn take_page_events(&mut self) -> Vec<(ViewId, PageEvent)> {
        self.page_events.take()
    }

    pub fn page_events_dropped(&self) -> u64 {
        self.page_events.dropped()
    }

    /// The view, until it has closed.
    pub fn view(&mut self, id: ViewId) -> Option<&mut View> {
        self.views
            .iter_mut()
            .find_map(|(view_id, view)| (*view_id == id).then_some(view))
    }

    /// Asks CEF to close one view. It stays counted (and reachable through
    /// [`Self::view`]) until CEF reports it gone on a later pump. Unknown or
    /// already closing views are ignored.
    pub fn close_view(&mut self, id: ViewId) {
        if let Some(view) = self.view(id) {
            view.close();
        }
    }

    /// The engine still holds the view (CEF has not reported it closed).
    pub fn has_view(&self, id: ViewId) -> bool {
        self.views.iter().any(|(view_id, _)| *view_id == id)
    }

    /// The view is open and CEF believes it visible.
    pub fn is_visible(&self, id: ViewId) -> bool {
        self.views
            .iter()
            .any(|(view_id, view)| *view_id == id && view.is_open() && view.is_visible())
    }

    /// Views CEF has not yet reported closed, closing ones included.
    pub fn view_count(&self) -> usize {
        self.views.len()
    }

    /// Open views CEF believes visible.
    pub fn visible_views(&self) -> usize {
        self.views
            .iter()
            .filter(|(_, view)| view.is_open() && view.is_visible())
            .count()
    }

    /// CPU `OnPaint` frames across every view so far, closed ones included;
    /// `None` where welding does not count them.
    pub fn cpu_fallback_frames(&self) -> Option<u64> {
        self.views
            .iter()
            .try_fold(self.closed_cpu_fallback, |sum, (_, view)| {
                view.cpu_fallback_frames().map(|n| sum + n)
            })
    }

    fn apply(&mut self, event: Event) {
        let (next, effects) = state::transition(self.state, event);
        self.state = next;
        for effect in effects {
            self.execute(effect);
        }
    }

    /// Feeds `event` to the pump scheduler and runs its effects: work runs
    /// here, each followed by its `WorkDone`; timer requests go to the host.
    /// Nothing happens unless CEF is up.
    fn pump(&mut self, event: PumpEvent) -> PumpOutcome {
        let mut outcome = PumpOutcome::default();
        if !self.state.pumps() {
            return outcome;
        }
        let mut next = Some(event);
        while let Some(event) = next.take() {
            let (state, effects) = pump::transition(self.pump, event);
            self.pump = state;
            for effect in effects {
                match effect {
                    PumpEffect::StartTimer(delay) => outcome.timer = Some(PumpTimer::Start(delay)),
                    PumpEffect::CancelTimer => outcome.timer = Some(PumpTimer::Cancel),
                    PumpEffect::RunWork => {
                        outcome.events.extend(self.run_work());
                        outcome.worked = true;
                        next = Some(PumpEvent::WorkDone);
                    }
                }
            }
        }
        outcome
    }

    /// One `do_message_loop_work`, then the views' callbacks it produced.
    fn run_work(&mut self) -> Vec<(ViewId, WebEvent)> {
        if let Some(runtime) = &self.runtime {
            runtime.do_message_loop_work();
        }
        self.reap_closed_views()
    }

    /// The only place lifecycle effects touch CEF.
    fn execute(&mut self, effect: Effect) {
        match effect {
            // Synchronous pumps (tick, shutdown) have no host to hear the
            // views' web events; they are dropped.
            Effect::PumpOnce => {
                self.pump(PumpEvent::Schedule(0));
            }
            // Each closing view reports Event::BrowserClosed from a later
            // pump (`reap_closed_views`); the host's drain loop ends on Stopped.
            Effect::CloseAllBrowsers => {
                for (_, view) in &mut self.views {
                    view.close();
                }
            }
            // Dropping the runtime calls cef::shutdown.
            Effect::ShutdownCef => self.runtime = None,
        }
    }

    /// Collects the views' events, drops views CEF reported closed and
    /// reports each one to the lifecycle.
    fn reap_closed_views(&mut self) -> Vec<(ViewId, WebEvent)> {
        let mut events = Vec::new();
        let before = self.views.len();
        let mut gone_cpu = 0;
        let (mode, page_events) = (self.requests, &mut self.page_events);
        self.views.retain_mut(|(id, view)| {
            let (new, page, closed) = view.drain_events(mode);
            events.extend(new.into_iter().map(|event| (*id, event)));
            for event in page {
                page_events.push(*id, event);
            }
            if closed {
                gone_cpu += view.cpu_fallback_frames().unwrap_or(0);
            }
            !closed
        });
        self.closed_cpu_fallback += gone_cpu;
        for _ in self.views.len()..before {
            self.apply(Event::BrowserClosed);
        }
        events
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(exe_dir: &str, cache_dir: &str) -> EngineConfig {
        EngineConfig {
            exe_dir: PathBuf::from(exe_dir),
            cache_dir: PathBuf::from(cache_dir),
            pump_scheduler: None,
            content_filter: None,
            features: PageFeatures::default(),
        }
    }

    #[test]
    fn cef_config_uses_the_current_platform_paths() {
        let exe_dir = Path::new("/opt/wubble");
        let cef = cef_config(&config("/opt/wubble", "/c"));
        assert_eq!(cef.cef_path, Current::framework_dir(exe_dir));
        assert_eq!(cef.browser_subprocess_path, Current::helper_path(exe_dir));
    }

    #[test]
    fn linux_keeps_cef_and_helper_next_to_the_binary() {
        let exe_dir = Path::new("/opt/wubble");
        assert_eq!(Linux::framework_dir(exe_dir), exe_dir);
        let helper = Linux::helper_path(exe_dir).expect("helper set");
        assert_eq!(helper.parent(), Some(exe_dir));
        assert!(
            helper
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("wubble-helper"))
        );
    }

    #[test]
    fn macos_finds_cef_in_the_bundles_frameworks() {
        let exe_dir = Path::new("/Applications/wubble.app/Contents/MacOS");
        let framework = MacOs::framework_dir(exe_dir);
        assert_eq!(
            framework,
            Path::new("/Applications/wubble.app/Contents/MacOS/../Frameworks")
        );
        assert_eq!(MacOs::helper_path(exe_dir), None);
    }

    /// Regression: without it CEF used the shared `~/.config/cef_user_data`
    /// singleton lock and concurrent wubble processes failed or crashed.
    #[test]
    fn cache_path_is_always_wubbles_own() {
        let cef = cef_config(&config("/opt/wubble", "/run/user/1/wubble-smoke-7"));
        assert_eq!(
            cef.cache_path.as_deref(),
            Some(Path::new("/run/user/1/wubble-smoke-7"))
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_gets_only_the_shared_switches() {
        let cef = cef_config(&config("/opt/wubble", "/c"));
        assert_eq!(
            cef.command_line_switches,
            vec![
                (
                    "enable-features".to_string(),
                    Some("OverlayScrollbar,MiddleClickAutoscroll".to_string())
                ),
                (
                    "enable-blink-features".to_string(),
                    Some("MiddleClickAutoscroll".to_string())
                ),
            ]
        );
    }

    #[test]
    fn the_version_splits_into_cef_and_chromium() {
        assert_eq!(
            split_version("156.0.30+g6028375+chromium-156.0.8078.96"),
            ("156.0.30", "156.0.8078.96")
        );
        assert_eq!(split_version("x"), ("x", "unknown"));
        assert_ne!(cef_and_chromium().1, "unknown");
    }

    #[test]
    fn page_features_follow_the_settings() {
        let off = PageFeatures {
            overlay_scrollbars: false,
            middle_click_autoscroll: false,
            ..PageFeatures::default()
        };
        assert!(off.switches().is_empty());
        let scrollbars = PageFeatures {
            overlay_scrollbars: true,
            ..off
        };
        assert_eq!(
            scrollbars.switches(),
            [(
                "enable-features".to_string(),
                Some("OverlayScrollbar".to_string())
            )]
        );
    }

    #[test]
    fn memory_and_process_rows_become_switches() {
        let tuned = PageFeatures {
            middle_click_autoscroll: false,
            back_forward_cache: BackForwardCache::Small,
            max_decoded_image_mb: 64,
            site_isolation: false,
            renderer_limit: 8,
            spare_renderer: false,
            ..PageFeatures::default()
        };
        let s = |name: &str, value: Option<&str>| (name.to_string(), value.map(str::to_string));
        assert_eq!(
            tuned.switches(),
            [
                s(
                    "enable-features",
                    Some("OverlayScrollbar,BackForwardCache:cache_size/2")
                ),
                s("disable-features", Some("SpareRendererForSitePerProcess")),
                s("max-decoded-image-size-mb", Some("64")),
                s("disable-site-isolation-trials", None),
                s("renderer-process-limit", Some("8")),
            ]
        );
        let off = PageFeatures {
            back_forward_cache: BackForwardCache::Off,
            overlay_scrollbars: false,
            middle_click_autoscroll: false,
            ..PageFeatures::default()
        };
        assert_eq!(off.switches(), [s("disable-back-forward-cache", None)]);
    }

    #[test]
    fn probe_switches_parse_name_value_pairs() {
        assert!(probe_switches(None).is_empty());
        assert_eq!(
            probe_switches(Some("force-color-profile=hdr10;;disable-gpu-vsync")),
            vec![
                ("force-color-profile".to_string(), Some("hdr10".to_string())),
                ("disable-gpu-vsync".to_string(), None),
            ]
        );
    }

    fn lookup(vars: &[(&str, &str)]) -> impl Fn(&str) -> Option<OsString> {
        let vars: Vec<(String, String)> = vars
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |key| {
            vars.iter()
                .find(|(k, _)| k == key)
                .map(|(_, v)| OsString::from(v))
        }
    }

    #[test]
    fn linux_data_dir_follows_xdg() {
        let dir = |vars: &[(&str, &str)]| xdg_data_dir(&lookup(vars));
        let home = ("HOME", "/home/u");
        assert_eq!(
            dir(&[("XDG_DATA_HOME", "/data"), home]),
            Some(PathBuf::from("/data/wubble"))
        );
        let fallback = Some(PathBuf::from("/home/u/.local/share/wubble"));
        assert_eq!(dir(&[home]), fallback);
        assert_eq!(dir(&[("XDG_DATA_HOME", "rel"), home]), fallback);
        assert_eq!(dir(&[("XDG_DATA_HOME", ""), home]), fallback);
        assert_eq!(dir(&[("HOME", "rel")]), None);
        assert_eq!(dir(&[]), None);
    }

    #[test]
    fn mac_and_windows_data_dirs() {
        assert_eq!(
            mac_data_dir(&lookup(&[("HOME", "/Users/u")])),
            Some(PathBuf::from("/Users/u/Library/Application Support/wubble"))
        );
        assert_eq!(mac_data_dir(&lookup(&[("HOME", "rel")])), None);
        assert_eq!(mac_data_dir(&lookup(&[])), None);
        // `Path::is_absolute` is per host OS, so test a path absolute on both.
        let appdata = if cfg!(windows) {
            r"C:\Users\u\AppData\Roaming"
        } else {
            "/appdata"
        };
        assert_eq!(
            windows_data_dir(&lookup(&[("APPDATA", appdata)])),
            Some(PathBuf::from(appdata).join("wubble"))
        );
        assert_eq!(windows_data_dir(&lookup(&[("APPDATA", "rel")])), None);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn the_cef_cache_keeps_its_old_place() {
        let vars = lookup(&[("HOME", "/home/u")]);
        assert_eq!(
            Linux::user_data_dir(&vars).map(|d| d.join(CEF_CACHE)),
            Some(PathBuf::from("/home/u/.local/share/wubble/cef"))
        );
    }

    #[test]
    fn pump_scheduler_reaches_welding() {
        assert!(
            cef_config(&config("/opt/wubble", "/c"))
                .pump_scheduler
                .is_none()
        );
        let with = EngineConfig {
            pump_scheduler: Some(PumpScheduler::new(|_| {})),
            ..config("/opt/wubble", "/c")
        };
        assert!(cef_config(&with).pump_scheduler.is_some());
    }

    #[test]
    fn helper_missing_says_how_to_fix_it() {
        let err = EngineError::HelperMissing {
            path: PathBuf::from("/x/wubble-helper"),
        };
        let text = err.to_string();
        assert!(text.contains("/x/wubble-helper"), "{text}");
        assert!(text.contains("cargo build --workspace"), "{text}");
    }

    /// macOS has no helper path to check: CEF finds the helper apps itself.
    #[cfg(not(target_os = "macos"))]
    #[test]
    fn start_without_helper_fails_before_touching_cef() {
        let dir = std::env::temp_dir().join("wubble-engine-test-no-helper");
        let err = Engine::start(EngineConfig {
            exe_dir: dir.clone(),
            cache_dir: dir,
            pump_scheduler: None,
            content_filter: None,
            features: PageFeatures::default(),
        })
        .err();
        assert!(matches!(err, Some(EngineError::HelperMissing { .. })));
    }

    #[test]
    fn version_strips_the_nul() {
        assert_eq!(nul_terminated(b"154.0.34+g1\0"), "154.0.34+g1");
        let v = version();
        assert!(!v.contains('\0') && v.starts_with(char::is_numeric), "{v}");
    }
}
