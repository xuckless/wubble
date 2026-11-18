//! CEF initialisation, message-loop integration, process-wide switches.
//!
//! [`Engine`] owns the CEF runtime (via `welding::CefRuntime`, which sets
//! windowless rendering, the external message pump and `cef::shutdown` on drop).
//! The host loop calls [`Engine::tick`] repeatedly on the thread that started
//! the engine; every lifecycle change goes through [`state::transition`] and
//! its effects run in one place, `Engine::execute`.

pub mod state;

use std::cell::Cell;
use std::fmt;
use std::path::{Path, PathBuf};

use state::{Effect, Event, State};
use welding::{CefRuntime, CefRuntimeConfig, CefSandboxMode};

/// Sandbox off on every platform for now (TODO.md, M1 "Sandbox enabled").
/// wubble-helper passes the same mode to its `execute_process`; keep them equal.
const SANDBOX: CefSandboxMode = CefSandboxMode::UnsandboxedTrustedContent;

/// What the engine needs to start.
#[derive(Clone, Debug)]
pub struct EngineConfig {
    /// Directory holding the main binary, `wubble-helper` and the CEF files.
    pub exe_dir: PathBuf,
}

impl EngineConfig {
    /// Config for the running executable's directory.
    pub fn from_current_exe() -> Result<Self, EngineError> {
        let exe = std::env::current_exe().map_err(|e| EngineError::ExeDir(e.to_string()))?;
        let exe_dir = exe
            .parent()
            .ok_or_else(|| EngineError::ExeDir(format!("{} has no parent", exe.display())))?;
        Ok(Self {
            exe_dir: exe_dir.to_path_buf(),
        })
    }
}

#[derive(Debug)]
pub enum EngineError {
    /// The running executable's directory could not be determined.
    ExeDir(String),
    /// The CEF subprocess executable is not next to the main binary.
    HelperMissing { path: PathBuf },
    /// CEF refused to initialise.
    Init(String),
}

impl fmt::Display for EngineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ExeDir(e) => write!(f, "cannot locate the wubble executable: {e}"),
            Self::HelperMissing { path } => write!(
                f,
                "CEF helper not found at {}; run `cargo build --workspace` first",
                path.display()
            ),
            Self::Init(e) => write!(f, "CEF initialisation failed: {e}"),
        }
    }
}

impl std::error::Error for EngineError {}

/// Per-platform file layout. One impl per OS, chosen by cfg into [`Current`].
trait Platform {
    /// Where the CEF subprocess executable lives.
    fn helper_path(exe_dir: &Path) -> PathBuf;
    /// The directory holding the CEF binary distribution.
    fn cef_dir(exe_dir: &Path) -> PathBuf;
}

/// cef-dll-sys copies libcef.so and its resources next to the binary; the
/// rpath in .cargo/config.toml makes the loader find it there.
#[cfg(target_os = "linux")]
struct Linux;

#[cfg(target_os = "linux")]
impl Platform for Linux {
    fn helper_path(exe_dir: &Path) -> PathBuf {
        exe_dir.join("wubble-helper")
    }
    fn cef_dir(exe_dir: &Path) -> PathBuf {
        exe_dir.to_path_buf()
    }
}

/// Placeholder until macOS (helper `.app` bundle) and Windows get their own impl.
#[cfg(not(target_os = "linux"))]
struct SameDir;

#[cfg(not(target_os = "linux"))]
impl Platform for SameDir {
    fn helper_path(exe_dir: &Path) -> PathBuf {
        exe_dir.join(format!("wubble-helper{}", std::env::consts::EXE_SUFFIX))
    }
    fn cef_dir(exe_dir: &Path) -> PathBuf {
        exe_dir.to_path_buf()
    }
}

#[cfg(target_os = "linux")]
type Current = Linux;
#[cfg(not(target_os = "linux"))]
type Current = SameDir;

/// Refuses nested message-loop work and remembers that it was asked for.
///
/// Ported from CEF `tests/shared/browser/main_message_loop_external_pump.cc`
/// (`MainMessageLoopExternalPump::PerformMessageLoopWork`, `is_active_` /
/// `reentrancy_detected_`), chromiumembedded/cef, BSD-3-Clause. CEF reschedules
/// the work; we loop once more instead.
#[derive(Default)]
struct PumpGuard {
    active: Cell<bool>,
    pending: Cell<bool>,
}

impl PumpGuard {
    /// `true` if the caller may pump; `false` (and marks pending) if a pump is running.
    fn enter(&self) -> bool {
        if self.active.get() {
            self.pending.set(true);
            return false;
        }
        self.active.set(true);
        true
    }

    /// Ends a pump; `true` if a nested pump was refused meanwhile.
    fn leave(&self) -> bool {
        self.active.set(false);
        self.pending.replace(false)
    }
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

/// The running CEF process. Drive it only from the thread that started it.
pub struct Engine {
    runtime: Option<CefRuntime>,
    state: State,
    pump: PumpGuard,
}

impl Engine {
    /// Initialises CEF. Call once per process, on the UI thread.
    pub fn start(config: EngineConfig) -> Result<Self, EngineError> {
        let helper = Current::helper_path(&config.exe_dir);
        if !helper.is_file() {
            return Err(EngineError::HelperMissing { path: helper });
        }
        let mut cef_config = CefRuntimeConfig::new(Current::cef_dir(&config.exe_dir), SANDBOX);
        cef_config.browser_subprocess_path = Some(helper);

        let mut engine = Self {
            runtime: None,
            state: State::Uninitialised,
            pump: PumpGuard::default(),
        };
        match CefRuntime::initialize(cef_config) {
            Ok(runtime) => {
                engine.runtime = Some(runtime);
                engine.apply(Event::Initialised);
                Ok(engine)
            }
            Err(e) => {
                engine.apply(Event::InitFailed);
                Err(EngineError::Init(e.to_string()))
            }
        }
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
            Effect::PumpOnce => self.pump_once(),
            // No browsers exist yet (M1 "View lifecycle" adds them); this is
            // where they get closed, each reporting Event::BrowserClosed.
            Effect::CloseAllBrowsers => {}
            // Dropping the runtime calls cef::shutdown.
            Effect::ShutdownCef => self.runtime = None,
        }
    }

    fn pump_once(&self) {
        let Some(runtime) = &self.runtime else { return };
        if !self.pump.enter() {
            return;
        }
        // A refused nested pump means CEF asked for more work: do it now.
        loop {
            runtime.do_message_loop_work();
            if !self.pump.leave() || !self.pump.enter() {
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nested_pump_is_refused_and_flagged() {
        let guard = PumpGuard::default();
        assert!(guard.enter());
        assert!(!guard.enter(), "nested enter refused");
        assert!(guard.leave(), "pending reported");
        assert!(guard.enter());
        assert!(!guard.leave(), "pending reported only once");
    }

    #[test]
    fn plain_pump_has_nothing_pending() {
        let guard = PumpGuard::default();
        assert!(guard.enter());
        assert!(!guard.leave());
    }

    #[test]
    fn helper_and_cef_dir_sit_next_to_the_binary() {
        let dir = Path::new("/opt/wubble");
        let helper = Current::helper_path(dir);
        assert_eq!(helper.parent(), Some(dir));
        assert!(
            helper
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("wubble-helper"))
        );
        assert_eq!(Current::cef_dir(dir), dir);
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
        let err = Engine::start(EngineConfig { exe_dir: dir }).err();
        assert!(matches!(err, Some(EngineError::HelperMissing { .. })));
    }

    #[test]
    fn version_strips_the_nul() {
        assert_eq!(nul_terminated(b"154.0.34+g1\0"), "154.0.34+g1");
        let v = version();
        assert!(!v.contains('\0') && v.starts_with(char::is_numeric), "{v}");
    }
}
