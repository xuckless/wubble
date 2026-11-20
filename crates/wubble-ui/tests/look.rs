//! The design's margins and the settings page's structure, measured on the testing backend
//! (design repo @ 21838b8, browser.css and settings.css): the page card sits one inset from the
//! window's top, right and bottom and from the sidebar in every sidebar mode; the footer ends
//! where the page card does, less the sidebar's padding; the selected row's raised tile is the
//! row; and the settings page's section list is generated from the table, beside its rows, and
//! picking a section glides the page there on the shared page scroller.
//! Mock time only; one test, because the platform can be installed once per process.

use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use i_slint_backend_testing::{
    AccessibleRole, ElementHandle, init_integration_test_with_mock_time, mock_elapsed_time,
};
use slint::{ComponentHandle, LogicalSize, ModelRc, VecModel};
use wubble_core::pages::settings_page_from;
use wubble_core::seam::{INSET, SEAM, SIDEBAR_DEFAULT};
use wubble_core::settings::{Section, SettingDef, SettingKind};
use wubble_ui::driver;
use wubble_ui::settings::setting_items;
use wubble_ui::{AppWindow, DownloadItem, InternalPageKind, PageScroll, SidebarMode, TabItem};

const W: f32 = 1360.0;
const H: f32 = 880.0;
/// browser.css .wb-side: padding 6.
const SIDE_PAD: f32 = 6.0;
/// .wb-side-bottom > .wb-ib: 36 square.
const BUTTON: f32 = 36.0;
/// Chrome.row: tab rows 36 high.
const ROW: f32 = 36.0;

fn pump(ms: u64) {
    mock_elapsed_time(Duration::from_millis(ms));
}

/// The page card's margins: left, top, right, bottom.
fn margins(app: &AppWindow) -> (f32, f32, f32, f32) {
    let (x, y) = (app.get_content_x(), app.get_content_y());
    let (w, h) = (app.get_content_width(), app.get_content_height());
    (x, y, W - x - w, H - y - h)
}

fn settle(app: &AppWindow, driver: &Rc<std::cell::RefCell<driver::Driver>>) {
    for _ in 0..4000 {
        if !driver.borrow_mut().advance(app, 0.016) {
            pump(600);
            return;
        }
    }
    panic!("springs never settled");
}

const fn def(key: &'static str, section: Section) -> SettingDef {
    SettingDef {
        key,
        kind: SettingKind::Bool,
        default: "1",
        label: key,
        section,
        restart: false,
    }
}

fn nav_rows(app: &AppWindow, title: &str) -> Vec<ElementHandle> {
    ElementHandle::find_by_accessible_label(app, title)
        .filter(|h| h.accessible_role() == Some(AccessibleRole::Tab))
        .collect()
}

#[test]
fn look_scenarios() {
    init_integration_test_with_mock_time();
    let app = AppWindow::new().unwrap();
    app.show().unwrap();
    app.window().set_size(LogicalSize::new(W, H));
    let driver = driver::wire(&app, wubble_core::seam::Seam::new()).driver;
    settle(&app, &driver);
    let inset = INSET as f32;

    // Extended: the page card one inset from the sidebar and from the window's other edges.
    assert_eq!(app.get_sidebar_mode(), SidebarMode::Extended);
    let side = SIDEBAR_DEFAULT as f32;
    assert_eq!(margins(&app), (inset + side + inset, inset, inset, inset));

    // The footer ends where the page card does, less the sidebar's padding: with downloads, the
    // button's bottom is the card's bottom minus 6.
    app.set_downloads(ModelRc::from([DownloadItem::default()].as_slice()));
    pump(1);
    let e = app.get_side_edges();
    assert_eq!(
        e.downloads_y + BUTTON + SIDE_PAD,
        app.get_content_y() + app.get_content_height(),
        "the footer's bottom margin is the page card's"
    );

    // The selected row's raised tile is the row: the list's full width, one row high, on it.
    let tab = |id| TabItem {
        id,
        title: "page".into(),
        ..Default::default()
    };
    app.set_tabs(ModelRc::from([tab(1), tab(2), tab(3)].as_slice()));
    app.set_active_tab_id(2);
    app.invoke_shell_model_changed();
    settle(&app, &driver);
    let ind = ElementHandle::find_by_element_id(&app, "Sidebar::indicator")
        .next()
        .expect("the selected row's tile");
    let (pos, size) = (ind.absolute_position(), ind.size());
    let e = app.get_side_edges();
    // the follow spring rests within a hair of its target
    let near = |a: f32, b: f32| (a - b).abs() < 0.01;
    assert!(
        near(pos.x, e.rows) && near(size.width, e.rows_end - e.rows),
        "{pos:?} {size:?} {e:?}"
    );
    assert!(near(size.height, ROW));
    assert!(ind.computed_opacity() > 0.99);

    // Rail: the same margins, the rail's width beside.
    app.invoke_seam_double_clicked();
    settle(&app, &driver);
    assert_eq!(app.get_sidebar_mode(), SidebarMode::Rail);
    assert_eq!(
        margins(&app),
        (inset + SEAM::rail as f32 + inset, inset, inset, inset)
    );

    // Hidden: the page card one inset from all four edges.
    app.invoke_toggle_sidebar();
    settle(&app, &driver);
    assert_eq!(app.get_sidebar_mode(), SidebarMode::Hidden);
    assert_eq!(margins(&app), (inset, inset, inset, inset));
    app.invoke_toggle_sidebar();
    settle(&app, &driver);

    // The settings page: one section in the list per section of the table, then the page's own
    // four, left of the rows; a new section in the table is a new row in the list, no UI code.
    app.set_page(InternalPageKind::Settings);
    const DEFS: [SettingDef; 3] = [
        def("a.one", Section::General),
        def("a.two", Section::General),
        def("b.one", Section::Privacy),
    ];
    let table = |defs: &[SettingDef]| {
        let defs: Vec<&SettingDef> = defs.iter().collect();
        ModelRc::from(Rc::new(VecModel::from(setting_items(&settings_page_from(
            &defs,
            &HashMap::new(),
        )))))
    };
    app.set_settings_rows(table(&DEFS[..2]));
    pump(500);
    assert_eq!(nav_rows(&app, "General").len(), 1);
    assert!(nav_rows(&app, "Privacy").is_empty());
    for own in ["Keyboard shortcuts", "Spaces", "Sites", "About"] {
        assert_eq!(nav_rows(&app, own).len(), 1, "{own}");
    }
    app.set_settings_rows(table(&DEFS));
    pump(500);
    let general = &nav_rows(&app, "General")[0];
    let privacy = nav_rows(&app, "Privacy");
    assert_eq!(privacy.len(), 1, "a section added to the table is listed");
    // .st-nav: rows 36, 2 apart, in the table's order
    assert_eq!(
        privacy[0].absolute_position().y - general.absolute_position().y,
        38.0
    );
    // two columns: the list (232 wide) left of the rows, 48 apart
    let row = ElementHandle::find_by_element_id(&app, "SettingsPage::setting-row")
        .next()
        .unwrap();
    let list_end = general.absolute_position().x + general.size().width;
    assert_eq!(row.absolute_position().x - list_end, 48.0);
    assert_eq!(general.size().width, 232.0);
    // picking a section in the list glides the page there (it scrolls: the scroller measures
    // the page's size as the pick arrives, not the size of before its layout)
    privacy[0].invoke_accessible_default_action();
    pump(500);
    settle(&app, &driver);
    assert!(app.global::<PageScroll>().get_y() > 0.0);
    app.global::<PageScroll>().invoke_reset();

    // A page that scrolls: picking the last section glides there (scroller.tsx `to`) on the
    // driver's page scroller, not in one jump.
    let mut long: Vec<SettingDef> = (0..40)
        .map(|i| {
            def(
                Box::leak(format!("a.{i}").into_boxed_str()),
                Section::General,
            )
        })
        .collect();
    long.push(def("b.one", Section::Privacy));
    app.set_settings_rows(table(&long));
    pump(500);
    settle(&app, &driver);
    let page = app.global::<PageScroll>();
    assert_eq!(page.get_y(), 0.0);
    nav_rows(&app, "About")[0].invoke_accessible_default_action();
    pump(16);
    driver.borrow_mut().advance(&app, 0.016);
    let first = page.get_y();
    settle(&app, &driver);
    let there = page.get_y();
    assert!(there > 0.0, "the page moved");
    assert!(
        first > 0.0 && first < there / 2.0,
        "a glide: {first} of {there} after a frame"
    );
}
