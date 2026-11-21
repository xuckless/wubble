//! The native pages beside settings on the testing backend: each page shows a row per line of
//! the model it is given, a click and a middle-click on a row report the address differently, a
//! folder's click toggles it, the downloads page shares the popover's row, a locked passwords
//! page lists nothing, and the address pill's star and badge report their clicks. Mock time
//! only; one test, because the platform can be installed once per process.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use i_slint_backend_testing::{
    ElementHandle, init_integration_test_with_mock_time, mock_elapsed_time,
};
use slint::platform::{PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, LogicalSize, ModelRc, VecModel};
use wubble_ui::{
    AppWindow, BookmarkLineItem, DownloadItem, DownloadStatus, HistoryLineItem, InternalPageKind,
    LoginItem, PageScroll, PasswordsView, TaskItem, VaultState,
};

fn pump() {
    mock_elapsed_time(Duration::from_millis(500));
}

fn model<T: Clone + 'static>(items: Vec<T>) -> ModelRc<T> {
    ModelRc::from(Rc::new(VecModel::from(items)))
}

fn count(app: &AppWindow, id: &str) -> usize {
    ElementHandle::find_by_element_id(app, id).count()
}

/// Presses and releases `button` at the middle of the first element called `id`.
fn click(app: &AppWindow, id: &str, button: PointerEventButton) {
    let el = ElementHandle::find_by_element_id(app, id)
        .next()
        .unwrap_or_else(|| panic!("no element {id}"));
    let (p, s) = (el.absolute_position(), el.size());
    let position = LogicalPosition::new(p.x + s.width / 2.0, p.y + s.height / 2.0);
    let w = app.window();
    w.dispatch_event(WindowEvent::PointerMoved { position });
    w.dispatch_event(WindowEvent::PointerPressed { position, button });
    w.dispatch_event(WindowEvent::PointerReleased { position, button });
    pump();
}

fn visit(title: &str, url: &str) -> HistoryLineItem {
    HistoryLineItem {
        header: false,
        key: 0,
        label: Default::default(),
        title: title.into(),
        url: url.into(),
        time: Default::default(),
        favicon: Default::default(),
    }
}

fn bookmark(id: i32, depth: i32, folder: bool, url: &str) -> BookmarkLineItem {
    BookmarkLineItem {
        id,
        depth,
        title: format!("b{id}").into(),
        url: url.into(),
        path: Default::default(),
        favicon: Default::default(),
        is_folder: folder,
        expanded: false,
    }
}

fn download(id: i32, status: DownloadStatus) -> DownloadItem {
    DownloadItem {
        id,
        name: format!("f{id}").into(),
        detail: "".into(),
        progress: 0.5,
        status,
    }
}

fn login(id: &str) -> LoginItem {
    LoginItem {
        id: id.into(),
        site: "a.test".into(),
        origin: "https://a.test".into(),
        username: "me".into(),
        readable: true,
        revealed: Default::default(),
        has_revealed: false,
    }
}

#[test]
fn page_scenarios() {
    init_integration_test_with_mock_time();
    let app = AppWindow::new().unwrap();
    app.window().set_size(LogicalSize::new(1280.0, 4000.0));

    // History: a header is no row; a click and a middle-click report the row's address.
    app.set_page(InternalPageKind::History);
    let header = HistoryLineItem {
        header: true,
        key: -1,
        label: "Today".into(),
        title: Default::default(),
        url: Default::default(),
        time: Default::default(),
        favicon: Default::default(),
    };
    app.set_history_lines(model(vec![
        header,
        visit("A", "https://a.test/"),
        visit("B", "https://b.test/"),
    ]));
    pump();
    assert_eq!(count(&app, "HistoryPage::visit"), 2);
    let opened = Rc::new(RefCell::new(vec![]));
    let (o1, o2) = (opened.clone(), opened.clone());
    app.on_page_opened(move |u| o1.borrow_mut().push(format!("here {u}")));
    app.on_page_opened_new_tab(move |u| o2.borrow_mut().push(format!("tab {u}")));
    click(&app, "ListRow::ta", PointerEventButton::Left);
    click(&app, "ListRow::ta", PointerEventButton::Middle);
    assert_eq!(
        *opened.borrow(),
        ["here https://a.test/", "tab https://a.test/"]
    );
    app.set_history_lines(model(vec![]));
    pump();
    assert_eq!(count(&app, "HistoryPage::visit"), 0);

    // Bookmarks: a folder's click toggles it, a link's click opens it.
    app.set_page(InternalPageKind::Bookmarks);
    app.set_bookmark_lines(model(vec![
        bookmark(1, 0, true, ""),
        bookmark(2, 1, false, "https://l.test/"),
    ]));
    pump();
    assert_eq!(count(&app, "BookmarksPage::bookmark-row"), 2);
    let toggled = Rc::new(RefCell::new(vec![]));
    let t = toggled.clone();
    app.on_bookmark_toggled(move |id| t.borrow_mut().push(id));
    click(&app, "ListRow::ta", PointerEventButton::Left);
    assert_eq!(*toggled.borrow(), [1]);

    // Downloads: one row per item of the filtered page model.
    app.set_page(InternalPageKind::Downloads);
    app.set_downloads_page_items(model(vec![
        download(1, DownloadStatus::Running),
        download(2, DownloadStatus::Complete),
        download(3, DownloadStatus::Failed),
    ]));
    pump();
    assert_eq!(count(&app, "DownloadRow::row-touch"), 3);

    // Passwords: nothing is listed while locked; rows show once unlocked.
    app.set_page(InternalPageKind::Passwords);
    let view = |state| PasswordsView {
        state,
        title: "t".into(),
        body: Default::default(),
        notice: Default::default(),
    };
    app.set_passwords_view(view(VaultState::Locked));
    app.set_logins(model(vec![]));
    pump();
    assert_eq!(count(&app, "PasswordsPage::login-row"), 0);
    app.set_passwords_view(view(VaultState::Unlocked));
    app.set_logins(model(vec![login("1"), login("2")]));
    pump();
    assert_eq!(count(&app, "PasswordsPage::login-row"), 2);

    // The task manager: its column starts where the other list pages' does (the table grows to
    // the right), its End button is a row's size, and a long table scrolls.
    app.window().set_size(LogicalSize::new(1280.0, 700.0));
    let tasks: Vec<TaskItem> = (0..40)
        .map(|i| TaskItem {
            tab: if i < 30 { i } else { -1 },
            pid: 100 + i,
            title: format!("row {i}").into(),
            state: "Background".into(),
            memory: if i < 30 { "".into() } else { "12.0 MB".into() },
            cpu: if i < 30 { "".into() } else { "0.0 %".into() },
            busy: "".into(),
            never_sleep: false,
            can_sleep: i < 30,
            can_discard: i < 30,
            can_end: true,
        })
        .collect();
    app.set_tasks(model(tasks));
    // the task manager's title starts where the history page's does
    let heading = |app: &AppWindow| {
        ElementHandle::find_by_element_id(app, "PageFrame::heading")
            .map(|e| e.absolute_position().x)
            .collect::<Vec<_>>()
    };
    app.set_page(InternalPageKind::History);
    pump();
    let history = heading(&app);
    app.set_page(InternalPageKind::Tasks);
    pump();
    let tasks = heading(&app);
    assert!(
        history.len() == 1 && history == tasks,
        "{history:?} {tasks:?}"
    );
    let page = app.global::<PageScroll>();
    assert!(
        page.get_content() > page.get_viewport() && page.get_viewport() > 0.0,
        "the table scrolls: {} in {}",
        page.get_content(),
        page.get_viewport()
    );
    app.window().set_size(LogicalSize::new(1280.0, 4000.0));

    // The address pill: the star and the badge only when asked, and each reports its click.
    app.set_page(InternalPageKind::None);
    app.set_address_host("a.test".into());
    pump();
    assert_eq!(count(&app, "AddressPill::star"), 0);
    assert_eq!(count(&app, "AddressPill::badge"), 0);
    app.set_address_bookmarkable(true);
    app.set_address_blocker_visible(true);
    app.set_address_blocked(3);
    pump();
    assert_eq!(count(&app, "AddressPill::star"), 1);
    assert_eq!(count(&app, "AddressPill::badge"), 1);
    let seen = Rc::new(RefCell::new(vec![]));
    let (s1, s2) = (seen.clone(), seen.clone());
    app.on_address_bookmark_requested(move || s1.borrow_mut().push("star"));
    app.on_address_blocker_requested(move |_, _| s2.borrow_mut().push("badge"));
    click(&app, "AddressPill::star", PointerEventButton::Left);
    click(&app, "AddressPill::badge", PointerEventButton::Left);
    assert_eq!(*seen.borrow(), ["star", "badge"]);
}
