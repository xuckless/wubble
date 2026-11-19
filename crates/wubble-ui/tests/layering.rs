//! Where the scene's overlay images sit in Slint's tree (`app.slint`): each layer's glass is
//! drawn under what must show on it. Slint draws children in source order, so the order of
//! the elements in the file is the order on screen.

const APP: &str = include_str!("../ui/app.slint");

fn at(needle: &str) -> usize {
    APP.find(needle)
        .unwrap_or_else(|| panic!("{needle:?} not in app.slint"))
}

#[test]
fn the_peek_glass_is_under_the_peek_panel_and_the_sidebar() {
    // the peek panel (its key) and the sidebar's content after it
    let panel = at("key: GlassKeys.peek;");
    let sidebar = at("sidebar := Sidebar {");
    let glass = at("source: root.over-peek;");
    // the scene's glass of the peek is opaque inside the panel: drawn after them, it hides them
    assert!(
        glass < panel && glass < sidebar,
        "the over-peek image is drawn over the peek panel or the sidebar"
    );
}
