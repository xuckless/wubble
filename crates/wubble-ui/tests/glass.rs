//! The Glass component's backdrop logic, through the `GlassBackdrops` global
//! on the testing backend. Pure functions of properties; no timing.

use i_slint_backend_testing::init_integration_test_with_mock_time;
use slint::{ComponentHandle, Image, Rgba8Pixel, SharedPixelBuffer};
use wubble_ui::{AppWindow, GlassBackdrops, GlassTier};

fn backdrop(width: u32, height: u32) -> Image {
    Image::from_rgba8(SharedPixelBuffer::<Rgba8Pixel>::new(width, height))
}

/// The testing platform can be installed once per process, so the scenarios
/// run in order inside one test.
#[test]
fn glass_backdrop_selection_and_clip() {
    init_integration_test_with_mock_time();
    let app = AppWindow::new().unwrap();
    let glass = app.global::<GlassBackdrops>();

    // Each tier picks its own image.
    glass.set_thin(backdrop(10, 10));
    glass.set_regular(backdrop(20, 20));
    glass.set_thick(backdrop(30, 30));
    for (tier, side) in [
        (GlassTier::Thin, 10),
        (GlassTier::Regular, 20),
        (GlassTier::Thick, 30),
    ] {
        assert_eq!(glass.invoke_image(tier).size().width, side);
    }

    // A 1000x600 frame at x=268, y=8 (logical), backdrop at half size.
    glass.set_frame_x(268.0);
    glass.set_frame_y(8.0);
    glass.set_scale(0.5);
    let shows = |x, y, w, h| glass.invoke_shows(GlassTier::Regular, x, y, w, h);

    // An empty image (Flat: the host sets none) never shows a backdrop.
    glass.set_regular(Image::default());
    assert!(!shows(368.0, 58.0, 640.0, 300.0));
    assert!(!shows(268.0, 8.0, 0.0, 0.0));

    glass.set_regular(backdrop(500, 300));
    assert!(shows(368.0, 58.0, 640.0, 300.0));
    // Over the frame's edge or outside it: flat.
    assert!(!shows(200.0, 58.0, 100.0, 100.0));
    assert!(!shows(368.0, 0.0, 100.0, 100.0));
    assert!(!shows(900.0, 58.0, 400.0, 100.0));
    assert!(shows(268.0, 8.0, 1000.0, 600.0));
    // Flush with the right and bottom edges at odd offsets: the offset and the
    // size would each round up (50.5 + 449.5), yet the panel is inside.
    assert!(shows(268.0 + 101.0, 8.0 + 101.0, 899.0, 499.0));
    assert!(!shows(268.0 + 101.0, 8.0, 899.5, 100.0));
    // Clip end minus clip start stays inside the image: 500 - 51.
    assert_eq!(
        glass.invoke_to_backdrop(101.0 + 899.0) - glass.invoke_to_backdrop(101.0),
        449
    );

    // Source clip of a panel at (368, 58): 50 backdrop px in, 320 wide.
    assert_eq!(glass.invoke_to_backdrop(368.0 - 268.0), 50);
    assert_eq!(glass.invoke_to_backdrop(58.0 - 8.0), 25);
    assert_eq!(glass.invoke_to_backdrop(640.0), 320);
    // Scale folds in the window scale factor: 0.5 * 2.0 at HiDPI.
    glass.set_scale(1.0);
    assert_eq!(glass.invoke_to_backdrop(100.5), 101);
}
