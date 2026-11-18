//! The frameless window's behaviour: the shell reports intents (close, minimise, full screen,
//! begin a resize) and this module is the only place that acts on them.
//!
//! Moving needs no code here: Slint's `WindowMoveArea` starts the compositor's move
//! (winit `drag_window`) itself. A resize is handed to the compositor the same way
//! (`drag_resize_window`), because on Wayland a client cannot position or size itself during a
//! grab. Minimise goes straight to winit rather than through Slint's `minimized` property: on
//! Wayland winit cannot report the state back, so Slint's flag would stay set and the button would
//! work once.

use slint::ComponentHandle;
use slint::winit_030::WinitWindowAccessor;
use slint::winit_030::winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
use slint::winit_030::winit::window::ResizeDirection;

use crate::{AppWindow, WindowEdge};

/// The windowing system the window runs on, from its raw handle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Wayland,
    X11,
    MacOs,
    Windows,
    Other,
}

impl Kind {
    fn of(handle: RawWindowHandle) -> Kind {
        match handle {
            RawWindowHandle::Wayland(_) => Kind::Wayland,
            RawWindowHandle::Xlib(_) | RawWindowHandle::Xcb(_) => Kind::X11,
            RawWindowHandle::AppKit(_) => Kind::MacOs,
            RawWindowHandle::Win32(_) => Kind::Windows,
            _ => Kind::Other,
        }
    }

    /// A transparent window background shows through, so the window can have rounded corners.
    /// Belief, not measured on a display: Mesa and NVIDIA expose a pre-multiplied alpha mode on
    /// Wayland, macOS always composites; X11 cannot tell whether a compositor runs and Windows is
    /// unmeasured, so those stay square.
    pub fn transparent_corners(self) -> bool {
        matches!(self, Kind::Wayland | Kind::MacOs)
    }
}

/// The window's mode. Not stored: it is read from the window each time (one owner).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Frame {
    Normal,
    Fullscreen,
    Maximised,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Intent {
    ToggleFullscreen,
    Minimise,
    Close,
    BeginResize(WindowEdge),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Effect {
    SetFullscreen(bool),
    Minimise,
    Quit,
    DragResize(WindowEdge),
}

pub fn transition(frame: Frame, intent: Intent) -> (Frame, Vec<Effect>) {
    match (frame, intent) {
        (Frame::Fullscreen, Intent::ToggleFullscreen) => {
            (Frame::Normal, vec![Effect::SetFullscreen(false)])
        }
        (_, Intent::ToggleFullscreen) => (Frame::Fullscreen, vec![Effect::SetFullscreen(true)]),
        (frame, Intent::Minimise) => (frame, vec![Effect::Minimise]),
        (frame, Intent::Close) => (frame, vec![Effect::Quit]),
        // Only a normal window has edges to drag.
        (Frame::Normal, Intent::BeginResize(edge)) => {
            (Frame::Normal, vec![Effect::DragResize(edge)])
        }
        (frame, Intent::BeginResize(_)) => (frame, vec![]),
    }
}

pub fn direction(edge: WindowEdge) -> ResizeDirection {
    match edge {
        WindowEdge::N => ResizeDirection::North,
        WindowEdge::S => ResizeDirection::South,
        WindowEdge::E => ResizeDirection::East,
        WindowEdge::W => ResizeDirection::West,
        WindowEdge::Ne => ResizeDirection::NorthEast,
        WindowEdge::Nw => ResizeDirection::NorthWest,
        WindowEdge::Se => ResizeDirection::SouthEast,
        WindowEdge::Sw => ResizeDirection::SouthWest,
    }
}

fn frame_of(window: &slint::Window) -> Frame {
    if window.is_fullscreen() {
        Frame::Fullscreen
    } else if window.is_maximized() {
        Frame::Maximised
    } else {
        Frame::Normal
    }
}

/// The only impure part. Errors (`NotSupported` on macOS resize) are logged, not retried.
fn execute(window: &slint::Window, effect: Effect) {
    match effect {
        Effect::SetFullscreen(on) => window.set_fullscreen(on),
        Effect::Quit => {
            if let Err(e) = slint::quit_event_loop() {
                eprintln!("wubble: quit: {e}");
            }
        }
        Effect::Minimise => {
            if window
                .with_winit_window(|w| w.set_minimized(true))
                .is_none()
            {
                eprintln!("wubble: minimise: no window yet");
            }
        }
        Effect::DragResize(edge) => {
            if let Some(Err(e)) =
                window.with_winit_window(|w| w.drag_resize_window(direction(edge)))
            {
                eprintln!("wubble: resize: {e}");
            }
        }
    }
}

fn run(window: &slint::Window, intent: Intent) {
    let (_, effects) = transition(frame_of(window), intent);
    for effect in effects {
        execute(window, effect);
    }
}

/// Connect the shell's window intents. Call once, after `AppWindow::new`. The platform flags are
/// applied once the event loop runs, when the native window exists.
pub fn wire(app: &AppWindow) {
    let weak = app.as_weak();
    let on = move |intent: Intent| {
        let weak = weak.clone();
        move || {
            if let Some(app) = weak.upgrade() {
                run(app.window(), intent);
            }
        }
    };
    app.on_window_close_requested(on(Intent::Close));
    app.on_window_minimise_requested(on(Intent::Minimise));
    app.on_window_toggle_fullscreen_requested(on(Intent::ToggleFullscreen));
    let weak = app.as_weak();
    app.on_window_begin_resize(move |edge| {
        if let Some(app) = weak.upgrade() {
            run(app.window(), Intent::BeginResize(edge));
        }
    });

    let weak = app.as_weak();
    let _ = slint::invoke_from_event_loop(move || {
        let Some(app) = weak.upgrade() else { return };
        let kind = app.window().with_winit_window(|w| {
            w.window_handle()
                .map(|h| Kind::of(h.as_raw()))
                .unwrap_or(Kind::Other)
        });
        if kind.is_none() {
            eprintln!("wubble: rounded corners: no native window yet, staying square");
        }
        let square = std::env::var_os("WUBBLE_SQUARE_CORNERS").is_some();
        app.set_rounded_corners(!square && kind.is_some_and(Kind::transparent_corners));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    const EDGES: [WindowEdge; 8] = [
        WindowEdge::N,
        WindowEdge::S,
        WindowEdge::E,
        WindowEdge::W,
        WindowEdge::Ne,
        WindowEdge::Nw,
        WindowEdge::Se,
        WindowEdge::Sw,
    ];
    const FRAMES: [Frame; 3] = [Frame::Normal, Frame::Fullscreen, Frame::Maximised];

    #[test]
    fn direction_matches_each_edge() {
        let expected = [
            ResizeDirection::North,
            ResizeDirection::South,
            ResizeDirection::East,
            ResizeDirection::West,
            ResizeDirection::NorthEast,
            ResizeDirection::NorthWest,
            ResizeDirection::SouthEast,
            ResizeDirection::SouthWest,
        ];
        for (edge, want) in EDGES.into_iter().zip(expected) {
            assert_eq!(direction(edge), want);
        }
    }

    #[test]
    fn toggle_fullscreen_flips() {
        assert_eq!(
            transition(Frame::Normal, Intent::ToggleFullscreen),
            (Frame::Fullscreen, vec![Effect::SetFullscreen(true)])
        );
        assert_eq!(
            transition(Frame::Maximised, Intent::ToggleFullscreen),
            (Frame::Fullscreen, vec![Effect::SetFullscreen(true)])
        );
        assert_eq!(
            transition(Frame::Fullscreen, Intent::ToggleFullscreen),
            (Frame::Normal, vec![Effect::SetFullscreen(false)])
        );
    }

    #[test]
    fn close_and_minimise_keep_the_frame() {
        for frame in FRAMES {
            assert_eq!(
                transition(frame, Intent::Close),
                (frame, vec![Effect::Quit])
            );
            assert_eq!(
                transition(frame, Intent::Minimise),
                (frame, vec![Effect::Minimise])
            );
        }
    }

    #[test]
    fn resize_only_in_a_normal_window() {
        for edge in EDGES {
            assert_eq!(
                transition(Frame::Normal, Intent::BeginResize(edge)),
                (Frame::Normal, vec![Effect::DragResize(edge)])
            );
            for frame in [Frame::Fullscreen, Frame::Maximised] {
                assert_eq!(
                    transition(frame, Intent::BeginResize(edge)),
                    (frame, vec![])
                );
            }
        }
    }

    #[test]
    fn rounded_corners_where_the_compositor_blends() {
        let table = [
            (Kind::Wayland, true),
            (Kind::X11, false),
            (Kind::MacOs, true),
            (Kind::Windows, false),
            (Kind::Other, false),
        ];
        for (kind, corners) in table {
            assert_eq!(kind.transparent_corners(), corners, "{kind:?}");
        }
    }
}
