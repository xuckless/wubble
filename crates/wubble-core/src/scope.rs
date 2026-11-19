//! wubble's own CPU share under load: the browser runs in a systemd scope of its own with a CPU
//! weight above the default, so a machine saturated by other work (a build on every core) still
//! leaves the UI, the GPU process, the network service and the playing video their time.
//! Measured (TODO.md "Measured", b9): 28 busy processes cut a 4K60 video to 19.6 frames/s in
//! the terminal's scope; in a scope of its own at `CPUWeight=300` it played at 52.4.
//!
//! Nice values cannot do this unprivileged (`RLIMIT_NICE` is 0, and nice only orders threads
//! within one cgroup); a cgroup weight orders whole scopes. The scope is asked of the user's
//! systemd (`StartTransientUnit`, what `systemd-run --user --scope` does) by the `wubble`
//! binary's OS edge before CEF starts, so every Chromium process inherits it. This module is the
//! pure rule: whether to ask, and what.

/// Set (to anything) to keep wubble in the scope it was started in.
pub const NO_SCOPE_ENV: &str = "WUBBLE_NO_SCOPE";

/// The scope's CPU weight (systemd's default is 100). Modest on purpose: 1000 measured 54.5
/// frames/s against 52.4 for 300, and other apps keep a fair share.
pub const CPU_WEIGHT: u64 = 300;

/// One transient scope to start and move this process into.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScopeRequest {
    /// `app-wubble-<pid>.scope`: the XDG convention for an app's scope.
    pub unit: String,
    pub pid: u32,
    pub cpu_weight: u64,
    pub description: &'static str,
}

/// What the process knows when it decides.
#[derive(Clone, Copy, Debug)]
pub struct ScopeContext<'a> {
    pub pid: u32,
    /// The user's browser, not a smoke or a measurement (those keep the caller's limits).
    pub browsing: bool,
    /// [`NO_SCOPE_ENV`] is set.
    pub no_scope_env: bool,
    /// The text of `/proc/self/cgroup`, when readable.
    pub cgroup: Option<&'a str>,
}

/// The scope to ask for, or `None`: not browsing, the user opted out, or the process is
/// already in wubble's own scope (a restart from inside one). Any other scope, a desktop
/// launcher's `app-gnome-wubble-<pid>.scope` included, has the default weight, so the process
/// still moves.
pub fn plan(cx: ScopeContext) -> Option<ScopeRequest> {
    if !cx.browsing || cx.no_scope_env || cx.cgroup.and_then(scope_leaf).is_some_and(is_own_unit) {
        return None;
    }
    Some(ScopeRequest {
        unit: own_unit(cx.pid),
        pid: cx.pid,
        cpu_weight: CPU_WEIGHT,
        description: "wubble browser",
    })
}

/// The unit [`plan`] asks for.
fn own_unit(pid: u32) -> String {
    format!("app-wubble-{pid}.scope")
}

/// Whether `leaf` is a unit [`plan`] makes: exactly `app-wubble-<digits>.scope`.
fn is_own_unit(leaf: &str) -> bool {
    leaf.strip_prefix("app-wubble-")
        .and_then(|rest| rest.strip_suffix(".scope"))
        .is_some_and(|pid| !pid.is_empty() && pid.bytes().all(|b| b.is_ascii_digit()))
}

/// The last component of the cgroup v2 path in `/proc/self/cgroup`
/// (`0::/user.slice/.../app-wubble-12.scope`); v1 lines do not count.
fn scope_leaf(cgroup: &str) -> Option<&str> {
    cgroup
        .lines()
        .find_map(|l| l.strip_prefix("0::"))
        .and_then(|path| path.rsplit('/').next())
}

/// Whether the process is in `request`'s unit now: `StartTransientUnit` returns when the job
/// is queued, so the OS edge reads `/proc/self/cgroup` back until this holds.
pub fn moved(request: &ScopeRequest, cgroup: &str) -> bool {
    scope_leaf(cgroup) == Some(request.unit.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    const TERMINAL: &str =
        "0::/user.slice/user-1000.slice/user@1000.service/app.slice/app-ptyxis-spawn-1.scope\n";

    fn cx(browsing: bool, no_scope_env: bool, cgroup: Option<&str>) -> ScopeContext<'_> {
        ScopeContext {
            pid: 4242,
            browsing,
            no_scope_env,
            cgroup,
        }
    }

    #[test]
    fn the_browser_asks_for_its_own_weighted_scope() {
        assert_eq!(
            plan(cx(true, false, Some(TERMINAL))),
            Some(ScopeRequest {
                unit: "app-wubble-4242.scope".into(),
                pid: 4242,
                cpu_weight: CPU_WEIGHT,
                description: "wubble browser",
            })
        );
        // no cgroup file (not Linux, no /proc): still asked; the OS edge decides
        assert!(plan(cx(true, false, None)).is_some());
        const { assert!(CPU_WEIGHT > 100) };
    }

    #[test]
    fn moved_only_once_the_cgroup_names_the_unit() {
        let request = plan(cx(true, false, Some(TERMINAL))).unwrap();
        assert!(!moved(&request, TERMINAL));
        assert!(!moved(&request, ""));
        assert!(!moved(
            &request,
            "0::/user.slice/user@1000.service/app.slice/app-wubble-17.scope\n"
        ));
        assert!(moved(
            &request,
            "0::/user.slice/user@1000.service/app.slice/app-wubble-4242.scope\n"
        ));
    }

    #[test]
    fn skip_rules() {
        let cases = [
            (cx(false, false, Some(TERMINAL)), "a smoke or measurement"),
            (cx(true, true, Some(TERMINAL)), "WUBBLE_NO_SCOPE"),
            (
                cx(
                    true,
                    false,
                    Some("0::/user.slice/user@1000.service/app.slice/app-wubble-17.scope\n"),
                ),
                "already in its own scope",
            ),
        ];
        for (c, why) in cases {
            assert_eq!(plan(c), None, "{why}");
        }
        // a desktop launcher's scope has the default weight: still move
        for launcher in [
            "0::/user.slice/user@1000.service/app.slice/app-gnome-wubble-9.scope",
            "0::/user.slice/user@1000.service/app.slice/app-wubble-.scope",
            "0::/user.slice/user@1000.service/app.slice/app-wubble-9x.scope",
        ] {
            assert!(
                plan(cx(true, false, Some(launcher))).is_some(),
                "{launcher}"
            );
        }
        // cgroup v1 lines and other apps' scopes do not count
        assert!(
            plan(cx(
                true,
                false,
                Some("1:name=systemd:/app-wubble-1.scope\n")
            ))
            .is_some()
        );
        assert!(
            plan(cx(
                true,
                false,
                Some("0::/user.slice/user@1000.service/app.slice/wubble-build.service")
            ))
            .is_some()
        );
    }
}
