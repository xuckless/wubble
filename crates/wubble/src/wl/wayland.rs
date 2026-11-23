//! The Wayland worker: one thread with its own event queue on the windows'
//! `wl_display` (winit keeps its own queue), serving what winit 0.30 leaves
//! out on Wayland, for every window of the process:
//!
//! - the clipboard and the primary selection (`crate::clipboard`): a page's copy
//!   with every type it has, and a paste reading every type a page takes
//!   (`wubble_core::clipboard`), the primary selection text only;
//! - a `wl_data_device` drop target: drags from other apps (files, links,
//!   HTML, text) reach the page under the pointer;
//! - a `wl_data_source` drag source: a page drag that leaves its window
//!   becomes an OS drag (text, a link as `text/uri-list`, HTML, a picture as
//!   its bytes and as a file, `drag_file.rs`), with the page's drag image as
//!   its icon;
//! - `zwp_pointer_gestures_v1` pinch and swipe, and the touchpad's two-finger
//!   sideways scroll (`wl_pointer` axis frames from a finger) as the history swipe;
//! - the lock keys from the worker's `wl_keyboard`'s `modifiers` event.
//!
//! One worker and one data device: a second `wl_data_device` of the same
//! client made GNOME stop offering the selection to the first (the parity
//! smoke's clipboard items failed with smithay-clipboard beside it).
//!
//! Ported from smithay-clipboard 0.7.3 (MIT, THIRD_PARTY_NOTICES.md): the
//! thread, queue and command channel (`src/lib.rs` `Clipboard::new`,
//! `src/worker.rs` `worker_impl`), and the selection's seat, focus, serial,
//! store and load handling (`src/state.rs`, `src/mime.rs` `normalize_to_lf`).
//! The protocol objects are smithay-client-toolkit 0.20's (MIT), the gesture
//! objects wayland-protocols' (MIT). The drag icon follows sctk 0.20's
//! `examples/data_device.rs` (MIT: an shm buffer on a surface of its own,
//! committed before `start_drag`) and Chromium's
//! `ui/ozone/platform/wayland/host/wayland_data_drag_controller.cc`
//! (BSD-3-Clause: buffer scale, offset by minus the hotspot).

use std::io::{Read, Write};
use std::os::fd::{AsRawFd, BorrowedFd};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc;
use std::time::Duration;

use sctk::data_device_manager::data_device::{DataDevice, DataDeviceHandler};
use sctk::data_device_manager::data_offer::{DataOfferHandler, DragOffer};
use sctk::data_device_manager::data_source::{CopyPasteSource, DataSourceHandler, DragSource};
use sctk::data_device_manager::{DataDeviceManagerState, ReadPipe, WritePipe};
use sctk::primary_selection::PrimarySelectionManagerState;
use sctk::primary_selection::device::{PrimarySelectionDevice, PrimarySelectionDeviceHandler};
use sctk::primary_selection::selection::{PrimarySelectionSource, PrimarySelectionSourceHandler};
use sctk::reexports::calloop::EventLoop;
use sctk::reexports::calloop::channel::{self, Channel, Sender};
use sctk::reexports::calloop_wayland_source::WaylandSource;
use sctk::reexports::client::backend::{Backend, ObjectId};
use sctk::reexports::client::globals::{GlobalList, registry_queue_init};
use sctk::reexports::client::protocol::wl_compositor::WlCompositor;
use sctk::reexports::client::protocol::wl_data_device::WlDataDevice;
use sctk::reexports::client::protocol::wl_data_device_manager::DndAction;
use sctk::reexports::client::protocol::wl_data_source::WlDataSource;
use sctk::reexports::client::protocol::wl_keyboard::{self, WlKeyboard};
use sctk::reexports::client::protocol::wl_pointer::{AxisSource, WlPointer};
use sctk::reexports::client::protocol::wl_seat::WlSeat;
use sctk::reexports::client::protocol::wl_shm;
use sctk::reexports::client::protocol::wl_surface::{self, WlSurface};
use sctk::reexports::client::{Connection, Dispatch, Proxy, QueueHandle};
use sctk::reexports::protocols::wp::pointer_gestures::zv1::client::{
    zwp_pointer_gesture_pinch_v1::{self, ZwpPointerGesturePinchV1},
    zwp_pointer_gesture_swipe_v1::{self, ZwpPointerGestureSwipeV1},
    zwp_pointer_gestures_v1::ZwpPointerGesturesV1,
};
use sctk::reexports::protocols::wp::primary_selection::zv1::client::{
    zwp_primary_selection_device_v1::ZwpPrimarySelectionDeviceV1,
    zwp_primary_selection_source_v1::ZwpPrimarySelectionSourceV1,
};
use sctk::registry::{ProvidesRegistryState, RegistryState};
use sctk::seat::pointer::{AxisScroll, PointerEvent, PointerEventKind, PointerHandler};
use sctk::seat::{Capability, SeatHandler, SeatState};
use sctk::shm::slot::{Buffer as ShmBuffer, SlotPool};
use sctk::shm::{Shm, ShmHandler};
use sctk::{
    delegate_data_device, delegate_pointer, delegate_primary_selection, delegate_registry,
    delegate_seat, delegate_shm, registry_handlers,
};
use slint::winit_030::WinitWindowAccessor;
use slint::winit_030::winit::raw_window_handle::{
    HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle,
};
use wubble_core::clipboard::{self, Payload, Reader};
use wubble_core::dnd::{self, Kind, Part, UriList};
use wubble_core::gesture::{PinchInput, SwipeInput};
use wubble_core::window::WindowId;
use wubble_engine::input::Locks;
use wubble_engine::page::{DragData, DragImage, DragOps};

use super::drag_file::{Folders, LazyFile};
use super::{Aux, AuxEvent, deliver};
use crate::clipboard::Buffer;

/// The most a drop's data may be, per type: a page drop is files' paths, a
/// link or a selection, not the files themselves.
const DROP_CAP: u64 = 8 << 20;

/// How long a paste waits for the selection's owner to send it.
const LOAD_WAIT: Duration = Duration::from_secs(2);
/// How long a paste reads the offered types (all at once): what arrived by then is the paste,
/// within [`LOAD_WAIT`].
const READ_DEADLINE: Duration = Duration::from_millis(1500);

/// What a finished read is given to: a waiting [`Worker::load_payload`] (its serial on the
/// loads channel), or a callback.
pub enum Answer {
    Serial(u64),
    Then(Box<dyn FnOnce(Option<Payload>) + Send>),
}

impl Answer {
    fn give(self, loads: &mpsc::Sender<(u64, Option<Payload>)>, payload: Option<Payload>) {
        match self {
            Self::Serial(serial) => {
                let _ = loads.send((serial, payload));
            }
            Self::Then(done) => done(payload),
        }
    }
}

/// `wl_keyboard.modifiers` masks of the keymap's real modifiers. The eight
/// core modifiers have fixed indices in every xkb keymap (Shift 0, Lock 1,
/// Control 2, Mod1 to Mod5 3 to 7), and Num Lock is Mod2 in xkeyboard-config's
/// layouts, so the keymap itself need not be read.
const LOCK_MASK: u32 = 1 << 1;
const MOD2_MASK: u32 = 1 << 4;

enum Command {
    AddWindow(WindowId, WlSurface),
    RemoveWindow(WindowId),
    StartDrag {
        window: WindowId,
        data: DragData,
        allowed: DragOps,
    },
    DropAction(DragOps),
    /// A drop's data, read on its own thread, for the drop of this generation.
    Read {
        generation: u64,
        data: DragData,
    },
    /// Set the selection; whether it was set goes back.
    Store(Buffer, Payload, mpsc::Sender<bool>),
    /// Read what the reader takes of the selection, for the answer.
    Load(Buffer, Reader, Answer),
    Exit,
}

/// The process's worker; dropping it stops the thread.
pub struct Worker {
    connection: Connection,
    tx: Sender<Command>,
    loads: mpsc::Receiver<(u64, Option<Payload>)>,
    load_serial: std::cell::Cell<u64>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Worker {
    /// The worker on `window`'s Wayland connection; none when it is not a Wayland window.
    /// Pictures dragged out are written under `drag_dir` (`drag_file.rs`).
    pub fn open(window: &slint::Window, drag_dir: PathBuf) -> Option<Self> {
        let display = window
            .with_winit_window(|w| match w.display_handle().ok()?.as_raw() {
                RawDisplayHandle::Wayland(h) => Some(h.display.as_ptr()),
                _ => None,
            })
            .flatten()?;
        // Safety: winit's live `wl_display`, which outlives the windows and so
        // this worker (the host drops it with the clipboard, before the event loop ends).
        let backend = unsafe { Backend::from_foreign_display(display.cast()) };
        let connection = Connection::from_backend(backend);
        let (tx, rx) = channel::channel();
        let (load_tx, loads) = mpsc::channel();
        let worker_tx = tx.clone();
        let worker_connection = connection.clone();
        let thread = std::thread::Builder::new()
            .name("wubble-wayland".into())
            .spawn(move || run(worker_connection, rx, worker_tx, load_tx, drag_dir))
            .inspect_err(|e| eprintln!("wubble: wayland worker: {e}"))
            .ok()?;
        Some(Self {
            connection,
            tx,
            loads,
            load_serial: Default::default(),
            thread: Some(thread),
        })
    }

    /// Puts `text` on `buffer`; false when it was not set ([`Self::store_payload`]).
    pub fn store(&self, buffer: Buffer, text: &str) -> bool {
        let text = Some(text.to_owned());
        self.store_payload(
            buffer,
            Payload {
                text,
                ..Payload::default()
            },
        )
    }

    /// Puts `payload` on `buffer`, under every type it has (`wubble_core::clipboard::offers`);
    /// false when it was not set (no surface of the process has the keyboard, the compositor
    /// has no such selection, nothing to offer).
    pub fn store_payload(&self, buffer: Buffer, payload: Payload) -> bool {
        let (done, stored) = mpsc::channel();
        self.tx.send(Command::Store(buffer, payload, done)).is_ok()
            && stored.recv_timeout(LOAD_WAIT).unwrap_or(false)
    }

    /// Reads what `reader` takes from `buffer` and gives it to `done` on the reading thread,
    /// without waiting: within [`READ_DEADLINE`], `None` when the selection has nothing for it.
    pub fn load_then(
        &self,
        buffer: Buffer,
        reader: Reader,
        done: Box<dyn FnOnce(Option<Payload>) + Send>,
    ) {
        // A worker that has stopped answers nothing.
        if let Err(mpsc::SendError(Command::Load(_, _, Answer::Then(done)))) = self
            .tx
            .send(Command::Load(buffer, reader, Answer::Then(done)))
        {
            done(None);
        }
    }

    /// The text on `buffer`, waiting a little for its owner to send it.
    pub fn load(&self, buffer: Buffer) -> Option<String> {
        self.load_payload(buffer, Reader::Text)?.text
    }

    /// What `reader` takes from `buffer` (the primary selection: its text), waiting a little
    /// for its owner to send it.
    pub fn load_payload(&self, buffer: Buffer, reader: Reader) -> Option<Payload> {
        let serial = self.load_serial.get() + 1;
        self.load_serial.set(serial);
        self.tx
            .send(Command::Load(buffer, reader, Answer::Serial(serial)))
            .ok()?;
        loop {
            match self.loads.recv_timeout(LOAD_WAIT) {
                Ok((s, payload)) if s == serial => return payload,
                // A late answer to a load that gave up.
                Ok(_) => continue,
                Err(_) => return None,
            }
        }
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.tx.send(Command::Exit);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Aux for Worker {
    fn add_window(&self, id: WindowId, window: &slint::Window) {
        let Some(surface) = window
            .with_winit_window(|w| match w.window_handle().ok()?.as_raw() {
                RawWindowHandle::Wayland(h) => Some(h.surface.as_ptr()),
                _ => None,
            })
            .flatten()
        else {
            return;
        };
        // Safety: winit's live `wl_surface` of the window; `remove_window` forgets it
        // when the window closes.
        let surface = unsafe { ObjectId::from_ptr(WlSurface::interface(), surface.cast()) }
            .ok()
            .and_then(|id| WlSurface::from_id(&self.connection, id).ok());
        if let Some(surface) = surface {
            let _ = self.tx.send(Command::AddWindow(id, surface));
        }
    }

    fn remove_window(&self, id: WindowId) {
        let _ = self.tx.send(Command::RemoveWindow(id));
    }

    fn start_drag(&self, window: WindowId, data: &DragData, allowed: DragOps) -> bool {
        let data = data.clone();
        let command = Command::StartDrag {
            window,
            data,
            allowed,
        };
        self.tx.send(command).is_ok()
    }

    fn drop_action(&self, op: DragOps) {
        let _ = self.tx.send(Command::DropAction(op));
    }

    fn drags_out(&self) -> bool {
        true
    }
}

fn run(
    connection: Connection,
    rx: Channel<Command>,
    tx: Sender<Command>,
    loads: mpsc::Sender<(u64, Option<Payload>)>,
    drag_dir: PathBuf,
) {
    let Ok((globals, queue)) = registry_queue_init::<State>(&connection) else {
        return;
    };
    let qh = queue.handle();
    let Ok(mut event_loop) = EventLoop::<State>::try_new() else {
        return;
    };
    let folders = Folders::new(drag_dir);
    // A crashed run's files.
    folders.clear();
    let mut state = State::new(&globals, &qh, tx, loads, folders);
    let inserted = event_loop.handle().insert_source(rx, |event, _, state| {
        if let channel::Event::Msg(command) = event {
            state.command(command);
        }
    });
    if inserted.is_err()
        || WaylandSource::new(connection, queue)
            .insert(event_loop.handle())
            .is_err()
    {
        return;
    }
    while !state.exit {
        if event_loop.dispatch(None, &mut state).is_err() {
            break;
        }
    }
}

/// A drop from another app, from its `enter`; `O` is the offer (sctk's `DragOffer`).
enum Incoming<O> {
    None,
    /// Its data is being read; motion and the drop wait for it.
    Reading {
        window: WindowId,
        offer: O,
        generation: u64,
        at: (f32, f32),
        dropped: bool,
    },
    /// The page has it; `at` is the pointer's last point (the offer's own `x`/`y` are its
    /// `enter`'s: sctk updates only the device's copy).
    Entered {
        window: WindowId,
        offer: O,
        at: (f32, f32),
    },
}

impl<O> Incoming<O> {
    /// The pointer moved to `at`: what the page hears, once it has the drag.
    fn moved(&mut self, to: (f32, f32)) -> Option<(WindowId, AuxEvent)> {
        match self {
            Incoming::Reading { at, .. } => {
                *at = to;
                None
            }
            Incoming::Entered { window, at, .. } => {
                *at = to;
                Some((*window, AuxEvent::DropMove { x: to.0, y: to.1 }))
            }
            Incoming::None => None,
        }
    }

    /// The drop, at the pointer's last point: the page hears of it with the offer to finish
    /// now, or once the data is read.
    fn dropped(self) -> (Self, Option<(WindowId, AuxEvent, O)>) {
        match self {
            Incoming::Entered {
                window,
                offer,
                at: (x, y),
            } => (
                Incoming::None,
                Some((window, AuxEvent::Dropped { x, y }, offer)),
            ),
            Incoming::Reading {
                window,
                offer,
                generation,
                at,
                ..
            } => {
                let reading = Incoming::Reading {
                    window,
                    offer,
                    generation,
                    at,
                    dropped: true,
                };
                (reading, None)
            }
            Incoming::None => (Incoming::None, None),
        }
    }
}

struct State {
    registry_state: RegistryState,
    seat_state: SeatState,
    manager: Option<DataDeviceManagerState>,
    primary_manager: Option<PrimarySelectionManagerState>,
    gestures: Option<ZwpPointerGesturesV1>,
    /// For the drag icon's surface and buffer.
    compositor: Option<WlCompositor>,
    shm: Option<Shm>,
    qh: QueueHandle<State>,
    tx: Sender<Command>,
    loads: mpsc::Sender<(u64, Option<Payload>)>,
    exit: bool,

    windows: Vec<(WindowId, WlSurface)>,
    device: Option<DataDevice>,
    primary_device: Option<PrimarySelectionDevice>,
    pointer: Option<WlPointer>,
    keyboard: Option<WlKeyboard>,
    pinch: Option<ZwpPointerGesturePinchV1>,
    swipe: Option<ZwpPointerGestureSwipeV1>,
    /// The last input serial (key, button, focus), which sets the selection.
    serial: u32,
    /// The surface with keyboard focus: the compositor takes a selection only while one has
    /// it. A surface, not a window: focus may come before the window is added.
    focus: Option<WlSurface>,
    /// The serial of the last button press on a window, which starts a drag.
    press: Option<(WindowId, u32)>,
    /// The window a pinch or swipe began on, and the pinch's scale at its last update
    /// (Wayland's is since the begin).
    pinch_on: Option<(WindowId, f64)>,
    swipe_on: Option<WindowId>,
    /// The window a two-finger scroll swipe is going on in.
    scroll_on: Option<WindowId>,
    locks: Locks,

    drop: Incoming<DragOffer>,
    generation: u64,
    /// The OS drag a page drag became.
    source: Option<DragOut>,
    /// The pictures' files dragged out.
    folders: Folders,
    /// The selections stored, with what each offers, while offered. The old one is destroyed
    /// before a new one is set: the compositor (mutter, weston) ignores a `set_selection` whose
    /// serial is not newer than the current selection's, and the serial stays the same between
    /// input events.
    clipboard: Option<(CopyPasteSource, Selection)>,
    primary: Option<(PrimarySelectionSource, Selection)>,
}

/// What a stored selection offers: each type with its bytes.
type Selection = Arc<[(String, Arc<[u8]>)]>;

/// An OS drag a page drag became: its window, what it carries, the action
/// chosen, its icon (kept until it ends).
struct DragOut {
    window: WindowId,
    source: DragSource,
    offered: Arc<Offered>,
    action: DndAction,
    _icon: Option<Icon>,
}

/// What a drag out of the window offers, by MIME type.
struct Offered(Vec<(&'static str, Body)>);

/// What a drag out sends for a type: bytes, or the list naming a picture's
/// file, which is written when first asked for.
#[derive(Clone)]
enum Body {
    Bytes(Arc<[u8]>),
    File(Arc<LazyFile>),
}

/// A drag's thumbnail under the pointer: a surface of its own with an shm buffer.
struct Icon {
    surface: WlSurface,
    /// The surface's top left from the pointer (minus the hotspot), attached once the
    /// drag has started.
    offset: (i32, i32),
    buffer: ShmBuffer,
    _pool: SlotPool,
}

impl Drop for Icon {
    fn drop(&mut self) {
        self.surface.destroy();
    }
}

impl State {
    fn new(
        globals: &GlobalList,
        qh: &QueueHandle<Self>,
        tx: Sender<Command>,
        loads: mpsc::Sender<(u64, Option<Payload>)>,
        folders: Folders,
    ) -> Self {
        let gestures = globals
            .bind::<ZwpPointerGesturesV1, _, _>(qh, 1..=3, ())
            .ok();
        if gestures.is_none() {
            log::debug!("wayland: no pointer gestures");
        }
        Self {
            registry_state: RegistryState::new(globals),
            seat_state: SeatState::new(globals, qh),
            manager: DataDeviceManagerState::bind(globals, qh).ok(),
            primary_manager: PrimarySelectionManagerState::bind(globals, qh).ok(),
            gestures,
            // Up to 4: an attach with an offset is an error from version 5.
            compositor: globals.bind::<WlCompositor, _, _>(qh, 1..=4, ()).ok(),
            shm: Shm::bind(globals, qh).ok(),
            qh: qh.clone(),
            tx,
            loads,
            exit: false,
            windows: vec![],
            device: None,
            primary_device: None,
            pointer: None,
            keyboard: None,
            pinch: None,
            swipe: None,
            serial: 0,
            focus: None,
            press: None,
            pinch_on: None,
            swipe_on: None,
            scroll_on: None,
            locks: Locks::default(),
            drop: Incoming::None,
            generation: 0,
            source: None,
            folders,
            clipboard: None,
            primary: None,
        }
    }

    fn window_of(&self, surface: &WlSurface) -> Option<WindowId> {
        let id = surface.id();
        self.windows
            .iter()
            .find_map(|(w, s)| (s.id() == id).then_some(*w))
    }

    fn surface(&self, window: WindowId) -> Option<&WlSurface> {
        self.windows
            .iter()
            .find_map(|(w, s)| (*w == window).then_some(s))
    }

    fn command(&mut self, command: Command) {
        match command {
            Command::Exit => {
                self.exit = true;
                self.folders.clear();
            }
            Command::AddWindow(id, surface) => {
                self.windows.retain(|(w, _)| *w != id);
                if self.focus.as_ref().is_some_and(|f| f.id() == surface.id()) {
                    deliver(id, AuxEvent::Locks(self.locks));
                }
                self.windows.push((id, surface));
            }
            Command::RemoveWindow(id) => self.windows.retain(|(w, _)| *w != id),
            Command::DropAction(op) => {
                if let Incoming::Entered { offer, .. } | Incoming::Reading { offer, .. } =
                    &self.drop
                {
                    let action = dnd_action(op);
                    offer.set_actions(action, action);
                }
            }
            Command::Read { generation, data } => self.read(generation, data),
            Command::StartDrag {
                window,
                data,
                allowed,
            } => self.start_drag(window, data, allowed),
            Command::Store(buffer, payload, done) => {
                let _ = done.send(self.store(buffer, payload));
            }
            Command::Load(buffer, reader, answer) => self.load(buffer, reader, answer),
        }
    }

    /// A drop's data arrived: the page gets the drag, and the drop if it came meanwhile.
    fn read(&mut self, generation: u64, data: DragData) {
        let current =
            matches!(&self.drop, Incoming::Reading { generation: g, .. } if *g == generation);
        if !current {
            return;
        }
        let Incoming::Reading {
            window,
            offer,
            at,
            dropped,
            ..
        } = std::mem::replace(&mut self.drop, Incoming::None)
        else {
            return;
        };
        let (x, y) = at;
        deliver(window, AuxEvent::DropEnter { x, y, data });
        if dropped {
            deliver(window, AuxEvent::Dropped { x, y });
            finish(&offer);
        } else {
            self.drop = Incoming::Entered { window, offer, at };
        }
    }

    fn start_drag(&mut self, window: WindowId, data: DragData, allowed: DragOps) {
        let offered = Arc::new(Offered(offered(&data, || self.folders.next())));
        let ready = (
            &self.manager,
            &self.device,
            self.press.filter(|(w, _)| *w == window),
            self.surface(window),
        );
        let (Some(manager), Some(device), Some((_, serial)), Some(surface)) = ready else {
            return deliver(window, AuxEvent::DragEnded(DragOps::NONE));
        };
        if offered.0.is_empty() {
            return deliver(window, AuxEvent::DragEnded(DragOps::NONE));
        }
        let source = manager.create_drag_and_drop_source(
            &self.qh,
            offered.0.iter().map(|(mime, _)| *mime),
            dnd_action(allowed),
        );
        // The icon's buffer is committed before the drag starts (mutter and weston need
        // it); its hotspot offset only after, once the surface has its dnd-icon role:
        // the offset is the role's pending state (Chromium `WaylandDataDragController`
        // starts the drag, then draws the icon).
        let icon = data.image.as_ref().and_then(|image| self.icon(image));
        source.start_drag(device, surface, icon.as_ref().map(|i| &i.surface), serial);
        if let Some(icon) = &icon {
            icon.surface
                .attach(Some(icon.buffer.wl_buffer()), icon.offset.0, icon.offset.1);
            icon.surface.damage(0, 0, i32::MAX, i32::MAX);
            icon.surface.commit();
        }
        self.source = Some(DragOut {
            window,
            source,
            offered,
            action: DndAction::empty(),
            _icon: icon,
        });
    }

    /// The drag image on a surface of its own, its hotspot under the pointer.
    fn icon(&self, image: &DragImage) -> Option<Icon> {
        let (compositor, shm) = (self.compositor.as_ref()?, self.shm.as_ref()?);
        let (width, height, argb) = argb(&image.png)?;
        let (scale, (x, y)) = icon_geometry(width, image, compositor.version() >= 3);
        // A buffer's size must be a multiple of its scale (wlroots ends the connection).
        let (width, height, argb) = pad(width, height, argb, scale as u32);
        let mut pool = SlotPool::new(argb.len(), shm).ok()?;
        let stride = width as i32 * 4;
        let (buffer, canvas) = pool
            .create_buffer(
                width as i32,
                height as i32,
                stride,
                wl_shm::Format::Argb8888,
            )
            .ok()?;
        canvas.copy_from_slice(&argb);
        let surface = compositor.create_surface(&self.qh, ());
        if scale > 1 {
            surface.set_buffer_scale(scale);
        }
        buffer.attach_to(&surface).ok()?;
        surface.damage(0, 0, i32::MAX, i32::MAX);
        surface.commit();
        Some(Icon {
            surface,
            offset: (x, y),
            buffer,
            _pool: pool,
        })
    }

    /// The OS drag ended: the page drag ends with the action done.
    fn source_ended(&mut self, source: &WlDataSource, finished: bool) {
        let Some(out) = &self.source else {
            return;
        };
        if out.source.inner() != source {
            return;
        }
        let (window, op) = (
            out.window,
            if finished {
                drag_ops(out.action)
            } else {
                DragOps::NONE
            },
        );
        self.source = None;
        deliver(window, AuxEvent::DragEnded(op));
    }

    /// Sets the selection; false when it could not be.
    fn store(&mut self, buffer: Buffer, payload: Payload) -> bool {
        if self.focus.is_none() {
            return false;
        }
        let offers: Selection = clipboard::offers(&payload).into();
        if offers.is_empty() {
            return false;
        }
        let mimes = offers.iter().map(|(mime, _)| mime.as_str());
        match buffer {
            Buffer::Clipboard => {
                let (Some(manager), Some(device)) = (&self.manager, &self.device) else {
                    return false;
                };
                self.clipboard = None;
                let source = manager.create_copy_paste_source(&self.qh, mimes);
                source.set_selection(device, self.serial);
                self.clipboard = Some((source, offers));
            }
            Buffer::Primary => {
                let (Some(manager), Some(device)) = (&self.primary_manager, &self.primary_device)
                else {
                    return false;
                };
                self.primary = None;
                let source = manager.create_selection_source(&self.qh, mimes);
                source.set_selection(device, self.serial);
                self.primary = Some((source, offers));
            }
        }
        true
    }

    /// Starts reading what `reader` takes of `buffer` (the primary selection: its text, as on
    /// Chrome), for `answer`: at once with `None` when there is nothing to read.
    fn load(&mut self, buffer: Buffer, reader: Reader, answer: Answer) {
        let pipes = match buffer {
            Buffer::Clipboard => self
                .device
                .as_ref()
                .and_then(|d| d.data().selection_offer())
                .map(|offer| {
                    let plan = offer.with_mime_types(|mimes| clipboard::plan(mimes, reader));
                    receive_all(plan, |mime| offer.receive(mime).ok())
                }),
            Buffer::Primary => self
                .primary_device
                .as_ref()
                .and_then(|d| d.data().selection_offer())
                .map(|offer| {
                    let plan = offer.with_mime_types(|m| clipboard::plan(m, Reader::Text));
                    receive_all(plan, |mime| offer.receive(mime).ok())
                }),
        }
        .unwrap_or_default();
        if pipes.is_empty() {
            return answer.give(&self.loads, None);
        }
        // Shared so that a thread that could not start still answers.
        let answer = Arc::new(std::sync::Mutex::new(Some(answer)));
        let (loads, theirs) = (self.loads.clone(), answer.clone());
        let spawned = std::thread::Builder::new()
            .name("wubble-paste".into())
            .spawn(move || {
                let payload = read_pipes(pipes);
                if let Some(answer) = theirs.lock().ok().and_then(|mut a| a.take()) {
                    answer.give(&loads, Some(payload));
                }
            });
        if let Err(e) = spawned {
            eprintln!("wubble: paste: {e}");
            if let Some(answer) = answer.lock().ok().and_then(|mut a| a.take()) {
                answer.give(&self.loads, None);
            }
        }
    }

    /// Writes the stored selection's `mime` to a paste's pipe.
    fn send_selection(offers: &Selection, mime: &str, pipe: WritePipe) {
        let Some((_, bytes)) = offers.iter().find(|(m, _)| m == mime) else {
            return;
        };
        let bytes = bytes.clone();
        send_on_thread(&pipe, move |file| {
            let _ = file.write_all(&bytes);
        });
    }
}

/// What the offered types' pipes hold, read all at once under one [`READ_DEADLINE`]: a slow
/// source loses only its late types, in the plan's order (text first). A type past its cap is
/// dropped whole, never cut. Then a file manager's copy: the files' bytes, as Chromium hands a
/// pasted file to the page (only a `paste` event's reader takes them), within
/// `clipboard::FILES_CAP` together; empty or special files (`/proc`'s, a FIFO) are skipped.
fn read_pipes(pipes: Vec<(clipboard::Kind, String, std::fs::File)>) -> Payload {
    let deadline = std::time::Instant::now() + READ_DEADLINE;
    let (tx, rx) = mpsc::channel();
    let mut arrived = vec![None; pipes.len()];
    let mut reading = 0;
    for (i, (kind, mime, file)) in pipes.into_iter().enumerate() {
        let tx = tx.clone();
        let spawned = std::thread::Builder::new()
            .name("wubble-paste-type".into())
            .spawn(move || {
                let mut bytes = vec![];
                let read = file.take(kind.cap() + 1).read_to_end(&mut bytes);
                let whole = read.is_ok() && bytes.len() as u64 <= kind.cap();
                let _ = tx.send((i, whole.then_some((kind, mime, bytes))));
            });
        reading += usize::from(spawned.is_ok());
    }
    for _ in 0..reading {
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        match rx.recv_timeout(left) {
            Ok((i, part)) => arrived[i] = part,
            Err(_) => break,
        }
    }
    let (mut payload, paths) = clipboard::read(arrived.into_iter().flatten().collect());
    let mut total = 0;
    for path in paths {
        let Some(name) = path.file_name().map(|n| n.to_string_lossy().into_owned()) else {
            continue;
        };
        let Ok(meta) = std::fs::metadata(&path) else {
            continue;
        };
        if !meta.is_file() || meta.len() == 0 || total + meta.len() > clipboard::FILES_CAP {
            continue;
        }
        let mut bytes = vec![];
        let read =
            std::fs::File::open(&path).and_then(|f| f.take(meta.len()).read_to_end(&mut bytes));
        if read.is_ok() {
            total += bytes.len() as u64;
            clipboard::add_file(&mut payload, name, bytes);
        }
    }
    payload
}

/// The read ends of the types `plan` names, as files, each with its entry; `receive` asks the
/// offer for one type.
fn receive_all<T>(
    plan: Vec<(T, String)>,
    receive: impl Fn(String) -> Option<ReadPipe>,
) -> Vec<(T, String, std::fs::File)> {
    plan.into_iter()
        .filter_map(|(entry, mime)| {
            let pipe = receive(mime.clone())?;
            // Safety: the pipe is open while `pipe` lives; the clone outlives it.
            let file = own(unsafe { BorrowedFd::borrow_raw(pipe.as_raw_fd()) })?;
            Some((entry, mime, file))
        })
        .collect()
}

/// A dragged picture: its type when known, its bytes, its file's name.
struct Picture {
    mime: Option<&'static str>,
    bytes: Arc<[u8]>,
    name: String,
}

/// The picture a drag carries: the page's own bytes. Without them (CEF gave
/// none, or more than welding's cap) there is no file: the drag image is a
/// downscaled, 75%-opaque thumbnail (Chromium `kDragImageAlpha`), not the
/// picture, so the drag offers the link, HTML and URL only. Nothing is fetched.
fn picture(data: &DragData) -> Option<Picture> {
    let contents = data.file_contents.as_ref()?;
    let mime = dnd::sniff_image_mime(&contents.bytes);
    Some(Picture {
        mime,
        bytes: contents.bytes.clone(),
        name: dnd::drag_file_name(contents.name.as_deref(), mime),
    })
}

/// The drag-out's types (`dnd::offers`): a picture's file as `text/uri-list`
/// in a new folder from `folder` (else files and a link), the picture's
/// bytes, HTML, the text (else the link's URL).
fn offered(data: &DragData, folder: impl FnOnce() -> PathBuf) -> Vec<(&'static str, Body)> {
    let picture = picture(data);
    let file = picture
        .as_ref()
        .map(|p| Arc::new(LazyFile::new(folder(), &p.name, p.bytes.clone())));
    let uris: Vec<String> = data
        .files
        .iter()
        .map(|p| dnd::file_uri(p))
        .chain(data.link_url.clone())
        .collect();
    let text = data.text.as_ref().or(data.link_url.as_ref());
    let out = dnd::Outgoing {
        uris: file.is_some() || !uris.is_empty(),
        link: data.link_url.is_some(),
        picture: picture.as_ref().and_then(|p| p.mime),
        html: data.html.is_some(),
        text: text.is_some(),
    };
    let bytes = |text: &str| Body::Bytes(Arc::from(text.as_bytes()));
    dnd::offers(&out)
        .into_iter()
        .filter_map(|(mime, part)| {
            let body = match part {
                Part::UriList => match &file {
                    Some(file) => Body::File(file.clone()),
                    None => bytes(&(uris.join("\r\n") + "\r\n")),
                },
                Part::MozUrl => {
                    let url = data.link_url.as_ref()?;
                    let title = data.link_title.as_deref().unwrap_or(url);
                    bytes(&format!("{url}\n{title}"))
                }
                Part::Picture => Body::Bytes(picture.as_ref()?.bytes.clone()),
                Part::Html => bytes(data.html.as_ref()?),
                Part::Text => bytes(text?),
            };
            Some((mime, body))
        })
        .collect()
}

/// The drag image as premultiplied ARGB8888 (little endian: B, G, R, A), what `wl_shm` takes.
fn argb(png: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    let (width, height, rgba) = crate::shot_io::rgba(png).ok()?;
    let premultiply = |c: u8, a: u8| ((u16::from(c) * u16::from(a) + 127) / 255) as u8;
    let argb = rgba
        .as_chunks::<4>()
        .0
        .iter()
        .flat_map(|&[r, g, b, a]| [premultiply(b, a), premultiply(g, a), premultiply(r, a), a])
        .collect();
    Some((width, height, argb))
}

/// The pixels grown with transparent ones to the next multiple of `scale` on each side.
fn pad(width: u32, height: u32, argb: Vec<u8>, scale: u32) -> (u32, u32, Vec<u8>) {
    let grow = |n: u32| n.div_ceil(scale.max(1)) * scale.max(1);
    let (w, h) = (grow(width), grow(height));
    if (w, h) == (width, height) {
        return (width, height, argb);
    }
    let mut out = vec![0; (w * h * 4) as usize];
    let row = (width * 4) as usize;
    for (y, line) in argb.chunks_exact(row).enumerate() {
        let at = y * (w * 4) as usize;
        out[at..at + row].copy_from_slice(line);
    }
    (w, h, out)
}

/// The icon's buffer scale and its top left from the pointer, in surface units, for a PNG
/// `width` pixels wide. The scale is the PNG's over its DIP size, whole (Wayland's buffer
/// scale is an integer; 1 when the compositor `scales` no buffers).
fn icon_geometry(width: u32, image: &DragImage, scales: bool) -> (i32, (i32, i32)) {
    let png_scale = match image.size.0 {
        dip if dip > 0 => width as f32 / dip as f32,
        _ => 1.0,
    };
    let scale = if scales {
        png_scale.round().max(1.0) as i32
    } else {
        1
    };
    let to_surface = |v: i32| -(v as f32 * png_scale / scale as f32).round() as i32;
    (
        scale,
        (to_surface(image.hotspot.0), to_surface(image.hotspot.1)),
    )
}

/// Wayland has copy, move and ask; a link is a copy.
fn dnd_action(ops: DragOps) -> DndAction {
    let mut action = DndAction::empty();
    if ops.0 & (DragOps::COPY.0 | DragOps::LINK.0) != 0 {
        action |= DndAction::Copy;
    }
    if ops.0 & DragOps::MOVE.0 != 0 {
        action |= DndAction::Move;
    }
    action
}

fn drag_ops(action: DndAction) -> DragOps {
    if action.contains(DndAction::Move) {
        DragOps::MOVE
    } else if action.contains(DndAction::Copy) {
        DragOps::COPY
    } else {
        DragOps::NONE
    }
}

fn finish(offer: &DragOffer) {
    offer.finish();
    offer.destroy();
}

/// A pipe's fd as a file this thread owns.
fn own(fd: BorrowedFd<'_>) -> Option<std::fs::File> {
    fd.try_clone_to_owned().ok().map(std::fs::File::from)
}

/// Runs `send` with the pipe on a thread of its own (the reader may be slow, or us; a
/// picture's file is written there).
fn send_on_thread(pipe: &WritePipe, send: impl FnOnce(&mut std::fs::File) + Send + 'static) {
    // Safety: the pipe is open while `pipe` lives; the clone outlives it.
    let Some(mut file) = own(unsafe { BorrowedFd::borrow_raw(pipe.as_raw_fd()) }) else {
        return;
    };
    let _ = std::thread::Builder::new()
        .name("wubble-send".into())
        .spawn(move || send(&mut file));
}

/// Reads a drop's types on a thread and hands the data back to the worker.
fn read_drop(
    offer: &DragOffer,
    plan: Vec<(Kind, &'static str)>,
    generation: u64,
    tx: Sender<Command>,
) {
    let plan = plan.into_iter().map(|(kind, mime)| (kind, mime.to_owned()));
    let pipes = receive_all(plan.collect(), |mime| offer.receive(mime).ok());
    let spawned = std::thread::Builder::new()
        .name("wubble-drop".into())
        .spawn(move || {
            let mut data = DragData::default();
            for (kind, _, file) in pipes {
                let mut bytes = vec![];
                if file.take(DROP_CAP).read_to_end(&mut bytes).is_err() {
                    continue;
                }
                let text = String::from_utf8_lossy(&bytes).into_owned();
                match kind {
                    Kind::UriList => match dnd::parse_uri_list(&text) {
                        Some(UriList::Files(files)) => data.files = files,
                        Some(UriList::Link(url)) => data.link_url = Some(url),
                        None => {}
                    },
                    Kind::Html => data.html = Some(text),
                    Kind::Text => data.text = Some(text),
                }
            }
            let _ = tx.send(Command::Read { generation, data });
        });
    if let Err(e) = spawned {
        eprintln!("wubble: wayland drop: {e}");
    }
}

impl DataDeviceHandler for State {
    fn enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        device: &WlDataDevice,
        x: f64,
        y: f64,
        surface: &WlSurface,
    ) {
        let Some(window) = self.window_of(surface) else {
            return;
        };
        let Some(offer) = self
            .device
            .as_ref()
            .filter(|d| d.inner() == device)
            .and_then(|d| d.data().drag_offer())
        else {
            return;
        };
        let plan = offer.with_mime_types(dnd::plan);
        let Some(&(_, best)) = plan.first() else {
            offer.accept_mime_type(offer.serial, None);
            offer.set_actions(DndAction::empty(), DndAction::empty());
            return;
        };
        offer.accept_mime_type(offer.serial, Some(best.to_owned()));
        offer.set_actions(DndAction::Copy | DndAction::Move, DndAction::Copy);
        self.generation += 1;
        read_drop(&offer, plan, self.generation, self.tx.clone());
        self.drop = Incoming::Reading {
            window,
            offer,
            generation: self.generation,
            at: (x as f32, y as f32),
            dropped: false,
        };
    }

    fn leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataDevice) {
        match std::mem::replace(&mut self.drop, Incoming::None) {
            Incoming::Entered { window, .. } => deliver(window, AuxEvent::DropLeave),
            // A dropped offer stays until its data is read.
            reading @ Incoming::Reading { dropped: true, .. } => self.drop = reading,
            _ => {}
        }
    }

    fn motion(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataDevice, x: f64, y: f64) {
        if let Some((window, event)) = self.drop.moved((x as f32, y as f32)) {
            deliver(window, event);
        }
    }

    fn selection(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataDevice) {}

    fn drop_performed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataDevice) {
        let (next, now) = std::mem::replace(&mut self.drop, Incoming::None).dropped();
        self.drop = next;
        if let Some((window, event, offer)) = now {
            deliver(window, event);
            finish(&offer);
        }
    }
}

impl DataOfferHandler for State {
    fn source_actions(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &mut DragOffer,
        _: DndAction,
    ) {
    }

    fn selected_action(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &mut DragOffer,
        _: DndAction,
    ) {
    }
}

impl DataSourceHandler for State {
    fn accept_mime(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlDataSource,
        _: Option<String>,
    ) {
    }

    fn send_request(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        source: &WlDataSource,
        mime: String,
        pipe: WritePipe,
    ) {
        if let Some(out) = &self.source
            && out.source.inner() == source
        {
            if let Some((_, body)) = out.offered.0.iter().find(|(m, _)| *m == mime) {
                let body = body.clone();
                send_on_thread(&pipe, move |pipe| match body {
                    Body::Bytes(bytes) => {
                        let _ = pipe.write_all(&bytes);
                    }
                    // Nothing is sent for a file that could not be written.
                    Body::File(file) => {
                        if file.ensure_written() {
                            let _ = pipe.write_all(&file.uri_list());
                        }
                    }
                });
            }
            return;
        }
        if let Some((copy, offers)) = &self.clipboard
            && copy.inner() == source
        {
            State::send_selection(offers, &mime, pipe);
        }
    }

    fn cancelled(&mut self, _: &Connection, _: &QueueHandle<Self>, source: &WlDataSource) {
        if self
            .clipboard
            .as_ref()
            .is_some_and(|(copy, _)| copy.inner() == source)
        {
            self.clipboard = None;
        } else {
            self.source_ended(source, false);
        }
    }

    fn dnd_dropped(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataSource) {}

    fn dnd_finished(&mut self, _: &Connection, _: &QueueHandle<Self>, source: &WlDataSource) {
        self.source_ended(source, true);
    }

    fn action(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        source: &WlDataSource,
        action: DndAction,
    ) {
        if let Some(out) = &mut self.source
            && out.source.inner() == source
        {
            out.action = action;
        }
    }
}

impl PrimarySelectionDeviceHandler for State {
    fn selection(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &ZwpPrimarySelectionDeviceV1,
    ) {
    }
}

impl PrimarySelectionSourceHandler for State {
    fn send_request(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        source: &ZwpPrimarySelectionSourceV1,
        mime: String,
        pipe: WritePipe,
    ) {
        if let Some((ours, offers)) = &self.primary
            && ours.inner() == source
        {
            State::send_selection(offers, &mime, pipe);
        }
    }

    fn cancelled(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        source: &ZwpPrimarySelectionSourceV1,
    ) {
        if self
            .primary
            .as_ref()
            .is_some_and(|(ours, _)| ours.inner() == source)
        {
            self.primary = None;
        }
    }
}

impl SeatHandler for State {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }

    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: WlSeat) {}

    fn new_capability(
        &mut self,
        _: &Connection,
        qh: &QueueHandle<Self>,
        seat: WlSeat,
        capability: Capability,
    ) {
        // The first seat's devices; the selection devices are per seat, not per capability.
        if self.device.is_none() {
            self.device = self.manager.as_ref().map(|m| m.get_data_device(qh, &seat));
        }
        if self.primary_device.is_none() {
            self.primary_device = self
                .primary_manager
                .as_ref()
                .map(|m| m.get_selection_device(qh, &seat));
        }
        match capability {
            Capability::Pointer if self.pointer.is_none() => {
                let Ok(pointer) = self.seat_state.get_pointer(qh, &seat) else {
                    return;
                };
                if let Some(gestures) = &self.gestures {
                    self.pinch = Some(gestures.get_pinch_gesture(&pointer, qh, ()));
                    self.swipe = Some(gestures.get_swipe_gesture(&pointer, qh, ()));
                }
                self.pointer = Some(pointer);
            }
            Capability::Keyboard if self.keyboard.is_none() => {
                self.keyboard = Some(seat.get_keyboard(qh, ()));
            }
            _ => {}
        }
    }

    fn remove_capability(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: WlSeat,
        capability: Capability,
    ) {
        match capability {
            Capability::Pointer => {
                if let Some(pinch) = self.pinch.take() {
                    pinch.destroy();
                }
                if let Some(swipe) = self.swipe.take() {
                    swipe.destroy();
                }
                if let Some(pointer) = self.pointer.take()
                    && pointer.version() >= 3
                {
                    pointer.release();
                }
            }
            Capability::Keyboard => {
                self.focus = None;
                if let Some(keyboard) = self.keyboard.take()
                    && keyboard.version() >= 3
                {
                    keyboard.release();
                }
            }
            _ => {}
        }
    }

    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: WlSeat) {}
}

impl PointerHandler for State {
    fn pointer_frame(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlPointer,
        events: &[PointerEvent],
    ) {
        for event in events {
            match event.kind {
                PointerEventKind::Press { serial, .. } => {
                    self.serial = serial;
                    self.press = self.window_of(&event.surface).map(|w| (w, serial));
                }
                PointerEventKind::Release { serial, .. } => self.serial = serial,
                PointerEventKind::Axis {
                    horizontal,
                    vertical,
                    source,
                    ..
                } => {
                    let window = self.scroll_on.or_else(|| self.window_of(&event.surface));
                    let Some(window) = window else { continue };
                    let (on, inputs) =
                        scroll_swipe(self.scroll_on.is_some(), source, &horizontal, &vertical);
                    self.scroll_on = on.then_some(window);
                    let (x, y) = event.position;
                    for input in inputs {
                        let at = matches!(input, SwipeInput::Begin { .. })
                            .then_some((x as f32, y as f32));
                        deliver(window, AuxEvent::Swipe { input, at });
                    }
                }
                PointerEventKind::Leave { .. } => {
                    if let Some(window) = self.scroll_on.take() {
                        let input = SwipeInput::End { cancelled: true };
                        deliver(window, AuxEvent::Swipe { input, at: None });
                    }
                }
                _ => {}
            }
        }
    }
}

/// A scroll frame as history swipe input (`wubble_core::gesture`), given whether a swipe is
/// going on: a finger's scroll begins one, its `axis_stop` ends it, a wheel's scroll cancels
/// it. Wayland's values are the scroll's (positive: right, down), the swipe's the fingers'
/// with natural scrolling (positive: right), so a scroll to the left goes back. Whether a swipe
/// is going on after it, and the inputs.
fn scroll_swipe(
    on: bool,
    source: Option<AxisSource>,
    horizontal: &AxisScroll,
    vertical: &AxisScroll,
) -> (bool, Vec<SwipeInput>) {
    let (dx, dy) = (-horizontal.absolute, -vertical.absolute);
    let update = (dx != 0.0 || dy != 0.0).then_some(SwipeInput::Update { dx, dy });
    let stop = horizontal.stop || vertical.stop;
    match source {
        Some(AxisSource::Finger) | None if on => {
            let mut inputs: Vec<_> = update.into_iter().collect();
            if stop {
                inputs.push(SwipeInput::End { cancelled: false });
            }
            (!stop, inputs)
        }
        Some(AxisSource::Finger) if !stop && update.is_some() => (
            true,
            [SwipeInput::Begin { fingers: 2 }]
                .into_iter()
                .chain(update)
                .collect(),
        ),
        _ if on => (false, vec![SwipeInput::End { cancelled: true }]),
        _ => (false, vec![]),
    }
}

impl Dispatch<WlKeyboard, ()> for State {
    fn event(
        state: &mut Self,
        _: &WlKeyboard,
        event: wl_keyboard::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use wl_keyboard::Event as E;
        match event {
            E::Enter {
                serial, surface, ..
            } => {
                state.serial = serial;
                if let Some(window) = state.window_of(&surface) {
                    deliver(window, AuxEvent::Locks(state.locks));
                }
                state.focus = Some(surface);
            }
            E::Leave { .. } => state.focus = None,
            E::Key { serial, .. } => state.serial = serial,
            // The keymap's fd closes as the event drops: the masks are the core modifiers'.
            E::Modifiers {
                serial,
                mods_locked,
                ..
            } => {
                state.serial = serial;
                let locks = Locks {
                    caps: mods_locked & LOCK_MASK != 0,
                    num: mods_locked & MOD2_MASK != 0,
                };
                if locks != state.locks {
                    state.locks = locks;
                    if let Some(window) = state.focus.as_ref().and_then(|f| state.window_of(f)) {
                        deliver(window, AuxEvent::Locks(locks));
                    }
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<ZwpPointerGesturesV1, ()> for State {
    fn event(
        _: &mut Self,
        _: &ZwpPointerGesturesV1,
        _: <ZwpPointerGesturesV1 as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ZwpPointerGesturePinchV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &ZwpPointerGesturePinchV1,
        event: zwp_pointer_gesture_pinch_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use zwp_pointer_gesture_pinch_v1::Event as E;
        match event {
            E::Begin { surface, .. } => {
                state.pinch_on = state.window_of(&surface).map(|w| (w, 1.0));
                if let Some((window, _)) = state.pinch_on {
                    deliver(window, AuxEvent::Pinch(PinchInput::Begin));
                }
            }
            E::Update { scale, .. } if scale > 0.0 => {
                if let Some((window, last)) = &mut state.pinch_on {
                    let factor = scale / *last;
                    *last = scale;
                    deliver(*window, AuxEvent::Pinch(PinchInput::Scaled(factor)));
                }
            }
            E::End { .. } => {
                if let Some((window, _)) = state.pinch_on.take() {
                    deliver(window, AuxEvent::Pinch(PinchInput::End));
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<ZwpPointerGestureSwipeV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &ZwpPointerGestureSwipeV1,
        event: zwp_pointer_gesture_swipe_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use zwp_pointer_gesture_swipe_v1::Event as E;
        let (window, input) = match event {
            E::Begin {
                surface, fingers, ..
            } => {
                state.swipe_on = state.window_of(&surface);
                (state.swipe_on, SwipeInput::Begin { fingers })
            }
            E::Update { dx, dy, .. } => (state.swipe_on, SwipeInput::Update { dx, dy }),
            E::End { cancelled, .. } => {
                let input = SwipeInput::End {
                    cancelled: cancelled != 0,
                };
                (state.swipe_on.take(), input)
            }
            _ => return,
        };
        if let Some(window) = window {
            deliver(window, AuxEvent::Swipe { input, at: None });
        }
    }
}

impl Dispatch<WlCompositor, ()> for State {
    fn event(
        _: &mut Self,
        _: &WlCompositor,
        _: <WlCompositor as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

/// The drag icon's surface: its enter, leave and preferred scale are of no use.
impl Dispatch<WlSurface, ()> for State {
    fn event(
        _: &mut Self,
        _: &WlSurface,
        _: wl_surface::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl ShmHandler for State {
    fn shm_state(&mut self) -> &mut Shm {
        self.shm
            .as_mut()
            .expect("wl_shm events come only from the bound wl_shm")
    }
}

impl ProvidesRegistryState for State {
    registry_handlers![SeatState];

    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
}

delegate_registry!(State);
delegate_seat!(State);
delegate_pointer!(State);
delegate_data_device!(State);
delegate_primary_selection!(State);
delegate_shm!(State);

#[cfg(test)]
mod tests {
    use super::*;

    /// The types offered, with the bytes of each (a file's URI list).
    fn sent(data: &DragData, folder: &std::path::Path) -> Vec<(&'static str, Vec<u8>)> {
        offered(data, || folder.to_path_buf())
            .into_iter()
            .map(|(mime, body)| match body {
                Body::Bytes(bytes) => (mime, bytes.to_vec()),
                Body::File(file) => (mime, file.uri_list()),
            })
            .collect()
    }

    #[test]
    fn a_drag_out_offers_text_links_and_html() {
        let none = std::path::Path::new("/nowhere");
        let link = DragData {
            link_url: Some("https://a.test/".into()),
            link_title: Some("A test".into()),
            ..DragData::default()
        };
        let out = sent(&link, none);
        let types: Vec<_> = out.iter().map(|(m, _)| *m).collect();
        assert_eq!(
            types,
            [
                "text/uri-list",
                "text/x-moz-url",
                "text/plain;charset=utf-8",
                "UTF8_STRING",
                "text/plain"
            ]
        );
        // the link, then its title (Chromium's Wayland `text/x-moz-url`), and the URL as text
        assert_eq!(out[0].1, b"https://a.test/\r\n");
        assert_eq!(out[1].1, b"https://a.test/\nA test");
        assert_eq!(out[2].1, b"https://a.test/");
        let file = DragData {
            files: vec!["/tmp/a b.txt".into()],
            html: Some("<b>x</b>".into()),
            ..DragData::default()
        };
        let out = sent(&file, none);
        assert_eq!(
            out[0],
            ("text/uri-list", b"file:///tmp/a%20b.txt\r\n".to_vec())
        );
        assert_eq!(out[1].0, "text/html");
        assert!(sent(&DragData::default(), none).is_empty());
    }

    fn picture_drag(contents: Option<(&str, &[u8])>) -> DragData {
        DragData {
            link_url: Some("https://a.test/cat.jpg".into()),
            html: Some("<img src=\"https://a.test/cat.jpg\">".into()),
            file_contents: contents.map(|(name, bytes)| {
                Box::new(wubble_engine::page::DragFileContents {
                    name: Some(name.into()),
                    bytes: Arc::from(bytes),
                })
            }),
            image: Some(Box::new(DragImage {
                png: Arc::from(crate::shot_io::encode_png(1, 1, &[1, 2, 3, 255]).unwrap()),
                size: (1, 1),
                hotspot: (0, 0),
            })),
            ..DragData::default()
        }
    }

    #[test]
    fn a_picture_drag_offers_its_file_bytes_html_and_url() {
        let tmp = tempfile::tempdir().unwrap();
        let jpeg: &[u8] = b"\xff\xd8\xff\xe0jfif";
        let out = sent(&picture_drag(Some(("cat.png", jpeg))), tmp.path());
        let types: Vec<_> = out.iter().map(|(m, _)| *m).collect();
        assert_eq!(
            types,
            [
                "text/uri-list",
                "text/x-moz-url",
                "image/jpeg",
                "text/html",
                "text/plain;charset=utf-8",
                "UTF8_STRING",
                "text/plain"
            ]
        );
        // The file has the bytes' own extension, and is not written by offering it.
        let path = tmp.path().join("cat.jpg");
        assert_eq!(out[0].1, (dnd::file_uri(&path) + "\r\n").into_bytes());
        assert!(!path.exists());
        assert_eq!(out[2].1, jpeg);
        assert_eq!(out[6].1, b"https://a.test/cat.jpg");
        // Without the page's bytes there is no file and no picture: the drag
        // image is a faded thumbnail, so the link, HTML and URL go out alone.
        let out = sent(&picture_drag(None), tmp.path());
        assert_eq!(
            out[0],
            ("text/uri-list", b"https://a.test/cat.jpg\r\n".to_vec())
        );
        assert!(out.iter().all(|(m, _)| !m.starts_with("image/")));
        // A text drag's image is no picture.
        let text = DragData {
            text: Some("words".into()),
            html: Some("<b>words</b>".into()),
            ..picture_drag(None)
        };
        let types: Vec<_> = sent(&text, tmp.path())
            .into_iter()
            .map(|(m, _)| m)
            .collect();
        assert_eq!(types[0], "text/uri-list");
        assert!(!types.contains(&"image/png"));
    }

    #[test]
    fn the_drag_icon_is_premultiplied_argb_at_the_pngs_scale() {
        let png = crate::shot_io::encode_png(2, 1, &[255, 0, 0, 255, 255, 255, 255, 128]).unwrap();
        let (w, h, argb) = argb(&png).unwrap();
        assert_eq!((w, h), (2, 1));
        assert_eq!(argb, [0, 0, 255, 255, 128, 128, 128, 128]);
        assert!(super::argb(b"not a png").is_none());
        // Padded to the scale: a 3 x 1 image at scale 2 is 4 x 2, the new pixels clear.
        let px = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12];
        let (w, h, padded) = pad(3, 1, px.to_vec(), 2);
        assert_eq!((w, h), (4, 2));
        assert_eq!(&padded[..12], &px);
        assert!(padded[12..].iter().all(|&b| b == 0));
        assert_eq!(pad(4, 2, vec![7; 32], 2), (4, 2, vec![7; 32]));
        let image = |size: (i32, i32), hotspot| DragImage {
            png: Arc::from(&[][..]),
            size,
            hotspot,
        };
        // A 2x image: scale 2, the hotspot in surface units is its DIP.
        assert_eq!(
            icon_geometry(200, &image((100, 50), (10, 20)), true),
            (2, (-10, -20))
        );
        // 1.5x rounds to 2: the surface is 0.75 of the DIP size, and so is the offset.
        assert_eq!(
            icon_geometry(150, &image((100, 50), (10, 20)), true),
            (2, (-8, -15))
        );
        // No buffer scale: the pixels are the surface's.
        assert_eq!(
            icon_geometry(200, &image((100, 50), (10, 20)), false),
            (1, (-20, -40))
        );
        assert_eq!(
            icon_geometry(10, &image((0, 0), (3, 4)), true),
            (1, (-3, -4))
        );
    }

    #[test]
    fn a_drop_lands_where_the_pointer_was_released() {
        let w = WindowId(1);
        let mut entered = Incoming::Entered {
            window: w,
            offer: (),
            at: (5.0, 5.0),
        };
        assert_eq!(
            entered.moved((300.0, 200.0)),
            Some((w, AuxEvent::DropMove { x: 300.0, y: 200.0 }))
        );
        let (next, now) = entered.dropped();
        assert!(matches!(next, Incoming::None));
        assert_eq!(now, Some((w, AuxEvent::Dropped { x: 300.0, y: 200.0 }, ())));
        // While the data is read, motion is kept and the drop waits for it.
        let mut reading = Incoming::Reading {
            window: w,
            offer: (),
            generation: 1,
            at: (0.0, 0.0),
            dropped: false,
        };
        assert_eq!(reading.moved((40.0, 50.0)), None);
        let (next, now) = reading.dropped();
        assert_eq!(now, None);
        assert!(matches!(
            next,
            Incoming::Reading {
                at: (40.0, 50.0),
                dropped: true,
                ..
            }
        ));
    }

    #[test]
    fn a_two_finger_scroll_is_a_swipe_until_its_stop() {
        let axis = |absolute: f64, stop: bool| AxisScroll {
            absolute,
            stop,
            ..AxisScroll::default()
        };
        let finger = Some(AxisSource::Finger);
        let none = axis(0.0, false);
        // A finger's scroll to the left begins a swipe that goes back (positive dx).
        assert_eq!(
            scroll_swipe(false, finger, &axis(-8.0, false), &none),
            (
                true,
                vec![
                    SwipeInput::Begin { fingers: 2 },
                    SwipeInput::Update { dx: 8.0, dy: 0.0 }
                ]
            )
        );
        // Frames without a source go on with it; the stop ends it.
        assert_eq!(
            scroll_swipe(true, None, &axis(-4.0, false), &axis(1.0, false)),
            (true, vec![SwipeInput::Update { dx: 4.0, dy: -1.0 }])
        );
        assert_eq!(
            scroll_swipe(true, finger, &axis(0.0, true), &axis(0.0, true)),
            (false, vec![SwipeInput::End { cancelled: false }])
        );
        // A wheel never begins one and cancels one going on.
        let wheel = Some(AxisSource::Wheel);
        assert_eq!(
            scroll_swipe(false, wheel, &axis(-10.0, false), &none),
            (false, vec![])
        );
        assert_eq!(
            scroll_swipe(true, wheel, &axis(-10.0, false), &none),
            (false, vec![SwipeInput::End { cancelled: true }])
        );
        // A stop alone, or no motion, begins nothing.
        assert_eq!(
            scroll_swipe(false, finger, &axis(0.0, true), &none),
            (false, vec![])
        );
        assert_eq!(scroll_swipe(false, finger, &none, &none), (false, vec![]));
    }

    #[test]
    fn wayland_actions_map_both_ways() {
        let all = DragOps(DragOps::COPY.0 | DragOps::LINK.0 | DragOps::MOVE.0);
        assert_eq!(dnd_action(all), DndAction::Copy | DndAction::Move);
        assert_eq!(dnd_action(DragOps::LINK), DndAction::Copy);
        assert_eq!(dnd_action(DragOps::NONE), DndAction::empty());
        assert_eq!(drag_ops(DndAction::Move), DragOps::MOVE);
        assert_eq!(drag_ops(DndAction::Copy), DragOps::COPY);
        assert_eq!(drag_ops(DndAction::empty()), DragOps::NONE);
    }
}
