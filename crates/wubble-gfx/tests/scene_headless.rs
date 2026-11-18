//! The scene on a real wgpu device with no window: a fixed number of frames,
//! then stable properties of the image. Skips (passes) when there is no GPU
//! adapter at all. No timing is measured.

#[path = "support/headless.rs"]
mod headless;

use headless::{Run, THICK, VIEW, device, render};
use wubble_gfx::scene::Quality;

const FRAMES: u32 = 30;

/// RGBA of the pixels in a logical-px rectangle.
fn region(img: &[u8], width: u32, (x, y, w, h): (f32, f32, f32, f32)) -> Vec<[u8; 4]> {
    let s = VIEW.2;
    let (x0, y0) = ((x * s) as u32, (y * s) as u32);
    let (x1, y1) = (((x + w) * s) as u32, ((y + h) * s) as u32);
    let mut out = vec![];
    for py in y0..y1 {
        for px in x0..x1 {
            let o = ((py * width + px) * 4) as usize;
            out.push([img[o], img[o + 1], img[o + 2], img[o + 3]]);
        }
    }
    out
}

/// Distinct colours on a 16×16 grid over the image.
fn distinct_colours(img: &[u8], w: u32, h: u32) -> usize {
    let mut seen = std::collections::HashSet::new();
    for gy in 0..16 {
        for gx in 0..16 {
            let (x, y) = (gx * w / 16 + w / 32, gy * h / 16 + h / 32);
            let o = ((y * w + x) * 4) as usize;
            seen.insert([img[o], img[o + 1], img[o + 2]]);
        }
    }
    seen.len()
}

#[test]
fn scene_renders_glass_over_the_backdrop() {
    let Some((device, queue, name)) = device() else {
        eprintln!("skipping: no GPU adapter");
        return;
    };
    eprintln!("adapter: {name}");
    let full = |glass| Run {
        glass,
        quality: Quality::Full,
        frames: FRAMES,
    };
    let (w, h, with_glass) = render(&device, &queue, full(true));
    assert_eq!((w, h), (640, 400), "logical 320×200 at dpr 2");
    assert_eq!(with_glass.len(), (w * h * 4) as usize);
    assert!(
        with_glass.chunks(4).all(|p| p[3] == 255),
        "the scene is opaque"
    );
    assert!(distinct_colours(&with_glass, w, h) >= 8, "not a flat image");

    let (_, _, without) = render(&device, &queue, full(false));
    let inside = (
        THICK.x + 20.0,
        THICK.y + 20.0,
        THICK.w - 40.0,
        THICK.h - 40.0,
    );
    assert_ne!(
        region(&with_glass, w, inside),
        region(&without, w, inside),
        "glass changes what is under it"
    );
    // more than the glass quads' 70 px padding away from every panel
    let corner = (VIEW.0 - 40.0, VIEW.1 - 40.0, 40.0, 40.0);
    assert_eq!(
        region(&with_glass, w, corner),
        region(&without, w, corner),
        "the backdrop away from the glass is the same"
    );

    let (_, _, again) = render(&device, &queue, full(true));
    assert!(again == with_glass, "same inputs, same image");
}

#[test]
fn flat_quality_still_renders_glass_over_a_frozen_backdrop() {
    let Some((device, queue, _)) = device() else {
        eprintln!("skipping: no GPU adapter");
        return;
    };
    for quality in [Quality::Flat, Quality::Cached, Quality::HalfRes] {
        let (w, h, img) = render(
            &device,
            &queue,
            Run {
                glass: true,
                quality,
                frames: FRAMES,
            },
        );
        assert_eq!((w, h), (640, 400));
        assert!(img.chunks(4).all(|p| p[3] == 255));
        assert!(distinct_colours(&img, w, h) >= 8, "{quality:?} is not flat");
    }
}
