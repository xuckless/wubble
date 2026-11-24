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
    pub(super) over_page: bool,
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
                    let scale = w.scale_factor();
                    match image.and_then(|i| custom(event_loop, &fit(&i, scale))) {
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

/// `image` at the display's `scale`: CEF draws it for its own (`scale_percent`), a 1x CSS image
/// on a 2x display is half the size Chrome shows. Nearest neighbour, as a cursor's hard edges
/// want; the hotspot follows, inside the image.
fn fit(image: &CursorImage, scale: f64) -> CursorImage {
    let ratio = scale * 100.0 / f64::from(image.scale_percent.max(1));
    if (ratio - 1.0).abs() < 0.01 {
        return image.clone();
    }
    let size = |n: u16| ((f64::from(n) * ratio).round() as u16).max(1);
    let (width, height) = (size(image.width), size(image.height));
    let source = |n: u16, of: u16, to: u16| {
        (usize::from(n) * usize::from(of) / usize::from(to)).min(usize::from(of) - 1)
    };
    let rgba: Vec<u8> = (0..height)
        .flat_map(|y| (0..width).map(move |x| (x, y)))
        .flat_map(|(x, y)| {
            let at = (source(y, image.height, height) * usize::from(image.width)
                + source(x, image.width, width))
                * 4;
            image.rgba[at..at + 4].to_vec()
        })
        .collect();
    let hotspot = |n: u16, max: u16| ((f64::from(n) * ratio) as u16).min(max - 1);
    CursorImage {
        width,
        height,
        hotspot_x: hotspot(image.hotspot_x, width),
        hotspot_y: hotspot(image.hotspot_y, height),
        rgba: rgba.into(),
        scale_percent: (scale * 100.0).round() as u16,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_image_cursor_is_scaled_to_the_display() {
        // 2x2: red, green / blue, white, drawn for 1x, hotspot at 1,1
        let px = [
            [255, 0, 0, 255],
            [0, 255, 0, 255],
            [0, 0, 255, 255],
            [255; 4],
        ];
        let image = CursorImage {
            width: 2,
            height: 2,
            hotspot_x: 1,
            hotspot_y: 1,
            rgba: px.concat().into(),
            scale_percent: 100,
        };
        // (display scale, size, hotspot, the pixel rows by source index)
        for (scale, size, hotspot, rows) in [
            (1.0, 2, 1, vec![vec![0, 1], vec![2, 3]]),
            (
                2.0,
                4,
                2,
                vec![
                    vec![0, 0, 1, 1],
                    vec![0, 0, 1, 1],
                    vec![2, 2, 3, 3],
                    vec![2, 2, 3, 3],
                ],
            ),
            (1.5, 3, 1, vec![vec![0, 0, 1], vec![0, 0, 1], vec![2, 2, 3]]),
        ] {
            let got = fit(&image, scale);
            assert_eq!((got.width, got.height), (size, size), "{scale}");
            assert_eq!(
                (got.hotspot_x, got.hotspot_y),
                (hotspot, hotspot),
                "{scale}"
            );
            let want: Vec<u8> = rows.concat().iter().flat_map(|&i| px[i]).collect();
            assert_eq!(&got.rgba[..], &want[..], "{scale}");
        }
        // drawn for 2x on a 1x display: halved
        let big = fit(
            &CursorImage {
                scale_percent: 200,
                ..image.clone()
            },
            1.0,
        );
        assert_eq!((big.width, big.height, big.hotspot_x), (1, 1, 0));
    }

    /// The engine's `Cursor`, app.slint's `cursors` (indexed by it) and [`WINDOW_ICONS`] are
    /// one table: every cursor at its index, with the shape Slint shows, and the hidden ones
    /// exactly those the shell draws (autoscroll), the window shows (`WINDOW_ICONS`, the
    /// image) or the page hides.
    #[test]
    fn the_shells_cursor_table_follows_the_engines() {
        use Cursor as C;
        let table = [
            (C::Default, "default"),
            (C::None, "none"),
            (C::Pointer, "pointer"),
            (C::Text, "text"),
            (C::Wait, "wait"),
            (C::Progress, "progress"),
            (C::Crosshair, "crosshair"),
            (C::Move, "move"),
            (C::NotAllowed, "not-allowed"),
            (C::Help, "help"),
            (C::Grab, "grab"),
            (C::Grabbing, "grabbing"),
            (C::ResizeNs, "ns-resize"),
            (C::ResizeEw, "ew-resize"),
            (C::ResizeNeSw, "nesw-resize"),
            (C::ResizeNwSe, "nwse-resize"),
            (C::PanStill, "none"),
            (C::PanStillVertical, "none"),
            (C::PanStillHorizontal, "none"),
            (C::PanN, "none"),
            (C::PanNE, "none"),
            (C::PanE, "none"),
            (C::PanSE, "none"),
            (C::PanS, "none"),
            (C::PanSW, "none"),
            (C::PanW, "none"),
            (C::PanNW, "none"),
            (C::Copy, "copy"),
            (C::Alias, "alias"),
            (C::ColResize, "col-resize"),
            (C::RowResize, "row-resize"),
            (C::NoDrop, "no-drop"),
            (C::ResizeN, "n-resize"),
            (C::ResizeE, "e-resize"),
            (C::ResizeS, "s-resize"),
            (C::ResizeW, "w-resize"),
            (C::ResizeNe, "ne-resize"),
            (C::ResizeNw, "nw-resize"),
            (C::ResizeSe, "se-resize"),
            (C::ResizeSw, "sw-resize"),
            (C::ContextMenu, "none"),
            (C::Cell, "none"),
            (C::VerticalText, "none"),
            (C::ZoomIn, "none"),
            (C::ZoomOut, "none"),
            (C::Image, "none"),
        ];
        let app = include_str!("../../../wubble-ui/ui/app.slint");
        let list = app
            .split_once("property <[MouseCursor]> cursors: [")
            .and_then(|(_, rest)| rest.split_once("];"))
            .expect("app.slint's cursors")
            .0;
        let shown: Vec<&str> = list
            .lines()
            .filter_map(|l| l.split("//").next())
            .flat_map(|l| l.split(','))
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .map(|t| t.trim_start_matches("MouseCursor."))
            .collect();
        assert_eq!(shown.len(), table.len(), "app.slint's cursors: {shown:?}");
        for (i, (cursor, name)) in table.into_iter().enumerate() {
            assert_eq!(cursor as usize, i, "{cursor:?}");
            assert_eq!(shown[i], name, "{cursor:?}");
            let window = WINDOW_ICONS.iter().any(|(c, _)| *c == cursor) || cursor == C::Image;
            let hidden = cursor == C::None || cursor.pan() != Pan::None || window;
            assert_eq!(name == "none", hidden, "{cursor:?}");
        }
    }

    #[test]
    fn only_the_cursors_the_shell_hides_for_the_window_have_icons() {
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
