//! Worker thread priority: bulk work (blocker rebuilds, snapshot and media writes, network
//! lookups) runs below the UI thread and Chromium's threads, so a busy worker never takes a core
//! from a frame.
//!
//! Ported from Chromium (BSD-3-Clause), `ThreadType::kBackground` per OS:
//! `base/threading/platform_thread_linux.cc` (`ThreadTypeToNiceValue`: nice 10, set with
//! `setpriority` on the thread id), `base/threading/platform_thread_mac.mm`
//! (`QOS_CLASS_UTILITY`), `base/threading/platform_thread_win.cc`
//! (`THREAD_MODE_BACKGROUND_BEGIN`). Each lowers only the calling thread; none needs privileges.
//! Lowering cannot be undone on Linux without `RLIMIT_NICE`, so only threads that stay bulk
//! workers for their whole life call it.

/// Chromium's nice value for background threads.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
const BACKGROUND_NICE: i32 = 10;

/// Lowers the calling thread to background priority. Best effort: a refusal is ignored (the
/// thread keeps running at normal priority).
pub fn lower_current_thread() {
    #[cfg(target_os = "linux")]
    // SAFETY: plain syscalls on this thread's own id; no memory is passed.
    unsafe {
        let tid = libc::syscall(libc::SYS_gettid) as libc::id_t;
        libc::setpriority(libc::PRIO_PROCESS, tid, BACKGROUND_NICE);
    }
    #[cfg(target_os = "macos")]
    // SAFETY: sets the calling thread's own QoS class; no memory is passed.
    unsafe {
        libc::pthread_set_qos_class_self_np(libc::qos_class_t::QOS_CLASS_UTILITY, 0);
    }
    #[cfg(windows)]
    // SAFETY: the pseudo handle of the calling thread; no memory is passed.
    unsafe {
        use windows::Win32::System::Threading::{
            GetCurrentThread, SetThreadPriority, THREAD_MODE_BACKGROUND_BEGIN,
        };
        let _ = SetThreadPriority(GetCurrentThread(), THREAD_MODE_BACKGROUND_BEGIN);
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    /// Lowering is always allowed, so this checks the call reaches the calling thread only.
    #[test]
    fn lowers_this_thread_and_no_other() {
        let nice = || {
            // SAFETY: reads this thread's own nice value.
            unsafe {
                let tid = libc::syscall(libc::SYS_gettid) as libc::id_t;
                libc::getpriority(libc::PRIO_PROCESS, tid)
            }
        };
        let before = nice();
        let worker = std::thread::spawn(move || {
            lower_current_thread();
            nice()
        })
        .join()
        .unwrap();
        assert_eq!(worker, before.max(BACKGROUND_NICE));
        assert_eq!(nice(), before);
    }
}
