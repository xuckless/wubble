//! Slint shell. Generated component types are re-exported from here.

slint::include_modules!();

pub mod controls;
pub mod driver;
pub mod glass;
pub mod pages;
pub mod settings;
pub mod window;

use slint::{Model, VecModel};

/// The generated code carries the element ids `ElementHandle` finds (build.rs): a debug build, or
/// one built with `SLINT_EMIT_DEBUG_INFO=1`.
pub const ELEMENT_IDS: bool = option_env!("WUBBLE_ELEMENT_IDS").is_some();

/// Make `model` hold `rows`, in place: changed rows are rewritten, rows past the old end are
/// added and rows past the new end removed. A Slint `for` keeps an instance per row it did not
/// see removed, so a row's own state (hover, the press, a TouchArea's grab mid-drag, its tweens)
/// survives a re-render; replacing the whole model would rebuild every row.
pub fn sync<T: Clone + PartialEq + 'static>(model: &VecModel<T>, rows: Vec<T>) -> bool {
    sync_by(model, rows, PartialEq::eq)
}

/// [`sync`] with `same` deciding whether a row changed (rows holding wgpu images: see
/// [`same_image`]).
pub fn sync_by<T: Clone + 'static>(
    model: &VecModel<T>,
    rows: Vec<T>,
    same: impl Fn(&T, &T) -> bool,
) -> bool {
    let keep = model.row_count().min(rows.len());
    let mut changed = rows.len() != model.row_count();
    for _ in rows.len()..model.row_count() {
        model.remove(model.row_count() - 1);
    }
    for (i, row) in rows.into_iter().enumerate() {
        if i >= keep {
            model.push(row);
        } else if !model.row_data(i).is_some_and(|old| same(&old, &row)) {
            model.set_row_data(i, row);
            changed = true;
        }
    }
    changed
}

/// Whether two images are the same: Slint 1.19's `Image` equality has no case for wgpu
/// textures (`ImageInner::eq` falls through to `false`), so two clones of one texture compare
/// unequal; here they are the same when they hold the same texture.
pub fn same_image(a: &slint::Image, b: &slint::Image) -> bool {
    a == b
        || matches!(
            (a.to_wgpu_30_texture(), b.to_wgpu_30_texture()),
            (Some(x), Some(y)) if x == y
        )
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

    #[test]
    fn sync_by_keeps_rows_its_test_calls_the_same() {
        let m = VecModel::from(vec![(1, 'a'), (2, 'b')]);
        assert!(sync_by(&m, vec![(1, 'x'), (3, 'y')], |a, b| a.0 == b.0));
        // row 0 is "the same" and keeps its old data; row 1 changed
        assert_eq!(m.iter().collect::<Vec<_>>(), [(1, 'a'), (3, 'y')]);
        // nothing changed: says so (the shell then asks for no frame)
        assert!(!sync_by(&m, vec![(1, 'z'), (3, 'y')], |a, b| a.0 == b.0));
        assert!(sync(&VecModel::from(vec![1]), vec![1, 2]));
        assert!(sync(&VecModel::from(vec![1, 2]), vec![1]));
    }

    #[test]
    fn images_without_a_texture_compare_as_slint_does() {
        let pixels = |v| {
            let mut b = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::new(1, 1);
            b.make_mut_bytes().fill(v);
            slint::Image::from_rgba8(b)
        };
        let a = pixels(1);
        assert!(same_image(&a, &a.clone()));
        assert!(!same_image(&a, &pixels(2)));
        assert!(same_image(
            &slint::Image::default(),
            &slint::Image::default()
        ));
        assert!(!same_image(&a, &slint::Image::default()));
    }

    /// The testing platform can be installed once per process, so every
    /// scenario runs in this single test, on one window.
    #[test]
    fn shell_scenarios() {
        init_integration_test_with_mock_time();
        let app = AppWindow::new().unwrap();
        // the kit menu keeps its highlight and keys in the controls registry
        let _controls = crate::controls::wire(&app);
        // the command bar's machine runs in the driver
        let _driver = crate::driver::wire(&app, wubble_core::seam::Seam::new());
        app.window()
            .set_size(slint::LogicalSize::new(1280.0, 800.0));
        command_bar_lifecycle(&app);
        start_page_bar(&app);
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
            // the tab row menu's look: an icon, a shortcut, a danger row
            icon: if separator { "" } else { "close" }.into(),
            shortcut: if separator { "" } else { "Ctrl+W" }.into(),
            danger: label == "Reload",
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
                icons: true,
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
        assert!(app.get_menu_panel_x() + 236.0 <= width);
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
            remember: "Remember for this site".into(),
            allow: "Allow".into(),
            block: "Block".into(),
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
            none: false,
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

    /// The start page's capsule is the command bar's field (wubble_core::chrome::Placement):
    /// the page takes the keys into it, typing and the results go through the bar's own query
    /// and rows, Ctrl+L keeps the keys there, and the overlay never opens on the page.
    fn start_page_bar(app: &AppWindow) {
        let asked = Rc::new(Cell::new(0));
        app.on_address_focus_requested(count(&asked));
        let queries = Rc::new(std::cell::RefCell::new(Vec::<String>::new()));
        let q = queries.clone();
        app.on_command_query_changed(move |text| q.borrow_mut().push(text.into()));
        let chosen = Rc::new(std::cell::RefCell::new(vec![]));
        let c = chosen.clone();
        app.on_command_result_chosen(move |i| c.borrow_mut().push(i));
        let focused = || app.global::<StartBridge>().get_search_focused();
        let in_page = |state| {
            assert_eq!(app.get_command_placement(), CommandPlacement::InPage);
            assert_eq!(app.get_command_bar_state(), state);
        };
        app.set_command_query("".into());
        app.set_command_results(ModelRc::from(Vec::<CommandResult>::new().as_slice()));

        // A start page shown: its capsule holds the keys, the bar is open in it, for the tab.
        app.set_page(InternalPageKind::Start);
        settle();
        assert!(focused(), "the new start page's capsule takes the keys");
        in_page(CommandBarState::Open);
        assert_eq!(asked.get(), 1, "the host gives the bar the start tab");
        assert!(!app.get_web_focused());

        // Typed keys land in the capsule, as the bar's query.
        press(app, "w");
        assert_eq!(app.get_command_query(), "w");
        assert_eq!(*queries.borrow(), ["w"]);
        let rows: Vec<_> = (0..3).map(|i| result(&format!("s{i}"))).collect();
        app.set_command_results(ModelRc::from(rows.as_slice()));
        settle();

        // Ctrl+L again (the focus-address action): the keys stay in the capsule, the text is
        // selected, and the overlay stays shut.
        app.invoke_request_address_focus();
        settle();
        in_page(CommandBarState::Open);
        assert!(focused());
        press(app, "x");
        assert_eq!(app.get_command_query(), "x", "Ctrl+L selected the text");

        // The arrows move the selection; Enter picks it and the bar goes at once.
        press(app, Key::DownArrow);
        press(app, Key::Return);
        assert_eq!(*chosen.borrow(), [1]);
        assert_eq!(app.get_command_bar_state(), CommandBarState::Closed);
        settle();
        assert!(!focused(), "a pick lets go of the keys");

        // A click on the capsule takes the keys again; Escape clears, then lets go.
        let (x, y) = (
            app.global::<StartBridge>().get_search_x(),
            app.global::<StartBridge>().get_search_y(),
        );
        click(app, x, y);
        settle();
        assert!(focused());
        in_page(CommandBarState::Open);
        press(app, "abc");
        press(app, Key::Escape);
        assert_eq!(app.get_command_query(), "");
        in_page(CommandBarState::Open);
        press(app, Key::Escape);
        settle();
        assert!(!focused());
        assert_eq!(app.get_command_bar_state(), CommandBarState::Closed);

        // The window losing the keyboard (another app in front) is not leaving the capsule:
        // the bar stays open in it, and the keys come back to it.
        app.invoke_request_address_focus();
        settle();
        in_page(CommandBarState::Open);
        app.set_window_active(false);
        app.window()
            .dispatch_event(WindowEvent::WindowActiveChanged(false));
        settle();
        in_page(CommandBarState::Open);
        app.set_window_active(true);
        app.window()
            .dispatch_event(WindowEvent::WindowActiveChanged(true));
        settle();
        in_page(CommandBarState::Open);
        assert!(focused());
        press(app, Key::Escape);
        settle();
        assert_eq!(app.get_command_bar_state(), CommandBarState::Closed);

        // Every other page: Ctrl+L opens the overlay bar as before.
        app.set_page(InternalPageKind::None);
        settle();
        app.invoke_request_address_focus();
        settle();
        assert_eq!(app.get_command_placement(), CommandPlacement::Overlay);
        assert_eq!(app.get_command_bar_state(), CommandBarState::Open);
        press(app, Key::Escape);
        settle();
        assert_eq!(app.get_command_bar_state(), CommandBarState::Closed);
        app.set_command_results(ModelRc::from(Vec::<CommandResult>::new().as_slice()));
        app.set_command_query("".into());
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

        // A pick closes the bar at once (command.tsx `go`: `d.command = null`); a second
        // Enter chooses nothing.
        press(app, "\n");
        assert_eq!(app.get_command_bar_state(), CommandBarState::Closed);
        press(app, "\n");
        assert_eq!(chosen.get(), 1);

        // The opener prefills the query after the open (the host's `open_bar` on the URL): the
        // input selects that text, so typing replaces all of it, whatever it held before.
        app.set_command_query("news".into());
        let w = app.as_weak();
        app.on_address_focus_requested(move || {
            if let Some(w) = w.upgrade() {
                w.set_command_query("https://example.com/".into());
            }
        });
        app.invoke_request_address_focus();
        settle();
        press(app, "x");
        assert_eq!(app.get_command_query(), "x");
        // the same for New tab with a prefilled query (`search_for`: the bar on the text)
        press(app, Key::Escape);
        settle();
        let w = app.as_weak();
        app.on_new_tab_requested(move || {
            if let Some(w) = w.upgrade() {
                w.set_command_query("some words".into());
            }
        });
        app.invoke_request_new_tab();
        settle();
        press(app, "y");
        assert_eq!(app.get_command_query(), "y");
        press(app, Key::Escape);
        settle();
        dismissed.set(0);

        // Escape runs the exit: the dropdown's out duration (176 ms); a second Escape while it
        // closes does not dismiss again.
        app.invoke_request_address_focus();
        settle();
        assert_eq!(app.get_command_bar_state(), CommandBarState::Open);
        // Nine results show seven rows.
        let nine: Vec<_> = (0..9).map(|i| result(&format!("n{i}"))).collect();
        app.set_command_results(ModelRc::from(nine.as_slice()));
        assert_eq!(app.get_command_shown_rows(), 7);
        press(app, Key::Escape);
        assert_eq!(app.get_command_bar_state(), CommandBarState::Closing);
        press(app, Key::Escape);
        assert_eq!(dismissed.get(), 1);
        mock_elapsed_time(Duration::from_millis(170));
        assert_eq!(app.get_command_bar_state(), CommandBarState::Closing);
        mock_elapsed_time(Duration::from_millis(10));
        assert_eq!(app.get_command_bar_state(), CommandBarState::Closed);
    }
}
