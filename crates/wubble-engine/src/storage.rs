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
    CompletionCallback, ImplCompletionCallback, ImplCookieManager, ImplRequestContext,
    WrapCompletionCallback,
};
use welding::{CefRuntime, SharedProfile};
use wubble_core::space::ContainerId;

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

/// Marks `container`'s storage for deletion at the next start.
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

/// The open container contexts. Owned by the engine; empty after
/// [`Self::release`], which shutdown calls before `cef::shutdown`.
pub(crate) struct Profiles {
    root: PathBuf,
    open: BTreeMap<ContainerId, SharedProfile>,
}

impl Profiles {
    pub(crate) fn new(root: PathBuf) -> Self {
        Self {
            root,
            open: BTreeMap::new(),
        }
    }

    /// `container`'s context, created and brought up on first use (CEF is
    /// pumped until it reports the context initialized).
    pub(crate) fn get(
        &mut self,
        runtime: &CefRuntime,
        container: ContainerId,
    ) -> Result<SharedProfile, EngineError> {
        if let Some(profile) = self.open.get(&container) {
            return Ok(profile.clone());
        }
        let dir = container_dir(&self.root, container);
        let profile = SharedProfile::new(runtime, Some(dir))
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
        self.open.insert(container, profile.clone());
        Ok(profile)
    }

    /// `container`'s context if it is open in this run.
    pub(crate) fn open(&self, container: ContainerId) -> Option<&SharedProfile> {
        self.open.get(&container)
    }

    /// Drops the engine's references. Browsers still open keep theirs until
    /// they close, which a shutdown waits for before `cef::shutdown`.
    pub(crate) fn release(&mut self) {
        self.open.clear();
    }

    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    /// Drops `container`'s context, so a later use opens it afresh.
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
