//! GPU frame path. The only crate allowed to touch native texture handles.
//!
//! ```text
//! CEF OnAcceleratedPaint (dmabuf / D3D11 shared handle / IOSurface)
//!   -> import as wgpu::Texture on Slint's device   (welding / grafting)
//!   -> slint::Image                                 (unstable-wgpu-30)
//!   -> composited by Slint in the same frame
//! ```
//!
//! Invariant: no CPU readback on the hot path. `OnPaint` (shared-memory
//! bitmaps) is a fallback only and must be counted/logged when hit.

pub mod import;
pub mod pacing;

pub use import::{FramePresenter, GfxError, GpuContext, NativeFrame};

/// A size in physical pixels: frames, views and the content rectangle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameSize {
    pub width: u32,
    pub height: u32,
}

impl FrameSize {
    pub const fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }

    /// The same size with each side at least 1 pixel (CEF and wgpu reject 0).
    pub fn at_least_one(self) -> Self {
        Self::new(self.width.max(1), self.height.max(1))
    }
}
