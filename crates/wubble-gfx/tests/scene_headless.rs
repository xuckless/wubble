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
    assert_eq!(
        region(&with_glass, w, CORNER),
        region(&without, w, CORNER),
        "the backdrop away from the glass is the same"
    );

    let (_, _, again) = render(&device, &queue, full(true));
    assert!(again == with_glass, "same inputs, same image");
}

/// Largest difference of one channel between two pixels of `px`.
fn spread(px: &[[u8; 4]]) -> u8 {
    (0..3)
        .map(|c| {
            let (lo, hi) = px
                .iter()
                .fold((255, 0), |(lo, hi), p| (p[c].min(lo), p[c].max(hi)));
            hi - lo
        })
        .max()
        .unwrap_or(0)
}

/// A column down the thick panel: from 4 px below its top (past the 1 px rim;
/// the lens bezel and the specular band included), clear of its 20 px corners,
/// and 50 px above the bar, where the bloom of the bar's beam has faded.
const THICK_CORE: (f32, f32, f32, f32) = (THICK.x + 30.0, THICK.y + 4.0, THICK.w - 60.0, 40.0);
/// More than the glass quads' 70 px padding away from every panel.
const CORNER: (f32, f32, f32, f32) = (VIEW.0 - 40.0, VIEW.1 - 40.0, 40.0, 40.0);

#[test]
fn every_quality_draws_glass_and_flat_freezes_and_flattens() {
    let Some((device, queue, _)) = device() else {
        eprintln!("skipping: no GPU adapter");
        return;
    };
    let run = |quality, glass, frames| {
        let (w, h, img) = render(
            &device,
            &queue,
            Run {
                glass,
                quality,
                frames,
            },
        );
        assert_eq!((w, h), (640, 400));
        assert!(img.chunks(4).all(|p| p[3] == 255), "{quality:?} is opaque");
        img
    };
    let w = 640;
    for quality in [
        Quality::Full,
        Quality::HalfRes,
        Quality::Cached,
        Quality::Flat,
    ] {
        let with = run(quality, true, FRAMES);
        let without = run(quality, false, FRAMES);
        assert_ne!(
            region(&with, w, THICK_CORE),
            region(&without, w, THICK_CORE),
            "{quality:?} draws the glass"
        );
        assert!(
            distinct_colours(&with, w, 400) >= 8,
            "{quality:?} is not a flat image"
        );
        let core = spread(&region(&with, w, THICK_CORE));
        if quality == Quality::Flat {
            assert!(core <= 2, "Flat glass is one fill colour, spread {core}");
        } else {
            assert!(
                core > 4,
                "{quality:?} glass refracts the backdrop, spread {core}"
            );
        }
    }
    // Flat: the backdrop seen at frame 2 is the one seen at frame 30, while
    // at Full the liquid has moved.
    let corner = |quality, frames| region(&run(quality, true, frames), w, CORNER);
    assert_eq!(
        corner(Quality::Flat, 2),
        corner(Quality::Flat, FRAMES),
        "Flat freezes"
    );
    assert_ne!(
        corner(Quality::Full, 2),
        corner(Quality::Full, FRAMES),
        "Full moves"
    );
}
