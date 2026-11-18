//! CEF initialisation, message-loop integration, process-wide switches.
//!
//! [`Engine`] owns the CEF runtime (via `welding::CefRuntime`, which sets
//! windowless rendering, the external message pump and `cef::shutdown` on drop).
//! The host loop calls [`Engine::tick`] repeatedly on the thread that started
//! the engine; every lifecycle change goes through [`state::transition`] and
//! its effects run in one place, `Engine::execute`.

pub mod state;

use std::ffi::OsString;
use std::fmt;
use std::path::{Path, PathBuf};

use state::{Effect, Event, State};
use welding::{CefRuntime, CefRuntimeConfig, CefSandboxMode};

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
}

impl EngineConfig {
    /// Config for the running executable's directory and the per-user cache.
    pub fn from_current_exe() -> Result<Self, EngineError> {
        let exe = std::env::current_exe().map_err(|e| EngineError::ExeDir(e.to_string()))?;
        let exe_dir = exe
            .parent()
            .ok_or_else(|| EngineError::ExeDir(format!("{} has no parent", exe.display())))?;
        let cache_dir =
            Current::user_cache_dir(&|key| std::env::var_os(key)).ok_or(EngineError::CacheDir)?;
        Ok(Self {
            exe_dir: exe_dir.to_path_buf(),
            cache_dir,
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

/// The CEF subprocess executable: next to the main binary on every platform
/// so far (macOS's helper `.app` bundle will change this).
fn helper_path(exe_dir: &Path) -> PathBuf {
    exe_dir.join(format!("wubble-helper{}", std::env::consts::EXE_SUFFIX))
}

/// What differs per OS. One impl per OS, chosen by cfg into [`Current`].
trait Platform {
    /// Per-user directory for CEF's cache, given an environment lookup.
    fn user_cache_dir(env: &dyn Fn(&str) -> Option<OsString>) -> Option<PathBuf>;

    /// Chromium switches every wubble process gets.
    const CHROMIUM_SWITCHES: &[(&str, Option<&str>)];
}

#[cfg(target_os = "linux")]
struct Linux;

#[cfg(target_os = "linux")]
impl Platform for Linux {
    /// Skia on Vulkan in the GPU process. Without it, on NVIDIA's proprietary
    /// driver (615, Wayland and X11 alike) Chromium's GL path cannot draw into
    /// the dmabuf shared images that windowless shared-texture mode exports
    /// ("Unable to initialize SkSurface" / "OzoneImageBacking::ProduceSkiaGanesh
    /// failed to create GL representation") and no accelerated paint arrives.
    const CHROMIUM_SWITCHES: &[(&str, Option<&str>)] = &[("enable-features", Some("Vulkan"))];

    /// `$XDG_DATA_HOME/wubble/cef`, else `$HOME/.local/share/wubble/cef`.
    /// Relative values are ignored, as the XDG Base Directory spec requires.
    fn user_cache_dir(env: &dyn Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
        let absolute = |key| env(key).map(PathBuf::from).filter(|p| p.is_absolute());
        let data_home = absolute("XDG_DATA_HOME")
            .or_else(|| absolute("HOME").map(|home| home.join(".local/share")))?;
        Some(data_home.join("wubble/cef"))
    }
}

/// Placeholder until macOS and Windows get their own impl.
#[cfg(not(target_os = "linux"))]
struct Unsupported;

#[cfg(not(target_os = "linux"))]
impl Platform for Unsupported {
    const CHROMIUM_SWITCHES: &[(&str, Option<&str>)] = &[];

    fn user_cache_dir(_env: &dyn Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
        None
    }
}

#[cfg(target_os = "linux")]
type Current = Linux;
#[cfg(not(target_os = "linux"))]
type Current = Unsupported;

/// The welding config for `config`; pure so the mapping is testable.
fn cef_config(config: &EngineConfig) -> CefRuntimeConfig {
    let mut cef = CefRuntimeConfig::new(config.exe_dir.clone(), SANDBOX);
    cef.browser_subprocess_path = Some(helper_path(&config.exe_dir));
    cef.cache_path = Some(config.cache_dir.clone());
    cef.command_line_switches = Current::CHROMIUM_SWITCHES
        .iter()
        .map(|(name, value)| ((*name).into(), value.map(Into::into)))
        .collect();
    cef
}

/// The CEF version wubble is built against (the bundled libcef comes from the
/// same `cef-dll-sys` release).
pub fn version() -> &'static str {
    nul_terminated(cef::sys::CEF_VERSION)
}

fn nul_terminated(bytes: &'static [u8]) -> &'static str {
    std::ffi::CStr::from_bytes_until_nul(bytes)
        .ok()
        .and_then(|c| c.to_str().ok())
        .unwrap_or("unknown")
}

/// Identifies one view owned by the [`Engine`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ViewId(usize);

/// The running CEF process. Drive it only from the thread that started it.
pub struct Engine {
    /// Open views, each counted in `state` until it reports itself closed.
    /// Declared before `runtime` so views drop before CEF shuts down.
    views: Vec<(ViewId, View)>,
    next_view: usize,
    /// `Some` exactly while `state` is Running or ShuttingDown: an Engine only
    /// exists after CEF initialised, and `Effect::ShutdownCef` takes it.
    runtime: Option<CefRuntime>,
    state: State,
}

impl Engine {
    /// Initialises CEF. Call once per process, on the UI thread.
    pub fn start(config: EngineConfig) -> Result<Self, EngineError> {
        let cef = cef_config(&config);
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
            runtime: Some(runtime),
            state: State::Uninitialised,
        };
        engine.apply(Event::Initialised);
        Ok(engine)
    }

    /// Lets CEF do its pending work. Call regularly from the host loop.
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
        let view = View::open(runtime, config)?;
        let id = ViewId(self.next_view);
        self.next_view += 1;
        self.views.push((id, view));
        self.apply(Event::BrowserOpened);
        Ok(id)
    }

    /// The view, until it has closed.
    pub fn view(&mut self, id: ViewId) -> Option<&mut View> {
        self.views
            .iter_mut()
            .find_map(|(view_id, view)| (*view_id == id).then_some(view))
    }

    fn apply(&mut self, event: Event) {
        let (next, effects) = state::transition(self.state, event);
        self.state = next;
        for effect in effects {
            self.execute(effect);
        }
    }

    /// The only place lifecycle effects touch CEF.
    fn execute(&mut self, effect: Effect) {
        match effect {
            // Re-entrancy cannot happen yet: `tick` takes `&mut self` and no
            // CEF callback reaches the Engine. Port CEF's guard
            // (main_message_loop_external_pump.cc) with the schedule-driven pump.
            Effect::PumpOnce => {
                if let Some(runtime) = &self.runtime {
                    runtime.do_message_loop_work();
                }
                self.reap_closed_views();
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

    /// Drops views CEF has let go of and reports each one to the lifecycle.
    fn reap_closed_views(&mut self) {
        let before = self.views.len();
        self.views.retain_mut(|(_, view)| !view.poll_closed());
        for _ in self.views.len()..before {
            self.apply(Event::BrowserClosed);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(exe_dir: &str, cache_dir: &str) -> EngineConfig {
        EngineConfig {
            exe_dir: PathBuf::from(exe_dir),
            cache_dir: PathBuf::from(cache_dir),
        }
    }

    #[test]
    fn helper_and_cef_dir_sit_next_to_the_binary() {
        let cef = cef_config(&config("/opt/wubble", "/c"));
        let helper = cef.browser_subprocess_path.expect("helper set");
        assert_eq!(helper.parent(), Some(Path::new("/opt/wubble")));
        assert!(
            helper
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("wubble-helper"))
        );
        assert_eq!(cef.cef_path, Path::new("/opt/wubble"));
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
    fn linux_enables_vulkan_for_shared_textures() {
        let cef = cef_config(&config("/opt/wubble", "/c"));
        assert_eq!(
            cef.command_line_switches,
            vec![("enable-features".to_string(), Some("Vulkan".to_string()))]
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_cache_dir_follows_xdg() {
        let dir = |vars: &[(&str, &str)]| {
            Linux::user_cache_dir(&|key| {
                vars.iter()
                    .find(|(k, _)| *k == key)
                    .map(|(_, v)| OsString::from(v))
            })
        };
        let home = ("HOME", "/home/u");
        assert_eq!(
            dir(&[("XDG_DATA_HOME", "/data"), home]),
            Some(PathBuf::from("/data/wubble/cef"))
        );
        let fallback = Some(PathBuf::from("/home/u/.local/share/wubble/cef"));
        assert_eq!(dir(&[home]), fallback);
        assert_eq!(dir(&[("XDG_DATA_HOME", "rel"), home]), fallback);
        assert_eq!(dir(&[("XDG_DATA_HOME", ""), home]), fallback);
        assert_eq!(dir(&[("HOME", "rel")]), None);
        assert_eq!(dir(&[]), None);
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

    #[test]
    fn start_without_helper_fails_before_touching_cef() {
        let dir = std::env::temp_dir().join("wubble-engine-test-no-helper");
        let err = Engine::start(EngineConfig {
            exe_dir: dir.clone(),
            cache_dir: dir,
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
