//! How CEF's pump requests reach the UI thread: one [`Post`] impl per OS, chosen by cfg into
//! [`Current`].
//!
//! Everywhere but macOS a request is a Slint `invoke_from_event_loop` closure. On macOS that
//! starves the window: winit 0.30's AppKit backend runs proxy events from its run loop's
//! `BeforeWaiting` observer and drains its channel until it is empty
//! (`platform_impl/macos/event_loop.rs`, `receiver.try_iter()`), so a pump that asks for the
//! next one from the UI thread, as CEF does on every run while a page animates, keeps that one
//! drain going: no redraw, no display-link tick (Slint draws from a `CADisplayLink` on macOS)
//! and no exit in between. Measured on the M2 Pro: whole 5 s `--measure scroll` runs with no
//! Slint render at all, and `--smoke frames` checking frame after frame without quitting.
//! The main dispatch queue is drained one snapshot per run loop pass (libdispatch
//! `_dispatch_main_queue_drain`), so a request posted during a pump runs on the next pass,
//! after the window has had its turn.

/// Runs `work` on the UI thread, later, from any thread.
pub trait Post {
    fn post(work: impl FnOnce() + Send + 'static);
}

/// The Slint event loop's own queue. Dropped when no loop runs.
#[cfg_attr(target_os = "macos", allow(dead_code))]
pub struct SlintLoop;

impl Post for SlintLoop {
    fn post(work: impl FnOnce() + Send + 'static) {
        let _ = slint::invoke_from_event_loop(work);
    }
}

/// The main dispatch queue (GCD), which the main run loop drains once per pass.
#[cfg(target_os = "macos")]
pub struct MainQueue;

#[cfg(target_os = "macos")]
impl Post for MainQueue {
    fn post(work: impl FnOnce() + Send + 'static) {
        dispatch2::DispatchQueue::main().exec_async(work);
    }
}

#[cfg(target_os = "macos")]
pub type Current = MainQueue;
#[cfg(not(target_os = "macos"))]
pub type Current = SlintLoop;
