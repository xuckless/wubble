//! Browser model with no dependency on CEF, wgpu or Slint.
//!
//! Everything here must stay unit-testable without a GPU or a browser process.

pub mod action;
pub mod keys;
pub mod lifecycle;
pub mod model;
pub mod motion;
pub mod omnibox;
pub mod pump;
pub mod scroller;
pub mod seam;
pub mod session;
pub mod sleep;
pub mod space;
pub mod suggest;
pub mod tab;
pub mod tabs;
pub mod url;
