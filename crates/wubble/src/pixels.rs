//! `--smoke frames` correctness checks: did the web frame reach the window, and
//! does the scene's glass show (not a flat fill)?
//!
//! The smoke snapshots the window once after its last frame
//! (`Window::take_snapshot`, which re-renders the scene through the same
//! Skia-on-wgpu path into a readable texture) and samples the content
//! rectangle. Smoke only; never on the frame path.

use slint::{Rgba8Pixel, SharedPixelBuffer};

/// Samples per side of the grid laid over the content rectangle.
const GRID: u32 = 16;
/// Fewer distinct sampled colours than this counts as a uniform fill.
const MIN_DISTINCT: usize = 8;
/// A mean luma (0-255) below this counts as black.
const MIN_MEAN_LUMA: u32 = 40;

/// A rectangle in physical pixels of the snapshot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Region {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// What the sampled region looked like.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sample {
    pub distinct: usize,
    pub mean_luma: u32,
}

/// Samples a `GRID` x `GRID` grid inside `region` (cell centres, so rounded
/// corners are skipped): how many distinct colours, their mean luma, and the
/// first colour (for messages).
fn sample(
    snapshot: &SharedPixelBuffer<Rgba8Pixel>,
    region: Region,
) -> Result<(Sample, [u8; 3]), String> {
    let (width, height) = (snapshot.width(), snapshot.height());
    let right = region.x.saturating_add(region.width).min(width);
    let bottom = region.y.saturating_add(region.height).min(height);
    if region.x >= right || region.y >= bottom {
        return Err(format!(
            "region {region:?} is empty inside the {width}x{height} snapshot"
        ));
    }
    let pixels = snapshot.as_slice();
    let mut colours = Vec::new();
    let mut luma_sum = 0u32;
    for row in 0..GRID {
        for col in 0..GRID {
            let x = region.x + (right - region.x) * (2 * col + 1) / (2 * GRID);
            let y = region.y + (bottom - region.y) * (2 * row + 1) / (2 * GRID);
            let p = pixels[(y * width + x) as usize];
            // Rec. 601 integer luma.
            luma_sum += (299 * u32::from(p.r) + 587 * u32::from(p.g) + 114 * u32::from(p.b)) / 1000;
            let rgb = [p.r, p.g, p.b];
            if !colours.contains(&rgb) {
                colours.push(rgb);
            }
        }
    }
    let sample = Sample {
        distinct: colours.len(),
        mean_luma: luma_sum / (GRID * GRID),
    };
    Ok((sample, colours[0]))
}

/// The region is not a near-uniform fill.
pub fn check_varied(
    snapshot: &SharedPixelBuffer<Rgba8Pixel>,
    region: Region,
) -> Result<Sample, String> {
    let (sample, first) = sample(snapshot, region)?;
    if sample.distinct < MIN_DISTINCT {
        return Err(format!(
            "region {region:?} is near uniform: {} distinct colours in {} samples (first {first:?})",
            sample.distinct,
            GRID * GRID,
        ));
    }
    Ok(sample)
}

/// The content rectangle shows the page: varied and not near black.
pub fn check(snapshot: &SharedPixelBuffer<Rgba8Pixel>, region: Region) -> Result<Sample, String> {
    let sample = check_varied(snapshot, region)
        .map_err(|e| format!("content {e}, the web frame did not reach the window"))?;
    if sample.mean_luma < MIN_MEAN_LUMA {
        return Err(format!(
            "content region {region:?} is near black: mean luma {} < {MIN_MEAN_LUMA}, \
             the web frame did not reach the window",
            sample.mean_luma
        ));
    }
    Ok(sample)
}

/// Mean difference (0-255, per channel) above which the window does not show the scene.
const MAX_SCENE_DIFF: u32 = 4;

/// The window shows the scene's current frame in `region` (where Slint draws nothing over it):
/// `scene` is that frame read back, RGBA8, `(width, height, pixels)`. It is the snapshot's size,
/// or smaller above `DPR_MAX` (Slint stretches it to the window): each sample point is then read
/// at the same place in the frame. Returns the mean difference.
/// A frozen copy (Slint's rounded-clip layer cache) differs wherever the backdrop or the glass
/// changed.
pub fn check_shows_scene(
    snapshot: &SharedPixelBuffer<Rgba8Pixel>,
    (sw, sh, scene): (u32, u32, &[u8]),
    region: Region,
) -> Result<u32, String> {
    let (width, height) = (snapshot.width(), snapshot.height());
    // the frame's size is rounded from the window's logical size: its shape within a pixel
    let same_shape = (u64::from(sw) * u64::from(height)).abs_diff(u64::from(sh) * u64::from(width))
        <= u64::from(width.max(height));
    if scene.len() != (sw * sh * 4) as usize || sw > width || sh > height || !same_shape {
        return Err(format!(
            "the scene's frame ({sw}x{sh}) is not the window's ({width}x{height}) shape"
        ));
    }
    let pixels = snapshot.as_slice();
    // a snapshot pixel's centre, in the frame
    let at = |v: u32, from: u32, to: u32| {
        ((u64::from(v) * 2 + 1) * u64::from(to) / (u64::from(from) * 2)) as u32
    };
    let mut sum = 0u32;
    for row in 0..GRID {
        for col in 0..GRID {
            let x = region.x + region.width * (2 * col + 1) / (2 * GRID);
            let y = region.y + region.height * (2 * row + 1) / (2 * GRID);
            let i = (y * width + x) as usize;
            let j = (at(y, height, sh) * sw + at(x, width, sw)) as usize;
            let (p, q) = (pixels[i], &scene[j * 4..j * 4 + 3]);
            sum += [p.r, p.g, p.b]
                .iter()
                .zip(q)
                .map(|(a, b)| u32::from(a.abs_diff(*b)))
                .sum::<u32>();
        }
    }
    let mean = sum / (GRID * GRID * 3);
    if mean > MAX_SCENE_DIFF {
        return Err(format!(
            "region {region:?} is not the scene's current frame (mean difference {mean} > \
             {MAX_SCENE_DIFF}): the window shows a stale copy of the scene"
        ));
    }
    Ok(mean)
}

/// The bundled smoke page's swatches show their exact colours (see
/// [`wubble_gfx::swatch`]); `region` is where the page sits in the snapshot,
/// in physical pixels, and `scale` turns the swatches' page pixels into those
/// (at scale 2 the green swatch's centre is 256 px in, not 128).
pub fn check_swatches(
    snapshot: &SharedPixelBuffer<Rgba8Pixel>,
    region: Region,
    scale: f32,
) -> Result<(), String> {
    let width = snapshot.width() as usize;
    let pixels = snapshot.as_slice();
    let physical = |page: u32| (page as f32 * scale).round() as u32;
    wubble_gfx::swatch::check(|x, y| {
        pixels
            .get((region.y + physical(y)) as usize * width + (region.x + physical(x)) as usize)
            .map(|p| [p.r, p.g, p.b])
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn buffer(
        width: u32,
        height: u32,
        f: impl Fn(u32, u32) -> Rgba8Pixel,
    ) -> SharedPixelBuffer<Rgba8Pixel> {
        let mut buffer = SharedPixelBuffer::new(width, height);
        for (i, p) in buffer.make_mut_slice().iter_mut().enumerate() {
            let i = i as u32;
            *p = f(i % width, i / width);
        }
        buffer
    }

    const ALL: Region = Region {
        x: 0,
        y: 0,
        width: 64,
        height: 64,
    };

    #[test]
    fn gradient_passes() {
        let b = buffer(64, 64, |x, y| {
            Rgba8Pixel::new(100 + x as u8, 100 + y as u8, 200, 255)
        });
        let sample = check(&b, ALL).expect("a bright gradient passes");
        assert_eq!(sample.distinct, (GRID * GRID) as usize);
    }

    #[test]
    fn the_scene_shown_matches_its_frame() {
        let b = buffer(64, 64, |x, y| Rgba8Pixel::new(x as u8, y as u8, 9, 255));
        let rgba = |dx: u8| -> Vec<u8> {
            b.as_slice()
                .iter()
                .flat_map(|p| [p.r.saturating_add(dx), p.g, p.b, 255])
                .collect()
        };
        assert_eq!(check_shows_scene(&b, (64, 64, &rgba(0)), ALL), Ok(0));
        assert!(
            check_shows_scene(&b, (64, 64, &rgba(40)), ALL)
                .unwrap_err()
                .contains("stale")
        );
        assert!(check_shows_scene(&b, (1, 1, &[0; 4]), ALL).is_err());
    }

    #[test]
    fn a_scene_smaller_than_the_window_is_read_where_it_is_shown() {
        // a 3x window, the scene drawn at 2x: 96 px of window show 64 px of frame
        let b = buffer(96, 96, |x, y| {
            Rgba8Pixel::new((x * 2 / 3) as u8, (y * 2 / 3) as u8, 9, 255)
        });
        let frame: Vec<u8> = (0..64u32)
            .flat_map(|y| (0..64u32).flat_map(move |x| [x as u8, y as u8, 9, 255]))
            .collect();
        let all = Region {
            x: 0,
            y: 0,
            width: 96,
            height: 96,
        };
        assert!(check_shows_scene(&b, (64, 64, &frame), all).expect("shows it") <= 1);
        // a frame of another shape is not this window's
        assert!(check_shows_scene(&b, (64, 32, &frame[..64 * 32 * 4]), all).is_err());
    }

    #[test]
    fn uniform_fails() {
        let b = buffer(64, 64, |_, _| Rgba8Pixel::new(200, 200, 200, 255));
        assert!(check(&b, ALL).unwrap_err().contains("near uniform"));
    }

    #[test]
    fn dark_noise_fails() {
        let b = buffer(64, 64, |x, y| {
            Rgba8Pixel::new(x as u8 / 4, y as u8 / 4, 8, 255)
        });
        assert!(check(&b, ALL).unwrap_err().contains("near black"));
    }

    #[test]
    fn only_the_region_counts() {
        // Bright gradient on the left, black on the right.
        let b = buffer(64, 64, |x, y| {
            if x < 32 {
                Rgba8Pixel::new(100 + x as u8, 100 + y as u8, 200, 255)
            } else {
                Rgba8Pixel::new(0, 0, 0, 255)
            }
        });
        let right = Region {
            x: 32,
            y: 0,
            width: 32,
            height: 64,
        };
        assert!(check(&b, right).is_err());
        let left = Region {
            x: 0,
            width: 32,
            ..right
        };
        assert!(check(&b, left).is_ok());
    }

    #[test]
    fn dark_but_varied_is_varied() {
        let b = buffer(64, 64, |x, y| {
            Rgba8Pixel::new(x as u8 / 4, y as u8 / 4, 8, 255)
        });
        assert!(check_varied(&b, ALL).is_ok());
        let flat = buffer(64, 64, |_, _| Rgba8Pixel::new(8, 9, 13, 255));
        assert!(
            check_varied(&flat, ALL)
                .unwrap_err()
                .contains("near uniform")
        );
    }

    #[test]
    fn empty_region_fails() {
        let b = buffer(8, 8, |_, _| Rgba8Pixel::new(0, 0, 0, 255));
        let outside = Region {
            x: 8,
            y: 0,
            width: 4,
            height: 4,
        };
        assert!(check(&b, outside).unwrap_err().contains("empty"));
    }
}
