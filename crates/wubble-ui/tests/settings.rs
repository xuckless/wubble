//! The settings page on the testing backend: generated from a table, every kind of setting gets
//! its control with no code of its own, a control reports the new stored text, the search box
//! reports its query and a shortcut row arms the recorder. Mock time only; one test, because the
//! platform can be installed once per process.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use i_slint_backend_testing::{
    ElementHandle, init_integration_test_with_mock_time, mock_elapsed_time,
};
use slint::platform::{Key, WindowEvent};
use slint::{ComponentHandle, LogicalSize, Model, ModelRc, VecModel};
use wubble_core::keys::{Platform, binding_rows};
use wubble_core::pages::{settings_page, settings_page_from};
use wubble_core::settings::{ALL, Section, SettingDef, SettingKind};
use wubble_ui::settings::{binding_items, setting_items};
use wubble_ui::{AppWindow, InternalPageKind, SettingItem};

fn pump() {
    mock_elapsed_time(Duration::from_millis(500));
}

const fn def(key: &'static str, kind: SettingKind, default: &'static str) -> SettingDef {
    SettingDef {
        key,
        kind,
        default,
        label: key,
        section: Section::General,
        restart: false,
    }
}

/// One setting of every kind, none of them shipped: the page needs no code for any of them.
const DEFS: [SettingDef; 8] = [
    def("t.bool", SettingKind::Bool, "1"),
    def("t.count", SettingKind::Count { min: 0, max: 9 }, "3"),
    // a range too wide for a slider: the number is typed
    def(
        "t.wide",
        SettingKind::Count {
            min: 10,
            max: 86_400,
        },
        "300",
    ),
    def("t.float", SettingKind::Float { min: 0.0, max: 1.0 }, "0.5"),
    def("t.engine", SettingKind::SearchEngine, ""),
    def("t.choice", SettingKind::Choice(&["x", "y"]), "x"),
    def("t.list", SettingKind::List, "[\"a.test\"]"),
    def("t.map", SettingKind::Map, "{\"b.test\":125}"),
];

fn rows(items: Vec<SettingItem>) -> ModelRc<SettingItem> {
    ModelRc::from(Rc::new(VecModel::from(items)))
}

fn count(app: &AppWindow, id: &str) -> usize {
    ElementHandle::find_by_element_id(app, id).count()
}

#[test]
fn settings_page_scenarios() {
    init_integration_test_with_mock_time();
    let app = AppWindow::new().unwrap();
    // Tall enough for every row of the page: the element queries see only what is on screen.
    app.window().set_size(LogicalSize::new(1280.0, 14000.0));
    // the kit controls' machines (the engine's Select is the kit menu)
    let _wired = wubble_ui::driver::wire(&app, wubble_core::seam::Seam::new());
    app.set_page(InternalPageKind::Settings);

    // The shipped table: a row per visible setting.
    let page = settings_page(&HashMap::new());
    // rows shown only for another row's value (`SHOWN_WHEN`) are not, at the defaults
    let shown = ALL
        .iter()
        .filter(|d| d.section != Section::Hidden)
        .filter(|d| {
            !wubble_core::settings::SHOWN_WHEN
                .iter()
                .any(|(row, ..)| *row == d.key)
        })
        .count();
    app.set_settings_rows(rows(setting_items(&page)));
    pump();
    assert_eq!(app.get_settings_rows().row_count(), shown);
    assert_eq!(count(&app, "SettingsPage::setting-row"), shown);
    // A whole-number row's slider moves by one, so every number of its range can be set
    // (the default zoom's 100 % among them).
    let count_sliders: Vec<ElementHandle> =
        ElementHandle::find_by_element_id(&app, "SettingRow::stepper")
            .flat_map(|row| {
                row.query_descendants()
                    .match_accessible_role(i_slint_backend_testing::AccessibleRole::Slider)
                    .find_all()
            })
            .collect();
    assert!(!count_sliders.is_empty(), "the table has whole-number rows");
    for slider in &count_sliders {
        assert_eq!(slider.accessible_value_step(), Some(1.0));
    }

    // A table of every kind: each row shows its control.
    let defs: Vec<&SettingDef> = DEFS.iter().collect();
    app.set_settings_rows(rows(setting_items(&settings_page_from(
        &defs,
        &HashMap::new(),
    ))));
    pump();
    for (id, n) in [
        ("SettingRow::toggle", 1),
        ("SettingRow::stepper", 2),
        ("SettingRow::slider", 1),
        // the engine picks from a Select, as the design's; the other choice is segmented
        ("SettingRow::segmented", 1),
        ("SettingRow::select", 1),
        // the list, the map, and the built-in engines under the engine's Select
        ("SettingRow::entries", 3),
    ] {
        assert_eq!(count(&app, id), n, "{id}");
    }

    // A control reports the setting's key and its new stored text.
    let changed = Rc::new(RefCell::new(vec![]));
    let c = changed.clone();
    app.on_setting_changed(move |key, value| {
        c.borrow_mut().push((key.to_string(), value.to_string()))
    });
    let toggle = ElementHandle::find_by_element_id(&app, "SettingRow::toggle")
        .next()
        .unwrap();
    toggle.invoke_accessible_default_action();
    pump();
    let y = ElementHandle::find_by_accessible_label(&app, "Y")
        .find(|h| h.accessible_role() == Some(i_slint_backend_testing::AccessibleRole::Tab))
        .unwrap();
    y.invoke_accessible_default_action();
    pump();
    // the engine's Select opens its menu, whose row picks the engine; none is chosen yet, so
    // the first one (shown in the trigger) is a pick too
    let engine = ElementHandle::find_by_accessible_label(&app, "t.engine")
        .find(|h| h.accessible_role() == Some(i_slint_backend_testing::AccessibleRole::Combobox))
        .unwrap();
    engine.invoke_accessible_default_action();
    pump();
    let google = ElementHandle::find_by_accessible_label(&app, "Google")
        .find(|h| h.accessible_role() == Some(i_slint_backend_testing::AccessibleRole::ListItem))
        .unwrap();
    google.invoke_accessible_default_action();
    pump();
    // An empty number is refused, not written as the range's minimum; a typed one is clamped.
    let wide = ElementHandle::find_by_element_id(&app, "Stepper::number")
        .next()
        .unwrap()
        .query_descendants()
        .match_accessible_role(i_slint_backend_testing::AccessibleRole::TextInput)
        .find_first()
        .unwrap();
    wide.mock_single_click(slint::platform::PointerEventButton::Left);
    for text in ["", "-", "999999"] {
        wide.set_accessible_value(text);
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
        pump();
    }
    assert_eq!(
        *changed.borrow(),
        [
            ("t.bool".to_string(), "0".to_string()),
            ("t.choice".to_string(), "y".to_string()),
            ("t.engine".to_string(), "google".to_string()),
            ("t.wide".to_string(), "86400".to_string()),
        ]
    );

    // The search box reports its query.
    let queries = Rc::new(RefCell::new(vec![]));
    let q = queries.clone();
    app.on_settings_query_changed(move |text| q.borrow_mut().push(text.to_string()));
    let search = ElementHandle::find_by_accessible_label(&app, "Search settings")
        .find(|h| h.accessible_role() == Some(i_slint_backend_testing::AccessibleRole::TextInput))
        .unwrap();
    search.set_accessible_value("block");
    pump();
    assert_eq!(*queries.borrow(), ["block"]);

    // A shortcut row arms the recorder for its action.
    app.set_settings_bindings(ModelRc::from(Rc::new(VecModel::from(binding_items(
        &binding_rows(Platform::Linux, &[]),
    )))));
    pump();
    let recorded = Rc::new(RefCell::new(vec![]));
    let r = recorded.clone();
    app.on_binding_record(move |id| r.borrow_mut().push(id.to_string()));
    let row = ElementHandle::find_by_accessible_label(&app, "Settings")
        .find(|h| h.accessible_role() == Some(i_slint_backend_testing::AccessibleRole::Button))
        .unwrap();
    row.invoke_accessible_default_action();
    pump();
    assert_eq!(*recorded.borrow(), ["open-settings"]);

    // The built-in pages: a nav link each, after the sections, that runs the page's own action;
    // a search that names pages shows their redirect rows, counted after the settings.
    use wubble_core::pages::{PAGE_LINKS, page_links_matching};
    use wubble_ui::settings::page_link_items;
    let actions = Rc::new(RefCell::new(vec![]));
    let a = actions.clone();
    app.on_shell_action(move |id| a.borrow_mut().push(id.to_string()));
    app.set_settings_pages(ModelRc::from(Rc::new(VecModel::from(page_link_items(
        &PAGE_LINKS,
    )))));
    pump();
    let of_type = |label: &str, ty: &str| -> Vec<ElementHandle> {
        ElementHandle::find_by_accessible_label(&app, label)
            .filter(|h| h.type_name().as_deref() == Some(ty))
            .collect()
    };
    for link in &PAGE_LINKS {
        let rows = of_type(link.title(), "NavRow");
        assert_eq!(rows.len(), 1, "{}", link.title());
        assert_eq!(
            rows[0].accessible_role(),
            Some(i_slint_backend_testing::AccessibleRole::Button)
        );
    }
    let about = ElementHandle::find_by_accessible_label(&app, "About")
        .find(|h| h.type_name().as_deref() == Some("NavRow"))
        .unwrap();
    let history = &of_type("History", "NavRow")[0];
    assert!(
        history.absolute_position().y > about.absolute_position().y,
        "the pages come after the sections"
    );
    history.invoke_accessible_default_action();
    pump();
    assert_eq!(*actions.borrow(), ["open-history"]);
    assert!(
        of_type("Archive", "Row").is_empty(),
        "no redirect row unsearched"
    );
    app.set_settings_searching(true);
    app.set_settings_page_matches(ModelRc::from(Rc::new(VecModel::from(page_link_items(
        page_links_matching("tabs"),
    )))));
    pump();
    let redirect = of_type("Archive", "Row");
    assert_eq!(redirect.len(), 1);
    assert_eq!(
        redirect[0].accessible_description().as_deref(),
        Some(PAGE_LINKS[4].note)
    );
    assert_eq!(of_type("Task manager", "Row").len(), 1);
    let n = app.get_settings_rows().row_count() + app.get_settings_bindings().row_count();
    let line = format!("{n} settings, 2 pages");
    assert_eq!(
        ElementHandle::find_by_accessible_label(&app, &line).count(),
        1,
        "the count line says {line:?}"
    );
    redirect[0].invoke_accessible_default_action();
    pump();
    assert_eq!(*actions.borrow(), ["open-history", "open-archive"]);
}
