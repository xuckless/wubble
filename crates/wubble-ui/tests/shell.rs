//! The spring-driven shell on the testing backend: mock time only, explicit
//! frame deltas, no clocks. The platform can be installed once per process,
//! so the scenarios run in order inside one test.

use std::time::Duration;

use i_slint_backend_testing::{init_integration_test_with_mock_time, mock_elapsed_time};
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
    header_and_footer(&app, &driver);
    rail_centre_line(&app, &driver);
    peek_panel_holds_its_contents(&app, &driver);
    content_on_whole_pixels(&app);
}

/// Springs, then Slint's own tweens (fades, the peek's slide), to rest.
fn rest(app: &AppWindow, driver: &Shared) {
    settle(app, driver);
    pump(600);
}

/// Drags the seam to `raw` px right of the inset and lets go there.
fn drag_seam_to(app: &AppWindow, driver: &Shared, raw: f32) {
    let x = seam_x(app);
    pointer(app, x, 300.0);
    button(app, x, 300.0, true);
    pointer(app, INSET as f32 + raw, 300.0);
    frames(app, driver, 60);
    button(app, INSET as f32 + raw, 300.0, false);
    pointer(app, 900.0, 300.0);
    rest(app, driver);
}

/// A part of the sidebar (left and right edge) lies inside the panel `[start, end]`.
fn inside(what: &str, (a, b): (f32, f32), (start, end): (f32, f32)) {
    assert!(
        a >= start && b <= end,
        "{what} {a}..{b} leaves the panel {start}..{end}"
    );
}

/// How far a circle (centre, radius) lies inside a rounded rectangle; negative when it pokes out.
fn margin_in_rounded(
    (cx, cy, r): (f32, f32, f32),
    (x0, y0, x1, y1): (f32, f32, f32, f32),
    radius: f32,
) -> f32 {
    // the nearest corner circle's centre, or the point itself where the edge is straight
    let qx = cx.clamp(x0 + radius, x1 - radius);
    let qy = cy.clamp(y0 + radius, y1 - radius);
    let straight = (cx - x0).min(x1 - cx).min(cy - y0).min(y1 - cy);
    let d = if qx == cx || qy == cy {
        straight
    } else {
        radius - ((cx - qx).powi(2) + (cy - qy).powi(2)).sqrt()
    };
    d - r
}

/// browser.css `.wb-light`: 12 px dots; the gap is 8, 6 on the rail (`.wb-side-rail .wb-lights`).
const LIGHT: f32 = 12.0;

/// The header holds the lights and the four nav buttons, and they never meet; the downloads button
/// is the footer's secondary action (`.wb-side-bottom > .wb-ib`, 36 square), at its end, clear of
/// the Space tiles. At the default width and at the minimum.
fn header_and_footer(app: &AppWindow, driver: &Shared) {
    use wubble_ui::DownloadItem;
    app.set_downloads(ModelRc::from([DownloadItem::default()].as_slice()));
    for raw in [SIDEBAR_DEFAULT as f32, SEAM::min as f32] {
        drag_seam_to(app, driver, raw);
        assert_eq!(app.get_sidebar_mode(), SidebarMode::Extended);
        assert_eq!(app.get_sidebar_width(), raw);
        let e = app.get_side_edges();
        assert!(
            e.lights_end < e.nav,
            "at {raw}: the lights end at {} and the nav starts at {}",
            e.lights_end,
            e.nav
        );
        assert!(
            e.downloads_y > e.pill_y,
            "at {raw}: downloads in the header"
        );
        assert_eq!(e.downloads_end, e.end - SIDE_PAD, "at {raw}");
        assert_eq!(e.downloads_end - e.downloads, 36.0, "at {raw}");
        assert!(
            e.spaces_end < e.downloads,
            "at {raw}: tiles under downloads"
        );
    }
}

/// On the rail (browser.css `.wb-side-rail`: padding 6 0, everything centred) the lights, the
/// search button and its glyph, the favourites, the rows, the new-tab row, the Space tiles and the
/// downloads button share the rail's centre line; the glyph sits in the middle of its button; the
/// lights fit the rail's glass and clear the window's rounded corner.
fn rail_centre_line(app: &AppWindow, driver: &Shared) {
    use slint::{Image, Rgba8Pixel, SharedPixelBuffer};
    use wubble_ui::{Radius, TabItem};
    let icon = Image::from_rgba8(SharedPixelBuffer::<Rgba8Pixel>::new(16, 16));
    let tab = |id| TabItem {
        id,
        title: "page".into(),
        favicon: icon.clone(),
        ..Default::default()
    };
    app.set_pinned_tabs(ModelRc::from([tab(10)].as_slice()));
    app.set_tabs(ModelRc::from([tab(11), tab(12)].as_slice()));
    pump(1);
    drag_seam_to(app, driver, 100.0);
    assert_eq!(app.get_sidebar_mode(), SidebarMode::Rail);
    let rail = SEAM::rail as f32;
    assert_eq!(app.get_sidebar_width(), rail);
    let e = app.get_side_edges();
    let mid = INSET as f32 + rail / 2.0;
    for (what, a, b) in [
        ("lights", e.lights, e.lights_end),
        ("pill", e.pill, e.pill_end),
        ("rows", e.rows, e.rows_end),
        ("new tab", e.new_tab, e.new_tab_end),
        ("spaces", e.spaces, e.spaces_end),
        ("downloads", e.downloads, e.downloads_end),
    ] {
        assert_eq!((a + b) / 2.0, mid, "{what} {a}..{b} off the rail's centre");
    }
    assert_eq!((e.glyph_x, e.glyph_y), (mid, e.pill_y), "the search glyph");
    // the same vertical rhythm as extended: the 30 px top row, 12 below it the 36 px button
    assert_eq!(e.pill_y, INSET as f32 + SIDE_PAD + 30.0 + 12.0 + 18.0);

    // three dots 6 apart, inside the rail's glass and the window's corner
    assert_eq!(e.lights_end - e.lights, 3.0 * LIGHT + 2.0 * 6.0);
    let radius = app.global::<Radius>();
    let (w, h) = (1280.0, 800.0);
    let glass = (
        INSET as f32,
        INSET as f32,
        INSET as f32 + rail,
        h - INSET as f32,
    );
    for i in 0..3 {
        let dot = (
            e.lights + LIGHT / 2.0 + i as f32 * (LIGHT + 6.0),
            e.lights_y + LIGHT / 2.0,
            LIGHT / 2.0,
        );
        let in_glass = margin_in_rounded(dot, glass, radius.get_lg());
        let in_window = margin_in_rounded(dot, (0.0, 0.0, w, h), radius.get_window());
        assert!(in_glass >= 4.0, "dot {i} {in_glass} px inside the glass");
        assert!(in_window >= 4.0, "dot {i} {in_window} px inside the window");
    }
}

/// The peek's panel is the sidebar's box at the saved width, and every part of the contents lies
/// inside it with the panel's padding (`.wb-side-peek`: 10 8 10 10), whether the sidebar was a
/// rail or extended before it hid, and also when the pointer reaches the edge while it is still
/// closing. Leaving the peek takes everything off screen. While docked, the contents never reach
/// past the glass (the box clips them at the live width).
fn peek_panel_holds_its_contents(app: &AppWindow, driver: &Shared) {
    let check = |from: &str| {
        let e = app.get_side_edges();
        let saved = app.get_sidebar_saved();
        let panel = (e.start, e.end);
        assert_eq!(panel, (INSET as f32, INSET as f32 + saved), "{from}");
        assert_eq!(e.content_end, e.end, "{from}");
        assert_eq!(e.pill, e.start + 10.0, "{from}");
        assert_eq!(e.pill_end, e.end - 8.0, "{from}");
        for (what, part) in [
            ("lights", (e.lights, e.lights_end)),
            ("nav", (e.nav, e.nav_end)),
            ("pill", (e.pill, e.pill_end)),
            ("rows", (e.rows, e.rows_end)),
            ("new tab", (e.new_tab, e.new_tab_end)),
            ("spaces", (e.spaces, e.spaces_end)),
            ("downloads", (e.downloads, e.downloads_end)),
        ] {
            inside(&format!("{from}: {what}"), part, panel);
        }
    };
    let peek_in = |x: f32| {
        pointer(app, x, 300.0);
        pump(100);
        assert!(app.get_peeking());
        pump(400);
    };
    let peek_out = || {
        pointer(app, 900.0, 300.0);
        pump(300);
        assert!(!app.get_peeking());
        pump(500);
        let e = app.get_side_edges();
        assert!(e.end <= 0.0, "the peek left contents on screen: {}", e.end);
    };

    // from the rail (the previous scenario left it there)
    assert_eq!(app.get_sidebar_mode(), SidebarMode::Rail);
    app.invoke_toggle_sidebar();
    rest(app, driver);
    peek_in(2.0);
    check("from the rail");
    peek_out();

    // extended, then closing: the contents stay inside the glass every frame
    app.invoke_toggle_sidebar();
    rest(app, driver);
    assert_eq!(app.get_sidebar_mode(), SidebarMode::Extended);
    app.invoke_toggle_sidebar();
    for _ in 0..200 {
        let (w, e) = (app.get_sidebar_width(), app.get_side_edges());
        if w <= 0.5 {
            break;
        }
        assert!(
            e.end <= INSET as f32 + w + 0.01,
            "the contents reach {} past the glass at {}",
            e.end,
            INSET as f32 + w
        );
        frames(app, driver, 1);
    }
    rest(app, driver);
    peek_in(2.0);
    check("from extended");
    peek_out();

    // the pointer reaches the edge while the sidebar is still closing: the peek panel, at once
    app.invoke_toggle_sidebar();
    rest(app, driver);
    app.invoke_toggle_sidebar();
    frames(app, driver, 2);
    peek_in(2.0);
    assert!(app.get_sidebar_width() > 0.5, "still closing");
    check("while closing");
    // it finishes closing under the panel, which stays put
    settle(app, driver);
    check("closed under the peek");
    peek_out();

    // back to extended for what follows
    app.invoke_toggle_sidebar();
    rest(app, driver);
    app.set_downloads(ModelRc::default());
    pump(1);
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
