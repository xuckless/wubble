//! The page on a layer of the OS compositor (`wubble_core::page_layer`): a frame copied into an
//! OS buffer (welding's `OsBuffer`: an IOSurface on macOS, an exported dmabuf on Linux) is shown
//! by the compositor as it is, with no presenter blit and no shell render. One trait,
//! [`PageLayer`]; macOS implements it with a `CALayer` ([`CaLayer`]), Wayland with a
//! `wl_subsurface` ([`Subsurface`]), every other platform (and X11) has none ([`Unsupported`]).
//!
//! macOS: the layer sits beside the shell's `CAMetalLayer` (wgpu's, a sublayer of winit's view's
//! layer): over it in stage 1, shown only while the page is the window's whole content (page
//! fullscreen, the picture-in-picture window), so the shell's frame under it is never seen; under
//! it in stage 2, where the shell clears a hole over the pane (`insertSublayer:below:`). Every
//! change has its implicit animations off, on the main thread, in a `CATransaction` of its own;
//! while the shell presents with the transaction (a layer of the window is visible) it joins the
//! shell's instead, so a hide, a move and the shell's frame that matches them land together.

use crate::import::{GfxError, NativeFrame};

/// A page's layer in the compositor.
pub trait PageLayer {
    /// The layer tree it sits in: its height in points, and whether its origin is top left
    /// (`wubble_core::page_layer::place` takes both).
    fn root(&self) -> (f64, bool);

    /// Where the layer sits (`frame`: x, y, width, height in points), what of its buffer it
    /// shows (`contents`: the same in the buffer's unit square), the buffer's pixels per point
    /// and the corner radius in points. A placement the layer already has commits nothing (the
    /// host places it at each frame, and at each render while the shell draws its hole).
    fn place(&mut self, frame: [f64; 4], contents: [f64; 4], scale: f64, radius: f64);

    /// `frame`'s OS buffer onto the layer, shown. Fails for a frame copied without one.
    fn show(&mut self, frame: &NativeFrame) -> Result<(), GfxError>;

    /// Hides the layer and lets its buffers go.
    fn hide(&mut self);

    /// While `on`, the shell's drawable is shown with the Core Animation transaction that
    /// carries this layer's changes (`presentsWithTransaction`), so a hide and the shell's frame
    /// that replaces it land together. Read when the drawable is acquired: set before the render
    /// that needs it, cleared after the hide's drawable went out (the next render's start). Setting
    /// the value it has does nothing. Wayland needs none: a subsurface's place is the shell
    /// surface's state (it lands with the shell's commit anyway), and the layer itself syncs a
    /// hide and a change of its viewport with the shell's next commit, page frames unsynced.
    fn present_in_transaction(&mut self, _on: bool) {}

    /// The (DRM fourcc, modifier) pairs the compositor takes the layer's buffers in, for the
    /// view's OS buffers; empty where the OS buffers need none (macOS).
    fn buffer_formats(&self) -> Vec<(u32, u64)> {
        Vec::new()
    }

    /// The compositor places the layer in whole points (Wayland's integer logical coordinates):
    /// its frame is rounded outwards first (`wubble_core::page_layer::covering`).
    fn whole_points(&self) -> bool {
        false
    }
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
    /// animate for a quarter second): in the transaction the shell's drawable is presented with
    /// while `metal` presents with one (it lands with the shell's frame), else in one of its own.
    fn transaction(metal: &CAMetalLayer, change: impl FnOnce()) {
        if metal.presentsWithTransaction() {
            CATransaction::setDisableActions(true);
            change();
            return;
        }
        CATransaction::begin();
        CATransaction::setDisableActions(true);
        change();
        CATransaction::commit();
    }

    /// A plain `CALayer` over or under the shell's `CAMetalLayer`, its contents the page's
    /// IOSurface. Main thread only (it is not `Send`).
    pub struct CaLayer {
        layer: Retained<CALayer>,
        root: Retained<CALayer>,
        /// The shell's layer (wgpu's): its `presentsWithTransaction` is set from here.
        metal: Retained<CAMetalLayer>,
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
            transaction(&metal, || {
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

        fn place(&mut self, frame: [f64; 4], contents: [f64; 4], scale: f64, radius: f64) {
            let (frame, contents) = (rect(frame), rect(contents));
            let l = &self.layer;
            let same = l.frame() == frame
                && l.contentsRect() == contents
                && l.contentsScale() == scale
                && l.cornerRadius() == radius;
            if same {
                return;
            }
            transaction(&self.metal, || {
                self.layer.setFrame(frame);
                self.layer.setContentsRect(contents);
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
            transaction(&self.metal, || {
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
            // while the shell's drawable is presented with the current transaction, the layer
            // goes in the same commit as the frame that shows the page instead
            transaction(&self.metal, hide);
            self.shown = [None, None];
        }

        fn present_in_transaction(&mut self, on: bool) {
            if self.metal.presentsWithTransaction() != on {
                self.metal.setPresentsWithTransaction(on);
            }
        }
    }

    impl Drop for CaLayer {
        fn drop(&mut self) {
            transaction(&self.metal, || self.layer.removeFromSuperlayer());
        }
    }
}

#[cfg(target_os = "linux")]
pub use wayland::{Subsurface, read_compositor};

/// Wayland: one `wl_subsurface` per pane under the shell's `wl_surface` (winit's), showing the
/// view's OS buffers as `zwp_linux_dmabuf_v1` buffers, scaled by `wp_viewport`, with an empty
/// input region (input stays the shell's: Chromium's `wayland_subsurface.cc`). Page frames
/// commit unsynced, so the shell does not render for them; a hide and a viewport change are
/// synced, so they land with the shell's next commit (the frame that draws the page there, or
/// the hole). The shape is `layer_probe`'s (docs/research/linux-page-layer.md section 7), which
/// ran on NVIDIA under mutter.
#[cfg(target_os = "linux")]
mod wayland {
    use std::cell::RefCell;
    use std::ffi::c_void;
    use std::ptr::NonNull;
    use std::rc::{Rc, Weak};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

    use wayland_client::backend::{Backend, ObjectId};
    use wayland_client::globals::{GlobalListContents, registry_queue_init};
    use wayland_client::protocol::wl_buffer::{self, WlBuffer};
    use wayland_client::protocol::wl_compositor::WlCompositor;
    use wayland_client::protocol::wl_region::WlRegion;
    use wayland_client::protocol::wl_registry::WlRegistry;
    use wayland_client::protocol::wl_subcompositor::WlSubcompositor;
    use wayland_client::protocol::wl_subsurface::WlSubsurface;
    use wayland_client::protocol::wl_surface::WlSurface;
    use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle, delegate_noop};
    use wayland_protocols::wp::linux_dmabuf::zv1::client::zwp_linux_buffer_params_v1::{
        self, ZwpLinuxBufferParamsV1,
    };
    use wayland_protocols::wp::linux_dmabuf::zv1::client::zwp_linux_dmabuf_v1::{
        self, ZwpLinuxDmabufV1,
    };
    use wayland_protocols::wp::viewporter::client::wp_viewport::WpViewport;
    use wayland_protocols::wp::viewporter::client::wp_viewporter::WpViewporter;

    use super::PageLayer;
    use crate::import::{GfxError, NativeFrame};

    /// What the layers' queue hears: the compositor's formats and modifiers (v3 events);
    /// buffer releases go straight to their slot's hold.
    #[derive(Default)]
    struct Heard {
        formats: Vec<(u32, u64)>,
    }

    impl Dispatch<WlRegistry, GlobalListContents> for Heard {
        fn event(
            _: &mut Self,
            _: &WlRegistry,
            _: <WlRegistry as Proxy>::Event,
            _: &GlobalListContents,
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
        }
    }

    impl Dispatch<ZwpLinuxDmabufV1, ()> for Heard {
        fn event(
            heard: &mut Self,
            _: &ZwpLinuxDmabufV1,
            event: zwp_linux_dmabuf_v1::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            if let zwp_linux_dmabuf_v1::Event::Modifier {
                format,
                modifier_hi,
                modifier_lo,
            } = event
            {
                let modifier = (u64::from(modifier_hi) << 32) | u64::from(modifier_lo);
                heard.formats.push((format, modifier));
            }
        }
    }

    /// The compositor let the buffer go: its slot may be copied into again.
    impl Dispatch<WlBuffer, Arc<AtomicBool>> for Heard {
        fn event(
            _: &mut Self,
            _: &WlBuffer,
            event: wl_buffer::Event,
            hold: &Arc<AtomicBool>,
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            if let wl_buffer::Event::Release = event {
                hold.store(false, Ordering::Release);
            }
        }
    }

    delegate_noop!(Heard: WlCompositor);
    delegate_noop!(Heard: WlSubcompositor);
    delegate_noop!(Heard: WlSubsurface);
    delegate_noop!(Heard: WlRegion);
    delegate_noop!(Heard: WpViewporter);
    delegate_noop!(Heard: WpViewport);
    delegate_noop!(Heard: ignore ZwpLinuxBufferParamsV1);
    delegate_noop!(Heard: ignore WlSurface);

    /// Which connection a slot's `wl_buffer` was made on (a later one makes its own).
    static CONNECTIONS: AtomicU64 = AtomicU64::new(0);

    /// The process's layers' side of winit's Wayland connection: one queue, the globals, what
    /// the compositor said. Shared by every layer while one lives; the slots' buffers are made
    /// on its queue, so their releases are always heard.
    struct Compositor {
        conn: Connection,
        queue: RefCell<EventQueue<Heard>>,
        heard: RefCell<Heard>,
        compositor: WlCompositor,
        subcompositor: WlSubcompositor,
        viewporter: WpViewporter,
        dmabuf: ZwpLinuxDmabufV1,
        serial: u64,
    }

    thread_local! {
        static SHARED: RefCell<Weak<Compositor>> = const { RefCell::new(Weak::new()) };
    }

    impl Compositor {
        /// The live one, or one made on `display`.
        ///
        /// # Safety
        ///
        /// `display` is winit's live `wl_display`, which outlives every layer.
        unsafe fn get(display: NonNull<c_void>) -> Result<Rc<Self>, GfxError> {
            if let Some(shared) = SHARED.with(|s| s.borrow().upgrade()) {
                return Ok(shared);
            }
            let fail =
                |what: &str, e: &dyn std::fmt::Display| GfxError::Layer(format!("{what}: {e}"));
            // SAFETY: the caller's display.
            let conn = Connection::from_backend(unsafe {
                Backend::from_foreign_display(display.as_ptr().cast())
            });
            let (globals, mut queue) =
                registry_queue_init::<Heard>(&conn).map_err(|e| fail("the registry", &e))?;
            let qh = queue.handle();
            let compositor: WlCompositor = globals
                .bind(&qh, 4..=6, ())
                .map_err(|e| fail("wl_compositor", &e))?;
            let subcompositor: WlSubcompositor = globals
                .bind(&qh, 1..=1, ())
                .map_err(|e| fail("wl_subcompositor", &e))?;
            let viewporter: WpViewporter = globals
                .bind(&qh, 1..=1, ())
                .map_err(|e| fail("wp_viewporter", &e))?;
            // v3: the formats and modifiers come as events (v4 sends a table); enough on mutter
            let dmabuf: ZwpLinuxDmabufV1 = globals
                .bind(&qh, 3..=3, ())
                .map_err(|e| fail("zwp_linux_dmabuf_v1", &e))?;
            let mut heard = Heard::default();
            queue
                .roundtrip(&mut heard)
                .map_err(|e| fail("the first roundtrip", &e))?;
            let shared = Rc::new(Self {
                conn,
                queue: RefCell::new(queue),
                heard: RefCell::new(heard),
                compositor,
                subcompositor,
                viewporter,
                dmabuf,
                serial: CONNECTIONS.fetch_add(1, Ordering::Relaxed),
            });
            SHARED.with(|s| *s.borrow_mut() = Rc::downgrade(&shared));
            Ok(shared)
        }

        fn handle(&self) -> QueueHandle<Heard> {
            self.queue.borrow().handle()
        }

        /// What arrived for the layers (buffer releases), without blocking: the display's other
        /// readers (winit's loop, the clipboard's and wubble's Wayland threads) read the socket
        /// and queue the layers' events here. Reading it from here too waits, in
        /// `wl_display_read_events`, for every reader that has prepared one, and those sit in
        /// `poll` until the compositor sends something: with the main thread waiting, it never did.
        fn read(&self) {
            let (Ok(mut queue), Ok(mut heard)) =
                (self.queue.try_borrow_mut(), self.heard.try_borrow_mut())
            else {
                return;
            };
            let _ = queue.dispatch_pending(&mut *heard);
        }

        fn flush(&self) {
            let _ = self.conn.flush();
        }
    }

    /// The compositor's events for every page layer (buffer releases), read without blocking:
    /// before CEF paints, so its copy finds the slots the compositor let go. Nothing without a
    /// layer.
    pub fn read_compositor() {
        if let Some(shared) = SHARED.with(|s| s.borrow().upgrade()) {
            shared.read();
        }
    }

    /// A slot's `wl_buffer`, kept in the slot (welding's `OsBuffer::host_data`) and destroyed
    /// with it.
    struct SlotBuffer {
        serial: u64,
        buffer: WlBuffer,
    }

    impl Drop for SlotBuffer {
        fn drop(&mut self) {
            self.buffer.destroy();
        }
    }

    /// A page's `wl_subsurface` under the shell's surface. Main thread only.
    pub struct Subsurface {
        shared: Rc<Compositor>,
        surface: WlSurface,
        sub: WlSubsurface,
        viewport: WpViewport,
        /// Where it is (whole points) and what of its buffer it shows (unit square).
        placed: Option<([i32; 4], [f64; 4])>,
        /// Its commits wait for the shell's (a new subsurface starts so).
        synced: bool,
        /// The buffer on it (a live clone counts as in use in the pool).
        shown: Option<welding::OsBuffer>,
        /// The holds of every buffer it was given: let go by hand when it goes (mutter may never
        /// release a buffer whose surface went: Chromium's `wayland_frame_manager.cc`).
        holds: Vec<Arc<AtomicBool>>,
    }

    impl Subsurface {
        /// A hidden subsurface of `parent` (the shell window's `wl_surface`) on `display`: under
        /// it (`below`: the shell clears a hole where the page shows) or over it.
        ///
        /// # Safety
        ///
        /// `display` and `parent` are winit's live `wl_display` and the window's `wl_surface`,
        /// which outlive the layer (the host drops a window's layers before the window).
        pub unsafe fn new(
            display: NonNull<c_void>,
            parent: NonNull<c_void>,
            below: bool,
        ) -> Result<Self, GfxError> {
            // SAFETY: the caller's display.
            let shared = unsafe { Compositor::get(display) }?;
            // SAFETY: the caller's live surface.
            let parent =
                unsafe { ObjectId::from_ptr(WlSurface::interface(), parent.as_ptr().cast()) }
                    .ok()
                    .and_then(|id| WlSurface::from_id(&shared.conn, id).ok())
                    .ok_or_else(|| GfxError::Layer("the window's wl_surface".into()))?;
            let qh = shared.handle();
            let surface = shared.compositor.create_surface(&qh, ());
            let sub = shared
                .subcompositor
                .get_subsurface(&surface, &parent, &qh, ());
            // a later one goes next to the shell's, past the earlier ones (none overlap)
            if below {
                sub.place_below(&parent);
            } else {
                sub.place_above(&parent);
            }
            let region = shared.compositor.create_region(&qh, ());
            surface.set_input_region(Some(&region));
            region.destroy();
            let viewport = shared.viewporter.get_viewport(&surface, &qh, ());
            shared.flush();
            Ok(Self {
                shared,
                surface,
                sub,
                viewport,
                placed: None,
                synced: true,
                shown: None,
                holds: Vec::new(),
            })
        }

        fn sync(&mut self, on: bool) {
            if self.synced == on {
                return;
            }
            if on {
                self.sub.set_sync();
            } else {
                self.sub.set_desync();
            }
            self.synced = on;
        }

        /// The viewport's source: the placed part of the shown buffer, in its pixels (inside it:
        /// a source past the buffer is a protocol error).
        fn set_source(&self) {
            let (Some((_, [x, y, w, h])), Some(buffer)) = (self.placed, &self.shown) else {
                return;
            };
            let size = buffer.size();
            let (bw, bh) = (f64::from(size.width.max(1)), f64::from(size.height.max(1)));
            let (sx, sy) = ((x * bw).clamp(0.0, bw - 1.0), (y * bh).clamp(0.0, bh - 1.0));
            let (sw, sh) = ((w * bw).clamp(1.0, bw - sx), (h * bh).clamp(1.0, bh - sy));
            self.viewport.set_source(sx, sy, sw, sh);
        }

        /// The slot's `wl_buffer` on this connection, made once.
        fn wl_buffer(&self, buffer: &welding::OsBuffer) -> WlBuffer {
            let mut data = buffer.host_data().lock().unwrap_or_else(|e| e.into_inner());
            let made = data
                .as_ref()
                .and_then(|d| d.downcast_ref::<SlotBuffer>())
                .filter(|b| b.serial == self.shared.serial)
                .map(|b| b.buffer.clone());
            if let Some(made) = made {
                return made;
            }
            let qh = self.shared.handle();
            let params = self.shared.dmabuf.create_params(&qh, ());
            let ((offset, stride), (fourcc, modifier), size) =
                (buffer.offset_stride(), buffer.drm_format(), buffer.size());
            params.add(
                buffer.fd(),
                0,
                offset,
                stride,
                (modifier >> 32) as u32,
                modifier as u32,
            );
            let made = params.create_immed(
                size.width as i32,
                size.height as i32,
                fourcc,
                zwp_linux_buffer_params_v1::Flags::empty(),
                &qh,
                buffer.compositor_hold().clone(),
            );
            params.destroy();
            *data = Some(Box::new(SlotBuffer {
                serial: self.shared.serial,
                buffer: made.clone(),
            }));
            made
        }
    }

    impl PageLayer for Subsurface {
        /// Top left, as winit's surface; the height is not needed then.
        fn root(&self) -> (f64, bool) {
            (0.0, true)
        }

        fn place(&mut self, frame: [f64; 4], contents: [f64; 4], _scale: f64, _radius: f64) {
            // whole points already (`whole_points`); the corners are the shell's pixels
            let rect = frame.map(|v| v.round() as i32);
            if self.placed == Some((rect, contents)) {
                return;
            }
            let resized =
                self.placed.map(|(r, c)| (r[2], r[3], c)) != Some((rect[2], rect[3], contents));
            // the shell surface's state: lands with its next commit, synced or not
            self.sub.set_position(rect[0], rect[1]);
            self.placed = Some((rect, contents));
            if resized {
                // the layer's own state: synced, so it lands with the shell's frame too
                self.sync(true);
                self.viewport
                    .set_destination(rect[2].max(1), rect[3].max(1));
                self.set_source();
                self.surface.commit();
            }
            self.shared.flush();
        }

        fn show(&mut self, frame: &NativeFrame) -> Result<(), GfxError> {
            let buffer = frame
                .os_buffer()
                .ok_or_else(|| GfxError::Layer("the frame has no OS buffer".into()))?
                .clone();
            self.shared.read();
            let wl = self.wl_buffer(&buffer);
            let hold = buffer.compositor_hold().clone();
            hold.store(true, Ordering::Release);
            if !self.holds.iter().any(|h| Arc::ptr_eq(h, &hold)) {
                self.holds.push(hold);
            }
            let size = buffer.size();
            // a page frame commits on its own: no shell render for it
            self.sync(false);
            self.surface.attach(Some(&wl), 0, 0);
            // the whole buffer: its damage is against the frame before it, which another slot holds
            self.surface
                .damage_buffer(0, 0, size.width as i32, size.height as i32);
            self.shown = Some(buffer);
            self.set_source();
            self.surface.commit();
            self.shared.flush();
            Ok(())
        }

        fn hide(&mut self) {
            // synced: the null buffer lands with the shell's next commit, which draws the page
            // there (the compositor releases the last buffer then)
            self.sync(true);
            self.surface.attach(None, 0, 0);
            self.surface.commit();
            self.shown = None;
            self.shared.flush();
        }

        fn buffer_formats(&self) -> Vec<(u32, u64)> {
            self.shared.heard.borrow().formats.clone()
        }

        fn whole_points(&self) -> bool {
            true
        }
    }

    impl Drop for Subsurface {
        fn drop(&mut self) {
            self.viewport.destroy();
            self.sub.destroy();
            self.surface.destroy();
            for hold in &self.holds {
                hold.store(false, Ordering::Release);
            }
            self.shared.flush();
        }
    }
}
