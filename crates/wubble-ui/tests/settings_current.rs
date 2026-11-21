//! The settings page's section list names the section the reader is in. The page opens before
//! the host fills its rows (render_settings runs once the page shows): the list starts at
//! General, not at a section that passed the tracking line while the page was empty and whose
//! index then named another. It follows a click on each section and the scroll, and its rows
//! are the page's sections, with and without the Extensions section and while searching.
//! Mock time only; one test (one platform per process).

use std::collections::HashMap;
use std::rc::Rc;
use std::time::{Duration, Instant};

use i_slint_backend_testing::{
    AccessibleRole, ElementHandle, init_integration_test_with_mock_time, mock_elapsed_time,
};
use slint::platform::{PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, LogicalSize, ModelRc, VecModel};
use wubble_core::keys::{Platform, binding_rows};
use wubble_core::pages::{PAGE_LINKS, filter, settings_page};
use wubble_ui::driver::Wired;
use wubble_ui::settings::{binding_items, page_link_items, setting_items};
use wubble_ui::{AppWindow, InternalPageKind};

fn frames(app: &AppWindow, wired: &Wired, n: usize) {
    for _ in 0..n {
        mock_elapsed_time(Duration::from_millis(16));
        wired.frame(app, Instant::now());
    }
}

fn section_rows(app: &AppWindow) -> Vec<ElementHandle> {
    ElementHandle::find_by_element_type_name(app, "NavRow")
        .filter(|h| h.accessible_role() == Some(AccessibleRole::Tab))
        .filter(|h| h.size().height > 0.0)
        .collect()
}

fn current(app: &AppWindow) -> Vec<String> {
    section_rows(app)
        .iter()
        .filter(|h| h.accessible_item_selected() == Some(true))
        .filter_map(|h| h.accessible_label().map(|l| l.to_string()))
        .collect()
}

/// The section rows a reader can pick (a row for a section the search hides is disabled).
fn listed(app: &AppWindow) -> Vec<String> {
    section_rows(app)
        .iter()
        .filter(|h| h.accessible_enabled() != Some(false))
        .filter_map(|h| h.accessible_label().map(|l| l.to_string()))
        .collect()
}

/// The section heads on the page, top to bottom (read in a window tall enough for all of
/// them: the finder skips what the page's scroller clips away).
fn heads(app: &AppWindow, wired: &Wired) -> Vec<String> {
    let size = app.window().size().to_logical(1.0);
    app.window().set_size(LogicalSize::new(size.width, 12000.0));
    frames(app, wired, 30);
    let mut heads: Vec<(f32, String)> =
        ElementHandle::find_by_element_type_name(app, "SectionHeader")
            .filter(|h| h.size().height > 0.0)
            .filter_map(|h| Some((h.absolute_position().y, h.accessible_label()?.to_string())))
            .collect();
    heads.sort_by(|a, b| a.0.total_cmp(&b.0));
    app.window().set_size(size);
    frames(app, wired, 30);
    heads.into_iter().map(|(_, t)| t).collect()
}

fn rows(query: &str) -> ModelRc<wubble_ui::SettingItem> {
    let page = filter(&settings_page(&HashMap::new()), query);
    ModelRc::from(Rc::new(VecModel::from(setting_items(&page))))
}

#[test]
fn the_section_list_names_the_section_in_view() {
    init_integration_test_with_mock_time();
    let app = AppWindow::new().unwrap();
    app.window().set_size(LogicalSize::new(1360.0, 880.0));
    let wired = wubble_ui::driver::wire(&app, wubble_core::seam::Seam::new());
    app.show().unwrap();
    app.set_settings_pages(ModelRc::from(Rc::new(VecModel::from(page_link_items(
        &PAGE_LINKS,
    )))));
    frames(&app, &wired, 10);

    // the page opens empty; its rows come a frame later, as the host sends them
    app.set_page(InternalPageKind::Settings);
    frames(&app, &wired, 1);
    app.set_settings_rows(rows(""));
    app.set_settings_bindings(ModelRc::from(Rc::new(VecModel::from(binding_items(
        &binding_rows(Platform::Linux, &[]),
    )))));
    frames(&app, &wired, 60);
    assert_eq!(current(&app), ["General"], "at open");

    // the rows are the page's sections, without Extensions on an engine without them and with
    for extensions in [false, true] {
        app.set_settings_extensions_available(extensions);
        frames(&app, &wired, 30);
        assert_eq!(listed(&app), heads(&app, &wired), "extensions {extensions}");
        assert_eq!(listed(&app).iter().any(|t| t == "Extensions"), extensions);
        assert_eq!(current(&app), ["General"], "extensions {extensions}");
    }

    // a click on each section makes it the current one; a turn of the wheel then keeps the
    // section in view current (the scroll, not the click, names it)
    for title in listed(&app) {
        let row = section_rows(&app)
            .into_iter()
            .find(|h| h.accessible_label().as_deref() == Some(title.as_str()))
            .unwrap();
        row.mock_single_click(PointerEventButton::Left);
        frames(&app, &wired, 90);
        assert_eq!(
            current(&app),
            std::slice::from_ref(&title),
            "clicked {title}"
        );
        let position = LogicalPosition::new(900.0, 400.0);
        app.window()
            .dispatch_event(WindowEvent::PointerMoved { position });
        app.window().dispatch_event(WindowEvent::PointerScrolled {
            position,
            delta_x: 0.0,
            delta_y: -1.0,
        });
        frames(&app, &wired, 60);
        let now = current(&app);
        // the last sections are short: at the end of the page the list names the last one
        assert!(
            now == std::slice::from_ref(&title) || now == ["About"],
            "scrolled from {title}: {now:?}"
        );
    }

    // while searching: the sections with matches, and only those
    app.set_settings_searching(true);
    app.set_settings_rows(rows("zoom"));
    app.set_settings_bindings(ModelRc::default());
    frames(&app, &wired, 60);
    assert_eq!(listed(&app), heads(&app, &wired), "searching");
    assert!(!listed(&app).is_empty());
}
