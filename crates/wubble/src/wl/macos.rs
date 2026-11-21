//! macOS: what winit does not give a window for drags. A page drag that leaves the window
//! becomes an AppKit dragging session (`NSDraggingSource`), and drags from other apps (files,
//! links, HTML, text) reach the page or the shell's targets as on Wayland ([`AuxEvent`]).
//!
//! Ported from Chromium's `content/app_shim_remote_cocoa/web_drag_source_mac.mm` and
//! `web_contents_view_cocoa.mm` (BSD-3-Clause; the shape, no code copied): one dragging item
//! whose pasteboard item holds the link (`public.url` with its title under `public.url-name`),
//! HTML and text, a dragged picture promised as a file of its type (`NSFilePromiseProvider`,
//! written only when the target asks), the drag image Chromium made at the screen's scale
//! with its hotspot under the pointer, the operations the page allows as the source's mask,
//! and the session's end handed to the drag machine (`AuxEvent::DragEnded`). The types and
//! their order are `wubble_core::dnd::pasteboard` (pure, unit-tested on every platform).
//!
//! Drops: winit 0.30 registers the NSWindow for file names only and answers on its window
//! delegate (`HoveredFile`, `DroppedFile`). The view is registered here for files, links, HTML
//! and text, which makes it the destination before the window, and its class is given the four
//! `NSDraggingDestination` methods at the first window (`class_replaceMethod`; winit's view has
//! none of them); each answers from the page's last answer (`Aux::drop_action`), as Chromium's
//! `WebDragDest` answers `draggingUpdated:` from the renderer's last operation. File drops come
//! this way too then, not as winit's events.
//!
//! Coordinates: winit's view is flipped (`isFlipped`: the top left is the origin), so view
//! points are the window's logical px as they are, and a dragging frame's origin is its top
//! left.
//!
//! NOT RUN: written and compile-checked for aarch64-apple-darwin from Linux only; no drag has
//! gone through it on a Mac (TODO.md).

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::sync::Arc;

use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, ProtocolObject, Sel};
use objc2::{
    AnyThread, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel,
};
use objc2_app_kit::{
    NSApplication, NSDragOperation, NSDraggingContext, NSDraggingItem, NSDraggingSession,
    NSDraggingSource, NSEventType, NSFilePromiseProvider, NSFilePromiseProviderDelegate, NSImage,
    NSPasteboard, NSPasteboardItem, NSPasteboardWriting, NSView,
};
use objc2_foundation::{
    NSArray, NSData, NSError, NSObject, NSObjectProtocol, NSPoint, NSRect, NSSize, NSString, NSURL,
};
use wubble_core::dnd::{self, pasteboard};
use wubble_core::window::WindowId;
use wubble_engine::page::{DragData, DragOps};

use super::{Aux, AuxEvent, deliver};

/// The process's drag state on the main thread: the windows' views, the page's answer to a
/// drop from another app, the drag out under way.
#[derive(Default)]
struct State {
    views: HashMap<usize, (WindowId, Retained<NSView>)>,
    /// What a drop at the pointer would do, as the page last said.
    drop_op: Cell<DragOps>,
    /// The drag out's source, kept until AppKit says it ended.
    source: Option<Retained<DragSource>>,
    /// winit's view class had its dragging destination replaced.
    patched: bool,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::default();
}

/// The macOS extras; the state lives on the main thread ([`STATE`]).
#[derive(Default)]
pub struct Mac;

/// The engine's operations as AppKit's.
fn ns_ops(ops: DragOps) -> NSDragOperation {
    let mut out = NSDragOperation::None;
    if ops.has(DragOps::COPY) {
        out |= NSDragOperation::Copy;
    }
    if ops.has(DragOps::LINK) {
        out |= NSDragOperation::Link;
    }
    if ops.has(DragOps::MOVE) {
        out |= NSDragOperation::Move;
    }
    out
}

/// AppKit's operation as the engine's: copy before move before link, as Chromium reads it.
fn engine_op(op: NSDragOperation) -> DragOps {
    if op.contains(NSDragOperation::Copy) {
        DragOps::COPY
    } else if op.contains(NSDragOperation::Move) {
        DragOps::MOVE
    } else if op.contains(NSDragOperation::Link) {
        DragOps::LINK
    } else {
        DragOps::NONE
    }
}

/// The picture a drag out promises: its bytes and its file name.
struct Promised {
    bytes: Arc<[u8]>,
    name: String,
}

struct SourceIvars {
    window: WindowId,
    mask: NSDragOperation,
    picture: Option<Promised>,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements; the ivars are plain Rust values.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "WubbleDragSource"]
    #[ivars = SourceIvars]
    struct DragSource;

    unsafe impl NSObjectProtocol for DragSource {}

    unsafe impl NSDraggingSource for DragSource {
        #[unsafe(method(draggingSession:sourceOperationMaskForDraggingContext:))]
        fn operation_mask(
            &self,
            _session: &NSDraggingSession,
            _context: NSDraggingContext,
        ) -> NSDragOperation {
            self.ivars().mask
        }

        #[unsafe(method(draggingSession:endedAtPoint:operation:))]
        fn ended(&self, _session: &NSDraggingSession, _at: NSPoint, operation: NSDragOperation) {
            let window = self.ivars().window;
            deliver(window, AuxEvent::DragEnded(engine_op(operation)));
            STATE.with(|s| s.borrow_mut().source = None);
        }
    }

    unsafe impl NSFilePromiseProviderDelegate for DragSource {
        #[unsafe(method_id(filePromiseProvider:fileNameForType:))]
        fn file_name(
            &self,
            _provider: &NSFilePromiseProvider,
            _ty: &NSString,
        ) -> Retained<NSString> {
            let name = self
                .ivars()
                .picture
                .as_ref()
                .map_or("image", |p| p.name.as_str());
            NSString::from_str(name)
        }

        // The target asked: the bytes the page loaded go into its file, nothing fetched.
        #[unsafe(method(filePromiseProvider:writePromiseToURL:completionHandler:))]
        fn write(
            &self,
            _provider: &NSFilePromiseProvider,
            url: &NSURL,
            done: &block2::DynBlock<dyn Fn(*mut NSError)>,
        ) {
            let written = match (&self.ivars().picture, url.path()) {
                (Some(p), Some(path)) => std::fs::write(path.to_string(), &p.bytes).is_ok(),
                _ => false,
            };
            if !written {
                eprintln!("wubble: a dragged picture could not be written where it was dropped");
            }
            // AppKit reads a null error as success; a failed write leaves no file behind
            done.call((std::ptr::null_mut(),));
        }
    }
);

impl DragSource {
    fn new(mtm: MainThreadMarker, ivars: SourceIvars) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(ivars);
        // SAFETY: NSObject's designated initialiser.
        unsafe { msg_send![super(this), init] }
    }
}

/// The view of window `id`, with its window.
fn view_of(id: WindowId) -> Option<Retained<NSView>> {
    STATE.with(|s| {
        s.borrow()
            .views
            .values()
            .find(|(w, _)| *w == id)
            .map(|(_, v)| v.clone())
    })
}

/// The window whose view is `view`.
fn window_of(view: &AnyObject) -> Option<WindowId> {
    let key = view as *const AnyObject as usize;
    STATE.with(|s| s.borrow().views.get(&key).map(|(w, _)| *w))
}

/// The pasteboard item and the promise a drag out carries.
fn pasteboard_writers(
    data: &DragData,
    source: &DragSource,
) -> Vec<Retained<ProtocolObject<dyn NSPasteboardWriting>>> {
    let picture = source.ivars().picture.is_some();
    let out = dnd::Outgoing {
        uris: !data.files.is_empty() || data.link_url.is_some(),
        link: data.link_url.is_some(),
        picture: data
            .file_contents
            .as_ref()
            .and_then(|c| dnd::sniff_image_mime(&c.bytes))
            .filter(|_| picture),
        html: data.html.is_some(),
        text: data.text.is_some() || data.link_url.is_some(),
    };
    let item = NSPasteboardItem::new();
    let mut writers: Vec<Retained<ProtocolObject<dyn NSPasteboardWriting>>> = vec![];
    let mut strings = 0;
    for write in pasteboard::writes(&out) {
        match write {
            pasteboard::Item::Files => {
                for path in &data.files {
                    let url = NSURL::from_file_path(path);
                    if let Some(url) = url {
                        writers.push(ProtocolObject::from_retained(url));
                    }
                }
            }
            pasteboard::Item::String(ty, from) => {
                let value = match from {
                    pasteboard::Source::Link => data.link_url.clone(),
                    pasteboard::Source::Title => {
                        data.link_title.clone().or_else(|| data.link_url.clone())
                    }
                    pasteboard::Source::Html => data.html.clone(),
                    pasteboard::Source::Text => data.text.clone().or_else(|| data.link_url.clone()),
                };
                if let Some(value) = value {
                    item.setString_forType(&NSString::from_str(&value), &NSString::from_str(ty));
                    strings += 1;
                }
            }
            pasteboard::Item::Promise(uti) => {
                let promise = NSFilePromiseProvider::initWithFileType_delegate(
                    NSFilePromiseProvider::alloc(),
                    &NSString::from_str(uti),
                    ProtocolObject::from_ref(source),
                );
                writers.push(ProtocolObject::from_retained(promise));
            }
        }
    }
    if strings > 0 {
        writers.insert(0, ProtocolObject::from_retained(item));
    }
    writers
}

impl Aux for Mac {
    fn add_window(&self, id: WindowId, window: &slint::Window) {
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        use slint::winit_030::WinitWindowAccessor;
        let view = window
            .with_winit_window(|w| match w.window_handle().ok()?.as_raw() {
                RawWindowHandle::AppKit(h) => Some(h.ns_view.as_ptr() as *mut NSView),
                _ => None,
            })
            .flatten();
        // SAFETY: winit's NSView for this window, alive while the window is.
        let Some(view) = view.and_then(|v| unsafe { Retained::retain(v) }) else {
            return;
        };
        let key = Retained::as_ptr(&view) as usize;
        let class = view.class();
        // links, HTML and text too, beside winit's files
        let types = NSArray::from_retained_slice(&[
            NSString::from_str(pasteboard::FILE_URL),
            NSString::from_str(pasteboard::URL),
            NSString::from_str(pasteboard::HTML),
            NSString::from_str(pasteboard::TEXT),
        ]);
        view.registerForDraggedTypes(&types);
        STATE.with(|s| {
            let mut s = s.borrow_mut();
            if !s.patched {
                s.patched = true;
                patch_destination(class);
            }
            s.views.insert(key, (id, view));
        });
    }

    fn remove_window(&self, id: WindowId) {
        STATE.with(|s| s.borrow_mut().views.retain(|_, (w, _)| *w != id));
    }

    fn start_drag(&self, id: WindowId, data: &DragData, allowed: DragOps) -> bool {
        let Some(mtm) = MainThreadMarker::new() else {
            return false;
        };
        let (Some(view), Some(event)) = (
            view_of(id),
            NSApplication::sharedApplication(mtm).currentEvent(),
        ) else {
            return false;
        };
        let picture = data.file_contents.as_ref().and_then(|c| {
            let mime = dnd::sniff_image_mime(&c.bytes)?;
            pasteboard::image_uti(mime)?;
            Some(Promised {
                bytes: c.bytes.clone(),
                name: dnd::drag_file_name(c.name.as_deref(), Some(mime)),
            })
        });
        let source = DragSource::new(
            mtm,
            SourceIvars {
                window: id,
                mask: ns_ops(allowed),
                picture,
            },
        );
        // AppKit starts a session only from a mouse event: the drag left the window during
        // winit's handling of a `mouseDragged:` (winit reports motion outside the view while a
        // button is held), which is the application's current event then
        if !matches!(
            event.r#type(),
            NSEventType::LeftMouseDragged | NSEventType::LeftMouseDown
        ) {
            return false;
        }
        let writers = pasteboard_writers(data, &source);
        if writers.is_empty() {
            return false;
        }
        // The pointer in the (flipped) view, and the image's frame there with Chromium's hotspot
        // under it; the image at its DIP size, so a 2x PNG stays sharp.
        let at = view.convertPoint_fromView(event.locationInWindow(), None);
        let image = data.image.as_ref().and_then(|i| {
            let png = NSData::with_bytes(&i.png);
            let image = NSImage::initWithData(NSImage::alloc(), &png)?;
            let size = NSSize::new(f64::from(i.size.0), f64::from(i.size.1));
            image.setSize(size);
            let origin = NSPoint::new(at.x - f64::from(i.hotspot.0), at.y - f64::from(i.hotspot.1));
            Some((image, NSRect::new(origin, size)))
        });
        let items: Vec<Retained<NSDraggingItem>> = writers
            .iter()
            .enumerate()
            .map(|(n, writer)| {
                let item =
                    NSDraggingItem::initWithPasteboardWriter(NSDraggingItem::alloc(), writer);
                // the first item carries the image; the others sit under it, unseen
                let (frame, contents) = match (&image, n) {
                    (Some((image, frame)), 0) => (*frame, Some(&**image as &AnyObject)),
                    (Some((_, frame)), _) => (*frame, None),
                    (None, _) => (NSRect::new(at, NSSize::new(1.0, 1.0)), None),
                };
                // SAFETY: `contents` is an NSImage or nothing, as the method takes.
                unsafe { item.setDraggingFrame_contents(frame, contents) };
                item
            })
            .collect();
        let items = NSArray::from_retained_slice(&items);
        view.beginDraggingSessionWithItems_event_source(
            &items,
            &event,
            ProtocolObject::from_ref(&*source),
        );
        STATE.with(|s| s.borrow_mut().source = Some(source));
        true
    }

    fn drop_action(&self, op: DragOps) {
        STATE.with(|s| s.borrow().drop_op.set(op));
    }

    fn drags_out(&self) -> bool {
        true
    }
}

/// What a drop from another app carries, read from its pasteboard in the order
/// `pasteboard::plan` gives.
fn read_drop(board: &NSPasteboard) -> DragData {
    let types: Vec<String> = board
        .types()
        .map(|t| t.iter().map(|t| t.to_string()).collect())
        .unwrap_or_default();
    let string = |ty: &str| {
        board
            .stringForType(&NSString::from_str(ty))
            .map(|s| s.to_string())
    };
    let mut data = DragData::default();
    for (kind, ty) in pasteboard::plan(&types) {
        match kind {
            dnd::Kind::UriList if ty == pasteboard::FILE_URL => {
                let uris: Vec<String> = board
                    .pasteboardItems()
                    .map(|items| {
                        items
                            .iter()
                            .filter_map(|i| i.stringForType(&NSString::from_str(ty)))
                            .map(|s| s.to_string())
                            .collect()
                    })
                    .unwrap_or_default();
                if let Some(dnd::UriList::Files(files)) = dnd::parse_uri_list(&uris.join("\n")) {
                    data.files = files;
                }
            }
            dnd::Kind::UriList => {
                data.link_url = string(ty);
                data.link_title = string(pasteboard::URL_NAME);
            }
            dnd::Kind::Html => data.html = string(ty),
            dnd::Kind::Text => data.text = string(ty),
        }
    }
    data
}

/// Where the drag is in the view, window-logical px from the top left (the view is flipped).
fn drag_point(view: &AnyObject, info: &AnyObject) -> (f32, f32) {
    // SAFETY: `view` is winit's NSView and `info` an `id<NSDraggingInfo>`, as AppKit calls.
    unsafe {
        let view: &NSView = &*(view as *const AnyObject as *const NSView);
        let at: NSPoint = msg_send![info, draggingLocation];
        let at = view.convertPoint_fromView(at, None);
        (at.x as f32, at.y as f32)
    }
}

extern "C-unwind" fn dragging_entered(
    view: &AnyObject,
    _: Sel,
    info: &AnyObject,
) -> NSDragOperation {
    let Some(id) = window_of(view) else {
        return NSDragOperation::None;
    };
    // SAFETY: `draggingPasteboard` of an `id<NSDraggingInfo>`.
    let board: Retained<NSPasteboard> = unsafe { msg_send![info, draggingPasteboard] };
    let (x, y) = drag_point(view, info);
    STATE.with(|s| s.borrow().drop_op.set(DragOps::NONE));
    deliver(
        id,
        AuxEvent::DropEnter {
            x,
            y,
            data: read_drop(&board),
        },
    );
    NSDragOperation::Copy
}

extern "C-unwind" fn dragging_updated(
    view: &AnyObject,
    _: Sel,
    info: &AnyObject,
) -> NSDragOperation {
    let Some(id) = window_of(view) else {
        return NSDragOperation::None;
    };
    let (x, y) = drag_point(view, info);
    deliver(id, AuxEvent::DropMove { x, y });
    ns_ops(STATE.with(|s| s.borrow().drop_op.get()))
}

extern "C-unwind" fn dragging_exited(view: &AnyObject, _: Sel, _info: *mut AnyObject) {
    if let Some(id) = window_of(view) {
        deliver(id, AuxEvent::DropLeave);
    }
}

extern "C-unwind" fn perform_drag(
    view: &AnyObject,
    _: Sel,
    info: &AnyObject,
) -> objc2::runtime::Bool {
    let Some(id) = window_of(view) else {
        return objc2::runtime::Bool::NO;
    };
    let (x, y) = drag_point(view, info);
    deliver(id, AuxEvent::Dropped { x, y });
    objc2::runtime::Bool::YES
}

/// winit's view class takes every drop through [`AuxEvent`]s (file drops included): the
/// methods are added (winit's view has none; `class_replaceMethod` adds a missing one).
fn patch_destination(class: &AnyClass) {
    type Op = extern "C-unwind" fn(&AnyObject, Sel, &AnyObject) -> NSDragOperation;
    type Exit = extern "C-unwind" fn(&AnyObject, Sel, *mut AnyObject);
    type Perform = extern "C-unwind" fn(&AnyObject, Sel, &AnyObject) -> objc2::runtime::Bool;
    let methods: [(Sel, objc2::runtime::Imp, &std::ffi::CStr); 4] = [
        (
            sel!(draggingEntered:),
            // SAFETY: the same ABI as the selector's (self, _cmd, id) -> NSUInteger.
            unsafe { std::mem::transmute::<Op, objc2::runtime::Imp>(dragging_entered) },
            c"Q@:@",
        ),
        (
            sel!(draggingUpdated:),
            unsafe { std::mem::transmute::<Op, objc2::runtime::Imp>(dragging_updated) },
            c"Q@:@",
        ),
        (
            sel!(draggingExited:),
            unsafe { std::mem::transmute::<Exit, objc2::runtime::Imp>(dragging_exited) },
            c"v@:@",
        ),
        (
            sel!(performDragOperation:),
            unsafe { std::mem::transmute::<Perform, objc2::runtime::Imp>(perform_drag) },
            c"B@:@",
        ),
    ];
    for (sel, imp, types) in methods {
        // SAFETY: replacing a method of winit's own view class with one of the same signature.
        unsafe {
            objc2::ffi::class_replaceMethod(
                class as *const AnyClass as *mut AnyClass,
                sel,
                imp,
                types.as_ptr(),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operations_map_both_ways() {
        assert_eq!(engine_op(ns_ops(DragOps::ANY)), DragOps::COPY);
        assert_eq!(engine_op(ns_ops(DragOps::MOVE)), DragOps::MOVE);
        assert_eq!(engine_op(NSDragOperation::None), DragOps::NONE);
    }
}
