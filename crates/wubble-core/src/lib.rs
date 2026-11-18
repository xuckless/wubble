//! Browser model with no dependency on CEF, wgpu or Slint.
//!
//! Everything here must stay unit-testable without a GPU or a browser process.

pub mod action;
pub mod apply;
pub mod autofill;
pub mod bow;
pub mod busy;
pub mod devtools;
pub mod download;
pub mod favicon;
pub mod import;
pub mod keys;
pub mod lifecycle;
pub mod model;
pub mod motion;
pub mod neterror;
pub mod omnibox;
pub mod pages;
pub mod pressure;
pub mod pump;
pub mod scroller;
pub mod seam;
pub mod secret;
pub mod session;
pub mod settings;
pub mod site;
pub mod sleep;
pub mod snapshot;
pub mod space;
pub mod split;
pub mod stretch;
pub mod suggest;
pub mod tab;
pub mod tabs;
pub mod tasks;
pub mod url;
pub mod zoom;
