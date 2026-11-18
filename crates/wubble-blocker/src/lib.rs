//! Content blocking on Brave's `adblock` crate. No CEF, wgpu or Slint here.
//!
//! The engine does all parsing and matching. This crate adds a typed API, a neutral
//! resource-type enum, per-site shields passed as data, and a list directory with a
//! compiled cache, and a list refresh whose only network access is the
//! fetcher the caller passes.

mod blocker;
mod decision;
mod lists;
mod refresh;
mod site;
mod types;

pub use blocker::{Blocker, BlockerError, Cosmetic};
pub use decision::Decision;
pub use lists::RESOURCES_FILE;
pub use refresh::{DEFAULT_LISTS, ListSpec, due, fetch_http, last_fetch, refresh};
pub use site::{Shields, SiteOverrides};
pub use types::{NetRequest, ResourceType};
