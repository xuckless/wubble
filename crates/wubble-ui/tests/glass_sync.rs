//! The scene's glass and Slint's content agree in every frame: a sidebar mode change, run
//! frame by frame through `Wired::frame` (what the rendering notifier calls before Slint draws),
//! delivers the glass inside the sidebar (the pill, the favourites) where Slint shows it that
//! same frame, over the window's own glass, and reaches rest on its own. Testing backend, explicit frame times, no clocks. Its own process: the platform can
//! be installed once per process.

use std::time::{Duration, Instant};

use i_slint_backend_testing::init_integration_test_with_mock_time;
use slint::{ComponentHandle, LogicalSize};
use wubble_core::seam::{INSET, SEAM, Seam};
use wubble_ui::driver::{self, Wired};
use wubble_ui::{AppWindow, GlassItem, GlassKeys, GlassLayer};

/// 120 Hz.
const FRAME: Duration = Duration::from_micros(8_333);
/// kit/derived.slint `Shell.sidebar-open`.
const SIDEBAR_OPEN: f32 = 0.5;
/// Frames a transition may take before the test calls it stalled. A count, not a clock.
const MAX_FRAMES: usize = 2_000;

/// The glass the scene holds: the last list a frame delivered.
fn sidebar(scene: &[GlassItem], key: i32) -> Option<GlassItem> {
    scene.iter().find(|i| i.key == key).cloned()
}

/// Frames from `now` until nothing moves, checking each one; returns the time after them.
fn run_to_rest(
    app: &AppWindow,
    wired: &Wired,
    scene: &mut Vec<GlassItem>,
    mut now: Instant,
) -> Instant {
    let key = app.global::<GlassKeys>().get_sidebar();
    for frame in 0..MAX_FRAMES {
        let (again, out) = wired.frame(app, now);
        if let Some(glass) = out.glass {
            *scene = glass;
        }
        // What Slint draws this frame: the window's glass under the sidebar and the page, in
        // every mode (the design's one window surface; the sidebar has no panel of its own).
        let item = sidebar(scene, key).unwrap_or_else(|| panic!("frame {frame}: no window glass"));
        assert!(
            (item.x, item.y, item.width, item.height) == (0.0, 0.0, 1280.0, 800.0),
            "frame {frame}: the scene's glass {item:?} is not the window"
        );
        check_inside(app, scene, frame);
        now += FRAME;
        if !again {
            return now;
        }
    }
    panic!("the transition never reached rest in {MAX_FRAMES} frames");
}

#[test]
fn sidebar_glass_follows_every_frame_of_a_mode_change() {
    init_integration_test_with_mock_time();
    let app = AppWindow::new().unwrap();
    app.show().unwrap();
    app.window().set_size(LogicalSize::new(1280.0, 800.0));
    app.global::<GlassLayer>().set_scene_active(true);
    let wired = driver::wire(&app, Seam::new());
    let mut scene = vec![];
    let mut now = run_to_rest(&app, &wired, &mut scene, Instant::now());

    // Each toggle wakes the loop once (the callback), then frames run until rest with no
    // further event.
    for (toggle, want) in [
        (
            AppWindow::invoke_seam_double_clicked as fn(&AppWindow),
            SEAM::rail,
        ),
        (AppWindow::invoke_seam_double_clicked, Seam::new().saved),
        (AppWindow::invoke_sidebar_toggle_requested, 0.0),
        (
            AppWindow::invoke_sidebar_toggle_requested,
            Seam::new().saved,
        ),
        // Extended -> Rail -> Hidden -> Extended (owner: the pill's glass in rail mode)
        (AppWindow::invoke_seam_double_clicked, SEAM::rail),
        (AppWindow::invoke_sidebar_toggle_requested, 0.0),
        (
            AppWindow::invoke_sidebar_toggle_requested,
            Seam::new().saved,
        ),
    ] {
        toggle(&app);
        now = run_to_rest(&app, &wired, &mut scene, now);
        assert_eq!(f64::from(app.get_sidebar_width()), want, "at rest");
    }

    // A seam drag (owner: "the glass doesn't grow past a certain size"): to the minimum, wider,
    // the maximum, past it (the rubber band), and back; every held frame and at rest.
    for target in [SEAM::min, 300.0, SEAM::max, SEAM::max + 120.0, 260.0] {
        now = drag_seam_to(&app, &wired, &mut scene, now, target as f32);
        let rest = app.get_sidebar_width();
        let want = (target as f32).clamp(SEAM::min as f32, SEAM::max as f32);
        assert!(
            (rest - want).abs() < 0.5,
            "dragged to {target}: at rest {rest}, want {want}"
        );
    }
}

/// Window x of the seam for a sidebar `w` wide (app.slint: the Seam's frame starts at the inset,
/// the string half an inset past the sidebar).
fn seam_x(w: f32) -> f32 {
    INSET as f32 + w + INSET as f32 / 2.0
}

/// One frame of a drag: the scene's window glass stays the window, and the address pill's glass
/// is the one Slint lays out.
fn check_drag_frame(app: &AppWindow, scene: &[GlassItem], frame: usize) {
    let keys = app.global::<GlassKeys>();
    let side = sidebar(scene, keys.get_sidebar())
        .unwrap_or_else(|| panic!("frame {frame}: no window glass"));
    assert!(
        (side.x, side.y, side.width) == (0.0, 0.0, 1280.0),
        "frame {frame}: the window's glass {side:?} moved with the seam"
    );
    check_inside(app, scene, frame);
}

/// Every glass under the page the scene holds but the window's lies inside the sidebar, which
/// clips what it shows (none may reach under the page; an overlay's is drawn above it), and the
/// address pill's is the pill Slint shows: none on the rail, where the pill is a search button
/// without glass (.wb-rail-addr).
fn check_inside(app: &AppWindow, scene: &[GlassItem], frame: usize) {
    let keys = app.global::<GlassKeys>();
    let edges = app.get_side_edges();
    let near = |a: f32, b: f32| (a - b).abs() < 0.01;
    for item in scene
        .iter()
        .filter(|i| i.key != keys.get_sidebar() && i.layer == 0)
    {
        assert!(
            item.x >= edges.start - 0.01 && item.x + item.width <= edges.end + 0.01,
            "frame {frame}: glass {item:?} reaches outside the sidebar {edges:?}"
        );
    }
    let pill = sidebar(scene, keys.get_address());
    if app.get_sidebar_rail() || app.get_sidebar_width() <= SIDEBAR_OPEN {
        assert!(
            pill.is_none(),
            "frame {frame}: pill glass on the rail: {pill:?}"
        );
    } else {
        let pill = pill.unwrap_or_else(|| panic!("frame {frame}: no address glass"));
        // the pill as the sidebar shows it: cut where it rides past the sidebar's edges
        let (x0, x1) = (edges.pill.max(edges.start), edges.pill_end.min(edges.end));
        assert!(
            near(pill.x, x0) && near(pill.width, x1 - x0),
            "frame {frame}: the pill's glass {pill:?} is not the pill {edges:?}"
        );
    }
}

/// Press the seam, move the pointer to where the sidebar is `target` wide over 30 frames, hold
/// it there until the width settles, release and run to rest; checks every frame.
fn drag_seam_to(
    app: &AppWindow,
    wired: &Wired,
    scene: &mut Vec<GlassItem>,
    mut now: Instant,
    target: f32,
) -> Instant {
    let from = seam_x(app.get_sidebar_width());
    // seam.rs: a held seam's width is the pointer's x less the inset
    let to = INSET as f32 + target;
    app.invoke_seam_grab(from, 300.0);
    for frame in 0..400 {
        let k = ((frame + 1) as f32 / 30.0).min(1.0);
        app.invoke_seam_drag(from + (to - from) * k, 300.0);
        let (_, out) = wired.frame(app, now);
        if let Some(glass) = out.glass {
            *scene = glass;
        }
        check_drag_frame(app, scene, frame);
        now += FRAME;
    }
    app.invoke_seam_release();
    let now = run_to_rest(app, wired, scene, now);
    check_drag_frame(app, scene, usize::MAX);
    now
}
