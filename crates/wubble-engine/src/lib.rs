//! Chromium embedding layer.
//!
//! Nothing outside this crate (and `wubble-gfx`) may name a CEF type. The UI
//! talks to web content only through the API exported here, so the engine
//! underneath (prebuilt CEF now, our own ungoogled build later) can change
//! without touching the shell.

pub mod app;
mod cdp;
pub mod devtools;
pub mod download;
pub mod drag;
pub mod extensions;
pub mod favicon;
pub mod filter;
pub mod find;
pub mod ime;
pub mod input;
pub mod lifecycle;
pub mod media;
pub mod nav;
pub mod page;
pub mod popup;
pub mod view;
pub mod zoom;
