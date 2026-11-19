//! The `alloc-count` feature (off by default): a counting global allocator over the system one,
//! so `--measure memory` can say how much of the browser process's memory is wubble's own Rust
//! heap (Slint, wgpu's CPU side, the blocker, the store, the shell) and how much is Chromium's
//! browser process, the drivers and mapped code. Chromium allocates through its own
//! PartitionAlloc and the C allocator, which this does not see. The pattern is the `GlobalAlloc`
//! example in Rust's std documentation (MIT OR Apache-2.0).

/// `live`, `peak` and `allocations` of the Rust heap as a report line, with the feature on.
pub fn line() -> Option<String> {
    #[cfg(feature = "alloc-count")]
    {
        use std::sync::atomic::Ordering::Relaxed;
        let mib = |b: usize| wubble_core::memreport::mib(b as u64);
        Some(format!(
            "rust heap (alloc-count): live {} MiB, peak {} MiB, {} allocations",
            mib(counting::LIVE.load(Relaxed)),
            mib(counting::PEAK.load(Relaxed)),
            counting::COUNT.load(Relaxed)
        ))
    }
    #[cfg(not(feature = "alloc-count"))]
    None
}

#[cfg(feature = "alloc-count")]
mod counting {
    use std::alloc::{GlobalAlloc, Layout, System};
    use std::sync::atomic::AtomicUsize;
    use std::sync::atomic::Ordering::Relaxed;

    pub static LIVE: AtomicUsize = AtomicUsize::new(0);
    pub static PEAK: AtomicUsize = AtomicUsize::new(0);
    pub static COUNT: AtomicUsize = AtomicUsize::new(0);

    struct Counting;

    fn added(size: usize) {
        let live = LIVE.fetch_add(size, Relaxed) + size;
        PEAK.fetch_max(live, Relaxed);
        COUNT.fetch_add(1, Relaxed);
    }

    // SAFETY: every call is forwarded to `System` unchanged; the counters only observe.
    unsafe impl GlobalAlloc for Counting {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            // SAFETY: the caller's contract, passed on.
            let p = unsafe { System.alloc(layout) };
            if !p.is_null() {
                added(layout.size());
            }
            p
        }

        unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
            // SAFETY: the caller's contract, passed on.
            let p = unsafe { System.alloc_zeroed(layout) };
            if !p.is_null() {
                added(layout.size());
            }
            p
        }

        unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
            // SAFETY: the caller's contract, passed on.
            unsafe { System.dealloc(ptr, layout) };
            LIVE.fetch_sub(layout.size(), Relaxed);
        }

        unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
            // SAFETY: the caller's contract, passed on.
            let p = unsafe { System.realloc(ptr, layout, new_size) };
            if !p.is_null() {
                LIVE.fetch_sub(layout.size(), Relaxed);
                added(new_size);
            }
            p
        }
    }

    #[global_allocator]
    static ALLOC: Counting = Counting;
}
