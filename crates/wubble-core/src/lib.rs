//! Browser model with no dependency on CEF, wgpu or Slint.
//!
//! Everything here must stay unit-testable without a GPU or a browser process.

pub mod lifecycle;
pub mod model;
pub mod motion;
pub mod pump;
pub mod session;
pub mod sleep;
pub mod space;
pub mod suggest;
pub mod tab;
pub mod url;
