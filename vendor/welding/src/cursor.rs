// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Latest cursor shape reported by CEF.
//!
//! Under windowless rendering the host owns the pointer, so CEF can only ask
//! for a shape and hope someone applies it. Ignore `OnCursorChange` and the
//! page looks inert: no I-beam over text, no hand over links.

use std::sync::Mutex;

use crate::surface::CursorShape;

#[derive(Debug, Default)]
pub(crate) struct LatestCursor(Mutex<Option<CursorShape>>);

impl LatestCursor {
    pub(crate) fn set(&self, shape: CursorShape) {
        *self.0.lock().unwrap() = Some(shape);
    }

    /// Take the pending shape, if it changed since the last poll.
    pub(crate) fn take(&self) -> Option<CursorShape> {
        self.0.lock().unwrap().take()
    }
}

/// Map CEF's cursor type onto the shared vocabulary.
///
/// The two names that matter are crossed, which is exactly the sort of thing
/// that gets miswired once and then looks "nearly right" forever: CEF's
/// `POINTER` is the ordinary arrow, while `CursorShape::Pointer` is the CSS
/// `pointer`, i.e. the link hand, which CEF calls `HAND`.
///
/// wubble change (60): every type Chromium sends has its own shape (the
/// table's rows follow `ui/base/cursor/mojom/cursor_type.mojom`), and
/// `CUSTOM` carries the page's image from `info`.
#[cfg(feature = "cef-runtime")]
pub(crate) fn from_cef(type_: cef::CursorType, info: Option<&cef::CursorInfo>) -> CursorShape {
    use crate::surface::Panning as P;
    use CursorShape as S;
    use cef::CursorType as C;

    // Rows are the whole mapping: add a CEF type as a row, not as a branch.
    // A local array rather than a static: CursorType's variants are
    // associated consts of a newtype over a #[non_exhaustive] C enum.
    let table = [
        (C::POINTER, S::Default),
        (C::HAND, S::Pointer),
        (C::IBEAM, S::Text),
        (C::VERTICALTEXT, S::VerticalText),
        (C::CROSS, S::Crosshair),
        (C::CELL, S::Cell),
        (C::WAIT, S::Wait),
        (C::PROGRESS, S::Progress),
        (C::HELP, S::Help),
        (C::CONTEXTMENU, S::ContextMenu),
        (C::MOVE, S::Move),
        (C::DND_MOVE, S::Move),
        (C::NOTALLOWED, S::NotAllowed),
        (C::DND_NONE, S::NotAllowed),
        (C::NODROP, S::NoDrop),
        (C::COPY, S::Copy),
        (C::DND_COPY, S::Copy),
        (C::ALIAS, S::Alias),
        (C::DND_LINK, S::Alias),
        (C::ZOOMIN, S::ZoomIn),
        (C::ZOOMOUT, S::ZoomOut),
        (C::GRAB, S::Grab),
        (C::GRABBING, S::Grabbing),
        (C::NORTHSOUTHRESIZE, S::ResizeNs),
        (C::EASTWESTRESIZE, S::ResizeEw),
        (C::NORTHEASTSOUTHWESTRESIZE, S::ResizeNeSw),
        (C::NORTHWESTSOUTHEASTRESIZE, S::ResizeNwSe),
        (C::COLUMNRESIZE, S::ColResize),
        (C::ROWRESIZE, S::RowResize),
        (C::NORTHRESIZE, S::ResizeN),
        (C::EASTRESIZE, S::ResizeE),
        (C::SOUTHRESIZE, S::ResizeS),
        (C::WESTRESIZE, S::ResizeW),
        (C::NORTHEASTRESIZE, S::ResizeNe),
        (C::NORTHWESTRESIZE, S::ResizeNw),
        (C::SOUTHEASTRESIZE, S::ResizeSe),
        (C::SOUTHWESTRESIZE, S::ResizeSw),
        // wubble change (32): autoscroll's cursors, by name.
        (C::MIDDLEPANNING, S::Panning(P::Middle)),
        (C::MIDDLE_PANNING_VERTICAL, S::Panning(P::MiddleVertical)),
        (C::MIDDLE_PANNING_HORIZONTAL, S::Panning(P::MiddleHorizontal)),
        (C::NORTHPANNING, S::Panning(P::North)),
        (C::NORTHEASTPANNING, S::Panning(P::NorthEast)),
        (C::EASTPANNING, S::Panning(P::East)),
        (C::SOUTHEASTPANNING, S::Panning(P::SouthEast)),
        (C::SOUTHPANNING, S::Panning(P::South)),
        (C::SOUTHWESTPANNING, S::Panning(P::SouthWest)),
        (C::WESTPANNING, S::Panning(P::West)),
        (C::NORTHWESTPANNING, S::Panning(P::NorthWest)),
    ];
    if let Some((_, shape)) = table.into_iter().find(|(c, _)| *c == type_) {
        shape
    } else if type_ == C::CUSTOM {
        // Chromium falls back to the arrow when the image is unusable.
        info.and_then(image_of).map_or(S::Default, S::Image)
    } else if type_ == C::NONE {
        // The shared vocabulary has no "hidden" shape. Named rather than
        // silently mapped to Default, because a host that wants to honour it
        // can, and one that does not at least sees the difference.
        S::Custom("none".into())
    } else {
        S::Custom("cef-other".into())
    }
}

/// wubble change (60): the page's `cursor: url()` image. CEF's buffer is
/// `size.width * size.height * 4` bytes of premultiplied BGRA, upper-left
/// origin (`cef_cursor_info_t`); `None` when it is empty or too big for
/// winit's `u16` sizes.
#[cfg(feature = "cef-runtime")]
fn image_of(info: &cef::CursorInfo) -> Option<crate::surface::CursorImage> {
    let size = |n: i32| u16::try_from(n).ok().filter(|&n| n > 0);
    let (width, height) = (size(info.size.width)?, size(info.size.height)?);
    if info.buffer.is_null() {
        return None;
    }
    let len = usize::from(width) * usize::from(height) * 4;
    // SAFETY: CEF documents `buffer` as `len` bytes, valid for this callback.
    let bgra = unsafe { std::slice::from_raw_parts(info.buffer as *const u8, len) };
    Some(crate::surface::CursorImage {
        width,
        height,
        // winit rejects a hotspot outside the image.
        hotspot_x: u16::try_from(info.hotspot.x).unwrap_or(0).min(width - 1),
        hotspot_y: u16::try_from(info.hotspot.y).unwrap_or(0).min(height - 1),
        rgba: straight_rgba(bgra).into(),
    })
}

/// Premultiplied BGRA to straight-alpha RGBA, which is what winit's custom
/// cursors take.
#[cfg(feature = "cef-runtime")]
fn straight_rgba(bgra: &[u8]) -> Vec<u8> {
    bgra.chunks_exact(4)
        .flat_map(|px| {
            let a = u32::from(px[3]);
            let straight = |c: u8| {
                if a == 0 {
                    0
                } else {
                    ((u32::from(c) * 255 + a / 2) / a).min(255) as u8
                }
            };
            [straight(px[2]), straight(px[1]), straight(px[0]), px[3]]
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn latest_cursor_yields_once_per_change() {
        let c = LatestCursor::default();
        assert_eq!(c.take(), None);
        c.set(CursorShape::Text);
        assert_eq!(c.take(), Some(CursorShape::Text));
        assert_eq!(c.take(), None, "a shape should only be reported once");
    }

    #[cfg(feature = "cef-runtime")]
    #[test]
    fn the_crossed_pointer_names_map_the_right_way_round() {
        // CEF POINTER is the arrow; CSS/scrying Pointer is the link hand.
        assert_eq!(from_cef(cef::CursorType::POINTER, None), CursorShape::Default);
        assert_eq!(from_cef(cef::CursorType::HAND, None), CursorShape::Pointer);
        assert_eq!(from_cef(cef::CursorType::IBEAM, None), CursorShape::Text);
    }

    #[cfg(feature = "cef-runtime")]
    #[test]
    fn chromiums_cursor_types_keep_their_own_shapes() {
        use CursorShape as S;
        use cef::CursorType as C;
        for (type_, shape) in [
            (C::CONTEXTMENU, S::ContextMenu),
            (C::CELL, S::Cell),
            (C::VERTICALTEXT, S::VerticalText),
            (C::COPY, S::Copy),
            (C::ALIAS, S::Alias),
            (C::NODROP, S::NoDrop),
            (C::NOTALLOWED, S::NotAllowed),
            (C::COLUMNRESIZE, S::ColResize),
            (C::ROWRESIZE, S::RowResize),
            (C::EASTRESIZE, S::ResizeE),
            (C::SOUTHWESTRESIZE, S::ResizeSw),
            (C::EASTWESTRESIZE, S::ResizeEw),
            (C::ZOOMIN, S::ZoomIn),
            (C::MIDDLEPANNING, S::Panning(crate::surface::Panning::Middle)),
            (C::NONE, S::Custom("none".into())),
            // A custom cursor with no usable image is the arrow.
            (C::CUSTOM, S::Default),
        ] {
            assert_eq!(from_cef(type_, None), shape, "{type_:?}");
        }
    }

    #[cfg(feature = "cef-runtime")]
    #[test]
    fn premultiplied_bgra_becomes_straight_rgba() {
        // opaque, half-covered white (premultiplied 128), clear
        let bgra = [10, 20, 30, 255, 128, 128, 128, 128, 7, 7, 7, 0];
        assert_eq!(
            straight_rgba(&bgra),
            [30, 20, 10, 255, 255, 255, 255, 128, 0, 0, 0, 0]
        );
    }
}
