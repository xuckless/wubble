//! The scene's glass and Slint's content agree in every frame: a sidebar mode change, run
//! frame by frame through `Wired::frame` (what the rendering notifier calls before Slint draws),
//! delivers the sidebar's glass at the width Slint shows that same frame, and reaches rest on
//! its own. Testing backend, explicit frame times, no clocks. Its own process: the platform can
//! be installed once per process.

use std::time::{Duration, Instant};

use i_slint_backend_testing::init_integration_test_with_mock_time;
use slint::{ComponentHandle, LogicalSize};
use wubble_core::seam::{INSET, SEAM, Seam};
use wubble_ui::driver::{self, Wired};
use wubble_ui::{AppWindow, GlassItem, GlassKeys, GlassLayer};

/// 120 Hz.
const FRAME: Duration = Duration::from_micros(8_333);
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
        // What Slint draws this frame: the sidebar's glass up to the seam.
        let width = app.get_sidebar_width();
        let item = sidebar(scene, key);
        if width > 1.0 {
            let item = item.unwrap_or_else(|| panic!("frame {frame}: no sidebar glass at {width}"));
            assert_eq!(
                (item.x, item.y, item.width),
                (INSET as f32, INSET as f32, width),
                "frame {frame}: the scene's glass is not this frame's sidebar"
            );
        } else {
            assert!(
                item.is_none(),
                "frame {frame}: glass left at width {width}: {item:?}"
            );
        }
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
    ] {
        toggle(&app);
        now = run_to_rest(&app, &wired, &mut scene, now);
        assert_eq!(f64::from(app.get_sidebar_width()), want, "at rest");
    }
}
