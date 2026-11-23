//! The system clipboard and the primary selection, through the window's own
//! Wayland connection.
//!
//! The pages' Chromium cannot reach them on Wayland (it has no surface, so no
//! input serial to set the selection with and no keyboard focus to be offered
//! one), so their copies and pastes come here (wubble-helper `clipboard.js`,
//! `PageEvent::Clipboard`). The window has both. On Wayland they go through
//! wubble's one Wayland worker (`crate::wl::wayland`, ported from
//! smithay-clipboard, the crate Slint's winit backend uses for the same job): a
//! page's copy carries every type it has, a paste every type a page takes
//! (`wubble_core::clipboard`), the primary selection text only. Copy Image goes
//! there too (`host::builtins`), so one data source owns the selection;
//! screenshots go through the one worker that owns an `arboard` clipboard for
//! them (`shot_io`), which offers `image/png`, as Copy Image does elsewhere.
//! Elsewhere (X11, macOS, Windows) Chromium's own clipboard works and the
//! bridge is off for the pages; on X11 the shell's own copies go through Slint,
//! on macOS and Windows through arboard (the same system clipboard, which the
//! parity smoke also reads back).

use wubble_core::clipboard::{Payload, Reader};

/// Which clipboard.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Buffer {
    Clipboard,
    /// The X11/Wayland primary selection (select to copy, middle-click to paste).
    Primary,
}

/// Whether the pages' clipboard goes through the host: Linux under Wayland, where
/// Chromium picks its Wayland platform.
pub fn bridge_needed(wayland_display: Option<&std::ffi::OsStr>) -> bool {
    cfg!(target_os = "linux") && wayland_display.is_some_and(|d| !d.is_empty())
}

/// Whether this run's pages' clipboard goes through the host ([`bridge_needed`] for its
/// session): what the engine is started with, before any window. The host's own choices
/// follow the window ([`Bridge::carries_pages`]).
pub fn bridged() -> bool {
    bridge_needed(std::env::var_os("WAYLAND_DISPLAY").as_deref())
}

/// The host side of the pages' clipboard. One impl per OS, chosen by cfg into [`Clipboard`].
pub trait Bridge {
    /// The clipboard of a run whose CEF cache is `cache_dir` (where a picture dragged out to
    /// another app is written, `crate::wl`).
    fn new(cache_dir: &std::path::Path) -> Self;

    /// Opens the clipboard now, before the window takes keyboard focus: the compositor
    /// offers the selection to a client when focus enters, so a clipboard opened later has
    /// nothing to paste until the selection changes.
    fn open(&mut self, window: &slint::Window);

    /// Puts `text` on `buffer`; false when this window has no host clipboard or it was not set,
    /// and the caller copies through Slint instead.
    fn store(&mut self, window: &slint::Window, buffer: Buffer, text: &str) -> bool;

    /// The text on `buffer`, if any.
    fn load(&mut self, window: &slint::Window, buffer: Buffer) -> Option<String>;

    /// Whether the host carries the pages' clipboard in `window` (its Wayland connection).
    fn carries_pages(&mut self, _window: &slint::Window) -> bool {
        false
    }

    /// Puts a page's copy on the clipboard under every type it has; false where the host does
    /// not carry the pages' clipboard (Chromium's own does) or it was not set, and the caller
    /// copies its text.
    fn store_rich(&mut self, _window: &slint::Window, _payload: Payload) -> bool {
        false
    }

    /// Reads what `reader` takes from `buffer` and hands it to `done` (on another thread, `None`
    /// when it holds nothing `reader` takes), without waiting; false where the host does not
    /// carry the pages' clipboard, and `done` is dropped uncalled.
    fn load_then(
        &mut self,
        _window: &slint::Window,
        _buffer: Buffer,
        _reader: Reader,
        _done: Box<dyn FnOnce(Option<Payload>) + Send>,
    ) -> bool {
        false
    }

    /// The platform's extras on the same connection (`crate::wl`), once open.
    fn extras(&self) -> Option<&dyn crate::wl::Aux> {
        None
    }
}

#[cfg(target_os = "linux")]
pub type Clipboard = wayland::Wayland;
#[cfg(not(target_os = "linux"))]
pub type Clipboard = Arboard;

// Slint's own text fields (the shell's: the command bar, the start page's capsule, the find bar,
// every kit field) copy and paste through Slint's platform (`crate::slint_backend`), which asks
// these first. The compositor offers the selection to one `wl_data_device` per client, the
// worker's, never to the one Slint's winit backend opens for itself, so Slint's own clipboard
// pastes nothing on Wayland; the host's is the only one that works there.
#[cfg(target_os = "linux")]
pub use wayland::{field_load, field_store};

/// Puts `text` on `buffer` for a Slint text field; false when the host has no clipboard of its
/// own here, and Slint's serves.
#[cfg(not(target_os = "linux"))]
pub fn field_store(_buffer: Buffer, _text: &str) -> bool {
    false
}

/// The text on `buffer` for a Slint text field's paste; `None` when the host has no clipboard
/// of its own here, and Slint's serves.
#[cfg(not(target_os = "linux"))]
pub fn field_load(_buffer: Buffer) -> Option<Option<String>> {
    None
}

/// macOS and Windows: the system clipboard through arboard (MIT OR Apache-2.0). Chromium's own
/// clipboard serves the pages there, so this carries only the shell's own copies and the parity
/// smoke's reads. They have no primary selection.
#[cfg(not(target_os = "linux"))]
#[derive(Default)]
pub struct Arboard {
    /// Opened on first use.
    clipboard: Option<arboard::Clipboard>,
    /// Drags out of and into the windows (`crate::wl::macos`).
    #[cfg(target_os = "macos")]
    mac: crate::wl::macos::Mac,
}

#[cfg(not(target_os = "linux"))]
impl Arboard {
    fn get(&mut self) -> Option<&mut arboard::Clipboard> {
        if self.clipboard.is_none() {
            self.clipboard = arboard::Clipboard::new()
                .inspect_err(|e| eprintln!("wubble: no system clipboard: {e}"))
                .ok();
        }
        self.clipboard.as_mut()
    }

    fn store_text(&mut self, buffer: Buffer, text: &str) -> bool {
        match (buffer, self.get()) {
            (Buffer::Clipboard, Some(clipboard)) => clipboard
                .set_text(text)
                .inspect_err(|e| eprintln!("wubble: clipboard: {e}"))
                .is_ok(),
            _ => false,
        }
    }

    fn load_text(&mut self, buffer: Buffer) -> Option<String> {
        match (buffer, self.get()) {
            (Buffer::Clipboard, Some(clipboard)) => clipboard
                .get_text()
                .inspect_err(|e| log::debug!("clipboard: nothing to paste: {e}"))
                .ok(),
            _ => None,
        }
    }
}

#[cfg(not(target_os = "linux"))]
impl Bridge for Arboard {
    fn new(_cache_dir: &std::path::Path) -> Self {
        Self::default()
    }

    fn open(&mut self, _window: &slint::Window) {
        self.get();
    }

    fn store(&mut self, _window: &slint::Window, buffer: Buffer, text: &str) -> bool {
        self.store_text(buffer, text)
    }

    fn load(&mut self, _window: &slint::Window, buffer: Buffer) -> Option<String> {
        self.load_text(buffer)
    }

    #[cfg(target_os = "macos")]
    fn extras(&self) -> Option<&dyn crate::wl::Aux> {
        Some(&self.mac)
    }
}

#[cfg(target_os = "linux")]
mod wayland {
    use std::cell::RefCell;
    use std::rc::{Rc, Weak};

    use super::{Bridge, Buffer, Payload, Reader};
    use crate::wl::wayland::Worker;

    thread_local! {
        /// The host's open worker, for Slint's text fields (`super::field_load`). The host
        /// owns it: it closes with the host's clipboard.
        static OPEN: RefCell<Weak<Worker>> = const { RefCell::new(Weak::new()) };
    }

    /// Puts `text` on `buffer` for a Slint text field; false before the host opened its
    /// clipboard (or under X11, where it has none), and Slint's serves.
    pub fn field_store(buffer: Buffer, text: &str) -> bool {
        OPEN.with_borrow(Weak::upgrade)
            .map(|worker| worker.store(buffer, text))
            .is_some()
    }

    /// The text on `buffer` for a Slint text field's paste; `None` when the host has no
    /// clipboard open, and Slint's serves.
    pub fn field_load(buffer: Buffer) -> Option<Option<String>> {
        OPEN.with_borrow(Weak::upgrade)
            .map(|worker| worker.load(buffer))
    }

    /// The window's Wayland clipboard; none under X11, where Chromium's own works. The
    /// worker also serves drops, drags out, gestures and lock keys (`crate::wl`): one data
    /// device for the process.
    pub struct Wayland {
        /// Opened on first use: the window's Wayland connection exists by then.
        worker: Option<Rc<Worker>>,
        /// Where pictures dragged out are written (`crate::wl::drag_file`).
        drag_dir: std::path::PathBuf,
    }

    impl Wayland {
        fn get(&mut self, window: &slint::Window) -> Option<&Worker> {
            if self.worker.is_none() {
                self.worker = Worker::open(window, self.drag_dir.clone()).map(Rc::new);
                if let Some(worker) = &self.worker {
                    OPEN.set(Rc::downgrade(worker));
                }
            }
            self.worker.as_deref()
        }
    }

    impl Bridge for Wayland {
        fn new(cache_dir: &std::path::Path) -> Self {
            Self {
                worker: None,
                drag_dir: crate::wl::drag_file::base_dir(cache_dir),
            }
        }

        fn open(&mut self, window: &slint::Window) {
            self.get(window);
        }

        fn store(&mut self, window: &slint::Window, buffer: Buffer, text: &str) -> bool {
            self.get(window).is_some_and(|w| w.store(buffer, text))
        }

        fn load(&mut self, window: &slint::Window, buffer: Buffer) -> Option<String> {
            self.get(window)?.load(buffer)
        }

        fn carries_pages(&mut self, window: &slint::Window) -> bool {
            self.get(window).is_some()
        }

        fn store_rich(&mut self, window: &slint::Window, payload: Payload) -> bool {
            self.get(window)
                .is_some_and(|w| w.store_payload(Buffer::Clipboard, payload))
        }

        fn load_then(
            &mut self,
            window: &slint::Window,
            buffer: Buffer,
            reader: Reader,
            done: Box<dyn FnOnce(Option<Payload>) + Send>,
        ) -> bool {
            let Some(worker) = self.get(window) else {
                return false;
            };
            worker.load_then(buffer, reader, done);
            true
        }

        fn extras(&self) -> Option<&dyn crate::wl::Aux> {
            self.worker.as_deref().map(|w| w as &dyn crate::wl::Aux)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsStr;

    #[test]
    fn the_bridge_runs_under_wayland_on_linux_only() {
        assert_eq!(
            bridge_needed(Some(OsStr::new("wayland-0"))),
            cfg!(target_os = "linux")
        );
        assert!(!bridge_needed(Some(OsStr::new(""))));
        assert!(!bridge_needed(None));
    }

    /// There is no primary selection here: storing to it fails and loading from it is empty,
    /// without touching the system clipboard.
    #[cfg(not(target_os = "linux"))]
    #[test]
    fn arboard_has_no_primary_selection() {
        let mut bridge = Arboard::default();
        assert!(!bridge.store_text(Buffer::Primary, "wubble clipboard test"));
        assert_eq!(bridge.load_text(Buffer::Primary), None);
    }

    /// What is stored is what loads back. Overwrites the system clipboard and races any other
    /// app writing to it, so it runs by hand only (`cargo test -- --ignored`); the parity smoke
    /// covers the round trip.
    #[cfg(not(target_os = "linux"))]
    #[test]
    #[ignore = "overwrites the system clipboard"]
    fn arboard_stores_and_loads_text() {
        let mut bridge = Arboard::default();
        let text = "wubble clipboard test";
        assert!(bridge.store_text(Buffer::Clipboard, text));
        assert_eq!(bridge.load_text(Buffer::Clipboard).as_deref(), Some(text));
    }
}
