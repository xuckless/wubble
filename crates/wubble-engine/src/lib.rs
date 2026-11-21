//! Chromium embedding layer.
//!
//! Nothing outside this crate (and `wubble-gfx`) may name a CEF type. The UI
//! talks to web content only through the API exported here, so the engine
//! underneath (prebuilt CEF now, our own ungoogled build later) can change
//! without touching the shell.

pub mod app;
pub mod cdp;
pub mod cosmetic;
pub mod devtools;
pub mod download;
pub mod drag;
pub mod ext_api;
pub mod extensions;
pub mod favicon;
pub mod fetch;
pub mod filter;
pub mod find;
pub mod ime;
pub mod input;
pub mod lifecycle;
#[cfg(target_os = "macos")]
mod mac_app;
pub mod media;
pub mod nav;
pub mod page;
pub mod pinch;
pub mod popup;
pub mod save;
pub mod storage;
pub mod view;
pub mod zoom;

/// The value of a CEF bit-flag type (`cef_event_flags_t(..).0` and its kin) as the `u32` the
/// engine keeps flags in. bindgen gives those types `i32` on Windows, where MSVC makes C enums
/// `int`, and `u32` elsewhere; every CEF flag is a small non-negative bit, so one conversion
/// serves both and no comparison needs a per-OS cast. Only the tests compare flag tables; the
/// engine hands flags to CEF through its own `c_int` parameters (`fetch.rs`).
#[cfg(test)]
pub(crate) fn cef_flags(raw: impl Into<i64>) -> u32 {
    let raw = raw.into();
    u32::try_from(raw).unwrap_or_else(|_| panic!("CEF flag value out of range: {raw}"))
}
