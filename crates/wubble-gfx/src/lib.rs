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

    /// The part of a frame of this size that fits in `area`, anchored top
    /// left. A frame smaller than `area` is kept whole (and stretched).
    pub fn cropped_to(self, area: FrameSize) -> Self {
        Self::new(self.width.min(area.width), self.height.min(area.height))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crop_keeps_only_what_fits() {
        let area = FrameSize::new(1001, 600);
        assert_eq!(FrameSize::new(1002, 601).cropped_to(area), area);
        assert_eq!(
            FrameSize::new(800, 700).cropped_to(area),
            FrameSize::new(800, 600)
        );
        assert_eq!(area.cropped_to(area), area);
    }
}
