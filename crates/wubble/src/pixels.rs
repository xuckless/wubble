//! `--smoke frames` correctness check: did the web frame reach the window?
//!
//! The smoke snapshots the window once after its last frame
//! (`Window::take_snapshot`, which re-renders the scene through the same
//! femtovg-wgpu path into a readable texture) and samples the content
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
/// corners are skipped) and fails if it is near uniform or near black.
pub fn check(snapshot: &SharedPixelBuffer<Rgba8Pixel>, region: Region) -> Result<Sample, String> {
    let (width, height) = (snapshot.width(), snapshot.height());
    let right = region.x.saturating_add(region.width).min(width);
    let bottom = region.y.saturating_add(region.height).min(height);
    if region.x >= right || region.y >= bottom {
        return Err(format!(
            "content region {region:?} is empty inside the {width}x{height} snapshot"
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
    if sample.distinct < MIN_DISTINCT {
        return Err(format!(
            "content region {region:?} is near uniform: {} distinct colours in {} samples \
             (first {:?}), the web frame did not reach the window",
            sample.distinct,
            GRID * GRID,
            colours[0]
        ));
    }
    if sample.mean_luma < MIN_MEAN_LUMA {
        return Err(format!(
            "content region {region:?} is near black: mean luma {} < {MIN_MEAN_LUMA}, \
             the web frame did not reach the window",
            sample.mean_luma
        ));
    }
    Ok(sample)
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
