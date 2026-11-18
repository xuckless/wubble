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
