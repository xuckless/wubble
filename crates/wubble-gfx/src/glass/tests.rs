//! Shader validation without a GPU and a GPU correctness check that is
//! skipped when no adapter is found. Nothing here measures time.

use welding::wgpu;

use super::plan::saturation_matrix;
use super::*;
use crate::GpuContext;
use crate::import::{ENCODED_VIEW_FORMAT, FULLSCREEN_VS_WGSL};

#[test]
fn kawase_wgsl_parses_and_validates() {
    use wgpu::naga;
    let source = format!("{FULLSCREEN_VS_WGSL}{}", include_str!("kawase.wgsl"));
    let module = naga::front::wgsl::parse_str(&source).expect("kawase.wgsl parses");
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::empty(),
    )
    .validate(&module)
    .expect("kawase.wgsl validates");
    for entry in ["vs_main", "fs_down", "fs_copy", "fs_up"] {
        assert!(
            module.entry_points.iter().any(|e| e.name == entry),
            "{entry}"
        );
    }
}

/// What a flat frame of `rgb` must come out as: unchanged by the blur, then
/// saturated as the CPU matrix says, on the encoded values.
fn expected(rgb: [u8; 3], tier: Tier) -> [u8; 3] {
    let encoded = rgb.map(|c| c as f32 / 255.0);
    let m = saturation_matrix(tier.saturation());
    let row = |r: usize| m[r][0] * encoded[0] + m[r][1] * encoded[1] + m[r][2] * encoded[2];
    [0, 1, 2].map(|r| (row(r).clamp(0.0, 1.0) * 255.0).round() as u8)
}
fn read_back(gpu: &GpuContext, texture: &wgpu::Texture) -> Vec<[u8; 4]> {
    let device = gpu.device();
    let size = texture.size();
    let row = (size.width * 4).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("glass test readback"),
        size: (row * size.height) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(row),
                rows_per_image: None,
            },
        },
        size,
    );
    gpu.queue().submit([encoder.finish()]);
    buffer
        .slice(..)
        .map_async(wgpu::MapMode::Read, |r| r.expect("map"));
    device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        })
        .expect("poll");
    let data = buffer.slice(..).get_mapped_range().expect("mapped");
    (0..size.height)
        .flat_map(|y| {
            let start = (y * row) as usize;
            data[start..start + size.width as usize * 4]
                .as_chunks::<4>()
                .0
                .to_vec()
        })
        .collect()
}

/// A flat colour stays flat through the blur (energy preserving, clamped
/// edges) and is saturated like the CPU matrix; a hard edge comes out soft.
/// Skipped without a GPU adapter. Correctness only, nothing is timed.
#[test]
fn gpu_flat_frame_blurs_to_itself_and_saturates_like_the_cpu() {
    let gpu = match GpuContext::new() {
        Ok(gpu) => gpu,
        Err(e) => {
            eprintln!("skipped: {e}");
            return;
        }
    };
    let device = gpu.device();
    let side = 64;
    let extent = wgpu::Extent3d {
        width: side,
        height: side,
        depth_or_array_layers: 1,
    };
    let frame = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("glass test frame"),
        size: extent,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[ENCODED_VIEW_FORMAT],
    });
    let upload = |pixel: &dyn Fn(u32) -> [u8; 4]| {
        let data: Vec<u8> = (0..side * side).flat_map(|i| pixel(i % side)).collect();
        gpu.queue().write_texture(
            frame.as_image_copy(),
            &data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(side * 4),
                rows_per_image: None,
            },
            extent,
        );
    };
    let all: TierSet = Tier::ALL.into_iter().collect();
    let mut glass = GlassBackdrop::new(&gpu);
    for (rgb, quality) in [
        ([128, 128, 128], Quality::Full),
        ([200, 100, 50], Quality::Full),
        ([40, 90, 160], Quality::HalfRes),
    ] {
        upload(&|_| [rgb[0], rgb[1], rgb[2], 255]);
        let backdrops = glass.render(&frame, quality, all, 1.0).expect("render");
        let want_scale = if quality == Quality::Full { 0.5 } else { 0.25 };
        assert_eq!(backdrops.px_per_frame_px, want_scale);
        for tier in Tier::ALL {
            let texture = backdrops
                .get(tier)
                .and_then(slint::Image::to_wgpu_30_texture)
                .expect("a wgpu-backed backdrop per tier");
            assert_eq!(texture.width(), (side as f32 * want_scale) as u32);
            let want = expected(rgb, tier);
            for got in read_back(&gpu, &texture) {
                for c in 0..3 {
                    assert!(
                        got[c].abs_diff(want[c]) <= 2,
                        "{tier:?} {quality:?} {rgb:?}: got {got:?}, want {want:?}"
                    );
                }
                assert_eq!(got[3], 255);
            }
        }
    }
    // Black left half, white right half: the blurred middle row ramps.
    upload(&|x| {
        if x < side / 2 {
            [0, 0, 0, 255]
        } else {
            [255; 4]
        }
    });
    let thin = TierSet::EMPTY.with(Tier::Thin);
    let backdrops = glass
        .render(&frame, Quality::Full, thin, 1.0)
        .expect("edge");
    let texture = backdrops
        .get(Tier::Thin)
        .and_then(slint::Image::to_wgpu_30_texture);
    let pixels = read_back(&gpu, &texture.expect("thin backdrop"));
    let width = (side / 2) as usize;
    let row = &pixels[width * width / 2..][..width];
    let soft = row.iter().filter(|p| (20..=235).contains(&p[0])).count();
    assert!(soft >= 4, "edge stayed hard: {row:?}");
    assert!(
        row.windows(2).all(|w| w[0][0] <= w[1][0]),
        "not a ramp: {row:?}"
    );
    assert!(backdrops.get(Tier::Regular).is_none() && backdrops.get(Tier::Thick).is_none());

    let flat = glass.render(&frame, Quality::Flat, all, 1.0).expect("flat");
    assert!(Tier::ALL.iter().all(|t| flat.get(*t).is_none()));
}
