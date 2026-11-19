//! The springy controls on the testing backend: the settings page's toggle and slider take a
//! slot in `Controls`, draw its view, and a pointer tap on the toggle reports the setting
//! flipped while its knob springs across. Mock time only; one test (one platform per process).

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::{Duration, Instant};

use i_slint_backend_testing::{
    ElementHandle, init_integration_test_with_mock_time, mock_elapsed_time,
};
use slint::platform::{PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, LogicalSize, Model, ModelRc, VecModel};
use wubble_core::pages::settings_page_from;
use wubble_core::settings::{Section, SettingDef, SettingKind};
use wubble_ui::settings::setting_items;
use wubble_ui::{AppWindow, Controls, InternalPageKind, driver};

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

const DEFS: [SettingDef; 3] = [
    def("t.bool", SettingKind::Bool, "1"),
    def("t.float", SettingKind::Float { min: 0.0, max: 1.0 }, "0.5"),
    def(
        "t.choice",
        SettingKind::Choice(&["one", "two", "three"]),
        "one",
    ),
];

#[test]
fn toggle_and_slider_draw_their_springs() {
    init_integration_test_with_mock_time();
    let app = AppWindow::new().unwrap();
    app.window().set_size(LogicalSize::new(1280.0, 2000.0));
    let wired = driver::wire(&app, wubble_core::seam::Seam::new());
    app.set_page(InternalPageKind::Settings);
    let defs: Vec<&SettingDef> = DEFS.iter().collect();
    let rows = Rc::new(VecModel::from(setting_items(&settings_page_from(
        &defs,
        &HashMap::new(),
    ))));
    app.set_settings_rows(ModelRc::from(rows.clone()));
    mock_elapsed_time(Duration::from_millis(100));

    let views = app.global::<Controls>().get_views();
    assert!(views.row_count() >= 2, "toggle and slider took slots");
    let on_toggle = views.iter().find(|v| v.value == 1.0 && v.fill == 1.0);
    assert!(
        on_toggle.is_some_and(|v| v.x == 40.0),
        "an on toggle rests at the far end"
    );

    let changed = Rc::new(RefCell::new(vec![]));
    let c = changed.clone();
    app.on_setting_changed(move |key, value| {
        c.borrow_mut().push((key.to_string(), value.to_string()))
    });
    let toggle = ElementHandle::find_by_element_id(&app, "SettingRow::toggle")
        .next()
        .unwrap();
    let (p, s) = (toggle.absolute_position(), toggle.size());
    let at = LogicalPosition::new(p.x + s.width / 2.0, p.y + s.height / 2.0);
    let button = PointerEventButton::Left;
    app.window().dispatch_event(WindowEvent::PointerPressed {
        position: at,
        button,
    });
    app.window().dispatch_event(WindowEvent::PointerReleased {
        position: at,
        button,
    });
    mock_elapsed_time(Duration::from_millis(16));
    assert_eq!(*changed.borrow(), [("t.bool".to_string(), "0".to_string())]);

    // the knob springs to the off side over frames
    let mut now = Instant::now();
    let mut frames = 0;
    while wired.frame(&app, now).0 && frames < 2000 {
        now += Duration::from_millis(8);
        frames += 1;
    }
    assert!(frames > 1, "it moved over several frames");
    let views = app.global::<Controls>().get_views();
    assert!(
        views
            .iter()
            .any(|v| v.value == 0.0 && v.x == 0.0 && v.fill == 0.0)
    );

    // the segmented control (kind 2): its indicator sits on the chosen segment; a tap on the
    // last one chooses it, and the indicator springs there
    let seg_view = |app: &AppWindow| {
        app.global::<Controls>()
            .get_views()
            .iter()
            .find(|v| v.w > 0.0)
            .expect("the segmented control took a slot and reported its segments")
    };
    let before = seg_view(&app);
    assert_eq!(before.value, 0.0);
    let seg = ElementHandle::find_by_element_id(&app, "SettingRow::segmented")
        .next()
        .unwrap();
    let (p, s) = (seg.absolute_position(), seg.size());
    let at = LogicalPosition::new(p.x + s.width - 12.0, p.y + s.height / 2.0);
    app.window().dispatch_event(WindowEvent::PointerPressed {
        position: at,
        button,
    });
    app.window().dispatch_event(WindowEvent::PointerReleased {
        position: at,
        button,
    });
    mock_elapsed_time(Duration::from_millis(16));
    assert_eq!(
        changed.borrow().last().cloned(),
        Some(("t.choice".to_string(), "three".to_string()))
    );
    // the control is controlled (the design's `onChange`): the indicator goes where the
    // setting goes, so write it back as the host does
    let stored: HashMap<String, String> = changed.borrow().iter().cloned().collect();
    for (i, row) in setting_items(&settings_page_from(&defs, &stored))
        .into_iter()
        .enumerate()
    {
        rows.set_row_data(i, row);
    }
    mock_elapsed_time(Duration::from_millis(16));
    let mut frames = 0;
    while wired.frame(&app, now).0 && frames < 4000 {
        now += Duration::from_millis(8);
        frames += 1;
    }
    let after = seg_view(&app);
    assert_eq!(after.value, 2.0);
    assert!(after.x > before.x + 40.0, "{} -> {}", before.x, after.x);
}
