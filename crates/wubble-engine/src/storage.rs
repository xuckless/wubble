//! Web storage per container.
//!
//! A container ([`ContainerId`]) is the unit of web storage: every view opened
//! in it shares one CEF request context, so one cookie jar, local storage and
//! IndexedDB, HTTP cache, permission store and service-worker set; different
//! containers share nothing. Each container's context lives on disk at
//! [`container_dir`], session cookies included, so a restart keeps logins.
//! CEF 156 (Chrome runtime) makes a disk-backed context a Chrome profile,
//! which must be a direct child of `root_cache_path`
//! (`chrome_browser_context.cc`: "Cannot create profile at path", and the
//! context silently falls back to memory), hence `<root>/container-<id>`
//! rather than a `containers/` folder. The default container is no exception:
//! it is `container-0`, not CEF's process-global context, which welding never
//! hands a browser.
//!
//! One context per container rather than one per view on a shared path: CEF
//! shares storage between contexts on the same `cache_path` too, but a single
//! object is one owner, initializes once (a disk-backed context comes up
//! asynchronously, `SharedProfile::is_ready`) and is where per-container
//! settings and clearing go. The engine holds each context from first use
//! until shutdown begins ([`Profiles::release`]); browsers hold their own
//! reference meanwhile, so it outlives the last one to close.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use cef::rc::Rc;
use cef::{
    CompletionCallback, CookieVisitor, ImplCompletionCallback, ImplCookieManager,
    ImplCookieVisitor, ImplPreferenceManager, ImplRequestContext, ImplSetCookieCallback, ImplValue,
    SetCookieCallback, WrapCompletionCallback, WrapCookieVisitor, WrapSetCookieCallback,
};
use std::sync::{Arc, Mutex};
use welding::{CefRuntime, SharedProfile};
use wubble_core::privacy::PrefValue;
use wubble_core::space::{ContainerId, Storage};

use crate::app::EngineError;

/// The prefix of each container's directory under `root_cache_path`.
const PREFIX: &str = "container-";

/// Runs of CEF work a new context may take to initialize before opening a
/// view in it fails. A count, not a clock: each run sleeps [`READY_NAP`] so the
/// profile's own threads progress. One run was enough on Linux (CEF 156,
/// `cookie_smoke`); the bound only ends a hang.
const READY_RUNS: u32 = 10_000;
const READY_NAP: Duration = Duration::from_millis(1);

/// Where `container`'s cookies, storage and cache live: a direct child of
/// CEF's root cache.
pub fn container_dir(root: &Path, container: ContainerId) -> PathBuf {
    root.join(format!("{PREFIX}{}", container.0))
}

/// The container whose directory is named `name` ([`container_dir`]'s last part).
pub(crate) fn container_of_dir_name(name: &str) -> Option<ContainerId> {
    name.strip_prefix(PREFIX)?.parse().ok().map(ContainerId)
}

/// The marker that `container`'s storage is to go: written when the container
/// is deleted (its context may still have files open), acted on at the next
/// start, before CEF opens anything ([`sweep`]).
fn tombstone(root: &Path, container: ContainerId) -> PathBuf {
    root.join(format!("{PREFIX}{}.forget", container.0))
}

/// The container a tombstone file names, if `name` is one.
fn tombstoned(name: &str) -> Option<ContainerId> {
    name.strip_prefix(PREFIX)?
        .strip_suffix(".forget")?
        .parse()
        .ok()
        .map(ContainerId)
}

/// Deletes the storage of every container deleted in an earlier run. Called
/// before CEF starts, so nothing holds those files. Errors are logged: the
/// marker stays and the next start tries again.
pub(crate) fn sweep(root: &Path) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let Some(container) = entry.file_name().to_str().and_then(tombstoned) else {
            continue;
        };
        let dir = container_dir(root, container);
        match std::fs::remove_dir_all(&dir) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                eprintln!("wubble-engine: cannot remove {}: {e}", dir.display());
                continue;
            }
        }
        let _ = std::fs::remove_file(entry.path());
    }
}

/// Marks `container`'s storage for deletion at the next start. Never called
/// for a [`Storage::Memory`] container: it has no directory, and a tombstone
/// would name a disk container of that id.
pub(crate) fn forget(root: &Path, container: ContainerId) {
    let marker = tombstone(root, container);
    let written = marker
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|()| std::fs::write(&marker, b""));
    if let Err(e) = written {
        eprintln!(
            "wubble-engine: cannot mark container {} for deletion: {e}",
            container.0
        );
    }
}

// Answers a CEF call that needs a completion callback; nothing waits for it.
cef::wrap_completion_callback! {
    struct Ignored {}
    impl CompletionCallback {}
}

/// Asks CEF to delete `context`'s cookies, HTTP cache and HTTP auth
/// credentials (asynchronous; nothing waits for it). Each call gets a
/// callback: CEF 156 crashed the browser process when
/// `ClearHttpAuthCredentials` completed without one (`ConcurrentCallbacks`
/// running a null callback; seen in `cookie_smoke`); `ClearHttpCache` gets
/// one too rather than find out.
pub(crate) fn clear(context: &cef::RequestContext, container: ContainerId) {
    let cookies = context
        .cookie_manager(None)
        .is_some_and(|m| m.delete_cookies(None, None, None) != 0);
    if !cookies {
        eprintln!(
            "wubble-engine: container {}: CEF refused the cookie delete",
            container.0
        );
    }
    context.clear_http_cache(Some(&mut Ignored::new()));
    context.clear_http_auth_credentials(Some(&mut Ignored::new()));
}

/// Which cookies a copy takes: `(domain, path, secure)` as CEF names them (a leading dot for a
/// domain cookie). The host's matcher (`wubble_core::agent::shares_cookie`).
pub type CookieFilter = Box<dyn Fn(&str, &str, bool) -> bool + Send + Sync>;

/// What runs when a copy ended, with what it did.
pub type CopyDone = Box<dyn FnOnce(CopyReport) + Send>;

/// What a copy did: the cookies the source holds that the filter took, and how many of those
/// the target accepted.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CopyReport {
    pub taken: usize,
    pub set: usize,
}

/// One copy in flight, shared by its visitor and each cookie's set callback. Its end is its
/// drop: CEF releases the visitor once it visited every cookie (or at once when there are
/// none, never calling it) and each callback once its set answered, so the last release runs
/// `done`, on whichever CEF thread that is.
struct CopyJob {
    report: Mutex<CopyReport>,
    done: Mutex<Option<CopyDone>>,
    /// Set by [`CopyCancel::cancel`]: no cookie is set from then on, and the visit stops.
    cancelled: Arc<std::sync::atomic::AtomicBool>,
}

/// Stops a copy in flight ([`copy_cookies`]): a session stopped while its sign-ins are copied.
/// Visits and sets run on CEF's UI thread, as the host does, so after `cancel` no new set is
/// issued; the sets issued before go down the target's cookie manager ahead of the deletion a
/// teardown sends after them (`clear`).
#[derive(Clone, Debug, Default)]
pub struct CopyCancel(Arc<std::sync::atomic::AtomicBool>);

impl CopyCancel {
    pub fn cancel(&self) {
        self.0.store(true, std::sync::atomic::Ordering::SeqCst);
    }
}

impl Drop for CopyJob {
    fn drop(&mut self) {
        let report = *self.report.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(done) = self.done.lock().unwrap_or_else(|e| e.into_inner()).take() {
            done(report);
        }
    }
}

cef::wrap_set_cookie_callback! {
    struct CookieSet {
        job: Arc<CopyJob>,
    }
    impl SetCookieCallback {
        fn on_complete(&self, success: ::std::os::raw::c_int) {
            if success != 0 {
                self.job.report.lock().unwrap_or_else(|e| e.into_inner()).set += 1;
            }
        }
    }
}

cef::wrap_cookie_visitor! {
    struct CookieCopier {
        job: Arc<CopyJob>,
        filter: Arc<CookieFilter>,
        target: cef::CookieManager,
    }
    impl CookieVisitor {
        fn visit(
            &self,
            cookie: Option<&cef::Cookie>,
            _count: ::std::os::raw::c_int,
            _total: ::std::os::raw::c_int,
            _delete_cookie: Option<&mut ::std::os::raw::c_int>,
        ) -> ::std::os::raw::c_int {
            if self.job.cancelled.load(::std::sync::atomic::Ordering::SeqCst) {
                return 0;
            }
            if let Some(cookie) = cookie {
                copy_one(cookie, &self.filter, &self.target, &self.job);
            }
            1
        }
    }
}

/// Sets one visited cookie in the target, if the filter takes it: every attribute as read
/// (name, value, path, secure, httponly, creation, last access, expiry or none, same-site,
/// priority). A host-only cookie (no leading dot) is set with an empty domain, as CEF's
/// `SetCookie` makes a host cookie only then; the URL is the cookie's host and path, https for
/// a secure one. CEF's cookie struct has no partition key: a partitioned cookie arrives
/// unpartitioned (docs/research/mcp-server.md 0.3).
fn copy_one(
    cookie: &cef::Cookie,
    filter: &CookieFilter,
    target: &cef::CookieManager,
    job: &Arc<CopyJob>,
) {
    let domain = cookie.domain.to_string();
    let path = cookie.path.to_string();
    let secure = cookie.secure != 0;
    if !filter(&domain, &path, secure) {
        return;
    }
    let host = domain.strip_prefix('.').unwrap_or(&domain);
    let scheme = if secure { "https" } else { "http" };
    let url = format!(
        "{scheme}://{host}{}",
        if path.is_empty() { "/" } else { &path }
    );
    let mut copy = cookie.clone();
    if !domain.starts_with('.') {
        copy.domain = cef::CefString::from("");
    }
    if job.cancelled.load(std::sync::atomic::Ordering::SeqCst) {
        return;
    }
    job.report.lock().unwrap_or_else(|e| e.into_inner()).taken += 1;
    let mut callback = CookieSet::new(job.clone());
    let url = cef::CefString::from(url.as_str());
    if target.set_cookie(Some(&url), Some(&copy), Some(&mut callback)) == 0 {
        eprintln!("wubble-engine: CEF refused to copy a cookie of {host}");
    }
}

/// Copies the cookies of `from` that `filter` takes into `to` (the source is only read).
/// `done` runs once, when every cookie was visited and every set answered, on a CEF thread;
/// never when the copy could not start (`None`: either context has no cookie manager, or CEF
/// refused the visit), which the caller reports itself.
pub(crate) fn copy_cookies(
    from: &cef::RequestContext,
    to: &cef::RequestContext,
    filter: CookieFilter,
    done: CopyDone,
) -> Option<CopyCancel> {
    let (Some(source), Some(target)) = (from.cookie_manager(None), to.cookie_manager(None)) else {
        return None;
    };
    let cancel = CopyCancel::default();
    let job = Arc::new(CopyJob {
        report: Mutex::new(CopyReport::default()),
        done: Mutex::new(Some(done)),
        cancelled: cancel.0.clone(),
    });
    let mut visitor = CookieCopier::new(job.clone(), Arc::new(filter), target);
    if source.visit_all_cookies(Some(&mut visitor)) == 0 {
        // not started: the job's end must not report it as a copy
        job.done.lock().unwrap_or_else(|e| e.into_inner()).take();
        return None;
    }
    Some(cancel)
}

/// Sets Chromium preference `name` to `value` on `prefs`: a request context (a container's
/// profile prefs) or CEF's global manager (local state). `Err` when CEF refuses it: the
/// preference is not registered in this build, is managed, or the value has the wrong type.
pub(crate) fn set_pref(
    prefs: &impl ImplPreferenceManager,
    name: &str,
    value: &PrefValue,
) -> Result<(), String> {
    let key = cef::CefString::from(name);
    if prefs.can_set_preference(Some(&key)) == 0 {
        return Err(format!("{name}: not settable"));
    }
    let mut cef_value = cef::value_create().ok_or("CEF made no value")?;
    match value {
        PrefValue::Bool(on) => cef_value.set_bool(i32::from(*on)),
        PrefValue::Int(n) => cef_value.set_int(*n),
        PrefValue::Str(text) => cef_value.set_string(Some(&cef::CefString::from(&**text))),
    };
    // Not `CefString::default()`: that passes a null `error`, and CEF's C wrapper refuses the
    // call outright when an out-parameter is null.
    let mut error = cef::CefString::from("");
    if prefs.set_preference(Some(&key), Some(&mut cef_value), Some(&mut error)) == 0 {
        return Err(format!("{name}: {error}"));
    }
    Ok(())
}

/// The open container contexts, each with the storage it was opened with,
/// and the profile preferences every one of them is given. Owned by the
/// engine; empty after [`Self::release`], which shutdown calls before
/// `cef::shutdown`.
pub(crate) struct Profiles {
    root: PathBuf,
    open: BTreeMap<ContainerId, (Storage, SharedProfile)>,
    /// The last value of each preference [`Self::set_pref`] was given: set on
    /// each context as it comes up, so a container opened later has them too.
    prefs: BTreeMap<&'static str, PrefValue>,
}

impl Profiles {
    pub(crate) fn new(root: PathBuf) -> Self {
        Self {
            root,
            open: BTreeMap::new(),
            prefs: BTreeMap::new(),
        }
    }

    /// Preference `name` is `value` in every container: the open contexts now, the others as
    /// they come up ([`Self::get`]). A refusal is logged per context.
    pub(crate) fn set_pref(&mut self, name: &'static str, value: PrefValue) {
        for (container, (_, profile)) in &self.open {
            if let Err(e) = set_pref(&profile.request_context(), name, &value) {
                eprintln!("wubble-engine: container {}: {e}", container.0);
            }
        }
        self.prefs.insert(name, value);
    }

    /// `container`'s context with `storage`, created and brought up on first
    /// use (CEF is pumped until it reports the context initialized). A
    /// [`Storage::Memory`] context has no directory, which CEF's Chrome runtime
    /// makes an off-the-record profile (`ChromeBrowserContext`: an empty
    /// `cache_path` is a new `OTRProfileID::CreateUniqueForCEF` profile of the
    /// primary user profile, destroyed when the context shuts down). Fails
    /// closed: a context open with the other storage is refused, never reused.
    pub(crate) fn get(
        &mut self,
        runtime: &CefRuntime,
        container: ContainerId,
        storage: Storage,
    ) -> Result<SharedProfile, EngineError> {
        if let Some((opened, profile)) = self.open.get(&container) {
            if *opened != storage {
                return Err(EngineError::View(format!(
                    "container {}: open as {opened:?}, asked for {storage:?}",
                    container.0
                )));
            }
            return Ok(profile.clone());
        }
        let dir = match storage {
            Storage::Disk => Some(container_dir(&self.root, container)),
            Storage::Memory => None,
        };
        let profile = SharedProfile::new(runtime, dir)
            .map_err(|e| EngineError::View(format!("container {}: {e}", container.0)))?;
        let mut runs = 0;
        while !profile.is_ready() {
            if runs == READY_RUNS {
                return Err(EngineError::View(format!(
                    "container {}: CEF did not initialize its storage in {READY_RUNS} runs",
                    container.0
                )));
            }
            runtime.do_message_loop_work();
            std::thread::sleep(READY_NAP);
            runs += 1;
        }
        let context = profile.request_context();
        for (name, value) in &self.prefs {
            if let Err(e) = set_pref(&context, name, value) {
                eprintln!("wubble-engine: container {}: {e}", container.0);
            }
        }
        self.open.insert(container, (storage, profile.clone()));
        Ok(profile)
    }

    /// `container`'s context if it is open in this run.
    pub(crate) fn open(&self, container: ContainerId) -> Option<&SharedProfile> {
        self.open.get(&container).map(|(_, profile)| profile)
    }

    /// Drops the engine's references. Browsers still open keep theirs until
    /// they close, which a shutdown waits for before `cef::shutdown`.
    pub(crate) fn release(&mut self) {
        self.open.clear();
    }

    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    /// Drops `container`'s context, so a later use opens it afresh (CEF
    /// destroys an in-memory profile when its last browser has gone).
    pub(crate) fn close(&mut self, container: ContainerId) {
        self.open.remove(&container);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_container_has_its_own_directory_inside_the_root() {
        let root = Path::new("/p/cef");
        let a = container_dir(root, ContainerId::DEFAULT);
        let b = container_dir(root, ContainerId(7));
        assert_eq!(a, Path::new("/p/cef/container-0"));
        assert_eq!(b, Path::new("/p/cef/container-7"));
        assert_ne!(a, b);
        // CEF makes each a Chrome profile: a direct child of the root, or none at all.
        assert!(a.parent() == Some(root) && b.parent() == Some(root));
    }

    #[test]
    fn a_tombstone_names_its_container_and_nothing_else_does() {
        let root = Path::new("/p/cef");
        let marker = tombstone(root, ContainerId(12));
        let name = marker.file_name().unwrap().to_str().unwrap();
        assert_eq!(tombstoned(name), Some(ContainerId(12)));
        assert_eq!(tombstoned("container-12"), None);
        assert_eq!(tombstoned("container-x.forget"), None);
        assert_eq!(tombstoned("Default"), None);
        assert_eq!(marker.parent(), Some(root));
    }

    #[test]
    fn sweep_deletes_only_forgotten_containers() {
        let root = std::env::temp_dir().join(format!("wubble-storage-test-{}", std::process::id()));
        let kept = container_dir(&root, ContainerId(1));
        let gone = container_dir(&root, ContainerId(2));
        for dir in [&kept, &gone] {
            std::fs::create_dir_all(dir.join("Local Storage")).unwrap();
            std::fs::write(dir.join("Cookies"), b"x").unwrap();
        }
        forget(&root, ContainerId(2));
        sweep(&root);
        let (kept_ok, gone_ok) = (kept.join("Cookies").is_file(), !gone.exists());
        let marker_gone = !tombstone(&root, ContainerId(2)).exists();
        std::fs::remove_dir_all(&root).unwrap();
        assert!(kept_ok && gone_ok && marker_gone);
    }

    #[test]
    fn sweep_without_containers_does_nothing() {
        sweep(Path::new("/nonexistent/wubble/cef"));
    }
}
