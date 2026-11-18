//! The scene on a real wgpu device with no window: a fixed number of frames,
//! then stable properties of the image. Skips (passes) when there is no GPU
//! adapter at all. No timing is measured.

#[path = "support/headless.rs"]
mod headless;

use headless::{Run, THICK, VIEW, device, render};
use wubble_gfx::FrameSize;
use wubble_gfx::mask::{EdgeMask, MaskJob, Prev};
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
        bow: 0.0,
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
                bow: 0.0,
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

/// The scene builds without a validation error, and reports no frame cost
/// before it has rendered. Only presence is checked, never a value.
#[test]
fn scene_builds_and_has_no_cost_before_a_frame() {
    let Some((device, queue, _)) = device() else {
        eprintln!("skipping: no GPU adapter");
        return;
    };
    let mut r = wubble_gfx::scene::Renderer::from_device(device, queue, Default::default())
        .expect("the scene builds on the test device");
    assert_eq!(r.take_cost(), None);
}

/// A glass rect that grows (the sidebar dragged wider) is drawn at its new width at once,
/// across all of it: owner, "the glass effect doesn't grow past a certain size".
#[test]
fn a_glass_rect_that_grows_is_drawn_across_its_new_width() {
    use wubble_gfx::scene::{GlassRect, Rect, Renderer, Tier};
    let Some((device, queue, _)) = device() else {
        eprintln!("skipping: no GPU adapter");
        return;
    };
    let (w, h, dpr) = VIEW;
    let side = |width| GlassRect {
        rect: Rect::new(8.0, 8.0, width, h - 16.0),
        radius: 20.0,
        tier: Tier::Regular,
        pressed: false,
        hover: false,
        focus: false,
        bow: false,
        tint: [0.0; 3],
    };
    // the same liquid each run; `grow` widens the panel for the last frame
    let run = |glass: Option<(f32, f32)>| {
        let mut r = Renderer::from_device(device.clone(), queue.clone(), Default::default())
            .expect("the scene builds");
        r.set_quality(Quality::Full);
        for i in 0..FRAMES {
            if let Some((narrow, wide)) = glass {
                if i == 1 {
                    r.scene.set_glass(&[side(narrow)]);
                }
                if i == FRAMES - 1 {
                    r.scene.set_glass(&[side(wide)]);
                }
            }
            r.render_under_ui(w, h, dpr, 1.0 / 60.0);
        }
        wubble_gfx::scene::readback_rgba8(&device, &queue, r.output().unwrap())
    };
    let without = run(None);
    let grown = run(Some((100.0, 280.0)));
    let tw = (w * dpr) as u32;
    // a column well inside the wide panel and past the narrow one, clear of its corners
    for x in [150.0, 200.0, 250.0] {
        let column = (x, 40.0, 4.0, h - 80.0);
        assert_ne!(
            region(&grown, tw, column),
            region(&without, tw, column),
            "no glass at x {x} of a panel 280 wide"
        );
    }
}

/// The sidebar's glass follows the bowed seam: with its right edge bowed 30 px at mid height the
/// panel reaches into the strip right of its straight edge there, and nothing changes at the rows
/// above it, where the string is straight.
#[test]
fn a_bowed_glass_edge_differs_from_a_straight_one_only_where_it_bows() {
    let Some((device, queue, _)) = device() else {
        eprintln!("skipping: no GPU adapter");
        return;
    };
    let run = |bow| Run {
        glass: true,
        quality: Quality::Full,
        frames: 3,
        bow,
    };
    let (w, _, straight) = render(&device, &queue, run(0.0));
    let (_, _, bowed) = render(&device, &queue, run(30.0));
    let mid = THICK.y + THICK.h / 2.0;
    // just right of the straight edge, at the peak: glass now
    let strip = (THICK.x + THICK.w + 4.0, mid - 2.0, 16.0, 4.0);
    assert_ne!(region(&bowed, w, strip), region(&straight, w, strip));
    // the rows above the panel's top, where the bow is zero: the same
    let above = (THICK.x, 0.0, THICK.w + 40.0, THICK.y - 1.0);
    assert_eq!(region(&bowed, w, above), region(&straight, w, above));
}

/// A solid texture of `rgba`, `w`×`h`, sampled by the mask.
fn solid(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    w: u32,
    h: u32,
    rgba: [u8; 4],
) -> wgpu::Texture {
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("solid"),
        size: wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        // RENDER_ATTACHMENT: Slint takes it as an image (a stale frame)
        usage: wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_DST
            | wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let data: Vec<u8> = (0..w * h).flat_map(|_| rgba).collect();
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &tex,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        &data,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(w * 4),
            rows_per_image: Some(h),
        },
        wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
    );
    tex
}

fn edge(offset: &[f32]) -> Option<wubble_gfx::Edge<'_>> {
    Some(wubble_gfx::Edge {
        y0: 0.0,
        step: 4.0,
        offset,
    })
}

/// The page mask: inside the bowed edge the frame (alpha 1), in the cut nothing (alpha 0), in a
/// bulge past the frame the fill colour, and with straight edges the frame 1:1 with only the
/// margins and corners transparent.
#[test]
fn the_mask_cuts_and_bulges_along_the_tables() {
    let Some((device, queue, _)) = device() else {
        eprintln!("skipping: no GPU adapter");
        return;
    };
    let (pw, ph) = (200u32, 160u32);
    let red = [200, 30, 40, 255];
    let source = solid(&device, &queue, pw, ph, red);
    let mut mask = EdgeMask::new(&device);
    // left edge pinched in 20 px at mid height, right edge bulging out 20 px
    let cut: Vec<f32> = (0..=40)
        .map(|i| if (16..=24).contains(&i) { 20.0 } else { 0.0 })
        .collect();
    let bulge = cut.clone();
    let fill = [0.0, 1.0, 0.0];
    let job = MaskJob {
        pane: (pw as f32, ph as f32),
        scale: 1.0,
        stretch: (0.0, 0.0),
        clip: FrameSize::new(pw, ph),
        radius: 14.0,
        margin: 72.0,
        left: edge(&cut),
        right: edge(&bulge),
        fill,
        prev: None,
    };
    mask.run(&device, &queue, &source, &job).expect("mask");
    let tex = mask.latest().expect("drawn");
    assert_eq!((tex.width(), tex.height()), (pw + 144, ph));
    let img = wubble_gfx::scene::readback_rgba8(&device, &queue, tex);
    let at = |x: u32, y: u32| {
        let o = ((y * tex.width() + x) * 4) as usize;
        [img[o], img[o + 1], img[o + 2], img[o + 3]]
    };
    let m = 72;
    // inside: the frame, opaque
    assert_eq!(at(m + 100, 80), red);
    // the left cut at mid height (0..20 px in from the pane's edge): clear
    assert_eq!(at(m + 5, 80)[3], 0);
    // ...but not at the top, where the edge is straight
    assert_eq!(at(m + 5, 40), red);
    // the right bulge past the frame: the fill colour, opaque
    assert_eq!(at(m + pw + 10, 80), [0, 255, 0, 255]);
    // the margin beyond it, and beyond the straight right edge higher up: clear
    assert_eq!(at(m + pw + 30, 80)[3], 0);
    assert_eq!(at(m + pw + 5, 40)[3], 0);

    // straight edges: the frame 1:1 in the pane, nothing in the margins
    let straight = MaskJob {
        left: None,
        right: None,
        ..job
    };
    mask.run(&device, &queue, &source, &straight).expect("mask");
    let img = wubble_gfx::scene::readback_rgba8(&device, &queue, mask.latest().unwrap());
    let w = pw + 144;
    let px = |x: u32, y: u32| {
        let o = ((y * w + x) * 4) as usize;
        [img[o], img[o + 1], img[o + 2], img[o + 3]]
    };
    for y in [20, 80, 140] {
        for x in [0, 50, pw - 1] {
            assert_eq!(px(m + x, y), red, "({x}, {y})");
        }
        assert_eq!(px(m - 1, y)[3], 0);
        assert_eq!(px(m + pw, y)[3], 0);
    }

    // review finding: a stale frame fading out while the edge is bent (a frame at the new size
    // arrived mid-bend) is drawn over the new one at its opacity, inside the same cut
    let blue = [30, 40, 200, 255];
    let stale = slint::Image::try_from(solid(&device, &queue, pw, ph, blue)).expect("image");
    for (opacity, want) in [(1.0, blue), (0.0, red)] {
        let fading = MaskJob {
            prev: Some(Prev {
                image: &stale,
                clip: FrameSize::new(pw, ph),
                opacity,
            }),
            ..job
        };
        mask.run(&device, &queue, &source, &fading).expect("mask");
        let img = wubble_gfx::scene::readback_rgba8(&device, &queue, mask.latest().unwrap());
        let px = |x: u32, y: u32| {
            let o = ((y * w + x) * 4) as usize;
            [img[o], img[o + 1], img[o + 2], img[o + 3]]
        };
        assert_eq!(px(m + 100, 80), want, "opacity {opacity}");
        assert_eq!(px(m + 5, 80)[3], 0, "still cut");
    }
    let half = MaskJob {
        prev: Some(Prev {
            image: &stale,
            clip: FrameSize::new(pw, ph),
            opacity: 0.5,
        }),
        ..job
    };
    mask.run(&device, &queue, &source, &half).expect("mask");
    let img = wubble_gfx::scene::readback_rgba8(&device, &queue, mask.latest().unwrap());
    let o = ((80 * w + m + 100) * 4) as usize;
    // halfway between red and blue in each channel
    for (c, (a, b)) in red.iter().zip(blue).enumerate().take(3) {
        let mid = (i32::from(*a) + i32::from(b)) / 2;
        assert!(
            (i32::from(img[o + c]) - mid).abs() <= 2,
            "channel {c}: {}",
            img[o + c]
        );
    }
}
