//! The spring-driven shell on the testing backend: mock time only, explicit
//! frame deltas, no clocks. The platform can be installed once per process,
//! so the scenarios run in order inside one test.

use std::time::Duration;

use i_slint_backend_testing::{
    ElementHandle, init_integration_test_with_mock_time, mock_elapsed_time,
};
use slint::{
    ComponentHandle, LogicalPosition, LogicalSize, Model, ModelRc,
    platform::{PointerEventButton, WindowEvent},
};
use wubble_core::seam::{INSET, SEAM, SIDEBAR_DEFAULT};
use wubble_core::tabs::SpaceSwitcher;
use wubble_ui::driver::{self, Driver};
use wubble_ui::{AppWindow, PopupView, SidebarMode, SpaceItem, SplitPane, SplitSeam};

const DT: f32 = 0.016;

type Shared = std::rc::Rc<std::cell::RefCell<Driver>>;

/// Frame deltas until everything rests. Count-bounded, not time-bounded.
fn settle(app: &AppWindow, driver: &Shared) {
    for _ in 0..4000 {
        if !driver.borrow_mut().advance(app, DT) {
            return;
        }
    }
    panic!("springs never settled");
}

/// A fixed number of frames, for a held seam (it moves for as long as it is held).
fn frames(app: &AppWindow, driver: &Shared, n: usize) {
    for _ in 0..n {
        driver.borrow_mut().advance(app, DT);
    }
}

/// Lets Slint run its `changed` handlers, timers and tweens for `ms` of mock time.
fn pump(ms: u64) {
    mock_elapsed_time(Duration::from_millis(ms));
}

fn at(x: f32, y: f32) -> LogicalPosition {
    LogicalPosition::new(x, y)
}

fn pointer(app: &AppWindow, x: f32, y: f32) {
    app.window()
        .dispatch_event(WindowEvent::PointerMoved { position: at(x, y) });
    pump(1);
}

fn button(app: &AppWindow, x: f32, y: f32, down: bool) {
    let (position, button) = (at(x, y), PointerEventButton::Left);
    app.window().dispatch_event(if down {
        WindowEvent::PointerPressed { position, button }
    } else {
        WindowEvent::PointerReleased { position, button }
    });
    pump(1);
}

/// The seam's centre: the sidebar's edge plus half the inset.
fn seam_x(app: &AppWindow) -> f32 {
    INSET as f32 + app.get_sidebar_width() + INSET as f32 / 2.0
}

/// browser.css: `.wb-side` pads 6 inside the 8 px inset, `.wb-side-top` and every row pad 10
/// more; the header's buttons run 4 px past the padding; the peek panel pads 10 8 10 10.
const SIDE_PAD: f32 = 6.0;
const SIDE_GRID: f32 = 10.0;
const NAV_PULL: f32 = 4.0;

/// The header, the pill and the rows share one inset on both sides.
fn column_alignment(app: &AppWindow) {
    let e = app.get_side_edges();
    let left = INSET as f32 + SIDE_PAD;
    let right = INSET as f32 + app.get_sidebar_width() - SIDE_PAD;
    assert_eq!(e.pill, left);
    assert_eq!(e.rows, left);
    assert_eq!(e.new_tab, left);
    assert_eq!(e.lights, left + SIDE_GRID);
    assert_eq!(e.pill_end, right);
    assert_eq!(e.nav_end, right + NAV_PULL);
}

fn three_modes(app: &AppWindow, driver: &Shared) {
    settle(app, driver);
    assert_eq!(app.get_sidebar_mode(), SidebarMode::Extended);
    assert_eq!(app.get_sidebar_width(), SIDEBAR_DEFAULT as f32);
    assert_eq!(app.get_content_x(), 268.0);

    app.invoke_toggle_sidebar();
    assert_eq!(app.get_sidebar_mode(), SidebarMode::Hidden);
    settle(app, driver);
    assert_eq!(app.get_sidebar_width(), 0.0);
    assert_eq!(app.get_sidebar_opacity(), 0.0);
    assert_eq!(app.get_content_x(), 8.0);
    assert_eq!(app.get_content_width(), 1280.0 - 16.0);

    // The frame hook asks for no redraw once idle, and stays idle.
    let base = std::time::Instant::now();
    assert!(!driver.borrow_mut().frame(app, base));
    assert!(
        !driver
            .borrow_mut()
            .frame(app, base + Duration::from_millis(16))
    );

    app.invoke_toggle_sidebar();
    assert_eq!(app.get_sidebar_mode(), SidebarMode::Extended);
    settle(app, driver);
    assert_eq!(app.get_sidebar_width(), SIDEBAR_DEFAULT as f32);
    assert_eq!(app.get_content_x(), 268.0);
    assert!(!app.get_sidebar_rail());
}

fn peek(app: &AppWindow, driver: &Shared) {
    app.invoke_toggle_sidebar();
    settle(app, driver);
    pump(1);
    assert!(!app.get_peeking());

    // Into the edge strip: it peeks 60 ms later.
    pointer(app, 10.0, 300.0);
    pump(30);
    assert!(!app.get_peeking());
    pump(40);
    assert!(app.get_peeking());
    // The slide (340 ms), then over a child row: still peeking.
    pump(400);
    // The glowing border is on while peeking, off otherwise; the panel pads 10 8 10 10.
    assert_eq!(app.get_peek_glow(), 1.0);
    let e = app.get_side_edges();
    assert_eq!(e.pill, INSET as f32 + 10.0);
    assert_eq!(e.pill_end, INSET as f32 + SIDEBAR_DEFAULT as f32 - 8.0);
    pointer(app, 100.0, 80.0);
    pump(400);
    assert!(app.get_peeking());
    // Off the sidebar: it leaves 260 ms later.
    pointer(app, 900.0, 300.0);
    pump(200);
    assert!(app.get_peeking());
    pump(100);
    assert!(!app.get_peeking());
    pump(400);
    assert_eq!(app.get_peek_glow(), 0.0);

    // Pushed against the window's edge, over the west resize strip: it peeks too.
    pump(400);
    pointer(app, 2.0, 300.0);
    pump(100);
    assert!(app.get_peeking());
    pointer(app, 900.0, 300.0);
    pump(300);
    assert!(!app.get_peeking());

    // A peek never outlives the hidden mode (once the sidebar has slid away from the edge).
    pump(400);
    pointer(app, 10.0, 300.0);
    pump(100);
    assert!(app.get_peeking());
    app.invoke_toggle_sidebar();
    pump(1);
    assert!(!app.get_peek());
    settle(app, driver);
    pointer(app, 900.0, 300.0);
    assert_eq!(app.get_sidebar_mode(), SidebarMode::Extended);
}

fn seam_drag(app: &AppWindow, driver: &Shared) {
    // Inside the limits the width follows the pointer one to one (the pointer includes the inset).
    let x = seam_x(app);
    pointer(app, x, 300.0);
    button(app, x, 300.0, true);
    pointer(app, x + 30.0, 300.0);
    frames(app, driver, 600);
    assert_eq!(app.get_sidebar_width(), x + 30.0 - INSET as f32);
    assert!(app.get_seam_bowed());
    assert!(!app.get_seam_path().is_empty());
    // Far past the maximum it is banded, never beyond max + 34.
    pointer(app, 900.0, 300.0);
    frames(app, driver, 600);
    let held = app.get_sidebar_width();
    let max = SEAM::max as f32;
    assert!(held > max && held < max + SEAM::bandK as f32, "{held}");
    button(app, 900.0, 300.0, false);
    assert_eq!(app.get_sidebar_mode(), SidebarMode::Extended);
    settle(app, driver);
    assert_eq!(app.get_sidebar_width(), max);
    assert!(!app.get_seam_bowed());

    // Pulled under the rail snap point (128): the rail.
    let x = seam_x(app);
    pointer(app, x, 300.0);
    button(app, x, 300.0, true);
    pointer(app, INSET as f32 + 100.0, 300.0);
    frames(app, driver, 60);
    button(app, INSET as f32 + 100.0, 300.0, false);
    assert_eq!(app.get_sidebar_mode(), SidebarMode::Rail);
    settle(app, driver);
    assert_eq!(app.get_sidebar_width(), SEAM::rail as f32);
    assert!(app.get_sidebar_rail());

    // From the rail, under 22: hidden.
    let x = seam_x(app);
    pointer(app, x, 300.0);
    button(app, x, 300.0, true);
    pointer(app, INSET as f32 + 10.0, 300.0);
    frames(app, driver, 10);
    button(app, INSET as f32 + 10.0, 300.0, false);
    assert_eq!(app.get_sidebar_mode(), SidebarMode::Hidden);
    settle(app, driver);
    assert_eq!(app.get_sidebar_width(), 0.0);

    // The seam strip is gone; the sidebar toggle (the host routes its shortcut here) brings it
    // back at the saved width.
    app.invoke_toggle_sidebar();
    assert_eq!(app.get_sidebar_mode(), SidebarMode::Extended);
    settle(app, driver);
    assert_eq!(app.get_sidebar_width(), max);
    assert_eq!(app.get_sidebar_saved(), max);
    pointer(app, 900.0, 300.0);
}

fn space_capsule(app: &AppWindow, driver: &Shared) {
    let spaces: Vec<SpaceItem> = (1..=3)
        .map(|id| SpaceItem {
            id,
            name: "".into(),
            tint: Default::default(),
        })
        .collect();
    app.set_spaces(ModelRc::from(spaces.as_slice()));
    app.set_active_space_id(3);
    pump(1);
    settle(app, driver);
    assert_eq!(app.get_space_index(), 2);
    assert_eq!(app.get_capsule_offset(), 2.0 * SpaceSwitcher::PITCH as f32);
    app.set_active_space_id(1);
    pump(1);
    settle(app, driver);
    assert_eq!(app.get_capsule_offset(), 0.0);

    // Id set before the model: the index resolves when the model arrives.
    app.set_spaces(ModelRc::default());
    app.set_active_space_id(3);
    pump(1);
    assert_eq!(app.get_space_index(), -1);
    app.set_spaces(ModelRc::from(spaces.as_slice()));
    pump(1);
    settle(app, driver);
    assert_eq!(app.get_capsule_offset(), 2.0 * SpaceSwitcher::PITCH as f32);
}

/// The header's sidebar button (the first of the nav buttons right of the lights) asks for the
/// `toggle-sidebar` action, which the host runs like its shortcut. The header has no other
/// sidebar toggle.
fn sidebar_button(app: &AppWindow, driver: &Shared) {
    settle(app, driver);
    assert_eq!(app.get_sidebar_mode(), SidebarMode::Extended);
    let asked = std::rc::Rc::new(std::cell::RefCell::new(Vec::<String>::new()));
    let sink = asked.clone();
    app.on_shell_action(move |id| sink.borrow_mut().push(id.into()));
    // four 30 px buttons, 2 apart, ending at the nav edge, in the 30 high top row
    let x = app.get_side_edges().nav_end - 4.0 * 30.0 - 3.0 * 2.0 + 15.0;
    let y = INSET as f32 + SIDE_PAD + 15.0;
    pointer(app, x, y);
    button(app, x, y, true);
    button(app, x, y, false);
    assert_eq!(asked.borrow().as_slice(), ["toggle-sidebar"]);
    pointer(app, 900.0, 300.0);
}

/// A popup moved to fit the window is drawn where it fits, but a press on it reaches the page
/// at the engine's own rect, where CEF hit-tests the popup.
fn popup_input(app: &AppWindow) {
    let got = std::rc::Rc::new(std::cell::Cell::new(None));
    let sink = got.clone();
    app.on_web_pointer_pressed(move |pane, _, x, y| sink.set(Some((pane, x, y))));
    app.set_popup(PopupView {
        shown: true,
        // Drawn 100 px further left and 20 px lower than the engine put it.
        x: 1000.0,
        y: 100.0,
        width: 200.0,
        height: 100.0,
        origin_x: 1100.0,
        origin_y: 80.0,
    });
    pump(1);
    pointer(app, 1010.0, 110.0);
    button(app, 1010.0, 110.0, true);
    button(app, 1010.0, 110.0, false);
    assert_eq!(got.get(), Some((app.get_active_tab_id(), 1110.0, 90.0)));
    app.set_popup(PopupView::default());
    pump(1);
    pointer(app, 900.0, 300.0);
}

/// A fractional sidebar width (mid-spring, or a dragged width) leaves the page's rectangle on
/// whole pixels (scale 1 here), still filling the window to the inset.
fn content_on_whole_pixels(app: &AppWindow) {
    app.set_sidebar_width(252.4);
    pump(1);
    let (x, w) = (app.get_content_x(), app.get_content_width());
    assert_eq!(x, x.round(), "x {x}");
    assert_eq!(w, w.round(), "width {w}");
    assert_eq!(x + w + INSET as f32, 1280.0);
    assert_eq!(x, 268.0);
}

#[test]
fn shell_scenarios() {
    init_integration_test_with_mock_time();
    let app = AppWindow::new().unwrap();
    app.show().unwrap();
    app.window().set_size(LogicalSize::new(1280.0, 800.0));
    let driver = driver::wire(&app, wubble_core::seam::Seam::new()).driver;
    settle(&app, &driver);
    assert_eq!(app.get_peek_glow(), 0.0);
    column_alignment(&app);
    three_modes(&app, &driver);
    sidebar_button(&app, &driver);
    peek(&app, &driver);
    seam_drag(&app, &driver);
    space_capsule(&app, &driver);
    popup_input(&app);
    split_panes(&app, &driver);
    stretched_panes(&app);
    sections_ride_the_string(&app, &driver);
    content_on_whole_pixels(&app);
}

fn pane(id: i32, x0: f32, x1: f32) -> SplitPane {
    SplitPane {
        id,
        x0,
        x1,
        focused: id == 1,
        ..Default::default()
    }
}

/// One page is one pane filling the content area; three panes keep their rects, and every press
/// reaches the page under it with that pane's id and pane-relative coordinates. A seam's strip
/// takes the pointer over the gap.
fn split_panes(app: &AppWindow, driver: &Shared) {
    settle(app, driver);
    let pressed = std::rc::Rc::new(std::cell::RefCell::new(vec![]));
    let sink = pressed.clone();
    app.on_web_pointer_pressed(move |pane, _, x, y| sink.borrow_mut().push((pane, x, y)));
    let grabbed = std::rc::Rc::new(std::cell::Cell::new(None));
    let sink = grabbed.clone();
    app.on_split_grab(move |j, x, _| sink.set(Some((j, x))));
    let (cx, cy, w) = (
        app.get_content_x(),
        app.get_content_y(),
        app.get_content_width(),
    );
    let click = |x: f32, y: f32| {
        pointer(app, x, y);
        button(app, x, y, true);
        button(app, x, y, false);
    };

    app.set_panes(ModelRc::from([pane(1, 0.0, w)].as_slice()));
    pump(1);
    click(cx + 30.0, cy + 40.0);
    assert_eq!(pressed.borrow().last(), Some(&(1, 30.0, 40.0)));

    let third = 300.0;
    let panes = [
        pane(1, 0.0, third - 4.0),
        pane(2, third + 4.0, 2.0 * third - 4.0),
        pane(3, 2.0 * third + 4.0, w),
    ];
    app.set_panes(ModelRc::from(panes.as_slice()));
    let seams = [third, 2.0 * third].map(|x| SplitSeam {
        x,
        path: Default::default(),
    });
    app.set_seams(ModelRc::from(seams.as_slice()));
    pump(1);
    assert_eq!(app.get_panes().iter().collect::<Vec<_>>(), panes.to_vec());
    click(cx + 2.0 * third + 14.0, cy + 50.0);
    assert_eq!(pressed.borrow().last(), Some(&(3, 10.0, 50.0)));
    click(cx + third - 14.0, cy + 300.0);
    assert_eq!(pressed.borrow().last(), Some(&(1, third - 14.0, 300.0)));
    // the close button of the first pane is the shell's
    let closed = std::rc::Rc::new(std::cell::Cell::new(0));
    let sink = closed.clone();
    app.on_pane_close(move |id| sink.set(id));
    click(cx + third - 4.0 - 8.0 - 14.0, cy + 8.0 + 14.0);
    assert_eq!(closed.get(), 1);
    // the strip over the second seam
    let n = pressed.borrow().len();
    click(cx + 2.0 * third + 1.0, cy + 300.0);
    assert_eq!(grabbed.get(), Some((1, 2.0 * third + 1.0)));
    assert_eq!(
        pressed.borrow().len(),
        n,
        "the seam's press is not the page's"
    );

    app.set_panes(ModelRc::default());
    app.set_seams(ModelRc::default());
    pump(1);
}

/// The frames shown, as (x, y, width, height) in window logical px, with the pane rects.
fn frame_rects(app: &AppWindow) -> Vec<(f32, f32, f32, f32)> {
    ElementHandle::find_by_element_id(app, "SplitView::pane-frame")
        .map(|e| {
            let (p, s) = (e.absolute_position(), e.size());
            (p.x, p.y, s.width, s.height)
        })
        .collect()
}

/// TODO.md "Resize feel": a pane's frame always fills the pane (a stale one is scaled to it),
/// stretched further while the size moves and 1:1 on whole pixels at rest. One page alone follows
/// the window in the same layout pass, with nothing pushed from the host.
fn stretched_panes(app: &AppWindow) {
    let (cx, cy, w, h) = (
        app.get_content_x(),
        app.get_content_y(),
        app.get_content_width(),
        app.get_content_height(),
    );
    // a stale frame (clip of another size): fills the pane
    let stale = SplitPane {
        id: 1,
        fill: true,
        clip_width: 900,
        clip_height: 500,
        ..Default::default()
    };
    app.set_panes(ModelRc::from([stale.clone()].as_slice()));
    pump(1);
    assert_eq!(frame_rects(app), vec![(cx, cy, w, h)]);
    // stretched: anchored top left, 3% wider and taller; the pane clips the overshoot
    let stretched = SplitPane {
        stretch_x: 0.03,
        stretch_y: 0.03,
        ..stale.clone()
    };
    app.set_panes(ModelRc::from([stretched].as_slice()));
    pump(1);
    let got = frame_rects(app)[0];
    assert_eq!((got.0, got.1), (cx, cy));
    assert!(
        (got.2 - w * 1.03).abs() < 1e-3 && (got.3 - h * 1.03).abs() < 1e-3,
        "{got:?}"
    );
    // at rest, 1:1 on whole pixels
    app.set_panes(ModelRc::from([stale].as_slice()));
    pump(1);
    let got = frame_rects(app)[0];
    for v in [got.0, got.1, got.2, got.3] {
        assert_eq!(v, v.round(), "{got:?}");
    }
    // the window grows: the pane and its frame follow without the host
    app.window().set_size(LogicalSize::new(1400.0, 860.0));
    pump(1);
    let (w2, h2) = (app.get_content_width(), app.get_content_height());
    assert!(w2 > w && h2 > h);
    assert_eq!(frame_rects(app), vec![(cx, cy, w2, h2)]);
    app.window().set_size(LogicalSize::new(1280.0, 800.0));
    app.set_panes(ModelRc::default());
    pump(1);
}

/// While the seam is held and bowed the sidebar's sections ride the string at their own height
/// (design browser/chrome.tsx `[data-ride]`), the one at the grab height most; released and
/// settled, every one is back in its column.
fn sections_ride_the_string(app: &AppWindow, driver: &Shared) {
    settle(app, driver);
    let rest = app.get_side_edges();
    let x = seam_x(app);
    // grabbed at the pill's height, pulled out
    let pill_at = ElementHandle::find_by_element_id(app, "Sidebar::pill")
        .next()
        .expect("the address pill");
    let y = pill_at.absolute_position().y + pill_at.size().height / 2.0;
    pointer(app, x, y);
    button(app, x, y, true);
    pointer(app, x + 60.0, y);
    frames(app, driver, 3);
    assert!(app.get_seam_bowed());
    assert!(app.get_seam_ride().row_count() > 0);
    pump(1);
    let held = app.get_side_edges();
    let pill = held.pill - rest.pill;
    let lights = held.lights - rest.lights;
    assert!(pill > 1.0, "the pill rides: {pill}");
    assert!(
        lights < pill,
        "the header rides less, nearer the end: {lights} {pill}"
    );
    button(app, x + 60.0, y, false);
    settle(app, driver);
    pump(1);
    assert_eq!(app.get_seam_ride().row_count(), 0);
    assert_eq!(app.get_side_edges().pill, rest.pill);
    pointer(app, 900.0, 300.0);
}
