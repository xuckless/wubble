//! The OS edge of `wubble_core::scope`: moves the browser into a CPU-weighted scope of its own
//! before CEF starts. One trait, one implementation per platform; only Linux has one (the
//! user's systemd over D-Bus). macOS and Windows keep the default: nothing.

use std::time::Duration;

use wubble_core::scope::{self, ScopeContext, ScopeRequest};

/// How long start may wait for the user's systemd: a missing or hung bus must not delay the
/// window.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
const TIMEOUT: Duration = Duration::from_millis(500);

/// Where the process can be given its own CPU share.
trait ScopeHost {
    /// The process's cgroup text, when the OS has one.
    fn cgroup() -> Option<String> {
        None
    }
    /// Starts `request`'s scope with this process in it, returning once the process has moved
    /// (or the time is up).
    fn start(_request: &ScopeRequest) -> Result<(), String> {
        Ok(())
    }
}

/// Call first in `main`, before any thread or child process exists, for the user's browser
/// (`browsing`); smokes and measurements keep their caller's limits. A failure is one line.
pub fn own_scope(browsing: bool) {
    let cgroup = Current::cgroup();
    let Some(request) = scope::plan(ScopeContext {
        pid: std::process::id(),
        browsing,
        no_scope_env: std::env::var_os(scope::NO_SCOPE_ENV).is_some(),
        cgroup: cgroup.as_deref(),
    }) else {
        return;
    };
    if let Err(e) = Current::start(&request) {
        eprintln!(
            "wubble: no scope of its own ({e}); under load other work shares the CPU equally"
        );
    }
}

#[cfg(target_os = "linux")]
type Current = linux::Systemd;
#[cfg(not(target_os = "linux"))]
type Current = NoScope;

#[cfg(not(target_os = "linux"))]
struct NoScope;

#[cfg(not(target_os = "linux"))]
impl ScopeHost for NoScope {}

#[cfg(target_os = "linux")]
mod linux {
    use wubble_core::scope::{self, ScopeRequest};
    use zbus::zvariant::Value;

    /// The user's systemd manager (`org.freedesktop.systemd1.Manager.StartTransientUnit`, the
    /// call `systemd-run --user --scope` makes).
    pub struct Systemd;

    impl super::ScopeHost for Systemd {
        fn cgroup() -> Option<String> {
            std::fs::read_to_string("/proc/self/cgroup").ok()
        }

        fn start(request: &ScopeRequest) -> Result<(), String> {
            let bus = zbus::blocking::connection::Builder::session()
                .map_err(|e| e.to_string())?
                .method_timeout(super::TIMEOUT)
                .build()
                .map_err(|e| e.to_string())?;
            let properties: Vec<(&str, Value)> = vec![
                ("PIDs", Value::from(vec![request.pid])),
                ("CPUWeight", Value::U64(request.cpu_weight)),
                ("Description", Value::from(request.description)),
            ];
            let aux: Vec<(&str, Vec<(&str, Value)>)> = vec![];
            bus.call_method(
                Some("org.freedesktop.systemd1"),
                "/org/freedesktop/systemd1",
                Some("org.freedesktop.systemd1.Manager"),
                "StartTransientUnit",
                &(request.unit.as_str(), "fail", properties, aux),
            )
            .map_err(|e| e.to_string())?;
            // The call returns once the job is queued (`systemd-run --scope` waits for
            // `JobRemoved`); CEF must not fork its zygote and GPU process before the move, so
            // read the cgroup back within the same budget.
            let deadline = std::time::Instant::now() + super::TIMEOUT;
            loop {
                if Self::cgroup().is_some_and(|c| scope::moved(request, &c)) {
                    return Ok(());
                }
                if std::time::Instant::now() >= deadline {
                    return Err(format!(
                        "{} not entered within {} ms",
                        request.unit,
                        super::TIMEOUT.as_millis()
                    ));
                }
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
        }
    }
}
