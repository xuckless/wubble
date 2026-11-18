//! The system clipboard and the primary selection, through the window's own
//! Wayland connection.
//!
//! The pages' Chromium cannot reach them on Wayland (it has no surface, so no
//! input serial to set the selection with and no keyboard focus to be offered
//! one), so their copies and pastes come here (welding `clipboard_bridge`,
//! `PageEvent::Clipboard`). The window has both. smithay-clipboard (MIT) is the
//! crate Slint's winit backend uses for the same job; it carries text only.
//! Elsewhere (X11, macOS, Windows) Chromium's own clipboard works and the
//! bridge is off; the shell's own copies then go through Slint.

#[cfg(target_os = "linux")]
use slint::winit_030::WinitWindowAccessor;
#[cfg(target_os = "linux")]
use slint::winit_030::winit::raw_window_handle::{HasDisplayHandle, RawDisplayHandle};

#[cfg(target_os = "linux")]
type Wayland = smithay_clipboard::Clipboard;
/// No Wayland clipboard off Linux.
#[cfg(not(target_os = "linux"))]
enum Wayland {}

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

#[derive(Default)]
pub struct Clipboard {
    /// Opened on first use: the window's Wayland connection exists by then.
    wayland: Option<Wayland>,
}

impl Clipboard {
    #[cfg(not(target_os = "linux"))]
    fn wayland(&mut self, _window: &slint::Window) -> Option<&Wayland> {
        None
    }

    #[cfg(target_os = "linux")]
    fn wayland(&mut self, window: &slint::Window) -> Option<&Wayland> {
        if self.wayland.is_none() {
            let display = window
                .with_winit_window(|w| match w.display_handle().map(|h| h.as_raw()) {
                    Ok(RawDisplayHandle::Wayland(h)) => Some(h.display.as_ptr()),
                    _ => None,
                })
                .flatten()?;
            // Safety: the display is winit's live `wl_display`, which outlives
            // the window and so this clipboard (the host drops it first).
            self.wayland = Some(unsafe { smithay_clipboard::Clipboard::new(display) });
        }
        self.wayland.as_ref()
    }

    /// Opens the Wayland clipboard now, before the window takes keyboard focus: the
    /// compositor offers the selection to a client when focus enters, so a clipboard
    /// opened later has nothing to paste until the selection changes.
    pub fn open(&mut self, window: &slint::Window) {
        self.wayland(window);
    }

    /// Puts `text` on `buffer`; false when the window is not a Wayland one.
    pub fn store(&mut self, window: &slint::Window, buffer: Buffer, text: &str) -> bool {
        let Some(clipboard) = self.wayland(window) else {
            return false;
        };
        #[cfg(not(target_os = "linux"))]
        let _ = (clipboard, buffer, text);
        #[cfg(target_os = "linux")]
        match buffer {
            Buffer::Clipboard => clipboard.store(text),
            Buffer::Primary => clipboard.store_primary(text),
        }
        true
    }

    /// The text on `buffer`, if any.
    pub fn load(&mut self, window: &slint::Window, buffer: Buffer) -> Option<String> {
        let clipboard = self.wayland(window)?;
        #[cfg(not(target_os = "linux"))]
        return match *clipboard {};
        #[cfg(target_os = "linux")]
        {
            let text = match buffer {
                Buffer::Clipboard => clipboard.load(),
                Buffer::Primary => clipboard.load_primary(),
            };
            text.inspect_err(|e| log::debug!("clipboard: nothing to paste: {e}"))
                .ok()
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
}
