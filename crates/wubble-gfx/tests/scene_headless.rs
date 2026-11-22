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
    let full = |glass| Run::liquid(glass, Quality::Full, FRAMES);
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
        let (w, h, img) = render(&device, &queue, Run::liquid(glass, quality, frames));
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
        tint: [0.0; 3],
        shadow: 0.6,
        beam: false,
        layer: None,
        alpha: 1.0,
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

/// The browser window's still backdrop: the same inputs give the same image, and its composite and
/// pyramid render only when its dye, ripples or size change (a count, not a clock).
#[test]
fn a_still_backdrop_is_deterministic_and_renders_only_on_change() {
    use headless::renderer;
    use wubble_gfx::scene::{Backdrop, Renderer};
    let Some((device, queue, _)) = device() else {
        eprintln!("skipping: no GPU adapter");
        return;
    };
    let still = |glass| Run {
        glass,
        quality: Quality::Full,
        frames: FRAMES,
        backdrop: Backdrop::Still,
    };
    let (_, _, a) = render(&device, &queue, still(true));
    let (_, _, b) = render(&device, &queue, still(true));
    assert!(a == b, "two still renders of the same inputs are equal");
    assert!(distinct_colours(&a, 640, 400) >= 8, "the dye shows");

    // once the ripple of frame 1 has ended (1.15 s), nothing re-renders the backdrop
    let (w, h, dpr) = VIEW;
    let mut r: Renderer = renderer(&device, &queue, still(false));
    for _ in 0..80 {
        r.render_under_ui(w, h, dpr, 1.0 / 60.0);
    }
    let settled = r.backdrops();
    for _ in 0..5 {
        r.render_under_ui(w, h, dpr, 1.0 / 60.0);
    }
    assert_eq!(r.backdrops(), settled, "nothing changed");
    r.scene.tap(100.0, 100.0);
    r.render_under_ui(w, h, dpr, 1.0 / 60.0);
    assert_eq!(r.backdrops(), settled + 1, "a ripple runs through it");
    for _ in 0..200 {
        r.render_under_ui(w, h, dpr, 1.0 / 60.0);
    }
    let after = r.backdrops();
    r.render_under_ui(w, h, dpr, 1.0 / 60.0);
    assert_eq!(r.backdrops(), after, "the ripple ended");
    r.scene.set_palette([[1.0, 1.0, 1.0]; 3]);
    r.render_under_ui(w, h, dpr, 1.0 / 60.0);
    assert_eq!(r.backdrops(), after + 1, "a new palette");
    r.render_under_ui(w + 40.0, h, dpr, 1.0 / 60.0);
    assert!(r.backdrops() >= after + 2, "a new size");
}

/// A tiny view still builds every pyramid level (b3 is 1/16 of the view).
#[test]
fn a_tiny_view_builds_the_whole_pyramid() {
    use wubble_gfx::scene::{Backdrop, Renderer, SceneOptions};
    let Some((device, queue, _)) = device() else {
        eprintln!("skipping: no GPU adapter");
        return;
    };
    for backdrop in [Backdrop::Liquid, Backdrop::Still] {
        let opts = SceneOptions {
            backdrop,
            ..SceneOptions::default()
        };
        let mut r = Renderer::from_device(device.clone(), queue.clone(), opts).expect("builds");
        for (w, h) in [(160.0, 100.0), (3.0, 2.0)] {
            r.render_under_ui(w, h, 1.0, 1.0 / 60.0);
            assert!(r.output().is_some());
        }
    }
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

/// The page in the scene: overlay glass over a pane frosts the page (its colour, not the
/// backdrop's) in its layer's image; the glass under the page picks up the page's light at its
/// edge (the ambience); with nothing over the page a new page frame builds nothing.
#[test]
fn overlay_glass_frosts_the_page_and_the_window_glass_picks_up_its_light() {
    use wubble_gfx::scene::{
        Backdrop, GlassRect, OverLayer, PagePane, PageRect, Rect, Renderer, SceneOptions, Tier,
    };
    let Some((device, queue, _)) = device() else {
        eprintln!("skipping: no GPU adapter");
        return;
    };
    let (w, h, dpr) = VIEW;
    let red = solid(&device, &queue, 64, 64, [230, 20, 20, 255]);
    let page = |tex: &wgpu::Texture| PagePane {
        rect: PageRect {
            rect: Rect::new(160.0, 8.0, 152.0, 184.0),
            radius: 12.0,
        },
        texture: tex.clone(),
        clip: (64, 64),
    };
    let glass = |rect, layer| GlassRect {
        rect,
        radius: 12.0,
        tier: Tier::Thick,
        pressed: false,
        hover: false,
        focus: false,
        tint: [0.0; 3],
        shadow: 0.6,
        beam: false,
        layer,
        alpha: 1.0,
    };
    let menu = glass(Rect::new(190.0, 60.0, 100.0, 60.0), Some(OverLayer::Top));
    let window = glass(Rect::new(0.0, 0.0, w, h), None);
    let redness = |p: [u8; 4]| i32::from(p[0]) - (i32::from(p[1]) + i32::from(p[2])) / 2;
    let px = |img: &[u8], x: f32, y: f32| {
        let o = ((((y * dpr) as u32) * (w * dpr) as u32 + (x * dpr) as u32) * 4) as usize;
        [img[o], img[o + 1], img[o + 2], img[o + 3]]
    };
    let run = |pages: bool, items: &[GlassRect], ambience: f32| {
        let mut r = Renderer::from_device(
            device.clone(),
            queue.clone(),
            SceneOptions {
                backdrop: Backdrop::Still,
                ..SceneOptions::default()
            },
        )
        .expect("the scene builds");
        r.set_quality(Quality::Full);
        r.set_ambience(ambience);
        r.scene.set_glass(items);
        if pages {
            r.set_pages(vec![page(&red)]);
        }
        for _ in 0..3 {
            r.render_under_ui(w, h, dpr, 1.0 / 60.0);
        }
        r
    };

    // the menu over the red page, against the same menu over the backdrop alone
    let over = |r: &Renderer| {
        let t = r
            .over_texture(OverLayer::Top)
            .expect("the menu's layer has an image");
        wubble_gfx::scene::readback_rgba8(&device, &queue, t)
    };
    let with_page = run(true, &[menu], 0.0);
    let without = run(false, &[menu], 0.0);
    let (a, b) = (
        px(&over(&with_page), 240.0, 90.0),
        px(&over(&without), 240.0, 90.0),
    );
    assert_eq!(
        (a[3], b[3]),
        (255, 255),
        "the frost is opaque inside the menu"
    );
    assert!(
        redness(a) > redness(b) + 40,
        "the page's red under the menu: {a:?} against {b:?}"
    );
    // outside the menu (and its shadow) the layer is clear; the menu is not in the back image
    assert_eq!(px(&over(&with_page), 20.0, 180.0)[3], 0);
    assert!(with_page.over_texture(OverLayer::Card).is_none());
    // built with the backdrop's first renders, then kept while no new frame comes
    let mut kept = with_page;
    let built = kept.pages_built();
    assert!(built >= 1);
    for _ in 0..3 {
        kept.render_under_ui(w, h, dpr, 1.0 / 60.0);
    }
    assert_eq!(kept.pages_built(), built, "no new page frame, no build");
    // a new frame under the menu: built on the next frame
    let green = solid(&device, &queue, 64, 64, [20, 230, 20, 255]);
    kept.set_pages(vec![page(&green)]);
    kept.render_under_ui(w, h, dpr, 1.0 / 60.0);
    assert_eq!(kept.pages_built(), built + 1);
    // the same texture written in place (a page's damage): built again too
    kept.pages_written();
    kept.render_under_ui(w, h, dpr, 1.0 / 60.0);
    assert_eq!(kept.pages_built(), built + 2);

    // nothing over the page, no ambience: a new page frame builds nothing
    let mut idle = run(true, &[glass(Rect::new(8.0, 8.0, 100.0, 50.0), None)], 0.0);
    idle.set_pages(vec![page(&green)]);
    idle.render_under_ui(w, h, dpr, 1.0 / 60.0);
    assert_eq!(idle.pages_built(), 0);
    assert!(idle.over_texture(OverLayer::Top).is_none());

    // the ambience: the window's glass just left of the page is redder with it on
    let out =
        |r: &Renderer| wubble_gfx::scene::readback_rgba8(&device, &queue, r.output().unwrap());
    let lit = out(&run(true, &[window], 0.2));
    let dark = out(&run(true, &[window], 0.0));
    let (near, far) = ((150.0, 100.0), (20.0, 100.0));
    assert!(
        redness(px(&lit, near.0, near.1)) > redness(px(&dark, near.0, near.1)) + 3,
        "page light at the edge: {:?} against {:?}",
        px(&lit, near.0, near.1),
        px(&dark, near.0, near.1)
    );
    // and it falls off: far from the page the two agree within a few levels
    let (l, d) = (px(&lit, far.0, far.1), px(&dark, far.0, far.1));
    assert!((redness(l) - redness(d)).abs() <= 4, "{l:?} against {d:?}");
}

/// A renderer on the browser window's still backdrop, nothing else: no glass, no pointer, no
/// taps, so every frame away from the start page's card is the same.
fn still(device: &wgpu::Device, queue: &wgpu::Queue) -> wubble_gfx::scene::Renderer {
    use wubble_gfx::scene::{Backdrop, Renderer, SceneOptions};
    Renderer::from_device(
        device.clone(),
        queue.clone(),
        SceneOptions {
            backdrop: Backdrop::Still,
            ..SceneOptions::default()
        },
    )
    .expect("the scene builds on the test device")
}

fn frame(
    r: &mut wubble_gfx::scene::Renderer,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
) -> (u32, Vec<u8>) {
    let (w, h, dpr) = VIEW;
    r.render_under_ui(w, h, dpr, 1.0 / 60.0);
    let tex = r.output().expect("rendered");
    (
        tex.width(),
        wubble_gfx::scene::readback_rgba8(device, queue, tex),
    )
}

#[test]
fn the_start_page_lives_inside_its_card_and_nowhere_else() {
    use wubble_gfx::scene::{BubbleDraw, Rect, StartCard, StartDraw, StartFx};
    let Some((device, queue, _)) = device() else {
        eprintln!("skipping: no GPU adapter");
        return;
    };
    // the page card right of a 120 px sidebar; the bubble in its middle, page-local px
    let card = (120.0, 20.0, 180.0, 160.0);
    let (c, r) = ([90.0, 80.0], 40.0);
    let ext = r * 1.7 + 4.0;
    let draw = StartDraw {
        card: StartCard {
            rect: Rect::new(card.0, card.1, card.2, card.3),
            radius: 12.0,
        },
        bubbles: vec![BubbleDraw {
            quad: [c[0] - ext, c[1] - ext, ext * 2.0, ext * 2.0],
            c,
            r,
            body: 1.0,
            alpha: 1.0,
            has_w: true,
            w_inv: [1.0, 0.0, 0.0, 1.0],
            w_scale: r / 51.0,
            ..Default::default()
        }],
        pops: vec![],
        time: 0.0,
        pace: 1.0,
        rest: false,
    };
    let mut plain = still(&device, &queue);
    let mut live = still(&device, &queue);
    // a W field: the distance grows to the right (alpha), so the tube crosses the bubble
    let n = 64u32;
    let w: Vec<u8> = (0..n * n)
        .flat_map(|i| [0, 0, 0, ((i % n) * 255 / n) as u8])
        .collect();
    live.set_wfield(&w, n);
    live.start_frame(draw.clone());
    assert!(
        live.animating(),
        "a shown start page keeps the scene running"
    );
    let (width, a) = frame(&mut plain, &device, &queue);
    let (_, b) = frame(&mut live, &device, &queue);
    let sidebar = (0.0, 0.0, card.0 - 2.0, VIEW.1);
    assert_eq!(
        region(&a, width, sidebar),
        region(&b, width, sidebar),
        "outside the card the still backdrop is untouched"
    );
    let bubble = (card.0 + c[0] - 20.0, card.1 + c[1] - 20.0, 40.0, 40.0);
    assert_ne!(
        region(&a, width, bubble),
        region(&b, width, bubble),
        "the card shows the start page"
    );
    assert!(live.start_texture().is_some());
    // the liquid lives, poked and stirred: the card moves, the sidebar does not
    live.start_effect(StartFx::Ripple {
        x: 40.0,
        y: 40.0,
        reach: 260.0,
        strength: 1.0,
    });
    live.start_effect(StartFx::Light { x: 60.0, y: 60.0 });
    live.start_frame(draw.clone());
    let (_, b2) = frame(&mut live, &device, &queue);
    assert_eq!(region(&b, width, sidebar), region(&b2, width, sidebar));
    let page = (card.0 + 4.0, card.1 + 4.0, card.2 - 8.0, card.3 - 8.0);
    assert_ne!(region(&b, width, page), region(&b2, width, page));
    // no start page: the layer goes, and the window settles as if it never was
    live.start_end();
    assert!(!live.start_shown() && live.start_texture().is_none());
    let (_, c2) = frame(&mut live, &device, &queue);
    let (_, a2) = frame(&mut plain, &device, &queue);
    assert!(
        c2 == a2,
        "without the start page the frame is the plain one"
    );
    assert!(!live.animating());
}

#[test]
fn a_start_page_frame_redraws_only_its_card_and_draws_what_a_whole_frame_draws() {
    use wubble_gfx::scene::{Damage, GlassRect, Rect, StartCard, StartDraw, Tier};
    let Some((device, queue, _)) = device() else {
        eprintln!("skipping: no GPU adapter");
        return;
    };
    let card = StartCard {
        rect: Rect::new(120.0, 20.0, 180.0, 160.0),
        radius: 12.0,
    };
    let draw = StartDraw {
        card,
        pace: 1.0,
        ..StartDraw::default()
    };
    // a glass panel beside the card and under its rounded corner
    let panel = GlassRect {
        rect: Rect::new(8.0, 8.0, 140.0, 60.0),
        radius: 20.0,
        tier: Tier::Regular,
        pressed: false,
        hover: false,
        focus: false,
        tint: [0.0; 3],
        shadow: 0.6,
        beam: false,
        layer: None,
        alpha: 1.0,
    };
    // two windows on the same start page, frame for frame
    let mut partial = still(&device, &queue);
    let mut whole = still(&device, &queue);
    for r in [&mut partial, &mut whole] {
        r.scene.set_glass(&[panel]);
        r.start_frame(draw.clone());
        frame(r, &device, &queue);
        assert_eq!(r.take_damage(), Damage::Whole, "the first frame is whole");
    }
    for n in 0..3 {
        partial.start_frame(draw.clone());
        whole.start_frame(draw.clone());
        whole.draw_whole_next();
        let (_, a) = frame(&mut partial, &device, &queue);
        let (_, b) = frame(&mut whole, &device, &queue);
        // the liquid moved and nothing else: only the card is redrawn, for the scene and Slint
        assert_eq!(partial.take_damage(), Damage::Card(card.rect), "frame {n}");
        assert_eq!(whole.take_damage(), Damage::Whole);
        assert!(
            a == b,
            "frame {n}: the card's frame holds the whole frame's pixels"
        );
    }
    // the glass changed: whole
    partial.scene.set_glass(&[GlassRect {
        alpha: 0.5,
        ..panel
    }]);
    partial.start_frame(draw.clone());
    frame(&mut partial, &device, &queue);
    assert_eq!(partial.take_damage(), Damage::Whole);
    // the card's place changed: whole again; no start page and nothing moving: nothing
    partial.start_frame(StartDraw {
        card: StartCard {
            radius: 0.0,
            ..card
        },
        ..draw.clone()
    });
    frame(&mut partial, &device, &queue);
    assert_eq!(partial.take_damage(), Damage::Whole);
    partial.start_end();
    frame(&mut partial, &device, &queue);
    assert_eq!(partial.take_damage(), Damage::Whole, "the card leaves");
    frame(&mut partial, &device, &queue);
    assert_eq!(partial.take_damage(), Damage::Nothing);
    // a settled start page asks for no frames; awake, it does
    let mut r = still(&device, &queue);
    r.start_frame(StartDraw {
        rest: true,
        pace: 0.0,
        ..draw.clone()
    });
    frame(&mut r, &device, &queue);
    assert!(!r.animating(), "a page at rest");
    r.start_frame(draw.clone());
    assert!(r.animating(), "woken");
}
