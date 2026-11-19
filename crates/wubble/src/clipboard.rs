//! The system clipboard and the primary selection, through the window's own
//! Wayland connection.
//!
//! The pages' Chromium cannot reach them on Wayland (it has no surface, so no
//! input serial to set the selection with and no keyboard focus to be offered
//! one), so their copies and pastes come here (wubble-helper `clipboard.js`,
//! `PageEvent::Clipboard`). The window has both. smithay-clipboard (MIT) is the
//! crate Slint's winit backend uses for the same job; it carries text only.
//! Elsewhere (X11, macOS, Windows) Chromium's own clipboard works and the
//! bridge is off; the shell's own copies then go through Slint.

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

/// The host side of the pages' clipboard. One impl per OS, chosen by cfg into [`Clipboard`].
pub trait Bridge: Default {
    /// Opens the clipboard now, before the window takes keyboard focus: the compositor
    /// offers the selection to a client when focus enters, so a clipboard opened later has
    /// nothing to paste until the selection changes.
    fn open(&mut self, window: &slint::Window);

    /// Puts `text` on `buffer`; false when this window has no host clipboard, and the
    /// caller copies through Slint instead.
    fn store(&mut self, window: &slint::Window, buffer: Buffer, text: &str) -> bool;

    /// The text on `buffer`, if any.
    fn load(&mut self, window: &slint::Window, buffer: Buffer) -> Option<String>;
}

#[cfg(target_os = "linux")]
pub type Clipboard = wayland::Wayland;
#[cfg(not(target_os = "linux"))]
pub type Clipboard = NoBridge;

/// macOS and Windows: Chromium's own clipboard serves the pages, so nothing is bridged and
/// the shell's own copies go through Slint.
#[cfg_attr(target_os = "linux", allow(dead_code))]
#[derive(Default)]
pub struct NoBridge;

impl Bridge for NoBridge {
    fn open(&mut self, _window: &slint::Window) {}

    fn store(&mut self, _window: &slint::Window, _buffer: Buffer, _text: &str) -> bool {
        false
    }

    fn load(&mut self, _window: &slint::Window, _buffer: Buffer) -> Option<String> {
        None
    }
}

#[cfg(target_os = "linux")]
mod wayland {
    use super::{Bridge, Buffer};
    use slint::winit_030::WinitWindowAccessor;
    use slint::winit_030::winit::raw_window_handle::{HasDisplayHandle, RawDisplayHandle};

    /// The window's Wayland clipboard; none under X11, where Chromium's own works.
    #[derive(Default)]
    pub struct Wayland {
        /// Opened on first use: the window's Wayland connection exists by then.
        clipboard: Option<smithay_clipboard::Clipboard>,
    }

    impl Wayland {
        fn get(&mut self, window: &slint::Window) -> Option<&smithay_clipboard::Clipboard> {
            if self.clipboard.is_none() {
                let display = window
                    .with_winit_window(|w| match w.display_handle().map(|h| h.as_raw()) {
                        Ok(RawDisplayHandle::Wayland(h)) => Some(h.display.as_ptr()),
                        _ => None,
                    })
                    .flatten()?;
                // Safety: the display is winit's live `wl_display`, which outlives
                // the window and so this clipboard (the host drops it first).
                self.clipboard = Some(unsafe { smithay_clipboard::Clipboard::new(display) });
            }
            self.clipboard.as_ref()
        }
    }

    impl Bridge for Wayland {
        fn open(&mut self, window: &slint::Window) {
            self.get(window);
        }

        fn store(&mut self, window: &slint::Window, buffer: Buffer, text: &str) -> bool {
            let Some(clipboard) = self.get(window) else {
                return false;
            };
            match buffer {
                Buffer::Clipboard => clipboard.store(text),
                Buffer::Primary => clipboard.store_primary(text),
            }
            true
        }

        fn load(&mut self, window: &slint::Window, buffer: Buffer) -> Option<String> {
            let clipboard = self.get(window)?;
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
