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

mod extra;
pub mod state;

use std::ffi::OsString;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use extra::ExtraBrowsers;
use state::{Effect, Event, State};
pub use welding::PumpScheduler;
use welding::{CefRuntime, CefRuntimeConfig, CefSandboxMode};
use wubble_core::pump::{self, PumpEffect, PumpEvent, PumpState};

use crate::ext_api::{ActionClick, ActionState, ExtApi, ExtRequest, ExtViewTab, TabOpened};
use crate::filter::{ContentFilter, ResponseObserver};
use crate::nav::WebEvent;
use crate::page::{PageEvent, PageQueue};
use crate::storage::{self, Profiles};
use crate::view::{View, ViewConfig};
use wubble_core::space::{ContainerId, Storage};

/// The debugging argument that turns the sandbox off: Chromium's own switch, so CEF
/// also passes it on to every subprocess, where wubble-helper applies the same rule,
/// welding's `CefSandboxMode::for_direct_entry` (TODO.md Decided, "CEF sandbox").
pub const NO_SANDBOX_ARG: &str = "--no-sandbox";

/// Whether `args` (a process's command line) has [`NO_SANDBOX_ARG`]: what
/// [`EngineConfig::from_current_exe`] reads, and what a process that runs wubble
/// again must pass on.
pub fn no_sandbox_requested<S: AsRef<std::ffi::OsStr>>(args: impl IntoIterator<Item = S>) -> bool {
    args.into_iter().any(|a| a.as_ref() == NO_SANDBOX_ARG)
}

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
    /// Sees the headers of every response each view gets ([`crate::filter::ResponseObserver`]).
    /// `None`: nothing is observed.
    pub response_observer: Option<Arc<dyn ResponseObserver>>,
    /// Chromium features chosen by the user's settings; CEF takes them only at start.
    pub features: PageFeatures,
    /// Runs the page clipboard through the host ([`CLIPBOARD_SWITCH`]): on for
    /// Wayland, where Chromium's own clipboard cannot reach the system's.
    pub clipboard_bridge: bool,
    /// Whose profile `cache_dir` is.
    pub profile: ProfileKind,
    /// Runs CEF without its sandbox ([`NO_SANDBOX_ARG`]): for debugging only, logged
    /// loudly at start, never a stored setting.
    pub no_sandbox: bool,
    /// Containers whose storage is brought up at start, before any window
    /// (`storage`): the session's. Others come up when a view first needs
    /// them.
    pub containers: Vec<ContainerId>,
    /// Unpacked extension directories Chromium loads at start
    /// (`wubble_core::extensions::load_paths`); empty keeps the extension system off
    /// ([`extension_switches`]). Read only here: a change applies at the next start.
    pub extensions: Vec<PathBuf>,
}

/// Whose profile CEF runs on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ProfileKind {
    /// The user's: CEF uses the OS's secret store as it is.
    #[default]
    User,
    /// A smoke's or measurement's, removed afterwards: nothing in it is worth an OS secret
    /// store entry, and asking for one can block start (macOS: a keychain prompt on the
    /// owner's screen for every rebuilt, ad hoc signed binary).
    Throwaway,
}

/// The renderer switch that makes wubble-helper run its clipboard content
/// script (`clipboard.js`, on the `clipboard` frame channel). Must match
/// `CLIPBOARD_SWITCH` in wubble-helper/src/main.rs, duplicated on purpose as
/// [`NO_SANDBOX_ARG`] is: the helper cannot depend on this crate.
pub const CLIPBOARD_SWITCH: &str = "wubble-clipboard-bridge";

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
    /// `settings::DEVTOOLS_REMOTE_PORT` through `settings::remote_port`: Chromium's remote
    /// debugging endpoint, on 127.0.0.1. `None`: no socket is opened.
    pub remote_debugging_port: Option<u16>,
    /// Hardware video decode through VA-API on NVIDIA (`VaapiOnNvidiaGPUs`):
    /// `settings::VIDEO_HARDWARE_DECODE` (off by default) and [`hardware_video_decode_found`].
    /// Chromium keeps VA-API off on NVIDIA by default (crbug.com/1492880). On, the GPU
    /// process runs without its seccomp sandbox: libva-nvidia-driver starts CUDA threads in
    /// Chromium's pre-sandbox initialisation, and the sandbox only starts on one thread.
    pub hw_video_decode: bool,
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
            remote_debugging_port: None,
            hw_video_decode: false,
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
            (
                self.remote_debugging_port.is_some(),
                Switch(
                    "remote-debugging-port",
                    self.remote_debugging_port.map(|p| p.to_string()),
                ),
                "DevTools endpoint on 127.0.0.1 only; any local program can drive every tab",
            ),
            (
                self.remote_debugging_port.is_some(),
                Switch(
                    "remote-allow-origins",
                    self.remote_debugging_port
                        .map(|p| format!("http://127.0.0.1:{p}")),
                ),
                "the docked frontend's WebSocket carries the endpoint's own origin (Chromium 111+ refuses others); never *",
            ),
            (
                self.hw_video_decode,
                Enable("VaapiOnNvidiaGPUs"),
                "NVDEC through libva-nvidia-driver (crbug.com/1492880 keeps it off by default): \
                 4K60 VP9 measured 130 to 290 % of a core in software, 25 to 55 % on NVDEC; the GPU \
                 process then has no seccomp sandbox (CUDA threads exist when it would start)",
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
        let disable: Vec<&str> = QUIET_FEATURES
            .iter()
            .map(|(feature, _)| *feature)
            .chain(list(|k| match k {
                Knob::Disable(f) => Some(f),
                _ => None,
            }))
            .collect();
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
    /// Config for the running executable's directory and the per-user cache; the sandbox
    /// is off only when this process's command line has `--no-sandbox`.
    pub fn from_current_exe() -> Result<Self, EngineError> {
        let exe_dir = current_exe_dir()?;
        let cache_dir = user_data_dir()
            .map(|dir| dir.join(CEF_CACHE))
            .ok_or(EngineError::CacheDir)?;
        Ok(Self {
            exe_dir,
            cache_dir,
            pump_scheduler: None,
            clipboard_bridge: false,
            content_filter: None,
            response_observer: None,
            features: PageFeatures::default(),
            profile: ProfileKind::User,
            no_sandbox: no_sandbox_requested(std::env::args_os()),
            containers: vec![],
            extensions: vec![],
        })
    }

    /// Config for an engine example (`cargo run -p wubble-engine --example ..`) on a throwaway
    /// profile with CEF's cache in `cache_dir`: CEF from the build's output directory, or on
    /// macOS from the bundle `wubble-bundle --exe` made of the example.
    pub fn for_example(cache_dir: PathBuf) -> Result<Self, EngineError> {
        let exe = std::env::current_exe().map_err(|e| EngineError::ExeDir(e.to_string()))?;
        let exe_dir = Current::example_exe_dir(&exe)
            .ok_or_else(|| EngineError::ExeDir(format!("{} has no build dir", exe.display())))?;
        Ok(Self {
            exe_dir: exe_dir.to_path_buf(),
            cache_dir,
            pump_scheduler: None,
            clipboard_bridge: false,
            content_filter: None,
            response_observer: None,
            features: PageFeatures::default(),
            profile: ProfileKind::Throwaway,
            no_sandbox: false,
            containers: vec![],
            extensions: vec![],
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

/// The running executable's directory.
fn current_exe_dir() -> Result<PathBuf, EngineError> {
    let exe = std::env::current_exe().map_err(|e| EngineError::ExeDir(e.to_string()))?;
    exe.parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| EngineError::ExeDir(format!("{} has no parent", exe.display())))
}

/// Makes CEF's plain C functions (its zip reader) callable before [`Engine::start`]: on macOS
/// the framework must be loaded first, from where the running executable's CEF is.
pub(crate) fn prepare_cef_api() -> Result<(), EngineError> {
    CefRuntime::prepare_api(&Current::framework_dir(&current_exe_dir()?))
        .map_err(|e| EngineError::Init(e.to_string()))
}

/// What differs per OS. One impl per OS, chosen by cfg into [`Current`].
pub(crate) trait Platform {
    /// Per-user data directory, given an environment lookup.
    fn user_data_dir(env: &dyn Fn(&str) -> Option<OsString>) -> Option<PathBuf>;

    /// Where the CEF binaries are, given the main executable's directory:
    /// next to it by default.
    fn framework_dir(exe_dir: &Path) -> PathBuf {
        exe_dir.to_path_buf()
    }

    /// The main executable's directory for an example binary `exe`: `target/<profile>/examples/x`
    /// runs with CEF and the helper of `target/<profile>` by default.
    fn example_exe_dir(exe: &Path) -> Option<&Path> {
        exe.parent()?.parent()
    }

    /// The CEF subprocess executable, or `None` when CEF finds it itself:
    /// next to the main one by default.
    fn helper_path(exe_dir: &Path) -> Option<PathBuf> {
        Some(exe_dir.join(format!("wubble-helper{}", std::env::consts::EXE_SUFFIX)))
    }

    /// Chromium switches every wubble process gets on this OS, after
    /// [`CHROMIUM_SWITCHES`].
    const CHROMIUM_SWITCHES: &[(&str, Option<&str>)];

    /// Chromium switches added on a [`ProfileKind::Throwaway`] profile.
    const THROWAWAY_SWITCHES: &[(&str, Option<&str>)] = &[];

    /// Idle `do_message_loop_work` turns per second: what turns the shutdown
    /// budgets of [`state`] (counts, not clocks) into time on this OS. About
    /// 4 us per idle turn on the owner's Linux machine.
    const TURNS_PER_SECOND: u32 = 250_000;

    /// Process-wide setup before anything else touches the OS's application
    /// object (winit, `cef::initialize`), while the process has one thread. Nothing by default.
    fn prepare_process() {}

    /// Whether this machine has a hardware video decoder Chromium uses only when asked
    /// ([`PageFeatures::hw_video_decode`]). None by default.
    fn hardware_video_decode_found() -> bool {
        false
    }

    /// The character a named key's CEF event carries when it types none (`windows_key_code`
    /// is a Windows `VK_*`): none by default.
    fn named_key_character(_windows_key_code: i32) -> Option<u16> {
        None
    }
}

/// Process-wide setup that must come first in `main`, before the window
/// system and CEF: on macOS, the CEF-aware application class; on Linux with the NVIDIA VA-API
/// driver, its environment ([`NVIDIA_VAAPI_ENV`]).
pub fn prepare_process() {
    Current::prepare_process()
}

/// Registers or clears the callback invoked when macOS AppKit receives
/// `-[NSApplication terminate:]` (e.g. `Cmd+Q`, menu bar Quit, or Dock Quit).
/// Intercepting `terminate:` avoids AppKit's default `exit(0)` and lets the host
/// exit the event loop and run orderly CEF shutdown. No-op on non-macOS platforms.
pub fn set_quit_handler(_handler: Option<fn()>) {
    #[cfg(target_os = "macos")]
    crate::mac_app::set_quit_handler(_handler);
}

/// Whether hardware video decode is there to be asked for (Linux: [`nvidia_vaapi`]). Read once
/// per process, so the environment `prepare_process` set and the switch agree.
pub fn hardware_video_decode_found() -> bool {
    static FOUND: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *FOUND.get_or_init(Current::hardware_video_decode_found)
}

/// What libva needs to pick libva-nvidia-driver (its README): every process inherits it, the
/// GPU process is the one that reads it. A value the user set already is kept.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
const NVIDIA_VAAPI_ENV: [(&str, &str); 2] =
    [("LIBVA_DRIVER_NAME", "nvidia"), ("NVD_BACKEND", "direct")];

/// Where libva looks for drivers when `LIBVA_DRIVERS_PATH` is unset: Fedora's, Debian's and
/// Arch's directories.
#[cfg_attr(not(any(target_os = "linux", test)), allow(dead_code))]
const LIBVA_DRIVER_DIRS: [&str; 3] = [
    "/usr/lib64/dri",
    "/usr/lib/x86_64-linux-gnu/dri",
    "/usr/lib/dri",
];

/// Whether VA-API decode on NVIDIA can work here (through `env` and `exists`, so the rule is
/// testable): the proprietary kernel driver is loaded (`/proc/driver/nvidia/version`) and
/// libva-nvidia-driver (`nvidia_drv_video.so`, the system package `libva-nvidia-driver` on
/// Fedora, `nvidia-vaapi-driver` on Debian and Arch) is in `LIBVA_DRIVERS_PATH` (a `:` list),
/// else in libva's default directories. Without the package Chromium 156 has no hardware decode
/// on NVIDIA at all: it has no Vulkan video decoder (TODO.md "Decided", video decode on Linux).
#[cfg_attr(not(any(target_os = "linux", test)), allow(dead_code))]
fn nvidia_vaapi(env: &dyn Fn(&str) -> Option<OsString>, exists: &dyn Fn(&Path) -> bool) -> bool {
    if !exists(Path::new("/proc/driver/nvidia/version")) {
        return false;
    }
    let dirs: Vec<PathBuf> = match env("LIBVA_DRIVERS_PATH").filter(|v| !v.is_empty()) {
        Some(list) => std::env::split_paths(&list).collect(),
        None => LIBVA_DRIVER_DIRS.iter().map(PathBuf::from).collect(),
    };
    dirs.iter().any(|d| exists(&d.join("nvidia_drv_video.so")))
}

/// Chromium switches every wubble process gets on every OS. One entry per
/// switch, each with the reason. A switch given twice keeps one value, so a
/// platform table must not add its own `enable-features`: merge it here.
/// The `enable-features` switch is not here: [`PageFeatures::switches`] writes it, from the
/// user's settings, nor the extension system's: [`extension_switches`] writes those, from the
/// installed list.
///
/// "Nothing phones home": a fresh profile with no page open asked Google for the component
/// updater (`update.googleapis.com`, the on-device model's manifest among them), a clock
/// (`clients2.google.com/time`), the spelling dictionary (`redirector.gvt1.com`), the omnibox's
/// AI Mode eligibility (`www.google.com/async/folae`) and a translate ranker model
/// (`www.gstatic.com`), measured with a net log (`docs/research/extensions.md` section 3). The first two
/// switches turn off what Chromium honours; the on-device model's manifest component is
/// registered by `chrome_browser_main_extra_parts_optimization_guide.cc` whatever they say, so
/// the infrastructure hosts Chromium updates itself from (no page of anyone's is there) are
/// also made unresolvable inside Chromium's own network stack: the request is still tried and
/// fails, nothing is looked up or sent.
const CHROMIUM_SWITCHES: &[(&str, Option<&str>)] = &[
    ("disable-component-update", None),
    ("disable-background-networking", None),
    (
        "host-resolver-rules",
        Some(
            "MAP update.googleapis.com ~NOTFOUND, MAP *.gvt1.com ~NOTFOUND, MAP *.gvt2.com ~NOTFOUND",
        ),
    ),
];

/// Chromium features switched off for the same reason ([`CHROMIUM_SWITCHES`]): each is a request
/// of Chromium's own, made with no page open, and each with what it asks.
const QUIET_FEATURES: &[(&str, &str)] = &[
    (
        "NetworkTimeServiceQuerying",
        "clients2.google.com/time/1/current: a clock query",
    ),
    (
        "AimEnabled",
        "www.google.com/async/folae: the omnibox's AI Mode eligibility",
    ),
    (
        "TranslateRankerQuery",
        "www.gstatic.com translate ranker model (with the next)",
    ),
    (
        "TranslateRankerEnforcement",
        "the same model, loaded when this one is on",
    ),
    (
        "AutofillServerCommunication",
        "content-autofill.googleapis.com: Chromium's own autofill query for the fields of any page \
         with a form (net log: resolved, TCP connected); wubble's autofill is its own",
    ),
];

/// The extension system's switches for the installed, enabled extensions `paths`.
///
/// None: `disable-extensions`. CEF 156's Chrome runtime otherwise starts the extension system
/// with two extension renderers and two helper processes of its own before any tab opens:
/// about 60 MiB PSS (`--measure memory`, Linux, 1 and 10 tabs). Chromium's PDF viewer, a
/// component extension, still opens (`--smoke builtins`), and DevTools still docks
/// (`--smoke page`).
///
/// Some: `load-extension` with the directories joined by `,` (Chromium splits on it, so the
/// core list refuses paths with one; `extension_service.cc`, unbranded builds only), and
/// `disable-component-extensions-with-background-pages`, which keeps Chromium's own background
/// component extensions (likely the two extension renderers of those 60 MiB; some may use
/// Google's component updater) from starting; the PDF viewer is not one of them (`component_loader.cc`). Extensions loaded
/// this way are never auto-updated (`Manifest::IsAutoUpdateableLocation`), so nothing asks an
/// update server (docs/research/extensions.md).
fn extension_switches(paths: &[PathBuf]) -> Vec<(String, Option<String>)> {
    if paths.is_empty() {
        return vec![("disable-extensions".into(), None)];
    }
    let list: Vec<String> = paths
        .iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    vec![
        ("load-extension".into(), Some(list.join(","))),
        (
            "disable-component-extensions-with-background-pages".into(),
            None,
        ),
        ("disable-default-apps".into(), None),
    ]
}

#[cfg_attr(not(any(target_os = "linux", test)), allow(dead_code))]
pub(crate) struct Linux;

impl Platform for Linux {
    /// No entries: CEF 156.0.30's defaults hand over real, tiled dmabufs on
    /// NVIDIA's proprietary driver (CEF #4237, #4313). `enable-features=Vulkan`,
    /// which CEF 154 needed there, makes every 156 frame empty, so it is gone.
    /// Each entry added here gets a comment saying which bug it works around.
    ///
    /// `disable-gpu-memory-buffer-video-frames`: with Chromium's GpuMemoryBuffer video
    /// frame pool, software-decoded 10-bit video (AV1 10-bit, VP9 profile 2) shows black:
    /// in the composited frame and in a canvas `drawImage` of the video, while the same
    /// clip is right with `--disable-gpu` (so decoding is fine, the pool's high-bit-depth
    /// copy is not). Without the pool the frames are uploaded as textures by the
    /// compositor's video resource updater (hdr_probe, CEF 156, NVIDIA; frame_smoke's
    /// 10-bit check). It applies to all video: 8-bit streams look the same but take that
    /// upload path too. Measured (b9, 4K60 VP9 in software, 3 alternating runs, total CPU of
    /// browser, GPU and renderer): 128 to 187 % of a core with it, 202 to 273 % without, so it
    /// is the cheaper setting (without it the renderer copies each frame into a pool buffer);
    /// at 1080p within noise; on the VA-API path it does not matter (the decoder hands over its
    /// own dmabufs). CEF 156 has no 10-bit-only knob to narrow it to.
    const CHROMIUM_SWITCHES: &[(&str, Option<&str>)] =
        &[("disable-gpu-memory-buffer-video-frames", None)];

    fn user_data_dir(env: &dyn Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
        xdg_data_dir(env)
    }

    fn hardware_video_decode_found() -> bool {
        nvidia_vaapi(&|key| std::env::var_os(key), &|path| path.exists())
    }

    /// With the NVIDIA VA-API driver, its environment (with `VaapiOnNvidiaGPUs`, which the
    /// same detection turns on).
    #[cfg(target_os = "linux")]
    fn prepare_process() {
        if !hardware_video_decode_found() {
            return;
        }
        for (key, value) in NVIDIA_VAAPI_ENV {
            if std::env::var_os(key).is_none() {
                // SAFETY: `prepare_process` runs first in `main`, before any other thread.
                unsafe { std::env::set_var(key, value) };
            }
        }
    }
}

/// macOS runs from `wubble.app/Contents/MacOS` (built by `wubble-bundle`).
#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
pub(crate) struct MacOs;

impl Platform for MacOs {
    const CHROMIUM_SWITCHES: &[(&str, Option<&str>)] = &[];

    /// Chromium's test keychain (`os_crypt`'s mock): a smoke run of a rebuilt binary stalled
    /// before its first frame behind a keychain access prompt for "Chromium Safe Storage".
    const THROWAWAY_SWITCHES: &[(&str, Option<&str>)] = &[("use-mock-keychain", None)];

    /// An idle turn blocks in `-[NSApplication run]` until the next frame
    /// (16.7 ms measured at 60 Hz), so a turn is a frame, not microseconds.
    const TURNS_PER_SECOND: u32 = 60;

    fn user_data_dir(env: &dyn Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
        mac_data_dir(env)
    }

    #[cfg(target_os = "macos")]
    fn prepare_process() {
        crate::mac_app::install();
    }

    /// What AppKit puts in a named key's `characters` (CEF builds an NSEvent from it).
    fn named_key_character(windows_key_code: i32) -> Option<u16> {
        crate::input::appkit_character(windows_key_code)
    }

    /// The example runs bundled as `wubble.app/Contents/MacOS/wubble` (`wubble-bundle --exe`).
    fn example_exe_dir(exe: &Path) -> Option<&Path> {
        exe.parent()
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
pub(crate) struct Windows;

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
pub(crate) type Current = Linux;
#[cfg(target_os = "macos")]
pub(crate) type Current = MacOs;
#[cfg(windows)]
pub(crate) type Current = Windows;

/// The welding config for `config`; pure so the mapping is testable.
fn cef_config(config: &EngineConfig) -> CefRuntimeConfig {
    let mut cef = CefRuntimeConfig::new(
        Current::framework_dir(&config.exe_dir),
        CefSandboxMode::for_direct_entry(config.no_sandbox),
    );
    cef.browser_subprocess_path = Current::helper_path(&config.exe_dir);
    cef.cache_path = Some(config.cache_dir.clone());
    cef.command_line_switches = config.features.switches();
    cef.command_line_switches
        .extend(extension_switches(&config.extensions));
    let throwaway = match config.profile {
        ProfileKind::User => &[][..],
        ProfileKind::Throwaway => Current::THROWAWAY_SWITCHES,
    };
    cef.command_line_switches.extend(
        CHROMIUM_SWITCHES
            .iter()
            .chain(Current::CHROMIUM_SWITCHES)
            .chain(throwaway)
            .map(|(name, value)| ((*name).into(), value.map(Into::into))),
    );
    cef.pump_scheduler = config.pump_scheduler.clone();
    if config.clipboard_bridge {
        cef.child_process_switches = vec![CLIPBOARD_SWITCH.into()];
    }
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

/// The line to print at start when a kernel knob turns unprivileged user namespaces off
/// (read through `read`, so the rule is testable); `None` when nothing says so. Without
/// them Chromium has no namespace sandbox, and its setuid fallback (`chrome-sandbox`) is
/// not installed setuid by CEF, so the zygote host stops the whole browser at start
/// ("No usable sandbox!"). A warning, not an error: on Ubuntu (23.10 and later,
/// `apparmor_restrict_unprivileged_userns` 1 by default) an AppArmor profile for wubble
/// that allows `userns` makes the knob a false alarm.
#[cfg(any(target_os = "linux", test))]
fn linux_sandbox_warning(read: &dyn Fn(&str) -> Option<String>) -> Option<String> {
    const KNOBS: [(&str, &str); 3] = [
        ("/proc/sys/user/max_user_namespaces", "0"),
        ("/proc/sys/kernel/unprivileged_userns_clone", "0"),
        (
            "/proc/sys/kernel/apparmor_restrict_unprivileged_userns",
            "1",
        ),
    ];
    let (path, off) = KNOBS
        .iter()
        .find(|(path, off)| read(path).is_some_and(|v| v.trim() == *off))?;
    Some(format!(
        "wubble: the Linux sandbox needs unprivileged user namespaces, but {path} is {off}: \
         unless an AppArmor profile allows them for wubble, Chromium will stop at start \
         (\"No usable sandbox!\"). Enable them, or run with {NO_SANDBOX_ARG} (debugging only)."
    ))
}

/// The CEF version wubble is built against (the bundled libcef comes from the
/// same `cef-dll-sys` release).
/// `view`'s browser id while it is open.
fn open_browser(views: &[(ViewId, View)], view: ViewId) -> Option<i32> {
    views
        .iter()
        .find(|(id, v)| *id == view && v.is_open())
        .map(|(_, v)| v.browser_id())
}

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

impl ViewId {
    #[cfg(test)]
    pub(crate) fn for_test(n: usize) -> Self {
        Self(n)
    }
}

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
    /// Every other browser CEF created (DevTools windows), counted in `state`
    /// like the views until each reports itself closed.
    extra: ExtraBrowsers,
    /// CPU fallback frames of views already dropped, so the total never shrinks.
    closed_cpu_fallback: u64,
    /// `Some` exactly while `state` pumps (Running, ShuttingDown, Settling): an Engine only
    /// exists after CEF initialised, and `Effect::ShutdownCef` takes it.
    runtime: Option<CefRuntime>,
    state: State,
    pump: PumpState,
    requests: RequestMode,
    /// What the views reported beyond navigation, until the host takes it.
    page_events: PageQueue<ViewId>,
    content_filter: Option<Arc<dyn ContentFilter>>,
    response_observer: Option<Arc<dyn ResponseObserver>>,
    /// One CEF request context per container in use (`storage`), released
    /// when the shutdown begins.
    profiles: Profiles,
    /// The extension tab APIs of a patched CEF (`ext_api`); inert on a stock one.
    ext: ExtApi,
}

impl Engine {
    /// Initialises CEF. Call once per process, on the UI thread.
    pub fn start(config: EngineConfig) -> Result<Self, EngineError> {
        if config.no_sandbox {
            eprintln!(
                "wubble: WARNING: the CEF sandbox is OFF ({NO_SANDBOX_ARG}). Pages run \
                 with this user's full rights. Use it for debugging only."
            );
        } else {
            #[cfg(target_os = "linux")]
            if let Some(warning) = linux_sandbox_warning(&|path| std::fs::read_to_string(path).ok())
            {
                eprintln!("{warning}");
            }
        }
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
        // Deleted containers' storage goes before CEF can open any of it.
        storage::sweep(&config.cache_dir);
        crate::dictionary::install(&config.cache_dir);
        let runtime = CefRuntime::initialize(cef).map_err(|e| EngineError::Init(e.to_string()))?;
        let mut engine = Self {
            views: Vec::new(),
            next_view: 0,
            extra: ExtraBrowsers::default(),
            closed_cpu_fallback: 0,
            runtime: Some(runtime),
            state: State::Uninitialised,
            pump: PumpState::default(),
            requests: RequestMode::default(),
            page_events: PageQueue::default(),
            content_filter: config.content_filter,
            response_observer: config.response_observer,
            profiles: Profiles::new(config.cache_dir),
            ext: ExtApi::find(),
        };
        engine.apply(Event::Initialised);
        // Outside any host callback: bringing a context up pumps CEF, which a
        // macOS host cannot do from inside one.
        if let Some(runtime) = &engine.runtime {
            for container in config.containers {
                // the saved containers: every one is on disk
                if let Err(e) = engine.profiles.get(runtime, container, Storage::Disk) {
                    eprintln!("wubble-engine: {e}");
                }
            }
        }
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

    /// The host took a page frame; `refresh` is the display's refresh period. While pages
    /// paint the pump waits at most one refresh between runs (`wubble_core::pump::Tempo`), from
    /// the next timer it arms.
    pub fn page_painted(&mut self, refresh: Duration) {
        // A state change only: `Painted` has no effects to run.
        self.pump = pump::transition(self.pump, PumpEvent::Painted(refresh)).0;
    }

    /// Runs CEF's pending work now, for callers without an event loop
    /// (`--smoke boot`, the shutdown drain). Timer requests are dropped: the
    /// caller simply ticks again.
    pub fn tick(&mut self) {
        self.apply(Event::Tick);
    }

    /// Starts an orderly shutdown; keep calling [`Self::tick`] until [`Self::is_stopped`].
    pub fn request_shutdown(&mut self) {
        // Extensions ask nothing more of the host; their pending tab openings fail.
        self.ext.release();
        // Open browsers keep their own context references until they close,
        // which the shutdown waits for; none outlives `cef::shutdown`.
        self.profiles.release();
        // Browsers CEF created since the last pump count before the quit.
        self.apply_extra();
        self.apply(Event::ShutdownRequested);
    }

    /// The shutdown ended, cleanly or abandoned ([`Self::abandoned`]).
    pub fn is_stopped(&self) -> bool {
        matches!(self.state, State::Stopped | State::Abandoned { .. })
    }

    /// The shutdown gave up on browsers that never reported closed and left
    /// CEF up rather than crash in `cef::shutdown` (`state::SHUTDOWN_TURNS`).
    pub fn abandoned(&self) -> bool {
        matches!(self.state, State::Abandoned { .. })
    }

    /// Browsers CEF created beside the views (DevTools windows) and has not
    /// reported closed. Counted after the next pump.
    pub fn extra_browsers(&self) -> usize {
        self.extra.open()
    }

    /// The extra browsers' page titles, in creation order. For smokes.
    pub fn extra_titles(&self) -> Vec<String> {
        self.extra.titles()
    }

    /// Runs `script` in every extra browser's main frame. For smokes.
    pub fn run_script_in_extras(&self, script: &str) {
        self.extra.run_script(script);
    }

    /// Creates a windowless browser. The engine owns it, closes it on
    /// shutdown and waits for CEF to drop it before shutting CEF down.
    pub fn open_view(&mut self, config: ViewConfig) -> Result<ViewId, EngineError> {
        let (State::Running { .. }, Some(runtime)) = (self.state, &self.runtime) else {
            return Err(EngineError::NotRunning);
        };
        let profile = self
            .profiles
            .get(runtime, config.container, config.storage)?;
        let view = View::open(
            runtime,
            config,
            profile,
            self.content_filter.clone(),
            self.response_observer.clone(),
            self.extra.client(),
        )?;
        let id = ViewId(self.next_view);
        self.next_view += 1;
        self.views.push((id, view));
        self.apply(Event::BrowserOpened);
        Ok(id)
    }

    /// Deletes the cookies, HTTP cache and HTTP auth credentials of
    /// `containers` now (the host passes one, or every one it has), bringing
    /// up any not open yet. Site storage (local storage, IndexedDB, service
    /// workers) stays: [`View::forget_site`] clears one site's, deleting the
    /// container clears all of it.
    pub fn clear_browsing_data(
        &mut self,
        containers: &[(ContainerId, Storage)],
    ) -> Result<(), EngineError> {
        let (State::Running { .. }, Some(runtime)) = (self.state, &self.runtime) else {
            return Err(EngineError::NotRunning);
        };
        for &(container, kept) in containers {
            let context = self.profiles.get(runtime, container, kept)?.request_context();
            storage::clear(&context, container);
        }
        Ok(())
    }

    /// Copies the cookies of container `from` that `filter` takes into container `to`,
    /// bringing either up if it is not open (the source is only read: its own tabs keep
    /// theirs). `done` gets what was copied, once, on a CEF thread, when every cookie was
    /// visited and set (`storage::copy_cookies`); on an error it never runs. The handle stops
    /// the copy (a session stopped meanwhile).
    pub fn copy_cookies(
        &mut self,
        from: ContainerId,
        to: ContainerId,
        filter: storage::CookieFilter,
        done: storage::CopyDone,
    ) -> Result<storage::CopyCancel, EngineError> {
        let (State::Running { .. }, Some(runtime)) = (self.state, &self.runtime) else {
            return Err(EngineError::NotRunning);
        };
        // agent sign-ins: both are saved containers (a private one is never a source)
        let source = self
            .profiles
            .get(runtime, from, Storage::Disk)?
            .request_context();
        let target = self.profiles.get(runtime, to, Storage::Disk)?.request_context();
        storage::copy_cookies(&source, &target, filter, done).ok_or_else(|| {
            EngineError::View(format!(
                "container {}: CEF refused to read its cookies",
                from.0
            ))
        })
    }

    /// `container` was deleted (its views are closing): its cookies go now,
    /// the rest of a [`Storage::Disk`] one's storage at the next start, before
    /// CEF opens it (its files may still be open now). A [`Storage::Memory`]
    /// one has nothing on disk: CEF drops its profile with its last browser.
    pub fn forget_container(&mut self, container: ContainerId, kept: Storage) {
        if let Some(profile) = self.profiles.open(container) {
            storage::clear(&profile.request_context(), container);
        }
        self.profiles.close(container);
        if kept == Storage::Disk {
            storage::forget(self.profiles.root(), container);
        }
    }

    /// CEF's cache directory (the profile's, or the per-process smoke one).
    pub fn cache_dir(&self) -> &Path {
        self.profiles.root()
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

    /// This CEF carries wubble's patches (`patches/chromium`): extensions see wubble's tabs and
    /// windows through [`Self::set_ext_projection`], ask through [`Self::take_ext_requests`].
    pub fn ext_apis(&self) -> bool {
        self.ext.available()
    }

    /// This CEF applies a touchpad pinch (`patches/chromium/40-cef-touchpad-pinch.patch`, its own
    /// marker, apart from [`Self::ext_apis`]): views pinch through DevTools' touchpad gesture,
    /// else with two synthetic fingers (`pinch.rs`).
    pub fn touchpad_pinch(&self) -> bool {
        crate::pinch::touchpad_available()
    }

    /// wubble's windows and tabs as extensions are to see them (a projection of the host's
    /// model, `wubble_core::extensions::project`, its tabs by view). Kept, and sent to CEF now
    /// and whenever a view's browser comes or goes; nothing is sent while it stays the same.
    pub fn set_ext_projection(
        &mut self,
        windows: Vec<wubble_core::extensions::ExtWindow>,
        tabs: Vec<ExtViewTab>,
    ) {
        self.ext.set_projection(windows, tabs);
        self.sync_ext();
    }

    /// What extensions asked of the host's model since the last call.
    pub fn take_ext_requests(&mut self) -> Vec<ExtRequest> {
        let views = &self.views;
        let view_of = |browser: i32| {
            views
                .iter()
                .find(|(_, v)| v.browser_id() == browser)
                .map(|(id, _)| *id)
        };
        self.ext.take(view_of, self.profiles.root())
    }

    /// Answers [`ExtRequest::OpenTab`] `request` with what the host did. The projection is
    /// sent first, so the tab extensions get back has its place.
    pub fn answer_open_tab(&mut self, request: i32, answer: TabOpened) {
        self.sync_ext();
        let browser = match answer {
            TabOpened::View(v) => open_browser(&self.views, v).unwrap_or(0),
            TabOpened::Page => -1,
            TabOpened::Refused => 0,
        };
        self.ext.tab_opened(request, browser);
    }

    /// Clicks extension `extension`'s action on `view`'s page.
    pub fn action_click(&self, extension: &str, view: ViewId) -> ActionClick {
        match open_browser(&self.views, view) {
            Some(browser) => self.ext.action_click(extension, browser),
            None => ActionClick::None,
        }
    }

    /// Extension `extension`'s action as `view`'s page sees it; `None` without one.
    pub fn action_state(&self, extension: &str, view: ViewId) -> Option<ActionState> {
        self.ext
            .action_state(extension, open_browser(&self.views, view)?)
    }

    fn sync_ext(&mut self) {
        let views = &self.views;
        self.ext.sync(|view| open_browser(views, view));
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
                        let (events, slice_spent) = self.run_work();
                        outcome.events.extend(events);
                        outcome.worked = true;
                        next = Some(PumpEvent::WorkDone { slice_spent });
                    }
                }
            }
        }
        outcome
    }

    /// One `do_message_loop_work`, then the callbacks it produced: extra
    /// browsers' first, so one created in this run counts before a view's
    /// close could end a shutdown. Also whether CEF worked for its whole time slice
    /// (`pump::CEF_TIME_SLICE`).
    fn run_work(&mut self) -> (Vec<(ViewId, WebEvent)>, bool) {
        let at = Instant::now();
        if let Some(runtime) = &self.runtime {
            runtime.do_message_loop_work();
        }
        let slice_spent = at.elapsed() >= pump::CEF_TIME_SLICE;
        self.apply_extra();
        let events = self.reap_closed_views();
        self.sync_ext();
        (events, slice_spent)
    }

    /// Reports the extra browsers created or closed to the lifecycle.
    fn apply_extra(&mut self) {
        for event in self.extra.drain() {
            self.apply(event);
        }
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
                self.extra.close_all();
            }
            // Dropping the runtime calls cef::shutdown.
            Effect::ShutdownCef => self.runtime = None,
            // cef::shutdown would crash on the browsers still alive; the
            // process exits with CEF up instead (its subprocesses follow).
            Effect::AbandonCef => {
                eprintln!(
                    "wubble-engine: {:?} after {} pump turns: browsers never reported closed; \
                     exiting without cef::shutdown",
                    self.state,
                    state::SHUTDOWN_TURNS
                );
                std::mem::forget(self.runtime.take());
            }
        }
    }

    /// Collects the views' events, drops views CEF reported closed and
    /// reports each one to the lifecycle.
    fn reap_closed_views(&mut self) -> Vec<(ViewId, WebEvent)> {
        let mut events = Vec::new();
        let before = self.views.len();
        let mut gone_cpu = 0;
        let mut opened = Vec::new();
        let (mode, page_events) = (self.requests, &mut self.page_events);
        self.views.retain_mut(|(id, view)| {
            let (new, page, closed) = view.drain_events(mode);
            events.extend(new.into_iter().map(|event| (*id, event)));
            opened.extend(view.take_opened().into_iter().map(|(v, r)| (*id, v, r)));
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
        // pop-up browsers CEF created: views of their own, counted like the rest
        for (opener, view, request) in opened {
            let id = ViewId(self.next_view);
            self.next_view += 1;
            self.views.push((id, view));
            self.apply(Event::BrowserOpened);
            events.push((opener, WebEvent::PopupOpened { request, view: id }));
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
            response_observer: None,
            features: PageFeatures::default(),
            clipboard_bridge: false,
            profile: ProfileKind::User,
            no_sandbox: false,
            containers: vec![],
            extensions: vec![],
        }
    }

    #[test]
    fn extensions_are_off_until_one_is_installed() {
        assert_eq!(
            extension_switches(&[]),
            [("disable-extensions".to_string(), None)]
        );
        let two = [PathBuf::from("/p/a"), PathBuf::from("/home/x/my ext")];
        assert_eq!(
            extension_switches(&two),
            [
                (
                    "load-extension".to_string(),
                    Some("/p/a,/home/x/my ext".to_string())
                ),
                (
                    "disable-component-extensions-with-background-pages".to_string(),
                    None
                ),
                ("disable-default-apps".to_string(), None),
            ]
        );
        let cef = cef_config(&EngineConfig {
            extensions: two.to_vec(),
            ..config("/opt/wubble", "/c")
        });
        let names: Vec<&str> = cef
            .command_line_switches
            .iter()
            .map(|(n, _)| n.as_str())
            .collect();
        assert!(names.contains(&"load-extension"));
        assert!(!names.contains(&"disable-extensions"));
        let off = cef_config(&config("/opt/wubble", "/c"));
        assert!(
            off.command_line_switches
                .iter()
                .any(|(n, _)| n == "disable-extensions")
        );
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
    fn linux_gets_the_shared_switches_and_its_video_fix() {
        let cef = cef_config(&config("/opt/wubble", "/c"));
        let mut want = vec![
            (
                "enable-features".to_string(),
                Some("OverlayScrollbar,MiddleClickAutoscroll".to_string()),
            ),
            quiet_features(&[]),
            (
                "enable-blink-features".to_string(),
                Some("MiddleClickAutoscroll".to_string()),
            ),
            ("disable-extensions".to_string(), None),
        ];
        want.extend(quiet_switches());
        want.push(("disable-gpu-memory-buffer-video-frames".to_string(), None));
        assert_eq!(cef.command_line_switches, want);
    }

    /// The `disable-features` every run carries (`QUIET_FEATURES`), then `more`.
    fn quiet_features(more: &[&str]) -> (String, Option<String>) {
        let all: Vec<&str> = QUIET_FEATURES
            .iter()
            .map(|(f, _)| *f)
            .chain(more.iter().copied())
            .collect();
        ("disable-features".to_string(), Some(all.join(",")))
    }

    /// The plain switches every run carries (`CHROMIUM_SWITCHES`).
    #[cfg(target_os = "linux")]
    fn quiet_switches() -> Vec<(String, Option<String>)> {
        CHROMIUM_SWITCHES
            .iter()
            .map(|(n, v)| ((*n).to_string(), v.map(str::to_string)))
            .collect()
    }

    /// "Nothing phones home": what a fresh profile asked Google for with no page open is off by
    /// a switch or a feature on every command line, and the hosts Chromium updates itself from
    /// cannot be resolved (the component updater registers itself whatever the switches say).
    #[test]
    fn chromium_s_own_requests_are_switched_off() {
        let cef = cef_config(&config("/opt/wubble", "/c"));
        let has = |name: &str| {
            cef.command_line_switches
                .iter()
                .find(|(n, _)| n == name)
                .map(|(_, v)| v.clone())
        };
        assert_eq!(has("disable-component-update"), Some(None));
        assert_eq!(has("disable-background-networking"), Some(None));
        let rules = has("host-resolver-rules").flatten().unwrap();
        for host in ["update.googleapis.com", "*.gvt1.com", "*.gvt2.com"] {
            assert!(rules.contains(&format!("MAP {host} ~NOTFOUND")), "{rules}");
        }
        let features = has("disable-features").flatten().unwrap();
        for feature in [
            "NetworkTimeServiceQuerying",
            "AimEnabled",
            "TranslateRankerQuery",
            "TranslateRankerEnforcement",
            "AutofillServerCommunication",
        ] {
            assert!(features.split(',').any(|f| f == feature), "{features}");
        }
        // the user's own page features never undo it
        let spare_off = PageFeatures {
            spare_renderer: false,
            ..PageFeatures::default()
        };
        let merged = spare_off.switches();
        let off = merged
            .iter()
            .find(|(n, _)| n == "disable-features")
            .unwrap();
        assert!(
            off.1
                .as_deref()
                .unwrap()
                .contains("NetworkTimeServiceQuerying")
        );
        assert!(
            off.1
                .as_deref()
                .unwrap()
                .ends_with("SpareRendererForSitePerProcess")
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
        assert_eq!(off.switches(), [quiet_features(&[])]);
        let scrollbars = PageFeatures {
            overlay_scrollbars: true,
            ..off
        };
        assert_eq!(
            scrollbars.switches(),
            [
                (
                    "enable-features".to_string(),
                    Some("OverlayScrollbar".to_string())
                ),
                quiet_features(&[]),
            ]
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
                quiet_features(&["SpareRendererForSitePerProcess"]),
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
        assert_eq!(
            off.switches(),
            [quiet_features(&[]), s("disable-back-forward-cache", None)]
        );
    }

    #[test]
    fn the_debugging_endpoint_is_a_switch_only_with_a_port() {
        let s = |name: &str, value: Option<&str>| (name.to_string(), value.map(str::to_string));
        let base = PageFeatures {
            overlay_scrollbars: false,
            middle_click_autoscroll: false,
            ..PageFeatures::default()
        };
        assert_eq!(PageFeatures::default().remote_debugging_port, None);
        assert_eq!(base.switches(), [quiet_features(&[])]);
        let open = PageFeatures {
            remote_debugging_port: Some(9333),
            ..base
        };
        assert_eq!(
            open.switches(),
            [
                quiet_features(&[]),
                s("remote-debugging-port", Some("9333")),
                s("remote-allow-origins", Some("http://127.0.0.1:9333"))
            ]
        );
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

    // Unix paths: `Path::is_absolute` and the `:` list separator are the host OS's.
    #[cfg(unix)]
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
        // A Unix path is not absolute on Windows.
        #[cfg(unix)]
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
    fn the_sandbox_is_on_unless_asked_off() {
        let off = EngineConfig {
            no_sandbox: true,
            ..config("/opt/wubble", "/c")
        };
        assert_eq!(
            cef_config(&off).sandbox,
            CefSandboxMode::UnsandboxedTrustedContent
        );
        assert_eq!(
            cef_config(&config("/opt/wubble", "/c")).sandbox,
            CefSandboxMode::for_direct_entry(false)
        );
        assert_eq!(
            NO_SANDBOX_ARG.strip_prefix("--"),
            Some(CefSandboxMode::NO_SANDBOX_SWITCH)
        );
        assert!(no_sandbox_requested([
            "wubble",
            "--smoke",
            "session",
            "--no-sandbox"
        ]));
        assert!(!no_sandbox_requested([
            "wubble",
            "--no-sandboxes",
            "no-sandbox"
        ]));
    }

    #[test]
    fn a_kernel_without_unprivileged_user_namespaces_is_named() {
        let knobs = |pairs: &'static [(&'static str, &'static str)]| {
            move |path: &str| {
                pairs
                    .iter()
                    .find(|(p, _)| *p == path)
                    .map(|(_, v)| format!("{v}\n"))
            }
        };
        assert_eq!(linux_sandbox_warning(&knobs(&[])), None);
        assert_eq!(
            linux_sandbox_warning(&knobs(&[("/proc/sys/user/max_user_namespaces", "126785")])),
            None
        );
        // Ubuntu's default: the line says the browser stops, not just its pages, and how
        // to get past it.
        let ubuntu = linux_sandbox_warning(&knobs(&[(
            "/proc/sys/kernel/apparmor_restrict_unprivileged_userns",
            "1",
        )]))
        .expect("Ubuntu's knob is named");
        assert!(
            ubuntu.contains("apparmor_restrict_unprivileged_userns is 1"),
            "{ubuntu}"
        );
        assert!(ubuntu.contains("Chromium will stop at start"), "{ubuntu}");
        assert!(ubuntu.contains(NO_SANDBOX_ARG), "{ubuntu}");
        assert!(
            linux_sandbox_warning(&knobs(&[("/proc/sys/user/max_user_namespaces", "0")])).is_some()
        );
    }

    // Unix paths: `Path::is_absolute` and the `:` list separator are the host OS's.
    #[cfg(unix)]
    #[test]
    fn the_nvidia_vaapi_driver_is_found_where_libva_looks() {
        let nvidia = "/proc/driver/nvidia/version";
        let fedora = "/usr/lib64/dri/nvidia_drv_video.so";
        let check = |files: &[&str], drivers_path: Option<&str>| {
            let env = |key: &str| {
                (key == "LIBVA_DRIVERS_PATH")
                    .then_some(drivers_path)
                    .flatten()
                    .map(OsString::from)
            };
            nvidia_vaapi(&env, &|p: &Path| files.iter().any(|f| Path::new(f) == p))
        };
        let cases: &[(&[&str], Option<&str>, bool)] = &[
            (&[nvidia, fedora], None, true),
            (
                &[nvidia, "/usr/lib/x86_64-linux-gnu/dri/nvidia_drv_video.so"],
                None,
                true,
            ),
            (&[nvidia, "/usr/lib/dri/nvidia_drv_video.so"], None, true),
            // the package is missing: no hardware decode on NVIDIA in Chromium 156
            (&[nvidia], None, false),
            // the driver without the NVIDIA kernel driver (Mesa machine, nouveau)
            (&[fedora], None, false),
            // `LIBVA_DRIVERS_PATH` replaces the default directories
            (
                &[nvidia, "/opt/va/nvidia_drv_video.so"],
                Some("/x:/opt/va"),
                true,
            ),
            (&[nvidia, fedora], Some("/opt/va"), false),
            // set but empty: the defaults
            (&[nvidia, fedora], Some(""), true),
        ];
        for (files, path, found) in cases {
            assert_eq!(check(files, *path), *found, "{files:?} {path:?}");
        }
        assert!(NVIDIA_VAAPI_ENV.contains(&("LIBVA_DRIVER_NAME", "nvidia")));
    }

    #[test]
    fn hardware_decode_joins_the_one_enable_features() {
        let s = |name: &str, value: Option<&str>| (name.to_string(), value.map(str::to_string));
        let on = PageFeatures {
            hw_video_decode: true,
            ..PageFeatures::default()
        };
        assert_eq!(
            on.switches(),
            [
                s(
                    "enable-features",
                    Some("OverlayScrollbar,MiddleClickAutoscroll,VaapiOnNvidiaGPUs")
                ),
                quiet_features(&[]),
                s("enable-blink-features", Some("MiddleClickAutoscroll")),
            ]
        );
        assert!(!PageFeatures::default().hw_video_decode);
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
            response_observer: None,
            features: PageFeatures::default(),
            clipboard_bridge: false,
            containers: vec![],
            extensions: vec![],
            profile: ProfileKind::Throwaway,
            no_sandbox: false,
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
