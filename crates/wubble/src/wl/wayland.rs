//! The Wayland worker: one thread with its own event queue on the windows'
//! `wl_display` (winit keeps its own queue), serving what winit 0.30 leaves
//! out on Wayland, for every window of the process:
//!
//! - the clipboard and the primary selection (`crate::clipboard`);
//! - a `wl_data_device` drop target: drags from other apps (files, links,
//!   HTML, text) reach the page under the pointer;
//! - a `wl_data_source` drag source: a page drag that leaves its window
//!   becomes an OS drag (text, a link as `text/uri-list`, HTML);
//! - `zwp_pointer_gestures_v1` pinch and swipe;
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
//! objects wayland-protocols' (MIT).

use std::io::{Read, Write};
use std::os::fd::{AsRawFd, BorrowedFd};
use std::sync::Arc;
use std::sync::mpsc;
use std::time::Duration;

use sctk::data_device_manager::data_device::{DataDevice, DataDeviceHandler};
use sctk::data_device_manager::data_offer::{DataOfferHandler, DragOffer};
use sctk::data_device_manager::data_source::{CopyPasteSource, DataSourceHandler, DragSource};
use sctk::data_device_manager::{DataDeviceManagerState, WritePipe};
use sctk::primary_selection::PrimarySelectionManagerState;
use sctk::primary_selection::device::{PrimarySelectionDevice, PrimarySelectionDeviceHandler};
use sctk::primary_selection::selection::{PrimarySelectionSource, PrimarySelectionSourceHandler};
use sctk::reexports::calloop::EventLoop;
use sctk::reexports::calloop::channel::{self, Channel, Sender};
use sctk::reexports::calloop_wayland_source::WaylandSource;
use sctk::reexports::client::backend::{Backend, ObjectId};
use sctk::reexports::client::globals::{GlobalList, registry_queue_init};
use sctk::reexports::client::protocol::wl_data_device::WlDataDevice;
use sctk::reexports::client::protocol::wl_data_device_manager::DndAction;
use sctk::reexports::client::protocol::wl_data_source::WlDataSource;
use sctk::reexports::client::protocol::wl_keyboard::{self, WlKeyboard};
use sctk::reexports::client::protocol::wl_pointer::WlPointer;
use sctk::reexports::client::protocol::wl_seat::WlSeat;
use sctk::reexports::client::protocol::wl_surface::WlSurface;
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
use sctk::seat::pointer::{PointerEvent, PointerEventKind, PointerHandler};
use sctk::seat::{Capability, SeatHandler, SeatState};
use sctk::{
    delegate_data_device, delegate_pointer, delegate_primary_selection, delegate_registry,
    delegate_seat, registry_handlers,
};
use slint::winit_030::WinitWindowAccessor;
use slint::winit_030::winit::raw_window_handle::{
    HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle,
};
use wubble_core::dnd::{self, Kind, UriList};
use wubble_core::gesture::{PinchInput, SwipeInput};
use wubble_core::window::WindowId;
use wubble_engine::input::Locks;
use wubble_engine::page::{DragData, DragOps};

use super::{Aux, AuxEvent, deliver};
use crate::clipboard::Buffer;

/// The most a drop's data may be, per type: a page drop is files' paths, a
/// link or a selection, not the files themselves.
const DROP_CAP: u64 = 8 << 20;

/// How long a paste waits for the selection's owner to send it.
const LOAD_WAIT: Duration = Duration::from_secs(2);

/// `wl_keyboard.modifiers` masks of the keymap's real modifiers. The eight
/// core modifiers have fixed indices in every xkb keymap (Shift 0, Lock 1,
/// Control 2, Mod1 to Mod5 3 to 7), and Num Lock is Mod2 in xkeyboard-config's
/// layouts, so the keymap itself need not be read.
const LOCK_MASK: u32 = 1 << 1;
const MOD2_MASK: u32 = 1 << 4;

/// The text types the clipboard offers and takes (smithay-clipboard's).
const TEXT_TYPES: [&str; 3] = ["text/plain;charset=utf-8", "UTF8_STRING", "text/plain"];

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
    Store(Buffer, String),
    /// Read the selection; the answer goes to the loads channel with this serial.
    Load(Buffer, u64),
    Exit,
}

/// The process's worker; dropping it stops the thread.
pub struct Worker {
    connection: Connection,
    tx: Sender<Command>,
    loads: mpsc::Receiver<(u64, Option<String>)>,
    load_serial: std::cell::Cell<u64>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Worker {
    /// The worker on `window`'s Wayland connection; none when it is not a Wayland window.
    pub fn open(window: &slint::Window) -> Option<Self> {
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
            .spawn(move || run(worker_connection, rx, worker_tx, load_tx))
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

    /// Puts `text` on `buffer`.
    pub fn store(&self, buffer: Buffer, text: &str) {
        let _ = self.tx.send(Command::Store(buffer, text.to_owned()));
    }

    /// The text on `buffer`, waiting a little for its owner to send it.
    pub fn load(&self, buffer: Buffer) -> Option<String> {
        let serial = self.load_serial.get() + 1;
        self.load_serial.set(serial);
        self.tx.send(Command::Load(buffer, serial)).ok()?;
        loop {
            match self.loads.recv_timeout(LOAD_WAIT) {
                Ok((s, text)) if s == serial => return text,
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
    loads: mpsc::Sender<(u64, Option<String>)>,
) {
    let Ok((globals, queue)) = registry_queue_init::<State>(&connection) else {
        return;
    };
    let qh = queue.handle();
    let Ok(mut event_loop) = EventLoop::<State>::try_new() else {
        return;
    };
    let mut state = State::new(&globals, &qh, tx, loads);
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

/// A drop from another app, from its `enter`.
enum Incoming {
    None,
    /// Its data is being read; motion and the drop wait for it.
    Reading {
        window: WindowId,
        offer: DragOffer,
        generation: u64,
        at: (f32, f32),
        dropped: bool,
    },
    /// The page has it.
    Entered {
        window: WindowId,
        offer: DragOffer,
    },
}

struct State {
    registry_state: RegistryState,
    seat_state: SeatState,
    manager: Option<DataDeviceManagerState>,
    primary_manager: Option<PrimarySelectionManagerState>,
    gestures: Option<ZwpPointerGesturesV1>,
    qh: QueueHandle<State>,
    tx: Sender<Command>,
    loads: mpsc::Sender<(u64, Option<String>)>,
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
    locks: Locks,

    drop: Incoming,
    generation: u64,
    /// The OS drag a page drag became: its window, what it carries, the action chosen.
    source: Option<(WindowId, DragSource, Arc<Offered>, DndAction)>,
    /// The selections stored, while offered. The old one is destroyed before a new one is
    /// set: the compositor (mutter, weston) ignores a `set_selection` whose serial is not newer
    /// than the current selection's, and the serial stays the same between input events.
    clipboard: Option<(CopyPasteSource, Arc<[u8]>)>,
    primary: Option<(PrimarySelectionSource, Arc<[u8]>)>,
}

/// What a drag out of the window offers, by MIME type.
struct Offered(Vec<(&'static str, Vec<u8>)>);

impl State {
    fn new(
        globals: &GlobalList,
        qh: &QueueHandle<Self>,
        tx: Sender<Command>,
        loads: mpsc::Sender<(u64, Option<String>)>,
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
            locks: Locks::default(),
            drop: Incoming::None,
            generation: 0,
            source: None,
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
            Command::Exit => self.exit = true,
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
            Command::Store(buffer, text) => self.store(buffer, text),
            Command::Load(buffer, serial) => {
                if !self.load(buffer, serial) {
                    let _ = self.loads.send((serial, None));
                }
            }
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
            self.drop = Incoming::Entered { window, offer };
        }
    }

    fn start_drag(&mut self, window: WindowId, data: DragData, allowed: DragOps) {
        let offered = Arc::new(Offered(offered(&data)));
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
        source.start_drag(device, surface, None, serial);
        self.source = Some((window, source, offered, DndAction::empty()));
    }

    /// The OS drag ended: the page drag ends with the action done.
    fn source_ended(&mut self, source: &WlDataSource, finished: bool) {
        let Some((window, ours, _, action)) = &self.source else {
            return;
        };
        if ours.inner() != source {
            return;
        }
        let (window, op) = (
            *window,
            if finished {
                drag_ops(*action)
            } else {
                DragOps::NONE
            },
        );
        self.source = None;
        deliver(window, AuxEvent::DragEnded(op));
    }

    fn store(&mut self, buffer: Buffer, text: String) {
        if self.focus.is_none() {
            return;
        }
        let bytes: Arc<[u8]> = Arc::from(text.into_bytes());
        match buffer {
            Buffer::Clipboard => {
                let (Some(manager), Some(device)) = (&self.manager, &self.device) else {
                    return;
                };
                self.clipboard = None;
                let source = manager.create_copy_paste_source(&self.qh, TEXT_TYPES);
                source.set_selection(device, self.serial);
                self.clipboard = Some((source, bytes));
            }
            Buffer::Primary => {
                let (Some(manager), Some(device)) = (&self.primary_manager, &self.primary_device)
                else {
                    return;
                };
                self.primary = None;
                let source = manager.create_selection_source(&self.qh, TEXT_TYPES);
                source.set_selection(device, self.serial);
                self.primary = Some((source, bytes));
            }
        }
    }

    /// Starts reading `buffer`; false when there is nothing to read.
    fn load(&mut self, buffer: Buffer, serial: u64) -> bool {
        let pipe = match buffer {
            Buffer::Clipboard => self
                .device
                .as_ref()
                .and_then(|d| d.data().selection_offer())
                .and_then(|offer| {
                    let mime = offer.with_mime_types(text_type)?;
                    offer.receive(mime.to_owned()).ok()
                }),
            Buffer::Primary => self
                .primary_device
                .as_ref()
                .and_then(|d| d.data().selection_offer())
                .and_then(|offer| {
                    let mime = offer.with_mime_types(text_type)?;
                    offer.receive(mime.to_owned()).ok()
                }),
        };
        // Safety: the pipe is open while `pipe` lives; the clone outlives it.
        let Some(file) = pipe.and_then(|p| own(unsafe { BorrowedFd::borrow_raw(p.as_raw_fd()) }))
        else {
            return false;
        };
        let loads = self.loads.clone();
        std::thread::Builder::new()
            .name("wubble-paste".into())
            .spawn(move || {
                let mut bytes = vec![];
                let text = file
                    .take(DROP_CAP)
                    .read_to_end(&mut bytes)
                    .ok()
                    .map(|_| normalize_to_lf(String::from_utf8_lossy(&bytes).into_owned()));
                let _ = loads.send((serial, text));
            })
            .is_ok()
    }

    /// Writes the stored selection to a paste's pipe.
    fn send_selection(bytes: Arc<[u8]>, mime: &str, pipe: WritePipe) {
        if !TEXT_TYPES.contains(&mime) {
            return;
        }
        write_on_thread(&pipe, bytes.to_vec());
    }
}

/// The clipboard's text type among `offered` (smithay-clipboard's `find_allowed`: the
/// UTF-8 ones, else plain `text/plain`).
fn text_type(offered: &[String]) -> Option<&'static str> {
    TEXT_TYPES
        .iter()
        .find(|t| offered.iter().any(|o| o == *t))
        .copied()
}

/// CR and CRLF as LF (smithay-clipboard `mime.rs` `normalize_to_lf`).
fn normalize_to_lf(text: String) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

/// The drag-out's types: text, a link as `text/uri-list` (and its URL as
/// text when there is no text), HTML, files as `text/uri-list`.
fn offered(data: &DragData) -> Vec<(&'static str, Vec<u8>)> {
    let mut out = vec![];
    let uris: Vec<String> = data
        .files
        .iter()
        .map(|p| format!("file://{}", percent_encode(&p.to_string_lossy())))
        .chain(data.link_url.clone())
        .collect();
    if !uris.is_empty() {
        out.push(("text/uri-list", (uris.join("\r\n") + "\r\n").into_bytes()));
    }
    if let Some(html) = &data.html {
        out.push(("text/html", html.clone().into_bytes()));
    }
    if let Some(text) = data.text.as_ref().or(data.link_url.as_ref()) {
        for mime in TEXT_TYPES {
            out.push((mime, text.clone().into_bytes()));
        }
    }
    out
}

fn percent_encode(path: &str) -> String {
    path.bytes()
        .map(|b| match b {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'/' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
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

/// Writes `bytes` to the pipe on a thread of its own (the reader may be slow, or us).
fn write_on_thread(pipe: &WritePipe, bytes: Vec<u8>) {
    // Safety: the pipe is open while `pipe` lives; the clone outlives it.
    let Some(mut file) = own(unsafe { BorrowedFd::borrow_raw(pipe.as_raw_fd()) }) else {
        return;
    };
    let _ = std::thread::Builder::new()
        .name("wubble-send".into())
        .spawn(move || {
            let _ = file.write_all(&bytes);
        });
}

/// Reads a drop's types on a thread and hands the data back to the worker.
fn read_drop(
    offer: &DragOffer,
    plan: Vec<(Kind, &'static str)>,
    generation: u64,
    tx: Sender<Command>,
) {
    let pipes: Vec<(Kind, std::fs::File)> = plan
        .into_iter()
        .filter_map(|(kind, mime)| {
            let pipe = offer.receive(mime.to_owned()).ok()?;
            // Safety: the pipe is open while `pipe` lives; the clone outlives it.
            let file = own(unsafe { BorrowedFd::borrow_raw(pipe.as_raw_fd()) })?;
            Some((kind, file))
        })
        .collect();
    let spawned = std::thread::Builder::new()
        .name("wubble-drop".into())
        .spawn(move || {
            let mut data = DragData::default();
            for (kind, file) in pipes {
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
        let (x, y) = (x as f32, y as f32);
        match &mut self.drop {
            Incoming::Reading { at, .. } => *at = (x, y),
            Incoming::Entered { window, .. } => deliver(*window, AuxEvent::DropMove { x, y }),
            Incoming::None => {}
        }
    }

    fn selection(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataDevice) {}

    fn drop_performed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataDevice) {
        match std::mem::replace(&mut self.drop, Incoming::None) {
            Incoming::Entered { window, offer } => {
                let (x, y) = (offer.x as f32, offer.y as f32);
                deliver(window, AuxEvent::Dropped { x, y });
                finish(&offer);
            }
            Incoming::Reading {
                window,
                offer,
                generation,
                at,
                ..
            } => {
                self.drop = Incoming::Reading {
                    window,
                    offer,
                    generation,
                    at,
                    dropped: true,
                }
            }
            Incoming::None => {}
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
        if let Some((_, drag, offered, _)) = &self.source
            && drag.inner() == source
        {
            if let Some((_, bytes)) = offered.0.iter().find(|(m, _)| *m == mime) {
                write_on_thread(&pipe, bytes.clone());
            }
            return;
        }
        if let Some((copy, bytes)) = &self.clipboard
            && copy.inner() == source
        {
            State::send_selection(bytes.clone(), &mime, pipe);
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
        if let Some((_, ours, _, chosen)) = &mut self.source
            && ours.inner() == source
        {
            *chosen = action;
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
        if let Some((ours, bytes)) = &self.primary
            && ours.inner() == source
        {
            State::send_selection(bytes.clone(), &mime, pipe);
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
                _ => {}
            }
        }
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
            deliver(window, AuxEvent::Swipe(input));
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_drag_out_offers_text_links_and_html() {
        let link = DragData {
            link_url: Some("https://a.test/".into()),
            ..DragData::default()
        };
        let types: Vec<_> = offered(&link).into_iter().map(|(m, _)| m).collect();
        assert_eq!(
            types,
            [
                "text/uri-list",
                "text/plain;charset=utf-8",
                "UTF8_STRING",
                "text/plain"
            ]
        );
        let file = DragData {
            files: vec!["/tmp/a b.txt".into()],
            html: Some("<b>x</b>".into()),
            ..DragData::default()
        };
        let out = offered(&file);
        assert_eq!(
            out[0],
            ("text/uri-list", b"file:///tmp/a%20b.txt\r\n".to_vec())
        );
        assert_eq!(out[1].0, "text/html");
        assert!(offered(&DragData::default()).is_empty());
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

    #[test]
    fn pastes_take_utf8_text_first() {
        let offered = |t: &[&str]| t.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            text_type(&offered(&["text/plain", "UTF8_STRING"])),
            Some("UTF8_STRING")
        );
        assert_eq!(text_type(&offered(&["text/plain"])), Some("text/plain"));
        assert_eq!(text_type(&offered(&["image/png"])), None);
        assert_eq!(normalize_to_lf("a\r\nb\rc".into()), "a\nb\nc");
    }
}
