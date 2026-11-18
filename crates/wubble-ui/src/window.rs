//! The frameless window's behaviour: the shell reports intents (close, minimise, full screen,
//! begin a resize) and this module is the only place that acts on them.
//!
//! A move and a resize are handed to the compositor (winit `drag_window`,
//! `drag_resize_window`), because on Wayland a client cannot position or size itself during a
//! grab. Minimise goes straight to winit rather than through Slint's `minimized` property: on
//! Wayland winit cannot report the state back, so Slint's flag would stay set and the button would
//! work once.
//!
//! macOS keeps a native frame with a transparent, hidden title bar ([`NATIVE_FRAME`],
//! [`attributes`]): AppKit then gives the window its edge resize, shadow and rounded corners
//! (winit 0.30 has no `drag_resize_window` there), while the shell still draws its own window
//! buttons over the full-size content.

use slint::ComponentHandle;
use slint::winit_030::WinitWindowAccessor;
use slint::winit_030::winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
use slint::winit_030::winit::window::{ResizeDirection, WindowAttributes};

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

/// The OS frames the window (edge resize, shadow, corners) under the shell's drawn buttons.
pub const NATIVE_FRAME: bool = cfg!(target_os = "macos");

/// The native window's attributes; pass to `BackendSelector::with_winit_window_attributes_hook`.
/// macOS: a full-size content view under a transparent title bar with no title and no traffic
/// lights (the shell draws them), so only AppKit's frame behaviour remains. Elsewhere unchanged:
/// the window is frameless (`no-frame`).
pub fn attributes(attributes: WindowAttributes) -> WindowAttributes {
    #[cfg(target_os = "macos")]
    let attributes = {
        use slint::winit_030::winit::platform::macos::WindowAttributesExtMacOS;
        attributes
            .with_titlebar_transparent(true)
            .with_title_hidden(true)
            .with_fullsize_content_view(true)
            .with_titlebar_buttons_hidden(true)
    };
    attributes
}

/// Turns the OS's own window restoration off for this process, before the event loop starts.
/// wubble restores its session itself (`persist`). macOS: AppKit's restoration ("Resume") is
/// what shows "wubble quit unexpectedly while reopening windows" as a modal alert at launch
/// after a crash or a killed run, and that alert holds the UI thread before the first frame (a
/// sampled `--smoke frames` run sat in `-[NSPersistentUIRestorer
/// promptToIgnorePersistentStateWithCrashHistory:]` until its timeout). Registered, not saved:
/// `ApplePersistenceIgnoreState` in `NSUserDefaults`' registration domain. Elsewhere nothing.
pub fn no_os_window_restoration() {
    #[cfg(target_os = "macos")]
    {
        use objc2_foundation::{NSDictionary, NSNumber, NSString, NSUserDefaults};
        let key = NSString::from_str("ApplePersistenceIgnoreState");
        let yes = NSNumber::new_bool(true);
        let defaults = NSDictionary::from_slices(&[&*key], &[yes.as_ref()]);
        // SAFETY: the dictionary maps an `NSString` to an `NSNumber`, which is what
        // `registerDefaults:` takes.
        unsafe { NSUserDefaults::standardUserDefaults().registerDefaults(&defaults) };
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
    ToggleMaximise,
    BeginMove,
    Minimise,
    Close,
    BeginResize(WindowEdge),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Effect {
    SetFullscreen(bool),
    SetMaximised(bool),
    DragMove,
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
        (Frame::Normal, Intent::ToggleMaximise) => {
            (Frame::Maximised, vec![Effect::SetMaximised(true)])
        }
        (Frame::Maximised, Intent::ToggleMaximise) => {
            (Frame::Normal, vec![Effect::SetMaximised(false)])
        }
        // A full-screen window has no chrome to move or maximise from.
        (Frame::Fullscreen, Intent::ToggleMaximise | Intent::BeginMove) => {
            (Frame::Fullscreen, vec![])
        }
        // Dragging a maximised window is the compositor's to restore (GNOME and KWin do).
        (frame, Intent::BeginMove) => (frame, vec![Effect::DragMove]),
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
        Effect::SetMaximised(on) => window.set_maximized(on),
        Effect::DragMove => {
            if let Some(Err(e)) = window.with_winit_window(|w| w.drag_window()) {
                eprintln!("wubble: move: {e}");
            }
        }
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
/// applied later, by [`native_window_ready`].
pub fn wire(app: &AppWindow) {
    app.set_native_frame(NATIVE_FRAME);
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
    app.on_window_toggle_maximise_requested(on(Intent::ToggleMaximise));
    app.on_window_begin_move(on(Intent::BeginMove));
    let weak = app.as_weak();
    app.on_window_begin_resize(move |edge| {
        if let Some(app) = weak.upgrade() {
            run(app.window(), Intent::BeginResize(edge));
        }
    });
}

/// Round the corners if the platform shows a transparent window background. Call once the native
/// window exists: [`crate::driver::install`] does, on the renderer's `RenderingSetup`. (Queued
/// from `wire` with `invoke_from_event_loop` it ran before winit had created the window, so the
/// window always stayed square.)
pub fn native_window_ready(app: &AppWindow) {
    let kind = app.window().with_winit_window(|w| {
        w.window_handle()
            .map(|h| Kind::of(h.as_raw()))
            .unwrap_or(Kind::Other)
    });
    let square = std::env::var_os("WUBBLE_SQUARE_CORNERS").is_some();
    let rounded = !square && kind.is_some_and(Kind::transparent_corners);
    eprintln!("wubble: window {kind:?}, rounded corners {rounded}");
    app.set_rounded_corners(rounded);
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
    fn maximise_toggles_and_move_needs_chrome() {
        assert_eq!(
            transition(Frame::Normal, Intent::ToggleMaximise),
            (Frame::Maximised, vec![Effect::SetMaximised(true)])
        );
        assert_eq!(
            transition(Frame::Maximised, Intent::ToggleMaximise),
            (Frame::Normal, vec![Effect::SetMaximised(false)])
        );
        for frame in [Frame::Normal, Frame::Maximised] {
            assert_eq!(
                transition(frame, Intent::BeginMove),
                (frame, vec![Effect::DragMove])
            );
        }
        for intent in [Intent::ToggleMaximise, Intent::BeginMove] {
            assert_eq!(
                transition(Frame::Fullscreen, intent),
                (Frame::Fullscreen, vec![])
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
