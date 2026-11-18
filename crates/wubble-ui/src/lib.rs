//! Slint shell. Generated component types are re-exported from here.

slint::include_modules!();

pub mod driver;
pub mod glass;
pub mod window;

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

    /// The testing platform can be installed once per process, so every
    /// scenario runs in this single test, on one window.
    #[test]
    fn command_bar_lifecycle() {
        init_integration_test_with_mock_time();
        let app = AppWindow::new().unwrap();
        let rows: Vec<_> = (0..3).map(|i| result(&format!("r{i}"))).collect();
        app.set_command_results(ModelRc::from(rows.as_slice()));
        let chosen = Rc::new(Cell::new(0));
        let dismissed = Rc::new(Cell::new(0));
        let c = chosen.clone();
        app.on_command_result_chosen(move |_| c.set(c.get() + 1));
        let d = dismissed.clone();
        app.on_command_dismissed(move || d.set(d.get() + 1));
        app.show().unwrap();
        assert_eq!(app.get_command_bar_state(), CommandBarState::Closed);

        // Open, growing from the new-tab row.
        app.invoke_request_new_tab();
        assert!(app.get_command_origin().x > 0.0);
        settle();
        assert_eq!(app.get_command_bar_state(), CommandBarState::Open);
        assert_eq!(app.get_command_results().row_count(), 3);
        assert_eq!(app.get_command_shown_rows(), 3);

        // A second Enter while closing must not choose again.
        press(&app, "\n");
        assert_eq!(app.get_command_bar_state(), CommandBarState::Closing);
        press(&app, "\n");
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
        press(&app, Key::Escape);
        press(&app, Key::Escape);
        assert_eq!(dismissed.get(), 1);
        settle();
        assert_eq!(app.get_command_bar_state(), CommandBarState::Closed);
    }
}
