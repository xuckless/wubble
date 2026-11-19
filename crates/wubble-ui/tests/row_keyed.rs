//! A list row's stateful buttons stay with the item they act on. The host rewrites rows in place
//! by index (`set_row_data`, removing from the end), so a Slint row instance can show another
//! item mid-hold: the task manager's End must not finish on the process that moved into the row,
//! and after a hold-delete the next login, now in that row, must not show "Deleted" or refuse a
//! hold. A rewrite with the same item keeps the hold (the task sample runs every second). The
//! downloads popover's rows keep its pitch, the page's rows their own height. Mock time and
//! stepped frames only; one test (one platform per process).

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use i_slint_backend_testing::{
    AccessibleRole, ElementHandle, init_integration_test_with_mock_time, mock_elapsed_time,
};
use slint::platform::{PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, LogicalSize, Model, ModelRc, VecModel};
use wubble_ui::driver::{self, Wired};
use wubble_ui::{
    AppWindow, DownloadItem, DownloadStatus, InternalPageKind, LoginItem, PasswordsView,
    TaskAction, TaskItem, VaultState,
};

/// Steps the frames and the clock together for `ms`.
fn run(app: &AppWindow, wired: &Wired, now: &mut Instant, ms: u64) {
    for _ in 0..ms / 8 {
        *now += Duration::from_millis(8);
        mock_elapsed_time(Duration::from_millis(8));
        wired.frame(app, *now);
    }
}

/// The buttons whose accessible label is `label` (not the label texts inside them).
fn buttons(app: &AppWindow, label: &str) -> impl Iterator<Item = ElementHandle> {
    ElementHandle::find_by_accessible_label(app, label)
        .filter(|e| e.accessible_role() == Some(AccessibleRole::Button))
}

/// The middle of the first button labelled `label`.
fn at(app: &AppWindow, label: &str) -> LogicalPosition {
    let el = buttons(app, label)
        .next()
        .unwrap_or_else(|| panic!("nothing labelled {label}"));
    let (p, s) = (el.absolute_position(), el.size());
    LogicalPosition::new(p.x + s.width / 2.0, p.y + s.height / 2.0)
}

fn press(app: &AppWindow, position: LogicalPosition) {
    let w = app.window();
    w.dispatch_event(WindowEvent::PointerMoved { position });
    w.dispatch_event(WindowEvent::PointerPressed {
        position,
        button: PointerEventButton::Left,
    });
}

fn release(app: &AppWindow, position: LogicalPosition) {
    app.window().dispatch_event(WindowEvent::PointerReleased {
        position,
        button: PointerEventButton::Left,
    });
}

fn labelled(app: &AppWindow, label: &str) -> usize {
    buttons(app, label).count()
}

fn task(pid: i32, memory: &str) -> TaskItem {
    TaskItem {
        tab: -1,
        pid,
        title: format!("p{pid}").into(),
        state: Default::default(),
        memory: memory.into(),
        cpu: Default::default(),
        busy: Default::default(),
        never_sleep: false,
        can_discard: false,
        can_end: true,
    }
}

fn login(id: &str) -> LoginItem {
    LoginItem {
        id: id.into(),
        site: format!("{id}.test").into(),
        username: "me".into(),
        revealed: Default::default(),
        has_revealed: false,
    }
}

#[test]
fn row_buttons_follow_their_item() {
    init_integration_test_with_mock_time();
    let app = AppWindow::new().unwrap();
    app.window().set_size(LogicalSize::new(1280.0, 2000.0));
    let wired = driver::wire(&app, wubble_core::seam::Seam::new());
    let mut now = Instant::now();
    app.show().unwrap();

    // Task manager: a hold on row 0's End, and the sample moves another process into row 0.
    app.set_page(InternalPageKind::Tasks);
    let tasks = Rc::new(VecModel::from(vec![task(100, "1"), task(200, "1")]));
    app.set_tasks(ModelRc::from(tasks.clone()));
    let ended = Rc::new(RefCell::new(vec![]));
    let e = ended.clone();
    app.on_task_action(move |_, pid, a| {
        if a == TaskAction::End {
            e.borrow_mut().push(pid)
        }
    });
    run(&app, &wired, &mut now, 500);
    let end = at(&app, "End");
    press(&app, end);
    run(&app, &wired, &mut now, 400);
    tasks.set_row_data(0, task(300, "1"));
    run(&app, &wired, &mut now, 2000);
    release(&app, end);
    run(&app, &wired, &mut now, 100);
    assert!(
        ended.borrow().is_empty(),
        "a hold started on 100 ends nothing once 300 took its row: {:?}",
        ended.borrow()
    );

    // A sample that keeps the process in its row (new numbers) keeps the hold.
    press(&app, end);
    run(&app, &wired, &mut now, 400);
    tasks.set_row_data(0, task(300, "2"));
    run(&app, &wired, &mut now, 1200);
    release(&app, end);
    run(&app, &wired, &mut now, 100);
    assert_eq!(*ended.borrow(), [300]);

    // Passwords: hold-delete A; the host rewrites the rows by index, B moves into row 0.
    app.set_page(InternalPageKind::Passwords);
    app.set_passwords_view(PasswordsView {
        state: VaultState::Unlocked,
        title: "t".into(),
        body: Default::default(),
        notice: Default::default(),
    });
    let logins = Rc::new(VecModel::from(vec![login("a"), login("b"), login("c")]));
    app.set_logins(ModelRc::from(logins.clone()));
    let deleted = Rc::new(RefCell::new(vec![]));
    let d = deleted.clone();
    app.on_login_deleted(move |id| d.borrow_mut().push(id.to_string()));
    run(&app, &wired, &mut now, 500);
    let delete = at(&app, "Delete");
    press(&app, delete);
    run(&app, &wired, &mut now, 1500);
    release(&app, delete);
    run(&app, &wired, &mut now, 100);
    assert_eq!(*deleted.borrow(), ["a"]);
    assert_eq!(labelled(&app, "Deleted"), 1, "the finished hold says so");
    logins.set_row_data(0, login("b"));
    logins.set_row_data(1, login("c"));
    logins.remove(2);
    run(&app, &wired, &mut now, 100);
    assert_eq!(
        labelled(&app, "Deleted"),
        0,
        "b, now in a's row, is not shown deleted"
    );
    let delete = at(&app, "Delete");
    press(&app, delete);
    run(&app, &wired, &mut now, 1500);
    release(&app, delete);
    run(&app, &wired, &mut now, 100);
    assert_eq!(
        *deleted.borrow(),
        ["a", "b"],
        "b's row takes a hold at once"
    );

    // Downloads: the popover's row keeps its pitch, the page's row its own height (a note
    // makes it taller than the pitch).
    app.set_page(InternalPageKind::Downloads);
    app.set_downloads(ModelRc::from(Rc::new(VecModel::from(vec![DownloadItem {
        id: 1,
        name: "f".into(),
        detail: "1 MB".into(),
        progress: 0.5,
        status: DownloadStatus::Running,
    }]))));
    app.set_downloads_open(true);
    run(&app, &wired, &mut now, 500);
    let mut heights: Vec<f32> = ElementHandle::find_by_element_id(&app, "DownloadRow::row-touch")
        .map(|e| e.size().height)
        .collect();
    heights.sort_by(f32::total_cmp);
    assert_eq!(
        heights.len(),
        2,
        "a popover row and a page row: {heights:?}"
    );
    assert_eq!(heights[0], 60.0, "the popover's pitch: {heights:?}");
    assert!(heights[1] > 60.0, "the page row with a note: {heights:?}");
}
