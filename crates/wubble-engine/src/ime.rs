//! IME composition: the host's input method (text-input-v3 on Wayland, TSF,
//! NSTextInputClient) drives `View::ime_set_composition` / `ime_commit` /
//! `ime_finish` / `ime_cancel`, and places its candidate window from
//! [`PageEvent::ImeBounds`].
//!
//! CEF reports character bounds only while a composition is active
//! (`OnImeCompositionRangeChanged`), so there is no caret rectangle before the
//! first composed character; welding keeps only the union of the bounds.

use crate::page::{PageEvent, Rect};

/// The event for welding's latest composition range.
pub fn bounds(composition: welding::ImeComposition) -> PageEvent {
    PageEvent::ImeBounds {
        rect: Rect::from(composition.bounds),
        selection: (composition.selection_start, composition.selection_end),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounds_carry_the_rect_and_selection() {
        let c = welding::ImeComposition {
            bounds: welding::PopupRect {
                x: 1,
                y: 2,
                width: 30,
                height: 16,
            },
            selection_start: 2,
            selection_end: 3,
        };
        assert_eq!(
            bounds(c),
            PageEvent::ImeBounds {
                rect: Rect {
                    x: 1,
                    y: 2,
                    width: 30,
                    height: 16
                },
                selection: (2, 3)
            }
        );
    }
}
