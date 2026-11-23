//! A popup opened by a toggle button closes when that button is clicked again, and stays closed
//! (qa4): the press on the anchor is the scrim's outside press, which dismisses the popup and is
//! consumed whole (kit `Scrim`), so its release is no click that opens the popup again. For each
//! anchored popup with a toggle: the Spaces popup (the "^" and the current Space's tile), the
//! downloads popover, the address pill's site and media menus (the host's menu at a point, as the
//! host opens it) and a settings Select. A press elsewhere closes; a click on the anchor opens.
//! Mock time only; one test, because the platform can be installed once per process.

use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use i_slint_backend_testing::{
    AccessibleRole, ElementHandle, ElementQuery, init_integration_test_with_mock_time,
    mock_elapsed_time,
};
use slint::platform::{PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, LogicalSize, ModelRc, VecModel};
use wubble_core::pages::settings_page;
use wubble_ui::settings::setting_items;
use wubble_ui::{AppWindow, DownloadItem, InternalPageKind, MenuEntry, MenuView, SpaceItem};

fn pump(ms: u64) {
    mock_elapsed_time(Duration::from_millis(ms));
}

fn pointer(app: &AppWindow, (x, y): (f32, f32), event: u8) {
    let position = LogicalPosition::new(x, y);
    let button = PointerEventButton::Left;
    app.window().dispatch_event(match event {
        0 => WindowEvent::PointerMoved { position },
        1 => WindowEvent::PointerPressed { position, button },
        _ => WindowEvent::PointerReleased { position, button },
    });
    pump(1);
}

/// A whole click at `at`: move there, press, release.
fn click(app: &AppWindow, at: (f32, f32)) {
    pointer(app, at, 0);
    pointer(app, at, 1);
    pointer(app, at, 2);
}

fn centre_of(h: &ElementHandle) -> (f32, f32) {
    let (p, s) = (h.absolute_position(), h.size());
    (p.x + s.width / 2.0, p.y + s.height / 2.0)
}

fn button(app: &AppWindow, label: &str) -> (f32, f32) {
    let h = ElementHandle::find_by_accessible_label(app, label)
        .find(|h| h.accessible_role() == Some(AccessibleRole::Button))
        .unwrap_or_else(|| panic!("no button {label}"));
    centre_of(&h)
}

/// The toggle's rules for one popup: its anchor opens it, the anchor again closes it for good
/// (through the frames that follow too), a press elsewhere closes it.
fn toggles(app: &AppWindow, name: &str, anchor: (f32, f32), open: &dyn Fn() -> bool) {
    let elsewhere = (900.0, 420.0);
    assert!(!open(), "{name}: closed at first");
    click(app, anchor);
    pump(400);
    assert!(open(), "{name}: the anchor opens it");
    click(app, anchor);
    assert!(!open(), "{name}: the anchor closes it");
    for _ in 0..30 {
        pump(16);
        assert!(!open(), "{name}: and it stays closed");
    }
    click(app, anchor);
    pump(400);
    assert!(open(), "{name}: the anchor opens it again");
    click(app, elsewhere);
    pump(400);
    assert!(!open(), "{name}: a press elsewhere closes it");
}

fn model<T: Clone + 'static>(v: Vec<T>) -> ModelRc<T> {
    ModelRc::from(Rc::new(VecModel::from(v)))
}

fn menu_at(app: &AppWindow, x: f32, y: f32) {
    app.set_menu(MenuView {
        open: true,
        x,
        y,
        entries: model(vec![MenuEntry {
            label: "Allow on this site".into(),
            enabled: true,
            ..MenuEntry::default()
        }]),
        icons: false,
    });
}

#[test]
fn an_anchor_toggles_its_popup() {
    init_integration_test_with_mock_time();
    let app = AppWindow::new().unwrap();
    app.show().unwrap();
    app.window().set_size(LogicalSize::new(1280.0, 800.0));
    let _wired = wubble_ui::driver::wire(&app, wubble_core::seam::Seam::new());

    // The host's side: the Spaces popup opens through the overlays' store and an open one stays
    // open (the anchor never closes it itself); the downloads button asks the host to toggle
    // the popover; a press outside asks either to close; the address chips open the host's menu
    // at a point, which its dismissal closes.
    let w = app.as_weak();
    app.on_spaces_menu_requested(move || w.unwrap().set_spaces_open(true));
    let w = app.as_weak();
    app.on_spaces_dismissed(move || w.unwrap().set_spaces_open(false));
    let w = app.as_weak();
    app.on_downloads_requested(move || {
        let app = w.unwrap();
        app.set_downloads_open(!app.get_downloads_open());
    });
    let w = app.as_weak();
    app.on_downloads_dismissed(move || w.unwrap().set_downloads_open(false));
    let w = app.as_weak();
    app.on_address_blocker_requested(move |x, y| menu_at(&w.unwrap(), x, y));
    let w = app.as_weak();
    app.on_address_media_requested(move |x, y| menu_at(&w.unwrap(), x, y));
    let w = app.as_weak();
    app.on_menu_dismissed(move || {
        let app = w.unwrap();
        app.set_menu(MenuView {
            open: false,
            ..app.get_menu()
        });
    });

    app.set_spaces(model(vec![SpaceItem {
        id: 1,
        name: "Home".into(),
        ..SpaceItem::default()
    }]));
    app.set_active_space_id(1);
    app.set_downloads(model(vec![DownloadItem {
        id: 1,
        name: "a.zip".into(),
        ..DownloadItem::default()
    }]));
    app.set_address_host("example.com".into());
    app.set_address_blocker_visible(true);
    app.set_address_media_visible(true);
    pump(600);

    let spaces = || app.get_spaces_open();
    toggles(&app, "Spaces (^)", button(&app, "Spaces"), &spaces);
    toggles(&app, "Spaces (current tile)", button(&app, "Home"), &spaces);
    let downloads = || app.get_downloads_open();
    toggles(&app, "downloads", button(&app, "Downloads"), &downloads);
    let menu = || app.get_menu().open;
    let shield = button(&app, "Blocker: 0 blocked");
    toggles(&app, "site controls", shield, &menu);
    toggles(&app, "media menu", button(&app, "Save media"), &menu);

    // A settings Select: its trigger is its own toggle.
    app.set_page(InternalPageKind::Settings);
    app.set_settings_rows(model(setting_items(&settings_page(&HashMap::new()))));
    pump(600);
    let select = ElementQuery::from_root(&app)
        .match_descendants()
        .match_accessible_role(AccessibleRole::Combobox)
        .find_first()
        .expect("a Select on the settings page");
    let expanded = || select.accessible_expanded() == Some(true);
    toggles(&app, "Select", centre_of(&select), &expanded);
}
