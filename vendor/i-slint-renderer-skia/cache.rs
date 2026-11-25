// wubble change (4): Skia's resource cache, bounded and cleaned up by the application.
//
// Chromium runs a cache controller over its Skia contexts (`gpu::raster::GrCacheController`,
// BSD-3-Clause; the idea is ported, no code is copied): a bounded cache, a cleanup of what was
// not used for a few seconds once the context goes idle, and everything unlocked purged when it
// is hidden or under memory pressure. Upstream leaves Skia's default (256 MiB) and has no such
// call on its wgpu surface. Every window's context on this thread registers here when its
// surface is made (`wgpu_30_surface.rs`); the application decides when (wubble's
// `wubble_core::skia_cache`), this only carries it out.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use std::time::Duration;

use skia_safe::gpu::{DirectContext, PurgeResourceOptions};

/// What to free of every window's Skia cache.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CacheCleanup {
    /// The unlocked resources not used for this long (`performDeferredCleanup`).
    Deferred(Duration),
    /// Every unlocked resource (`purgeUnlockedResources`).
    PurgeAll,
}

thread_local! {
    static CONTEXTS: RefCell<Vec<Weak<RefCell<DirectContext>>>> = const { RefCell::new(Vec::new()) };
    /// The byte budget every context gets, `None`: Skia's own.
    static LIMIT: Cell<Option<usize>> = const { Cell::new(None) };
}

/// A window's context, from its surface's making on; it gets the current budget.
pub(crate) fn register(context: &Rc<RefCell<DirectContext>>) {
    if let (Some(limit), Ok(mut c)) = (LIMIT.get(), context.try_borrow_mut()) {
        c.set_resource_cache_limit(limit);
    }
    CONTEXTS.with_borrow_mut(|all| {
        all.retain(|w| w.strong_count() > 0);
        all.push(Rc::downgrade(context));
    });
}

/// `f` on every live context not in use right now (one that is rendering is skipped).
fn each(mut f: impl FnMut(&mut DirectContext)) {
    CONTEXTS.with_borrow_mut(|all| {
        all.retain(|w| w.strong_count() > 0);
        for context in all.iter().filter_map(Weak::upgrade) {
            if let Ok(mut c) = context.try_borrow_mut() {
                f(&mut c);
            }
        }
    });
}

/// Every window's cache bounded to `bytes` (over it, Skia purges least recently used first).
/// Setting the budget it has does nothing.
pub fn set_limit(bytes: usize) {
    if LIMIT.get() == Some(bytes) {
        return;
    }
    LIMIT.set(Some(bytes));
    each(|c| c.set_resource_cache_limit(bytes));
}

/// Frees `what` of every window's cache.
pub fn cleanup(what: CacheCleanup) {
    each(|c| {
        match what {
            CacheCleanup::Deferred(unused) => {
                c.perform_deferred_cleanup(unused, PurgeResourceOptions::AllResources);
            }
            CacheCleanup::PurgeAll => {
                c.purge_unlocked_resources(PurgeResourceOptions::AllResources);
            }
        }
    });
}

/// The bytes every window's cache holds now (`--measure`).
pub fn bytes() -> usize {
    let mut total = 0;
    each(|c| total += c.resource_cache_usage().resource_bytes);
    total
}
