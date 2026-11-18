//! The glass panels' items for the wgpu scene, on the testing backend: what each GlassPanel
//! reports through `GlassLayer.report` as layout, touch and the scene switch change. Mock time
//! only. The platform can be installed once per process, so the scenarios run in one test.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;
use std::time::Duration;

use i_slint_backend_testing::{init_integration_test_with_mock_time, mock_elapsed_time};
use slint::{
    ComponentHandle, LogicalPosition, ModelRc,
    platform::{PointerEventButton, WindowEvent},
};
use wubble_core::seam::INSET;
use wubble_ui::{AppWindow, GlassItem, GlassKeys, GlassLayer, GlassTier, SidebarMode, TabItem};

type Seen = Rc<RefCell<BTreeMap<i32, GlassItem>>>;

fn pump(ms: u64) {
    mock_elapsed_time(Duration::from_millis(ms));
}

fn item(seen: &Seen, key: i32) -> GlassItem {
    seen.borrow()
        .get(&key)
        .cloned()
        .unwrap_or_else(|| panic!("panel {key} never reported"))
}

fn press(app: &AppWindow, x: f32, y: f32, down: bool) {
    let (position, button) = (LogicalPosition::new(x, y), PointerEventButton::Left);
    app.window().dispatch_event(if down {
        WindowEvent::PointerPressed { position, button }
    } else {
        WindowEvent::PointerReleased { position, button }
    });
}

#[test]
fn glass_panels_report_their_items() {
    init_integration_test_with_mock_time();
    let app = AppWindow::new().unwrap();
    // The last report of every key, present or not (the registry would drop absent ones).
    let seen: Seen = Rc::default();
    let s = Rc::clone(&seen);
    let layer = app.global::<GlassLayer>();
    layer.on_report(move |item| {
        s.borrow_mut().insert(item.key, item);
    });
    let keys = app.global::<GlassKeys>();
    let (sidebar, address) = (keys.get_sidebar(), keys.get_address());
    app.show().unwrap();
    // The panels reported on `init`, before this handler: a bump asks again.
    layer.set_generation(layer.get_generation() + 1);
    pump(1);

    // Without the scene every panel is absent (the Slint glass draws itself).
    assert!(!item(&seen, address).present);
    assert!(seen.borrow().values().all(|i| !i.present));

    // With it: the sidebar's Regular glass up to the seam and the pill's Thin glass.
    layer.set_scene_active(true);
    pump(1);
    let side = item(&seen, sidebar);
    assert!(side.present);
    assert_eq!((side.x, side.y), (INSET as f32, INSET as f32));
    assert_eq!(side.width, app.get_sidebar_width());
    assert_eq!(side.tier, GlassTier::Regular);
    let pill = item(&seen, address);
    assert!(pill.present);
    assert_eq!(pill.tier, GlassTier::Thin);
    assert!(pill.x > side.x && pill.y > side.y && pill.width > 100.0);

    // The seam narrows the sidebar: its item follows.
    app.set_sidebar_width(220.0);
    pump(1);
    assert_eq!(item(&seen, sidebar).width, 220.0);

    // A pinned tab is a favourite tile, keyed by its tab id.
    let tab = TabItem {
        id: 5,
        title: "a".into(),
        url: "https://a.test/".into(),
        favicon: Default::default(),
    };
    app.set_pinned_tabs(ModelRc::from([tab].as_slice()));
    pump(1);
    let fav_key = keys.get_favourites() + 5;
    let fav = item(&seen, fav_key);
    assert!(fav.present && !fav.pressed);

    // Pressed, it shrinks to 0.97 about its centre, and the item says so.
    let (cx, cy) = (fav.x + fav.width / 2.0, fav.y + fav.height / 2.0);
    press(&app, cx, cy, true);
    pump(500);
    let down = item(&seen, fav_key);
    assert!(down.pressed && down.hover);
    let near = |a: f32, b: f32| (a - b).abs() < 0.01;
    assert!(
        near(down.width, fav.width * 0.97),
        "{} vs {}",
        down.width,
        fav.width
    );
    assert!(near(down.height, fav.height * 0.97));
    assert!(
        near(down.x + down.width / 2.0, cx),
        "scaled about the centre: {fav:?} {down:?}"
    );
    assert!(near(down.y + down.height / 2.0, cy));
    press(&app, cx, cy, false);
    pump(500);
    assert!(near(item(&seen, fav_key).width, fav.width));

    // A generation bump makes every live panel report again.
    seen.borrow_mut().clear();
    layer.set_generation(layer.get_generation() + 1);
    pump(1);
    for key in [sidebar, address, fav_key] {
        assert!(item(&seen, key).present, "{key} reported again");
    }

    // Hidden at rest the sidebar slides away with its parent (the peek's zone): the pill's
    // item follows an ancestor's move, and is absent while the scene cannot show it.
    app.set_sidebar_mode(SidebarMode::Hidden);
    app.set_sidebar_width(0.0);
    pump(1);
    let gone = item(&seen, address);
    assert!(!gone.present);
    assert!(gone.x < 0.0, "moved with the zone: x {}", gone.x);
    assert!(!item(&seen, sidebar).present);
    app.set_peek(true);
    pump(1000);
    assert!(near(item(&seen, address).x, pill.x), "back with the peek");

    // The scene off again: everything absent.
    app.set_peek(false);
    app.set_sidebar_mode(SidebarMode::Extended);
    app.set_sidebar_width(252.0);
    layer.set_scene_active(false);
    pump(1);
    assert!(seen.borrow().values().all(|i| !i.present));
}
