//! Where the scene's overlay images sit in Slint's tree (`app.slint`): each layer's glass is
//! drawn under what must show on it. Slint draws children in source order, so the order of
//! the elements in the file is the order on screen.

const APP: &str = include_str!("../ui/app.slint");

fn at(needle: &str) -> usize {
    APP.find(needle)
        .unwrap_or_else(|| panic!("{needle:?} not in app.slint"))
}

const SIDE_PEEK: &str = include_str!("../ui/kit/chrome/sidebar.slint");

#[test]
fn the_peek_glass_is_under_the_peek_panel_and_the_sidebar() {
    // in the kit's SidePeek: the scene's glass, then the peek panel (its key), then the children
    let find = |needle: &str| {
        SIDE_PEEK
            .find(needle)
            .unwrap_or_else(|| panic!("{needle:?} not in kit/chrome/sidebar.slint"))
    };
    let glass = find("source: root.over-peek;");
    let panel = find("key: GlassKeys.peek;");
    let children = find("@children");
    // the scene's glass of the peek is opaque inside the panel: drawn after them, it hides them
    assert!(
        glass < panel && panel < children,
        "the over-peek image is drawn over the peek panel or the sidebar"
    );
    // and the sidebar is SidePeek's child in app.slint
    let zone = at("zone := SidePeek {");
    let sidebar = at("sidebar := Sidebar {");
    assert!(zone < sidebar, "the sidebar sits in the peek's zone");
}
