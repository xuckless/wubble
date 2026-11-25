//! The page on a layer of the OS compositor (`wubble_core::page_layer`): a frame copied into an
//! OS buffer (welding's `OsBuffer`, an IOSurface on macOS) is shown by the compositor as it is,
//! with no presenter blit and no shell render. One trait, [`PageLayer`]; macOS implements it
//! with a `CALayer` ([`CaLayer`]), every other platform has none ([`Unsupported`]).
//!
//! macOS: the layer sits beside the shell's `CAMetalLayer` (wgpu's, a sublayer of winit's view's
//! layer): over it in stage 1, shown only while the page is the window's whole content (page
//! fullscreen, the picture-in-picture window), so the shell's frame under it is never seen; under
//! it in stage 2, where the shell clears a hole over the pane (`insertSublayer:below:`). Every
//! change has its implicit animations off, on the main thread, in a `CATransaction` of its own,
//! committed and flushed at once; inside a render whose drawable is presented with the
//! transaction (a hide or a move must land with its frame, [`PageLayer::join_transaction`]) it
//! joins that transaction instead, so the change and the shell's frame that matches it land
//! together. Never by `presentsWithTransaction` alone: that says what the next render does, and
//! a change between renders joined to it would wait, unflushed, while later drawables presented
//! on their own show the hole without it.

use crate::import::{GfxError, NativeFrame};

/// A page's layer in the compositor.
pub trait PageLayer {
    /// The layer tree it sits in: its height in points, and whether its origin is top left
    /// (`wubble_core::page_layer::place` takes both).
    fn root(&self) -> (f64, bool);

    /// Where the layer sits (`frame`: x, y, width, height in points), what of its buffer it
    /// shows (`contents`: the same in the buffer's unit square), the buffer's pixels per point
    /// and the corner radius in points. A placement the layer already has commits nothing (the
    /// host places it at each frame, and at each render while the shell draws its hole). Whether
    /// it moved.
    fn place(&mut self, frame: [f64; 4], contents: [f64; 4], scale: f64, radius: f64) -> bool;

    /// `frame`'s OS buffer onto the layer, shown. Fails for a frame copied without one.
    fn show(&mut self, frame: &NativeFrame) -> Result<(), GfxError>;

    /// Hides the layer and lets its buffers go.
    fn hide(&mut self);

    /// While `on`, the shell's drawable is shown with the Core Animation transaction that
    /// carries this layer's changes (`presentsWithTransaction`), so a hide or a move and the
    /// shell's frame that matches it land together (`wubble_core::page_layer::transacts`: only
    /// then; it makes the main thread wait for the shell's GPU work). Read when the drawable is
    /// acquired: set before the render that needs it. Setting the value it has does nothing.
    fn present_in_transaction(&mut self, _on: bool) {}

    /// While `on`, this layer's changes join the Core Animation transaction the render now
    /// running presents its drawable with (the render acquired it with
    /// [`Self::present_in_transaction`] on); else each commits on its own at once. Told at a
    /// render's start and after its present.
    fn join_transaction(&mut self, _on: bool) {}
}

/// No layer: every frame goes through the shell.
pub struct Unsupported;

impl PageLayer for Unsupported {
    fn root(&self) -> (f64, bool) {
        (0.0, true)
    }

    fn place(&mut self, _frame: [f64; 4], _contents: [f64; 4], _scale: f64, _radius: f64) -> bool {
        false
    }

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
    use objc2::runtime::AnyObject;
    use objc2::{MainThreadMarker, msg_send};
    use objc2_core_foundation::{CGPoint, CGRect, CGSize};
    use objc2_quartz_core::{CALayer, CAMetalLayer, CATransaction, kCAGravityResize};

    use super::PageLayer;
    use crate::import::{GfxError, NativeFrame};

    fn rect([x, y, w, h]: [f64; 4]) -> CGRect {
        CGRect::new(CGPoint::new(x, y), CGSize::new(w, h))
    }

    /// `change` with the implicit animations off (a moved layer or new contents would otherwise
    /// animate for a quarter second): in the transaction the running render's drawable is
    /// presented with while it `join`s one (it lands with the shell's frame), else in one of its
    /// own, flushed: an implicit transaction already open in this run-loop turn would otherwise
    /// hold the nested commit until the turn ends, which a busy main thread delays.
    fn transaction(join: bool, change: impl FnOnce()) {
        if join {
            CATransaction::setDisableActions(true);
            change();
            return;
        }
        CATransaction::begin();
        CATransaction::setDisableActions(true);
        change();
        CATransaction::commit();
        CATransaction::flush();
    }

    /// A plain `CALayer` over or under the shell's `CAMetalLayer`, its contents the page's
    /// IOSurface. Main thread only (it is not `Send`).
    pub struct CaLayer {
        layer: Retained<CALayer>,
        root: Retained<CALayer>,
        /// The shell's layer (wgpu's): its `presentsWithTransaction` is set from here.
        metal: Retained<CAMetalLayer>,
        /// Changes join the running render's transaction (`join_transaction`).
        join: bool,
        /// The buffer on the layer and the one before it: the compositor may still show that
        /// one until the transaction that replaced it lands, so neither is copied into again
        /// (welding's pool counts a live buffer as in use).
        shown: [Option<welding::OsBuffer>; 2],
    }

    impl CaLayer {
        /// A hidden layer over the `CAMetalLayer` in `ns_view`'s layer, or under it (`below`: the
        /// shell clears a hole where the page shows).
        ///
        /// # Safety
        ///
        /// `ns_view` is a live `NSView` (winit's view of the window), and outlives the layer.
        pub unsafe fn new(ns_view: NonNull<c_void>, below: bool) -> Result<Self, GfxError> {
            let fail = |why: &str| GfxError::Layer(why.into());
            MainThreadMarker::new()
                .ok_or_else(|| fail("the page layer is made off the main thread"))?;
            // SAFETY: the caller's view; `-[NSView layer]` returns its (optional) layer.
            let view: &AnyObject = unsafe { ns_view.cast::<AnyObject>().as_ref() };
            let root: Option<Retained<CALayer>> = unsafe { msg_send![view, layer] };
            let root = root.ok_or_else(|| fail("the window's view has no layer"))?;
            // SAFETY: `sublayers` is read on the main thread, where it is changed.
            let metal = unsafe { root.sublayers() }
                .map(|layers| layers.to_vec())
                .unwrap_or_default()
                .into_iter()
                .find_map(|layer| layer.downcast::<CAMetalLayer>().ok())
                .ok_or_else(|| fail("the window's view has no CAMetalLayer"))?;
            let layer = CALayer::new();
            transaction(false, || {
                layer.setHidden(true);
                layer.setOpaque(true);
                // SAFETY: Core Animation's own constant.
                layer.setContentsGravity(unsafe { kCAGravityResize });
                // a later one goes next to the shell's, over the earlier ones (none overlap)
                if below {
                    root.insertSublayer_below(&layer, Some(&**metal));
                } else {
                    root.insertSublayer_above(&layer, Some(&**metal));
                }
            });
            Ok(Self {
                layer,
                root,
                metal,
                join: false,
                shown: [None, None],
            })
        }
    }

    impl PageLayer for CaLayer {
        fn root(&self) -> (f64, bool) {
            (
                self.root.bounds().size.height,
                self.root.isGeometryFlipped(),
            )
        }

        fn place(&mut self, frame: [f64; 4], contents: [f64; 4], scale: f64, radius: f64) -> bool {
            let (frame, contents) = (rect(frame), rect(contents));
            let l = &self.layer;
            let same = l.frame() == frame
                && l.contentsRect() == contents
                && l.contentsScale() == scale
                && l.cornerRadius() == radius;
            if same {
                return false;
            }
            transaction(self.join, || {
                self.layer.setFrame(frame);
                self.layer.setContentsRect(contents);
                self.layer.setContentsScale(scale);
                self.layer.setCornerRadius(radius);
                self.layer.setMasksToBounds(radius > 0.0);
            });
            true
        }

        fn show(&mut self, frame: &NativeFrame) -> Result<(), GfxError> {
            let buffer = frame
                .os_buffer()
                .ok_or_else(|| GfxError::Layer("the frame has no OS buffer".into()))?;
            // SAFETY: an IOSurfaceRef is an Objective-C object (toll-free bridged with
            // `IOSurface`), which `contents` takes and retains; `buffer` keeps it alive here.
            let surface: &AnyObject = unsafe { buffer.io_surface().cast::<AnyObject>().as_ref() };
            transaction(self.join, || {
                // SAFETY: an IOSurface is one of the kinds of `contents` Core Animation draws.
                unsafe { self.layer.setContents(Some(surface)) };
                self.layer.setHidden(false);
            });
            let before = self.shown[0].replace(buffer.clone());
            self.shown[1] = before;
            Ok(())
        }

        fn hide(&mut self) {
            let hide = || {
                self.layer.setHidden(true);
                // SAFETY: no contents is always valid.
                unsafe { self.layer.setContents(None) };
            };
            // in a render presented with the transaction, the layer goes in the same commit as
            // the frame that shows the page instead
            transaction(self.join, hide);
            self.shown = [None, None];
        }

        fn present_in_transaction(&mut self, on: bool) {
            if self.metal.presentsWithTransaction() != on {
                self.metal.setPresentsWithTransaction(on);
            }
        }

        fn join_transaction(&mut self, on: bool) {
            self.join = on;
        }
    }

    impl Drop for CaLayer {
        fn drop(&mut self) {
            transaction(self.join, || self.layer.removeFromSuperlayer());
        }
    }
}
