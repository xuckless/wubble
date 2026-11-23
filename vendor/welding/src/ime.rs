// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! IME composition feedback from CEF.
//!
//! The host owns the input method under windowless rendering, so it needs to
//! know where the text being composed actually sits on screen; that is where
//! the candidate window goes. CEF reports it through
//! `OnImeCompositionRangeChanged`, in DIP.
//!
//! wubble change (61): a caret is zero wide, so a report of one (CEF names no
//! other callback for where the text cursor is) counts: only a zero-height
//! rectangle is padding now.

use std::sync::Mutex;

use crate::surface::{ImeComposition, PopupRect};

#[derive(Debug, Default)]
pub(crate) struct LatestComposition(Mutex<Option<ImeComposition>>);

impl LatestComposition {
    pub(crate) fn set(&self, composition: ImeComposition) {
        *self.0.lock().unwrap() = Some(composition);
    }

    pub(crate) fn take(&self) -> Option<ImeComposition> {
        self.0.lock().unwrap().take()
    }
}

/// The union of the character bounds CEF reports, which is the rectangle a
/// host wants to sit its candidate window under. CEF hands over one rect per
/// character; a host almost never wants them individually.
pub(crate) fn bounds_union(rects: &[PopupRect]) -> Option<PopupRect> {
    let mut it = rects.iter().filter(|r| r.height > 0);
    let first = *it.next()?;
    let (mut x0, mut y0) = (first.x, first.y);
    let (mut x1, mut y1) = (first.x + first.width as i32, first.y + first.height as i32);
    for r in it {
        x0 = x0.min(r.x);
        y0 = y0.min(r.y);
        x1 = x1.max(r.x + r.width as i32);
        y1 = y1.max(r.y + r.height as i32);
    }
    Some(PopupRect {
        x: x0,
        y: y0,
        width: (x1 - x0).max(0) as u32,
        height: (y1 - y0).max(0) as u32,
    })
}

/// wubble change (61): the underline Chromium's own input method puts under a
/// whole preedit (`InputMethodAuraLinux`: one thin span over the text, in the
/// text's colour: a transparent colour is how Blink says that). Blink draws
/// the same one when it is given none (`input_method_controller.cc`), so this
/// only spells it out.
#[cfg(feature = "cef-runtime")]
pub(crate) fn composition_underlines(text: &str) -> [cef::CompositionUnderline; 1] {
    [cef::CompositionUnderline {
        range: cef::Range {
            from: 0,
            to: text.encode_utf16().count() as u32,
        },
        ..Default::default()
    }]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(x: i32, y: i32, w: u32, h: u32) -> PopupRect {
        PopupRect {
            x,
            y,
            width: w,
            height: h,
        }
    }

    #[test]
    fn union_spans_every_character() {
        let got = bounds_union(&[r(10, 20, 8, 16), r(18, 20, 8, 16), r(26, 20, 8, 16)]);
        assert_eq!(got, Some(r(10, 20, 24, 16)));
    }

    #[test]
    fn empty_rects_are_ignored_not_counted() {
        // CEF pads with zero-size rects; letting one in would drag the union
        // to the origin and put the candidate window in the corner.
        assert_eq!(
            bounds_union(&[r(0, 0, 0, 0), r(40, 60, 10, 12)]),
            Some(r(40, 60, 10, 12))
        );
        assert_eq!(bounds_union(&[]), None);
        assert_eq!(bounds_union(&[r(0, 0, 0, 0)]), None);
    }

    #[test]
    fn a_caret_is_zero_wide_but_not_empty() {
        assert_eq!(bounds_union(&[r(40, 60, 0, 12)]), Some(r(40, 60, 0, 12)));
    }

    #[cfg(feature = "cef-runtime")]
    #[test]
    fn one_underline_covers_the_preedit_in_utf16_units() {
        let [u] = composition_underlines("a\u{65e5}\u{1f600}");
        assert_eq!((u.range.from, u.range.to), (0, 4));
    }
}
