//! Content blocking on Brave's `adblock` crate. No CEF, wgpu or Slint here.
//!
//! The engine does all parsing and matching. This crate adds the list
//! catalog (uBlock Origin's default set, bundled where the licence allows),
//! uBO's scriptlet and redirect resources, a typed API, a neutral
//! resource-type enum, per-site shields passed as data, a compiled cache,
//! and a list refresh whose only network access is the fetcher the caller
//! passes.

mod blocker;
pub mod catalog;
mod decision;
mod lists;
mod preparse;
mod refresh;
mod site;
mod types;

pub use blocker::{Blocker, BlockerError, Cosmetic, hide_rule};
pub use decision::Decision;
pub use lists::{
    ListRow, Origin, Source, bundled_sources, count_rules, rows, rule_counts, sources,
};
pub use refresh::{Due, ListSpec, due, fetch_http, is_list_url, last_fetch, refresh, specs};
pub use site::{Shields, SiteOverrides};
pub use types::{NetRequest, ResourceType};
