//! Content blocking on Brave's `adblock` crate. No CEF, wgpu or Slint here.
//!
//! The engine does all parsing and matching. This crate adds a typed API, a neutral
//! resource-type enum, per-site shields passed as data, and a pure list-refresh policy.

mod blocker;
mod decision;
mod refresh;
mod site;
mod types;

pub use blocker::{Blocker, BlockerError, Cosmetic};
pub use decision::Decision;
pub use refresh::{DEFAULT_LISTS, ListSpec, due};
pub use site::{Shields, SiteOverrides};
pub use types::{NetRequest, ResourceType};
