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
const DEFS: [SettingDef; 7] = [
    def("t.bool", SettingKind::Bool, "1"),
    def("t.count", SettingKind::Count { min: 0, max: 9 }, "3"),
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
    app.window().set_size(LogicalSize::new(1280.0, 6000.0));
    app.set_page(InternalPageKind::Settings);

    // The shipped table: a row per visible setting.
    let page = settings_page(&HashMap::new());
    let shown = ALL.iter().filter(|d| d.section != Section::Hidden).count();
    app.set_settings_rows(rows(setting_items(&page)));
    pump();
    assert_eq!(app.get_settings_rows().row_count(), shown);
    assert_eq!(count(&app, "SettingsPage::setting-row"), shown);

    // A table of every kind: each row shows its control.
    let defs: Vec<&SettingDef> = DEFS.iter().collect();
    app.set_settings_rows(rows(setting_items(&settings_page_from(
        &defs,
        &HashMap::new(),
    ))));
    pump();
    for (id, n) in [
        ("SettingRow::toggle", 1),
        ("SettingRow::stepper", 1),
        ("SettingRow::slider", 1),
        ("SettingRow::segmented", 2),
        ("SettingRow::entries", 2),
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
        .next()
        .unwrap();
    y.invoke_accessible_default_action();
    pump();
    let duck = ElementHandle::find_by_accessible_label(&app, "DuckDuckGo")
        .next()
        .unwrap();
    duck.invoke_accessible_default_action();
    pump();
    assert_eq!(
        *changed.borrow(),
        [
            ("t.bool".to_string(), "0".to_string()),
            ("t.choice".to_string(), "y".to_string()),
            ("t.engine".to_string(), "duckduckgo".to_string()),
        ]
    );

    // The search box reports its query.
    let queries = Rc::new(RefCell::new(vec![]));
    let q = queries.clone();
    app.on_settings_query_changed(move |text| q.borrow_mut().push(text.to_string()));
    let search = ElementHandle::find_by_element_id(&app, "Field::input")
        .next()
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
}
