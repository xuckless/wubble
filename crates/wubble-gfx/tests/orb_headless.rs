//! The orb painter on a real wgpu device with no window: coverage and blending as the canvas's
//! (an opaque dot is opaque at its centre and clear outside, a half-transparent pair blends
//! source-over, a line has butt caps), and the output is straight alpha. Skipped without an
//! adapter.

#[path = "support/headless.rs"]
#[allow(dead_code)]
mod headless;

use wubble_gfx::orb::{OrbMark, OrbPainter};
use wubble_gfx::scene::readback_rgba8;

fn px(rgba: &[u8], side: u32, x: u32, y: u32) -> [u8; 4] {
    let i = ((y * side + x) * 4) as usize;
    [rgba[i], rgba[i + 1], rgba[i + 2], rgba[i + 3]]
}

#[test]
fn dots_and_lines_cover_and_blend_as_the_canvas_does() {
    let Some((device, queue, _)) = headless::device() else {
        eprintln!("orb_headless: no adapter, skipped");
        return;
    };
    let mut painter = OrbPainter::from_device(device.clone(), queue.clone());
    let mut canvas = painter.canvas(64).expect("canvas");
    let white = [255.0, 255.0, 255.0, 1.0];
    let half_red = [255.0, 0.0, 0.0, 0.5];
    let half_blue = [0.0, 0.0, 255.0, 0.5];
    // scale 2: frame units are half a physical pixel (a 32 CSS px orb at dpr 2)
    painter.paint(
        &mut canvas,
        2.0,
        &[
            OrbMark::Line {
                x1: 2.0,
                y1: 4.0,
                x2: 30.0,
                y2: 4.0,
                w: 1.0,
                ink: white,
            },
            OrbMark::Dot {
                x: 8.0,
                y: 16.0,
                r: 4.0,
                ink: white,
            },
            OrbMark::Dot {
                x: 22.0,
                y: 22.0,
                r: 4.0,
                ink: half_red,
            },
            OrbMark::Dot {
                x: 22.0,
                y: 22.0,
                r: 4.0,
                ink: half_blue,
            },
        ],
    );
    let out = readback_rgba8(&device, &queue, canvas.latest());
    // the opaque dot: opaque white at its centre, nothing well outside it
    assert_eq!(px(&out, 64, 16, 32), [255, 255, 255, 255]);
    assert_eq!(px(&out, 64, 16, 44)[3], 0);
    assert_eq!(px(&out, 64, 0, 63)[3], 0);
    // red then blue at half alpha, source-over: alpha .75, colour 1/3 red, 2/3 blue (straight)
    let c = px(&out, 64, 44, 44);
    assert!(c[3].abs_diff(191) <= 1, "{c:?}");
    assert!(
        c[0].abs_diff(85) <= 2 && c[2].abs_diff(170) <= 2 && c[1] == 0,
        "{c:?}"
    );
    // the line: 2 px tall at y 8 (physical), butt caps at x 4 and 60
    assert_eq!(px(&out, 64, 32, 8)[3], 255);
    let col: Vec<u8> = (0..16).map(|y| px(&out, 64, 32, y)[3]).collect();
    assert_eq!(px(&out, 64, 32, 11)[3], 0, "{col:?}");
    assert_eq!(px(&out, 64, 2, 8)[3], 0);
    assert_eq!(px(&out, 64, 61, 8)[3], 0);
    // an edge pixel is partly covered (antialiased), not all or nothing
    let edge = (0..64 * 64)
        .map(|i| px(&out, 64, i % 64, i / 64)[3])
        .find(|&a| a > 0 && a < 191);
    assert!(edge.is_some(), "the dot's edge is antialiased");
}
