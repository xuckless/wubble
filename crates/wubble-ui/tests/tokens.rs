//! The physics in wubble-core carries the design's numbers as Rust constants (as the port
//! does); they must equal the Slint tokens. Chrome is read from the live global, the spring
//! presets from the token files' text (Slint cannot export the `Motion` global: it has easing
//! properties). No timing.

use std::fs;
use std::path::Path;

use i_slint_backend_testing::init_integration_test_with_mock_time;
use slint::ComponentHandle;
use wubble_core::motion::K;
use wubble_core::seam::{INSET, SEAM, SIDEBAR_DEFAULT};
use wubble_core::tabs::{SpaceSwitcher, TABS};
use wubble_ui::{AppWindow, Chrome};

/// The value of `name: <number><unit>;` in a token file.
fn token(file: &str, name: &str) -> f64 {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("ui").join(file);
    let text = fs::read_to_string(path).expect("read token file");
    let key = format!(" {name}: ");
    let at = text
        .find(&key)
        .unwrap_or_else(|| panic!("{name} in {file}"))
        + key.len();
    let value: String = text[at..]
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    value.parse().unwrap_or_else(|_| panic!("{name} = {value}"))
}

#[test]
fn tokens_match_physics() {
    init_integration_test_with_mock_time();
    let app = AppWindow::new().unwrap();
    let chrome = app.global::<Chrome>();
    assert_eq!(f64::from(chrome.get_inset()), INSET);
    assert_eq!(f64::from(chrome.get_sidebarMin()), SEAM::min);
    assert_eq!(f64::from(chrome.get_sidebarMax()), SEAM::max);
    assert_eq!(f64::from(chrome.get_sidebarDefault()), SIDEBAR_DEFAULT);
    assert_eq!(f64::from(chrome.get_rail()), SEAM::rail);
    assert_eq!(
        f64::from(chrome.get_row() + chrome.get_rowGap()),
        TABS::pitch
    );
    assert_eq!(f64::from(app.get_row_pitch()), TABS::pitch);
    assert_eq!(
        token("app-tokens.slint", "space-tile") + token("app-tokens.slint", "space-gap"),
        SpaceSwitcher::PITCH
    );

    for (name, kc) in [
        ("follow", K::follow),
        ("stretch", K::stretch),
        ("window", K::window),
        ("seamHeld", K::seamHeld),
        ("seamRelease", K::seamRelease),
        ("seamBend", K::seamBend),
        ("contents", K::contents),
        ("pop", K::pop),
    ] {
        let k = token("tokens.slint", &format!("{name}K"));
        let c = token("tokens.slint", &format!("{name}C"));
        assert_eq!([k, c], kc, "{name}");
    }
    // A favicon image is handed over at twice the side it is drawn at (wubble_core::favicon),
    // so the renderer's bilinear sample is an exact 2x2 average at scale 1.
    let shell_icon = f64::from(wubble_core::favicon::SHELL_ICON_PX);
    for name in ["icon", "page-favicon"] {
        assert_eq!(2.0 * token("app-tokens.slint", name), shell_icon, "{name}");
    }
}
