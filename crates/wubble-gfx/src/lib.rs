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
pub mod mask;
pub mod pacing;
pub mod scene;
pub mod swatch;

pub use import::{FramePresenter, FrameTiming, GfxError, GpuContext, NativeFrame};

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

    /// Where a frame of this size is shown in `area`: anchored top left, at
    /// one image pixel per physical pixel, never scaled. A larger frame is
    /// cropped to `area`; a smaller one (stale during a resize) is shown
    /// whole at its own size and the rest of `area` shows the background.
    /// The shell sizes the image to this, in physical pixels (`app.slint`).
    pub fn cropped_to(self, area: FrameSize) -> Self {
        Self::new(self.width.min(area.width), self.height.min(area.height))
    }
}

/// A bowed edge's displacement along x, sampled every `step` logical px of height from `y0`
/// (a borrowed `wubble_core::bow::EdgeTable`, which this crate does not name). At most
/// [`EDGE_SAMPLES`] samples are used.
#[derive(Clone, Copy, Debug)]
pub struct Edge<'a> {
    pub y0: f32,
    pub step: f32,
    pub offset: &'a [f32],
}

/// The samples one edge uniform holds (64 × vec4 in WGSL).
pub const EDGE_SAMPLES: usize = 256;

/// An edge as a shader reads it: (step, y0, samples) and the samples, 4 to a vec4. `None` and
/// fewer than two samples read as straight.
pub(crate) fn pack_edge(edge: Option<Edge>) -> ([f32; 3], [[f32; 4]; 64]) {
    let mut out = [[0.0; 4]; 64];
    let Some(e) = edge else {
        return ([0.0; 3], out);
    };
    let n = e.offset.len().min(EDGE_SAMPLES);
    for (i, &o) in e.offset[..n].iter().enumerate() {
        out[i / 4][i % 4] = o;
    }
    ([e.step, e.y0, n as f32], out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_edge_packs_four_to_a_vec4() {
        let offset: Vec<f32> = (0..300).map(|i| i as f32).collect();
        let (tab, out) = pack_edge(Some(Edge {
            y0: 8.0,
            step: 4.0,
            offset: &offset,
        }));
        assert_eq!(tab, [4.0, 8.0, EDGE_SAMPLES as f32]);
        assert_eq!(out[0], [0.0, 1.0, 2.0, 3.0]);
        assert_eq!(out[63][3], 255.0);
        assert_eq!(pack_edge(None).0, [0.0; 3]);
    }

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

    #[test]
    fn a_smaller_frame_keeps_its_own_size() {
        let area = FrameSize::new(1001, 600);
        let small = FrameSize::new(900, 500);
        assert_eq!(small.cropped_to(area), small);
        assert_eq!(
            FrameSize::new(1200, 500).cropped_to(area),
            FrameSize::new(1001, 500)
        );
        assert_eq!(
            FrameSize::new(1, 1).cropped_to(FrameSize::new(0, 0)),
            FrameSize::new(0, 0)
        );
    }
}
