//! The page on a layer of the OS compositor (`wubble_core::page_layer`): a frame copied into an
//! OS buffer (welding's `OsBuffer`, an IOSurface on macOS) is shown by the compositor as it is,
//! with no presenter blit and no shell render. One trait, [`PageLayer`]; macOS implements it
//! with a `CALayer` ([`CaLayer`]), every other platform has none ([`Unsupported`]).
//!
//! macOS, stage 1: the layer sits over the shell's `CAMetalLayer` (wgpu's, a sublayer of
//! winit's view's layer) and is shown only while the page is the window's whole content (page
//! fullscreen, the picture-in-picture window), so the shell's frame under it is never seen. With
//! the hole of stage 2 it goes under the shell's layer instead (`insertSublayer:below:`). Every
//! change is one `CATransaction` with its implicit animations off, on the main thread.

use crate::import::{GfxError, NativeFrame};

/// A page's layer in the compositor.
pub trait PageLayer {
    /// The layer tree it sits in: its height in points, and whether its origin is top left
    /// (`wubble_core::page_layer::place` takes both).
    fn root(&self) -> (f64, bool);

    /// Where the layer sits (`frame`: x, y, width, height in points), what of its buffer it
    /// shows (`contents`: the same in the buffer's unit square), the buffer's pixels per point
    /// and the corner radius in points.
    fn place(&mut self, frame: [f64; 4], contents: [f64; 4], scale: f64, radius: f64);

    /// `frame`'s OS buffer onto the layer, shown. Fails for a frame copied without one.
    fn show(&mut self, frame: &NativeFrame) -> Result<(), GfxError>;

    /// Hides the layer and lets its buffers go.
    fn hide(&mut self);
}

/// No layer: every frame goes through the shell.
pub struct Unsupported;

impl PageLayer for Unsupported {
    fn root(&self) -> (f64, bool) {
        (0.0, true)
    }

    fn place(&mut self, _frame: [f64; 4], _contents: [f64; 4], _scale: f64, _radius: f64) {}

    fn show(&mut self, _frame: &NativeFrame) -> Result<(), GfxError> {
        Err(GfxError::Layer("this platform has no page layer".into()))
    }

    fn hide(&mut self) {}
}

#[cfg(target_os = "macos")]
pub use mac::CaLayer;

#[cfg(target_os = "macos")]
mod mac {
    use std::ffi::c_void;
    use std::ptr::NonNull;

    use objc2::rc::Retained;
    use objc2::runtime::{AnyObject, NSObjectProtocol};
    use objc2::{ClassType, MainThreadMarker, msg_send};
    use objc2_core_foundation::{CGPoint, CGRect, CGSize};
    use objc2_quartz_core::{CALayer, CAMetalLayer, CATransaction, kCAGravityResize};

    use super::PageLayer;
    use crate::import::{GfxError, NativeFrame};

    fn rect([x, y, w, h]: [f64; 4]) -> CGRect {
        CGRect::new(CGPoint::new(x, y), CGSize::new(w, h))
    }

    /// `change` in one transaction with the implicit animations off: a moved layer or new
    /// contents would otherwise animate for a quarter second.
    fn transaction(change: impl FnOnce()) {
        CATransaction::begin();
        CATransaction::setDisableActions(true);
        change();
        CATransaction::commit();
    }

    /// A plain `CALayer` over the shell's `CAMetalLayer`, its contents the page's IOSurface.
    /// Main thread only (it is not `Send`).
    pub struct CaLayer {
        layer: Retained<CALayer>,
        root: Retained<CALayer>,
        /// The buffer on the layer and the one before it: the compositor may still show that
        /// one until the transaction that replaced it lands, so neither is copied into again
        /// (welding's pool counts a live buffer as in use).
        shown: [Option<welding::OsBuffer>; 2],
    }

    impl CaLayer {
        /// A hidden layer over the `CAMetalLayer` in `ns_view`'s layer.
        ///
        /// # Safety
        ///
        /// `ns_view` is a live `NSView` (winit's view of the window), and outlives the layer.
        pub unsafe fn new(ns_view: NonNull<c_void>) -> Result<Self, GfxError> {
            let fail = |why: &str| GfxError::Layer(why.into());
            MainThreadMarker::new().ok_or_else(|| fail("the page layer is made off the main thread"))?;
            // SAFETY: the caller's view; `-[NSView layer]` returns its (optional) layer.
            let view: &AnyObject = unsafe { ns_view.cast::<AnyObject>().as_ref() };
            let root: Option<Retained<CALayer>> = unsafe { msg_send![view, layer] };
            let root = root.ok_or_else(|| fail("the window's view has no layer"))?;
            // SAFETY: `sublayers` is read on the main thread, where it is changed.
            let metal = unsafe { root.sublayers() }
                .map(|layers| layers.to_vec())
                .unwrap_or_default()
                .into_iter()
                .find(|layer| layer.isKindOfClass(CAMetalLayer::class()))
                .ok_or_else(|| fail("the window's view has no CAMetalLayer"))?;
            let layer = CALayer::new();
            transaction(|| {
                layer.setHidden(true);
                layer.setOpaque(true);
                // SAFETY: Core Animation's own constant.
                layer.setContentsGravity(unsafe { kCAGravityResize });
                root.insertSublayer_above(&layer, Some(&*metal));
            });
            Ok(Self {
                layer,
                root,
                shown: [None, None],
            })
        }
    }

    impl PageLayer for CaLayer {
        fn root(&self) -> (f64, bool) {
            (self.root.bounds().size.height, self.root.isGeometryFlipped())
        }

        fn place(&mut self, frame: [f64; 4], contents: [f64; 4], scale: f64, radius: f64) {
            transaction(|| {
                self.layer.setFrame(rect(frame));
                self.layer.setContentsRect(rect(contents));
                self.layer.setContentsScale(scale);
                self.layer.setCornerRadius(radius);
                self.layer.setMasksToBounds(radius > 0.0);
            });
        }

        fn show(&mut self, frame: &NativeFrame) -> Result<(), GfxError> {
            let buffer = frame
                .os_buffer()
                .ok_or_else(|| GfxError::Layer("the frame has no OS buffer".into()))?;
            // SAFETY: an IOSurfaceRef is an Objective-C object (toll-free bridged with
            // `IOSurface`), which `contents` takes and retains; `buffer` keeps it alive here.
            let surface: &AnyObject = unsafe { buffer.io_surface().cast::<AnyObject>().as_ref() };
            transaction(|| {
                // SAFETY: an IOSurface is one of the kinds of `contents` Core Animation draws.
                unsafe { self.layer.setContents(Some(surface)) };
                self.layer.setHidden(false);
            });
            let before = self.shown[0].replace(buffer.clone());
            self.shown[1] = before;
            Ok(())
        }

        fn hide(&mut self) {
            transaction(|| {
                self.layer.setHidden(true);
                // SAFETY: no contents is always valid.
                unsafe { self.layer.setContents(None) };
            });
            self.shown = [None, None];
        }
    }

    impl Drop for CaLayer {
        fn drop(&mut self) {
            transaction(|| self.layer.removeFromSuperlayer());
        }
    }
}
