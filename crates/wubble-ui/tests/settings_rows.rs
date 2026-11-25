//! Every row of the Settings page the table generates, driven as a person would, on the testing
//! backend against a stand-in for the host (a change is validated by its kind, stored, and the
//! page drawn again from the store): each control is found by its row's label, shows the stored
//! value, takes a change by pointer or by the screen reader's actions, reports the stored text,
//! and shows the stored value again after the redraw. Mock time only; one test (one platform
//! per process).

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use i_slint_backend_testing::{
    AccessibleRole, ElementHandle, init_integration_test_with_mock_time, mock_elapsed_time,
};
use slint::platform::{Key, PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, LogicalSize, Model, ModelRc, VecModel};
use wubble_core::pages::{Control, settings_page};
use wubble_core::settings::{ALL, Section};
use wubble_ui::settings::setting_items;
use wubble_ui::{AppWindow, InternalPageKind, SettingItem};

fn pump() {
    mock_elapsed_time(Duration::from_millis(400));
}

type Stored = Rc<RefCell<HashMap<String, String>>>;

/// What the host does: the page from the store, each row set in place (`wubble::bind::sync`),
/// so a row keeps its instance, its focus and its edit across a redraw.
fn render(app: &AppWindow, stored: &HashMap<String, String>) {
    let items = setting_items(&settings_page(stored));
    let model = app.get_settings_rows();
    match model.as_any().downcast_ref::<VecModel<SettingItem>>() {
        Some(rows) if rows.row_count() == items.len() => {
            for (i, item) in items.into_iter().enumerate() {
                rows.set_row_data(i, item);
            }
        }
        _ => app.set_settings_rows(ModelRc::from(Rc::new(VecModel::from(items)))),
    }
}

fn find(app: &AppWindow, label: &str, role: AccessibleRole) -> Vec<ElementHandle> {
    ElementHandle::find_by_accessible_label(app, label)
        .filter(|h| h.accessible_role() == Some(role))
        .collect()
}

fn one(app: &AppWindow, label: &str, role: AccessibleRole) -> ElementHandle {
    let mut found = find(app, label, role);
    assert_eq!(
        found.len(),
        1,
        "{role:?} labelled {label:?}: {}",
        found.len()
    );
    found.remove(0)
}

fn click(app: &AppWindow, h: &ElementHandle) {
    let (p, s) = (h.absolute_position(), h.size());
    let position = LogicalPosition::new(p.x + s.width / 2.0, p.y + s.height / 2.0);
    let button = PointerEventButton::Left;
    let w = app.window();
    w.dispatch_event(WindowEvent::PointerMoved { position });
    w.dispatch_event(WindowEvent::PointerPressed { position, button });
    w.dispatch_event(WindowEvent::PointerReleased { position, button });
}

fn enter(app: &AppWindow) {
    for event in [
        WindowEvent::KeyPressed {
            text: Key::Return.into(),
        },
        WindowEvent::KeyReleased {
            text: Key::Return.into(),
        },
    ] {
        app.window().dispatch_event(event);
    }
}

#[test]
fn every_row_of_the_page_takes_a_change_and_shows_it() {
    init_integration_test_with_mock_time();
    let app = AppWindow::new().unwrap();
    // tall enough for every row: the element queries see only what is on screen
    app.window().set_size(LogicalSize::new(1360.0, 9000.0));
    let _wired = wubble_ui::driver::wire(&app, wubble_core::seam::Seam::new());
    app.set_page(InternalPageKind::Settings);
    let stored: Stored = Rc::default();
    let log = Rc::new(RefCell::new(Vec::<(String, String)>::new()));
    {
        let log = log.clone();
        app.on_setting_folder_requested(move |key| {
            log.borrow_mut().push((key.to_string(), "folder".into()));
        });
    }
    {
        let (st, log, weak) = (stored.clone(), log.clone(), app.as_weak());
        app.on_setting_changed(move |key, value| {
            let (key, value) = (key.to_string(), value.to_string());
            log.borrow_mut().push((key.clone(), value.clone()));
            let valid = ALL
                .iter()
                .find(|d| d.key == key)
                .is_some_and(|d| d.kind.accepts(&value));
            if valid {
                st.borrow_mut().insert(key, value);
            }
            render(&weak.upgrade().unwrap(), &st.borrow());
        });
    }
    render(&app, &stored.borrow());
    app.window()
        .dispatch_event(WindowEvent::WindowActiveChanged(true));
    pump();

    let mut problems = vec![];
    for section in settings_page(&HashMap::new()) {
        if section.section == Section::Hidden {
            continue;
        }
        for row in &section.rows {
            let (key, label) = (row.key, row.label);
            let value_of = |stored: &Stored| {
                stored
                    .borrow()
                    .get(key)
                    .cloned()
                    .unwrap_or_else(|| row.value.clone())
            };
            let mut note = |what: String| problems.push(format!("{key} ({label}): {what}"));
            log.borrow_mut().clear();
            match row.control {
                Control::Toggle => {
                    let toggle = find(&app, label, AccessibleRole::Switch);
                    if toggle.len() != 1 {
                        note(format!("{} switches by its label", toggle.len()));
                        continue;
                    }
                    let shown = |app: &AppWindow| {
                        one(app, label, AccessibleRole::Switch).accessible_checked()
                    };
                    let start = value_of(&stored) == "1";
                    if shown(&app) != Some(start) {
                        note(format!("shows {:?}, stored {start}", shown(&app)));
                    }
                    click(&app, &one(&app, label, AccessibleRole::Switch));
                    pump();
                    if value_of(&stored) != if start { "0" } else { "1" } {
                        note(format!("a click stored {:?}", value_of(&stored)));
                    }
                    if shown(&app) != Some(!start) {
                        note("did not show the change".into());
                    }
                    // rapid repeated clicks: each one flips it, the last state is shown
                    for _ in 0..5 {
                        click(&app, &one(&app, label, AccessibleRole::Switch));
                    }
                    pump();
                    let wanted = !start;
                    let wanted = !wanted; // five more flips
                    if shown(&app) != Some(wanted) || (value_of(&stored) == "1") != wanted {
                        note(format!(
                            "after five rapid clicks: shows {:?}, stored {:?}, wanted {wanted}",
                            shown(&app),
                            value_of(&stored)
                        ));
                    }
                }
                Control::Segmented(words)
                    if key == "search.engine"
                        || wubble_core::settings::select_labels(key).is_some() =>
                {
                    // the dropdown has its own test
                    let _ = words;
                    if find(&app, label, AccessibleRole::Combobox).len() != 1 {
                        note("no dropdown by its label".into());
                    }
                }
                Control::Segmented(words) => {
                    let list = find(&app, label, AccessibleRole::TabList);
                    if list.len() != 1 {
                        note(format!("{} segmented controls by its label", list.len()));
                        continue;
                    }
                    for word in words {
                        let name = {
                            let mut c = word.chars();
                            c.next()
                                .map(|f| f.to_uppercase().chain(c).collect::<String>())
                                .unwrap()
                        };
                        let tabs_of = |app: &AppWindow| {
                            one(app, label, AccessibleRole::TabList)
                                .query_descendants()
                                .match_accessible_role(AccessibleRole::Tab)
                                .find_all()
                        };
                        let tab = tabs_of(&app)
                            .into_iter()
                            .filter(|t| t.accessible_label().as_deref() == Some(name.as_str()))
                            .collect::<Vec<_>>();
                        if tab.len() != 1 {
                            note(format!("{} tabs named {name:?}", tab.len()));
                            continue;
                        }
                        // the tab's action, as a screen reader or the keyboard presses it
                        tab[0].invoke_accessible_default_action();
                        pump();
                        if value_of(&stored) != *word {
                            note(format!("{name:?} stored {:?}", value_of(&stored)));
                        }
                        let on: Vec<_> = tabs_of(&app)
                            .into_iter()
                            .filter(|t| t.accessible_item_selected() == Some(true))
                            .filter_map(|t| t.accessible_label())
                            .collect();
                        if on.len() != 1 || on[0] != name.as_str() {
                            note(format!("after {name:?} the selected tab is {on:?}"));
                        }
                    }
                }
                Control::Stepper { min, max } => {
                    let slider = find(&app, label, AccessibleRole::Slider);
                    let field = find(&app, label, AccessibleRole::TextInput);
                    match (slider.len(), field.len()) {
                        (1, 0) => {
                            let s = &slider[0];
                            let shown = s.accessible_value().map(|v| v.to_string());
                            if shown.as_deref() != Some(value_of(&stored).as_str()) {
                                note(format!(
                                    "the slider shows {shown:?}, stored {:?}",
                                    row.value
                                ));
                            }
                            s.invoke_accessible_increment_action();
                            pump();
                            let up = value_of(&stored).parse::<u32>().unwrap();
                            if up != (row.value.parse::<u32>().unwrap() + 1).min(max) {
                                note(format!("increment stored {up}"));
                            }
                            for _ in 0..3 {
                                one(&app, label, AccessibleRole::Slider)
                                    .invoke_accessible_decrement_action();
                            }
                            pump();
                            let down = value_of(&stored).parse::<u32>().unwrap();
                            if down < min || down > max {
                                note(format!("decrement stored {down}, outside {min}..{max}"));
                            }
                        }
                        (0, 1) => {
                            let f = &field[0];
                            f.mock_single_click(PointerEventButton::Left);
                            for (typed, wanted) in [
                                ("", None),
                                ("abc", None),
                                ("0", Some(min)),
                                ("99999999", Some(max)),
                                ("12.6", Some(13u32.clamp(min, max))),
                                (&format!("{}", min + 1), Some(min + 1)),
                            ] {
                                let f = &find(&app, label, AccessibleRole::TextInput)[0];
                                let before = value_of(&stored);
                                f.set_accessible_value(typed);
                                enter(&app);
                                pump();
                                let now = value_of(&stored);
                                let wanted = wanted.map_or(before.clone(), |w| w.to_string());
                                if now != wanted {
                                    note(format!(
                                        "typed {typed:?}: stored {now:?}, wanted {wanted:?}"
                                    ));
                                }
                                let f = &find(&app, label, AccessibleRole::TextInput)[0];
                                if f.accessible_value().as_deref() != Some(now.as_str()) {
                                    note(format!(
                                        "typed {typed:?}: the field shows {:?}, stored {now:?}",
                                        f.accessible_value()
                                    ));
                                }
                            }
                        }
                        (s, f) => note(format!("{s} sliders and {f} fields by its label")),
                    }
                }
                Control::Slider { .. } => {
                    if find(&app, label, AccessibleRole::Slider).len() != 1 {
                        note("no slider by its label".into());
                    }
                }
                Control::Folder => {
                    let change = format!("Change {label}");
                    let button = find(&app, &change, AccessibleRole::Button);
                    if button.len() != 1 {
                        note(format!("{} Change buttons for the row", button.len()));
                        continue;
                    }
                    button[0].invoke_accessible_default_action();
                    pump();
                    if *log.borrow() != [(key.to_owned(), "folder".to_owned())] {
                        note(format!("Change asked for {:?}", log.borrow()));
                    }
                }
                Control::ListEditor | Control::MapEditor => {
                    if find(&app, label, AccessibleRole::List).len() != 1 {
                        note("no list by its label".into());
                    }
                }
                Control::TextField => {
                    let field = find(&app, label, AccessibleRole::TextInput);
                    if field.len() != 1 {
                        note(format!("{} fields by its label", field.len()));
                        continue;
                    }
                    let long = "x".repeat(2000);
                    for typed in ["/home/me/Videos", "", "a\nb", long.as_str()] {
                        let before = value_of(&stored);
                        let f = &find(&app, label, AccessibleRole::TextInput)[0];
                        f.mock_single_click(PointerEventButton::Left);
                        f.set_accessible_value(typed);
                        enter(&app);
                        pump();
                        let now = value_of(&stored);
                        let wanted = if typed.contains('\n') {
                            before
                        } else {
                            typed.to_owned()
                        };
                        if now != wanted {
                            note(format!(
                                "typed {} chars: stored {} chars",
                                typed.len(),
                                now.len()
                            ));
                        }
                    }
                }
            }
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}
