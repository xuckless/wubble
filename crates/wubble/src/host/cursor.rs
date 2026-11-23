//! The page's pointer shape on the window.
//!
//! Slint's `MouseCursor` has most of Chromium's shapes (`cursors` in app.slint, indexed by the
//! engine's `Cursor`) but not the context menu, cell, vertical text, zoom in and zoom out, nor a
//! page's own image (`cursor: url(...)`). For those the shell's table says "hidden" and the host
//! sets winit's cursor on the window itself, after Slint has applied its own: Slint calls
//! `set_cursor` only when its value changes, so it never undoes what is set here until the shape
//! changes to one it shows.
//!
//! The window cursor is the host's only while the pointer is over a page: Slint decides the
//! cursor everywhere else (the sidebar's own shapes), and a page change then must not touch it.

use std::cell::RefCell;

use slint::ComponentHandle;
use slint::winit_030::winit::event_loop::ActiveEventLoop;
use slint::winit_030::winit::window::{CursorIcon, CustomCursor};
use slint::winit_030::{WinitWindowAccessor, invoke_from_active_event_loop};
use wubble_core::autoscroll::Pan;
use wubble_engine::nav::{Cursor, CursorImage};
use wubble_ui::AppWindow;

use super::Host;

/// The page cursors the window shows, not Slint. Add a shape as a row here and as a hidden one
/// in app.slint's `cursors`.
const WINDOW_ICONS: [(Cursor, CursorIcon); 5] = [
    (Cursor::ContextMenu, CursorIcon::ContextMenu),
    (Cursor::Cell, CursorIcon::Cell),
    (Cursor::VerticalText, CursorIcon::VerticalText),
    (Cursor::ZoomIn, CursorIcon::ZoomIn),
    (Cursor::ZoomOut, CursorIcon::ZoomOut),
];

thread_local! {
    /// The last image cursor built, with the image it was built from, so a page that asks for
    /// the same image again (every hover over its element) does not make a new cursor. The
    /// event loop's thread only: a `CustomCursor` is made from its `ActiveEventLoop`.
    static BUILT: RefCell<Option<(CursorImage, CustomCursor)>> = const { RefCell::new(None) };
}

/// One window's page cursor.
#[derive(Default)]
pub(super) struct PageCursor {
    now: Cursor,
    /// The image behind `Cursor::Image`.
    image: Option<CursorImage>,
    /// The pointer is over a page, so the window's cursor is the page's to set.
    over_page: bool,
}

impl PageCursor {
    /// Hidden by Slint (a hidden cursor, or the shell draws autoscroll's own).
    fn hidden(&self) -> bool {
        self.now == Cursor::None || self.now.pan() != Pan::None
    }
}

impl Host {
    /// The shown page asks for `cursor`, or the host resets it: the shell's table shows it, and
    /// the window's own takes over for the shapes Slint has no cursor for.
    pub(super) fn set_page_cursor(&mut self, window: &AppWindow, cursor: Cursor) {
        window.set_web_cursor(cursor as i32);
        self.w.cursor.now = cursor;
        // Slint applies a TouchArea's cursor only while it handles a mouse event
        // (i-slint-core 1.19 `window.rs`, after `process_mouse_input`), and CEF reports the
        // shape after the move that caused it. Replay the pointer where it is, once the host
        // is free again, so the shape shows without waiting for the mouse to move. Queued
        // before the window's own cursor, which has to come after Slint's.
        if let Some(position) = self.input.pointer {
            let _ = self.w.window.upgrade_in_event_loop(move |window| {
                window
                    .window()
                    .dispatch_event(slint::platform::WindowEvent::PointerMoved { position })
            });
        }
        self.apply_page_cursor();
    }

    /// The image of the page's cursor changed; it shows now if the cursor is the image.
    pub(super) fn set_cursor_image(&mut self, image: CursorImage) {
        self.w.cursor.image = Some(image);
        if self.w.cursor.now == Cursor::Image {
            self.apply_page_cursor();
        }
    }

    /// The pointer moved over, or left, a page. Entering puts the page's cursor back: Slint's
    /// own, on the way in, is the hidden one for the shapes the window shows.
    pub(super) fn pointer_over_page(&mut self, over: bool) {
        if self.w.cursor.over_page != over {
            self.w.cursor.over_page = over;
            if over {
                self.apply_page_cursor();
            }
        }
    }

    /// Sets the window's cursor from the event loop, after the pointer replay queued before it.
    fn apply_page_cursor(&self) {
        let c = &self.w.cursor;
        if !c.over_page {
            return;
        }
        let (cursor, image, hidden) = (c.now, c.image.clone(), c.hidden());
        let window = self.w.window.clone();
        let _ = invoke_from_active_event_loop(move |event_loop| {
            let Some(window) = window.upgrade() else {
                return;
            };
            window.window().with_winit_window(|w| {
                let icon = WINDOW_ICONS.iter().find(|(c, _)| *c == cursor);
                if let Some((_, icon)) = icon {
                    w.set_cursor(*icon);
                } else if cursor == Cursor::Image {
                    // The arrow, as Chromium shows when the image is unusable.
                    match image.and_then(|i| custom(event_loop, &i)) {
                        Some(built) => w.set_cursor(built),
                        None => w.set_cursor(CursorIcon::Default),
                    }
                } else {
                    // Slint set this one; only the hiding can be left over from the other kind.
                    w.set_cursor_visible(!hidden);
                    return;
                }
                w.set_cursor_visible(true);
            });
        });
    }
}

/// The cursor of `image`, built on the event loop, or the last one built for the same image.
fn custom(event_loop: &ActiveEventLoop, image: &CursorImage) -> Option<CustomCursor> {
    BUILT.with_borrow_mut(|built| {
        if let Some((have, cursor)) = built.as_ref()
            && have == image
        {
            return Some(cursor.clone());
        }
        let source = CustomCursor::from_rgba(
            image.rgba.to_vec(),
            image.width,
            image.height,
            image.hotspot_x,
            image.hotspot_y,
        )
        .ok()?;
        let cursor = event_loop.create_custom_cursor(source);
        *built = Some((image.clone(), cursor.clone()));
        Some(cursor)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_cursors_the_shell_hides_for_the_window_have_icons() {
        // app.slint's `cursors` hides these; the window shows them.
        for (cursor, _) in WINDOW_ICONS {
            assert_eq!(cursor.pan(), Pan::None, "{cursor:?}");
        }
        let mut c = PageCursor::default();
        for (cursor, hidden) in [
            (Cursor::Default, false),
            (Cursor::None, true),
            (Cursor::PanStill, true),
            (Cursor::PanNW, true),
            (Cursor::Cell, false),
            (Cursor::Image, false),
        ] {
            c.now = cursor;
            assert_eq!(c.hidden(), hidden, "{cursor:?}");
        }
    }
}
