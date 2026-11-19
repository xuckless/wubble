//! What winit does not give a window and the platform has: drops from and drags
//! to other apps, touchpad pinch and swipe, the lock keys. One impl per
//! platform behind [`Aux`]: on Linux under Wayland one worker on the windows'
//! Wayland connection (`wayland.rs`), which also carries the clipboard
//! (`crate::clipboard`): a client has one `wl_data_device` per seat, or the
//! compositor stops offering the selection to the clipboard's. Elsewhere none,
//! and these come from winit where it has them (macOS pinch, X11, macOS and
//! Windows file drops).

use wubble_core::gesture::{PinchInput, SwipeInput};
use wubble_core::window::WindowId;
use wubble_engine::input::Locks;
use wubble_engine::page::{DragData, DragOps};

#[cfg(target_os = "linux")]
pub mod wayland;

/// What the platform reports, delivered on the Slint thread to the window's
/// host (`Host::aux_event`). Points are window-relative logical px.
#[derive(Clone, Debug, PartialEq)]
pub enum AuxEvent {
    /// A drag from another app entered the window, carrying this.
    DropEnter {
        x: f32,
        y: f32,
        data: DragData,
    },
    DropMove {
        x: f32,
        y: f32,
    },
    DropLeave,
    Dropped {
        x: f32,
        y: f32,
    },
    /// The OS drag started by [`Aux::start_drag`] ended with this done.
    DragEnded(DragOps),
    Pinch(PinchInput),
    Swipe(SwipeInput),
    Locks(Locks),
}

pub trait Aux {
    /// The window's platform window exists: its events are reported from now on.
    fn add_window(&self, id: WindowId, window: &slint::Window);
    fn remove_window(&self, id: WindowId);
    /// Hands a page drag of window `id` that left it to the OS; false when it
    /// cannot, and the drag ends there.
    fn start_drag(&self, id: WindowId, data: &DragData, allowed: DragOps) -> bool;
    /// What the page under a drop from another app would do with it.
    fn drop_action(&self, op: DragOps);
    /// Whether a page drag that leaves the window becomes the OS's.
    fn drags_out(&self) -> bool;
}

/// No platform extras.
pub struct Null;

impl Aux for Null {
    fn add_window(&self, _id: WindowId, _window: &slint::Window) {}

    fn remove_window(&self, _id: WindowId) {}

    fn start_drag(&self, _id: WindowId, _data: &DragData, _allowed: DragOps) -> bool {
        false
    }

    fn drop_action(&self, _op: DragOps) {}

    fn drags_out(&self) -> bool {
        false
    }
}

/// Hands `event` for window `id` to the host on the Slint thread.
pub fn deliver(id: WindowId, event: AuxEvent) {
    let _ = slint::invoke_from_event_loop(move || {
        crate::with_host_in(id, |host| host.aux_event(event))
    });
}
