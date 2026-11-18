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

/// The samples one edge uniform holds (`EDGE_VEC4S` × vec4 in edge.wgsl).
pub const EDGE_SAMPLES: usize = 256;

/// One edge as edge.wgsl's `EdgeU` reads it: (step, y0, samples, unused) and the samples, 4 to
/// a vec4. Shaders bind two (`EDGES`, binding 5).
#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct EdgeU {
    tab: [f32; 4],
    off: [[f32; 4]; EDGE_SAMPLES / 4],
}

const _: () = assert!(std::mem::size_of::<EdgeU>() == 16 + EDGE_SAMPLES * 4);

impl EdgeU {
    /// `None` and fewer than two samples read as straight.
    pub(crate) fn pack(edge: Option<Edge>) -> Self {
        let mut u = Self {
            tab: [0.0; 4],
            off: [[0.0; 4]; EDGE_SAMPLES / 4],
        };
        if let Some(e) = edge {
            let n = e.offset.len().min(EDGE_SAMPLES);
            for (i, &o) in e.offset[..n].iter().enumerate() {
                u.off[i / 4][i % 4] = o;
            }
            u.tab = [e.step, e.y0, n as f32, 0.0];
        }
        u
    }
}

/// A uniform mirror's bytes, for `queue.write_buffer` and the scene's arena.
pub(crate) fn bytes<T: Copy>(v: &T) -> &[u8] {
    // SAFETY: the uniform mirrors are #[repr(C)], Copy, and made only of f32/i32 (no padding
    // bytes the compiler could leave uninitialised: every gap is an explicit field).
    unsafe { std::slice::from_raw_parts(v as *const T as *const u8, std::mem::size_of::<T>()) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_edge_packs_four_to_a_vec4() {
        let offset: Vec<f32> = (0..300).map(|i| i as f32).collect();
        let u = EdgeU::pack(Some(Edge {
            y0: 8.0,
            step: 4.0,
            offset: &offset,
        }));
        assert_eq!(u.tab, [4.0, 8.0, EDGE_SAMPLES as f32, 0.0]);
        assert_eq!(u.off[0], [0.0, 1.0, 2.0, 3.0]);
        assert_eq!(u.off[EDGE_SAMPLES / 4 - 1][3], 255.0);
        assert_eq!(EdgeU::pack(None).tab, [0.0; 4]);
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
