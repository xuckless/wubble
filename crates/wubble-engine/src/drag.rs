//! Drags out of a page. CEF hands a page-started drag to the host
//! (`StartDragging`) and then waits for `DragSourceEndedAt` and
//! `DragSourceSystemDragEnded`. Slint 1.19 has no OS drag source, so welding
//! refuses every page drag until the host calls `View::set_drag_source(true)`;
//! this gate keeps the host from ending a drag CEF never started.

/// Whether a page drag is waiting for the host to end it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DragSource {
    #[default]
    Idle,
    Dragging,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DragSourceInput {
    /// welding reported a page drag.
    Started,
    /// The host's drag loop ended.
    Ended,
}

/// The next state and whether the input is acted on (report the drag, or
/// tell CEF it ended). A second start while dragging and an end while idle
/// are refused.
pub fn transition(state: DragSource, input: DragSourceInput) -> (DragSource, bool) {
    use {DragSource::*, DragSourceInput::*};
    match (state, input) {
        (Idle, Started) => (Dragging, true),
        (Dragging, Ended) => (Idle, true),
        _ => (state, false),
    }
}

#[cfg(test)]
mod tests {
    use super::{DragSource::*, DragSourceInput::*, *};

    #[test]
    fn ended_only_while_dragging() {
        assert_eq!(transition(Idle, Ended), (Idle, false));
        assert_eq!(transition(Idle, Started), (Dragging, true));
        assert_eq!(transition(Dragging, Started), (Dragging, false));
        assert_eq!(transition(Dragging, Ended), (Idle, true));
    }
}
