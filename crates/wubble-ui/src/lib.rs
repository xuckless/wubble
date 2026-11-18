//! Slint shell. Generated component types are re-exported from here.

slint::include_modules!();

pub mod driver;
pub mod glass;
pub mod pages;
pub mod settings;
pub mod window;

use slint::{Model, VecModel};

/// Make `model` hold `rows`, in place: changed rows are rewritten, rows past the old end are
/// added and rows past the new end removed. A Slint `for` keeps an instance per row it did not
/// see removed, so a row's own state (hover, the press, a TouchArea's grab mid-drag, its tweens)
/// survives a re-render; replacing the whole model would rebuild every row.
pub fn sync<T: Clone + PartialEq + 'static>(model: &VecModel<T>, rows: Vec<T>) {
    let keep = model.row_count().min(rows.len());
    for _ in rows.len()..model.row_count() {
        model.remove(model.row_count() - 1);
    }
    for (i, row) in rows.into_iter().enumerate() {
        if i >= keep {
            model.push(row);
        } else if model.row_data(i).as_ref() != Some(&row) {
            model.set_row_data(i, row);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use i_slint_backend_testing::{init_integration_test_with_mock_time, mock_elapsed_time};
    use slint::{
        Model, ModelRc, SharedString,
        platform::{Key, WindowEvent},
    };
    use std::{cell::Cell, rc::Rc, time::Duration};

    fn result(title: &str) -> CommandResult {
        CommandResult {
            kind: CommandKind::OpenTab,
            title: title.into(),
            subtitle: "".into(),
            shortcut: "".into(),
            icon: Default::default(),
        }
    }

    fn press(app: &AppWindow, text: impl Into<SharedString>) {
        let text = text.into();
        app.window()
            .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
        app.window()
            .dispatch_event(WindowEvent::KeyReleased { text });
    }

    fn settle() {
        mock_elapsed_time(Duration::from_millis(500));
    }

    #[test]
    fn sync_rewrites_only_what_changed() {
        let m = VecModel::from(vec![1, 2, 3]);
        sync(&m, vec![1, 5, 3, 4]);
        assert_eq!(m.iter().collect::<Vec<_>>(), [1, 5, 3, 4]);
        sync(&m, vec![7]);
        assert_eq!(m.iter().collect::<Vec<_>>(), [7]);
        sync(&m, vec![]);
        assert_eq!(m.row_count(), 0);
    }

    /// The testing platform can be installed once per process, so every
    /// scenario runs in this single test, on one window.
    #[test]
    fn shell_scenarios() {
        init_integration_test_with_mock_time();
        let app = AppWindow::new().unwrap();
        app.window()
            .set_size(slint::LogicalSize::new(1280.0, 800.0));
        command_bar_lifecycle(&app);
        page_surfaces(&app);
    }

    fn count(cell: &Rc<Cell<i32>>) -> impl Fn() + 'static {
        let c = cell.clone();
        move || c.set(c.get() + 1)
    }

    fn click(app: &AppWindow, x: f32, y: f32) {
        let position = slint::LogicalPosition::new(x, y);
        let button = slint::platform::PointerEventButton::Left;
        app.window()
            .dispatch_event(WindowEvent::PointerPressed { position, button });
        app.window()
            .dispatch_event(WindowEvent::PointerReleased { position, button });
    }

    fn entry(label: &str, separator: bool) -> MenuEntry {
        MenuEntry {
            label: label.into(),
            enabled: true,
            separator,
        }
    }

    /// The context menu, the dialog sheet, the find bar and page
    /// fullscreen: each reports its intents once and takes the keyboard.
    fn page_surfaces(app: &AppWindow) {
        let chosen = Rc::new(std::cell::RefCell::new(vec![]));
        let c = chosen.clone();
        app.on_menu_chosen(move |i| c.borrow_mut().push(i));
        let dismissed = Rc::new(Cell::new(0));
        app.on_menu_dismissed(count(&dismissed));
        let page_presses = Rc::new(Cell::new(0));
        let p = page_presses.clone();
        app.on_web_pointer_pressed(move |_, _, _, _| p.set(p.get() + 1));

        // ── Menu: arrow keys skip the divider, Enter chooses, Escape and an
        // outside click dismiss.
        let entries = [
            entry("Back", false),
            entry("", true),
            entry("Reload", false),
        ];
        let open_menu = |x: f32| {
            app.set_menu(MenuView {
                open: true,
                x,
                y: 40.0,
                entries: ModelRc::from(entries.as_slice()),
            });
            settle();
        };
        open_menu(40.0);
        assert!(!app.get_web_focused(), "the menu takes the keyboard");
        press(app, Key::DownArrow);
        press(app, Key::DownArrow);
        press(app, Key::Return);
        assert_eq!(*chosen.borrow(), vec![2], "the divider is skipped");
        press(app, Key::Escape);
        assert_eq!(dismissed.get(), 1);
        // A click outside the panel is the menu's, not the page's.
        click(app, 1200.0, 700.0);
        assert_eq!((dismissed.get(), page_presses.get()), (2, 0));
        // Near the right edge the panel opens to the pointer's left.
        let width = app.window().size().width as f32 / app.window().scale_factor();
        open_menu(width - 40.0);
        assert!(app.get_menu_panel_x() + 220.0 <= width);
        app.set_menu(MenuView::default());
        settle();
        assert!(
            app.get_web_focused(),
            "closing gives the keyboard back to the page"
        );

        // ── Dialog: Enter accepts, Escape cancels, a prompt answers its text,
        // the scrim keeps the page from the pointer.
        let answers = Rc::new(std::cell::RefCell::new(vec![]));
        let a = answers.clone();
        app.on_dialog_answered(move |accept, text, secret| {
            a.borrow_mut()
                .push((accept, text.to_string(), secret.to_string()))
        });
        let dialog = |prompt: bool, serial: i32| DialogView {
            open: true,
            title: "a.test says".into(),
            message: "hello".into(),
            accept: "OK".into(),
            cancel: "Cancel".into(),
            prompt,
            secret: false,
            input: "seed".into(),
            serial,
        };
        app.set_dialog(dialog(false, 1));
        settle();
        assert!(!app.get_web_focused());
        click(
            app,
            app.get_content_x() + 20.0,
            app.get_content_y() + app.get_content_height() - 20.0,
        );
        assert_eq!(page_presses.get(), 0, "the scrim blocks the page");
        press(app, Key::Return);
        press(app, Key::Escape);
        app.set_dialog(dialog(true, 2));
        settle();
        // The prompt's field holds the keyboard with the seed selected.
        press(app, "x");
        press(app, Key::Return);
        // Sign-in: Enter in the user name moves to the password, Enter there signs in.
        app.set_dialog(DialogView {
            secret: true,
            input: "".into(),
            ..dialog(true, 3)
        });
        settle();
        press(app, "u");
        press(app, Key::Return);
        press(app, "p");
        press(app, Key::Return);
        let s = String::new;
        assert_eq!(
            *answers.borrow(),
            vec![
                (true, s(), s()),
                (false, s(), s()),
                (true, "x".to_string(), s()),
                (true, "u".to_string(), "p".to_string())
            ]
        );
        app.set_dialog(DialogView::default());
        settle();
        assert!(app.get_web_focused());

        // ── Error panel: takes the keyboard, Enter is the primary button.
        let primaries = Rc::new(Cell::new(0));
        app.on_error_primary(count(&primaries));
        app.set_page_error(ErrorView {
            open: true,
            title: "Your connection is not private".into(),
            primary: "Go back".into(),
            secondary: "Proceed to a.test (unsafe)".into(),
            cert: true,
            ..Default::default()
        });
        settle();
        assert!(!app.get_web_focused());
        press(app, Key::Return);
        assert_eq!(primaries.get(), 1);
        app.set_page_error(ErrorView::default());
        settle();
        assert!(app.get_web_focused());

        // ── Permission prompt: never takes the keyboard, so no keystroke meant
        // for the page answers it (keyjacking); only its buttons do.
        let permits = Rc::new(std::cell::RefCell::new(vec![]));
        let p = permits.clone();
        app.on_permission_answered(move |allow, remember| p.borrow_mut().push((allow, remember)));
        app.set_permission(PermissionView {
            open: true,
            title: "a.test wants to".into(),
            lines: ModelRc::new(slint::VecModel::from(vec![SharedString::from(
                "Use your camera",
            )])),
            serial: 4,
        });
        settle();
        assert!(app.get_web_focused());
        press(app, Key::Return);
        press(app, Key::Escape);
        assert!(permits.borrow().is_empty());
        app.set_permission(PermissionView::default());
        settle();
        assert!(app.get_web_focused());

        // ── Find bar: typing searches, Enter / Shift+Enter step, Escape closes.
        let edits = Rc::new(std::cell::RefCell::new(vec![]));
        let e = edits.clone();
        app.on_find_edited(move |q| e.borrow_mut().push(q.to_string()));
        let steps = Rc::new(std::cell::RefCell::new(vec![]));
        let s = steps.clone();
        app.on_find_step(move |forward| s.borrow_mut().push(forward));
        let closed = Rc::new(Cell::new(0));
        app.on_find_closed(count(&closed));
        app.set_find(FindView {
            open: true,
            query: "".into(),
            counter: "".into(),
            focus_serial: 1,
        });
        settle();
        press(app, "n");
        press(app, Key::Return);
        app.window().dispatch_event(WindowEvent::KeyPressed {
            text: Key::Shift.into(),
        });
        press(app, Key::Return);
        app.window().dispatch_event(WindowEvent::KeyReleased {
            text: Key::Shift.into(),
        });
        press(app, Key::Escape);
        assert_eq!(*edits.borrow(), vec!["n".to_string()]);
        assert_eq!(*steps.borrow(), vec![true, false]);
        assert_eq!(closed.get(), 1);
        app.set_find(FindView::default());
        settle();
        assert!(app.get_web_focused());

        // ── Page fullscreen: the page fills the window.
        assert!(app.get_content_x() > 0.0);
        app.set_page_fullscreen(true);
        assert_eq!((app.get_content_x(), app.get_content_y()), (0.0, 0.0));
        assert_eq!(app.get_content_width(), width);
        app.set_page_fullscreen(false);
        assert!(app.get_content_x() > 0.0);
    }

    fn command_bar_lifecycle(app: &AppWindow) {
        let rows: Vec<_> = (0..3).map(|i| result(&format!("r{i}"))).collect();
        app.set_command_results(ModelRc::from(rows.as_slice()));
        let chosen = Rc::new(Cell::new(0));
        let dismissed = Rc::new(Cell::new(0));
        let c = chosen.clone();
        app.on_command_result_chosen(move |_| c.set(c.get() + 1));
        let d = dismissed.clone();
        app.on_command_dismissed(move || d.set(d.get() + 1));
        app.show().unwrap();
        settle();
        assert_eq!(app.get_command_bar_state(), CommandBarState::Closed);

        // Open, growing from the new-tab row.
        app.invoke_request_new_tab();
        assert!(app.get_command_origin().x > 0.0);
        settle();
        assert_eq!(app.get_command_bar_state(), CommandBarState::Open);
        assert_eq!(app.get_command_results().row_count(), 3);
        assert_eq!(app.get_command_shown_rows(), 3);

        // A second Enter while closing must not choose again.
        press(app, "\n");
        assert_eq!(app.get_command_bar_state(), CommandBarState::Closing);
        press(app, "\n");
        assert_eq!(chosen.get(), 1);
        // The close timer is the dropdown's out duration (176 ms).
        mock_elapsed_time(Duration::from_millis(170));
        assert_eq!(app.get_command_bar_state(), CommandBarState::Closing);
        mock_elapsed_time(Duration::from_millis(10));
        assert_eq!(app.get_command_bar_state(), CommandBarState::Closed);
        assert_eq!(app.get_command_bar_state(), CommandBarState::Closed);

        // A second Escape while closing must not dismiss again.
        app.invoke_request_address_focus();
        settle();
        assert_eq!(app.get_command_bar_state(), CommandBarState::Open);
        // Nine results show seven rows.
        let nine: Vec<_> = (0..9).map(|i| result(&format!("n{i}"))).collect();
        app.set_command_results(ModelRc::from(nine.as_slice()));
        assert_eq!(app.get_command_shown_rows(), 7);
        press(app, Key::Escape);
        press(app, Key::Escape);
        assert_eq!(dismissed.get(), 1);
        settle();
        assert_eq!(app.get_command_bar_state(), CommandBarState::Closed);
    }
}
