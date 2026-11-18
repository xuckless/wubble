//! The tab list on the testing backend: the driver follows the models, a click
//! activates, a middle click closes, a drag reports a reorder, the wheel over a
//! row scrolls the list. Mock time and explicit frame deltas only; one test,
//! because the platform can be installed once per process.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use i_slint_backend_testing::{init_integration_test_with_mock_time, mock_elapsed_time};
use slint::{
    ComponentHandle, LogicalPosition, LogicalSize, Model, ModelRc, VecModel,
    platform::{PointerEventButton, WindowEvent},
};
use wubble_core::tabs::TABS;
use wubble_ui::driver::{self, Driver};
use wubble_ui::{AppWindow, TabItem};

const DT: f32 = 0.016;
/// Top of the first row with no pinned tabs and no Spaces: the inset (8), the
/// sidebar padding (6), the window-controls row (30), the address pill (36) and a group gap (12)
/// under each.
const LIST_TOP: f32 = 8.0 + 6.0 + 30.0 + 12.0 + 36.0 + 12.0;
/// A point on a row's title, clear of its close button.
const ROW_X: f32 = 100.0;

type Shared = Rc<RefCell<Driver>>;

fn settle(app: &AppWindow, driver: &Shared) {
    for _ in 0..4000 {
        if !driver.borrow_mut().advance(app, DT) {
            return;
        }
    }
    panic!("springs never settled");
}

fn pump() {
    mock_elapsed_time(Duration::from_millis(1));
}

fn row_centre(i: usize) -> f32 {
    LIST_TOP + i as f32 * TABS::pitch as f32 + 18.0
}

fn dispatch(app: &AppWindow, event: WindowEvent) {
    app.window().dispatch_event(event);
    pump();
}

fn moved(app: &AppWindow, x: f32, y: f32) {
    dispatch(
        app,
        WindowEvent::PointerMoved {
            position: LogicalPosition::new(x, y),
        },
    );
}

fn press(app: &AppWindow, x: f32, y: f32, button: PointerEventButton) {
    moved(app, x, y);
    let position = LogicalPosition::new(x, y);
    dispatch(app, WindowEvent::PointerPressed { position, button });
}

fn release(app: &AppWindow, x: f32, y: f32, button: PointerEventButton) {
    let position = LogicalPosition::new(x, y);
    dispatch(app, WindowEvent::PointerReleased { position, button });
}

fn tabs(ids: impl IntoIterator<Item = i32>) -> ModelRc<TabItem> {
    let items: Vec<TabItem> = ids
        .into_iter()
        .map(|id| TabItem {
            id,
            title: format!("tab {id}").into(),
            url: "".into(),
            favicon: Default::default(),
            ..Default::default()
        })
        .collect();
    ModelRc::from(items.as_slice())
}

fn rows(app: &AppWindow) -> Vec<f32> {
    app.get_row_y().iter().collect()
}

fn model_changed_syncs_rows(app: &AppWindow, driver: &Shared) {
    app.set_tabs(tabs([10, 11, 12]));
    app.set_active_tab_id(10);
    pump();
    settle(app, driver);
    assert_eq!(app.get_selected_row(), 0);
    assert_eq!(rows(app), [0.0, 40.0, 80.0]);
    assert_eq!(app.get_indicator_y(), 0.0);

    app.set_active_tab_id(12);
    pump();
    settle(app, driver);
    assert_eq!(app.get_selected_row(), 2);
    assert_eq!(app.get_indicator_y(), 2.0 * TABS::pitch as f32);

    // The active tab is not a row (pinned): no indicator.
    app.set_active_tab_id(99);
    pump();
    assert_eq!(app.get_selected_row(), -1);
    app.set_active_tab_id(10);
    pump();
    settle(app, driver);
}

fn tab_click_activates(app: &AppWindow, driver: &Shared, log: &Log) {
    let y = row_centre(1);
    press(app, ROW_X, y, PointerEventButton::Left);
    release(app, ROW_X, y, PointerEventButton::Left);
    assert_eq!(log.borrow().activated, [11]);
    settle(app, driver);
}

fn tab_close_middle_click(app: &AppWindow, driver: &Shared, log: &Log) {
    let y = row_centre(2);
    press(app, ROW_X, y, PointerEventButton::Middle);
    release(app, ROW_X, y, PointerEventButton::Middle);
    assert_eq!(log.borrow().closed, [12]);
    // Neither a pick nor a drag.
    assert_eq!(log.borrow().activated, [11]);
    settle(app, driver);
}

fn tab_drag_emits_tab_moved(app: &AppWindow, driver: &Shared, log: &Log) {
    let y = row_centre(0);
    press(app, ROW_X, y, PointerEventButton::Left);
    // Past the 6 px threshold, then two slots down in steps.
    for k in 1..=17 {
        moved(app, ROW_X, y + k as f32 * 5.0);
        driver.borrow_mut().advance(app, DT);
    }
    release(app, ROW_X, y + 85.0, PointerEventButton::Left);
    assert_eq!(log.borrow().moved, [(10, 2)]);
    assert_eq!(log.borrow().activated, [11], "a drag is not a click");
    // The host has not reordered the model: the rows spring back to its order.
    settle(app, driver);
    assert_eq!(rows(app), [0.0, 40.0, 80.0]);
}

/// Drag row 0 two slots down; halfway, the host re-renders the rows (a title changed) with
/// `rerender`. Returns how far the dragged row has followed the pointer just before the release.
fn drag_across_rerender(app: &AppWindow, driver: &Shared, rerender: impl Fn(Vec<TabItem>)) -> f32 {
    let y = row_centre(0);
    press(app, ROW_X, y, PointerEventButton::Left);
    for k in 1..=17 {
        if k == 8 {
            let mut items: Vec<TabItem> = app.get_tabs().iter().collect();
            items[1].title = format!("retitled {k}").into();
            rerender(items);
            app.invoke_shell_model_changed();
            pump();
        }
        moved(app, ROW_X, y + k as f32 * 5.0);
        driver.borrow_mut().advance(app, DT);
    }
    for _ in 0..60 {
        driver.borrow_mut().advance(app, DT);
    }
    let followed = rows(app)[0];
    release(app, ROW_X, y + 85.0, PointerEventButton::Left);
    settle(app, driver);
    followed
}

/// A re-render that replaces the model rebuilds every row, and the dragged row's TouchArea
/// with its grab: the row stops following the pointer. One synced in place (`wubble_ui::sync`)
/// keeps the row and the drag.
fn drag_survives_an_in_place_rerender(app: &AppWindow, driver: &Shared) {
    let model = Rc::new(VecModel::from(
        tabs([10, 11, 12]).iter().collect::<Vec<_>>(),
    ));
    app.set_tabs(ModelRc::from(model.clone()));
    pump();
    settle(app, driver);
    let m = model.clone();
    let kept = drag_across_rerender(app, driver, move |items| wubble_ui::sync(&m, items));
    let replaced = drag_across_rerender(app, driver, |items| {
        app.set_tabs(ModelRc::from(items.as_slice()))
    });
    assert!(
        kept > 70.0,
        "the row follows the pointer to the end: {kept}"
    );
    assert!(replaced < 50.0, "the rebuilt row lost the drag: {replaced}");
    app.set_tabs(tabs([10, 11, 12]));
    pump();
    settle(app, driver);
}

fn tab_scroll_reaches_scroller(app: &AppWindow, driver: &Shared) {
    app.set_tabs(tabs(0..30));
    pump();
    settle(app, driver);
    assert!(app.get_scroll_content() > app.get_scroll_viewport());
    assert_eq!(app.get_scroll_y(), 0.0);
    // Over a row: the row has no wheel handler, so the scroller under it takes it.
    let position = LogicalPosition::new(ROW_X, row_centre(1));
    moved(app, position.x, position.y);
    dispatch(
        app,
        WindowEvent::PointerScrolled {
            position,
            delta_x: 0.0,
            delta_y: -120.0,
        },
    );
    settle(app, driver);
    assert_eq!(app.get_scroll_y(), 120.0);
}

#[derive(Default)]
struct Calls {
    activated: Vec<i32>,
    closed: Vec<i32>,
    moved: Vec<(i32, i32)>,
}

type Log = Rc<RefCell<Calls>>;

/// Touch sizing: one flag (`Shell.touch`) makes rows 44 px and the springs carry the rows to the
/// new pitch; turning it off brings them back.
fn touch_sizing_resizes_rows(app: &AppWindow, driver: &Shared) {
    app.set_tabs(tabs([1, 2, 3]));
    app.invoke_shell_model_changed();
    settle(app, driver);
    let shell = app.global::<wubble_ui::Shell>();
    assert_eq!(shell.get_row(), 36.0);
    shell.set_touch(true);
    assert_eq!(shell.get_row(), 44.0);
    settle(app, driver);
    assert_eq!(rows(app), vec![0.0, 48.0, 96.0]);
    shell.set_touch(false);
    settle(app, driver);
    assert_eq!(rows(app), vec![0.0, 40.0, 80.0]);
}

#[test]
fn tab_list_scenarios() {
    init_integration_test_with_mock_time();
    let app = AppWindow::new().unwrap();
    app.show().unwrap();
    app.window().set_size(LogicalSize::new(1280.0, 800.0));
    let log = Log::default();
    let l = log.clone();
    app.on_tab_activated(move |id| l.borrow_mut().activated.push(id));
    let l = log.clone();
    app.on_tab_closed(move |id| l.borrow_mut().closed.push(id));
    let l = log.clone();
    app.on_tab_moved(move |id, to| l.borrow_mut().moved.push((id, to)));
    let driver = driver::wire(&app, wubble_core::seam::Seam::new()).driver;
    model_changed_syncs_rows(&app, &driver);
    tab_click_activates(&app, &driver, &log);
    tab_close_middle_click(&app, &driver, &log);
    tab_drag_emits_tab_moved(&app, &driver, &log);
    drag_survives_an_in_place_rerender(&app, &driver);
    tab_scroll_reaches_scroller(&app, &driver);
    touch_sizing_resizes_rows(&app, &driver);
}
