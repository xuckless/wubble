//! OSR popup widgets: `<select>` dropdowns, date and colour pickers,
//! autofill lists. CEF paints them as a second frame stream
//! (`View::take_popup_frame`) placed by [`PageEvent::PopupShown`] /
//! [`PageEvent::PopupMoved`]; the host draws it as a layer that may leave
//! the view's rectangle. (`window.open` is `WebEvent::PopupRequested`; JS
//! dialogs and context menus are requests in [`crate::page`].)

use crate::page::{PageEvent, Rect};

/// Whether a popup widget is showing, and where.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PopupWidget {
    #[default]
    Hidden,
    Shown(Rect),
}

/// `placement` is welding's `popup_rect()`: `Some` while CEF shows a sized
/// popup. Emits only on a change.
pub fn transition(state: PopupWidget, placement: Option<Rect>) -> (PopupWidget, Option<PageEvent>) {
    use PopupWidget::*;
    match (state, placement) {
        (Hidden, Some(rect)) => (Shown(rect), Some(PageEvent::PopupShown(rect))),
        (Shown(old), Some(rect)) if old != rect => (Shown(rect), Some(PageEvent::PopupMoved(rect))),
        (Shown(_), None) => (Hidden, Some(PageEvent::PopupHidden)),
        _ => (state, None),
    }
}

#[cfg(test)]
mod tests {
    use super::{PopupWidget::*, *};

    const A: Rect = Rect {
        x: 10,
        y: 20,
        width: 100,
        height: 200,
    };
    const B: Rect = Rect {
        x: 10,
        y: -40,
        width: 100,
        height: 260,
    };

    #[test]
    fn show_move_hide() {
        assert_eq!(transition(Hidden, None), (Hidden, None));
        assert_eq!(
            transition(Hidden, Some(A)),
            (Shown(A), Some(PageEvent::PopupShown(A)))
        );
        assert_eq!(transition(Shown(A), Some(A)), (Shown(A), None));
        assert_eq!(
            transition(Shown(A), Some(B)),
            (Shown(B), Some(PageEvent::PopupMoved(B)))
        );
        assert_eq!(
            transition(Shown(B), None),
            (Hidden, Some(PageEvent::PopupHidden))
        );
    }
}
