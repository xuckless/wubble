//! The one definition of a usable page icon, shared by the engine (which
//! produces icons) and the store (which keeps them), and the one way a larger
//! image is brought down to it.

/// Largest icon side kept, in pixels.
pub const MAX_ICON_PX: u32 = 64;

/// True if `width` x `height` is a usable icon size (both sides in
/// `1..=MAX_ICON_PX`) and `len` is exactly `width * height * 4` RGBA bytes.
pub fn valid_rgba(width: u32, height: u32, len: usize) -> bool {
    let side = 1..=MAX_ICON_PX;
    side.contains(&width) && side.contains(&height) && len == (width * height * 4) as usize
}

/// The longest side of the favicon image the shell is given: twice the side it
/// draws a favicon at (`Shell.icon`, 16 px; checked in wubble-ui's tokens
/// test). Slint's renderer samples an image bilinearly without mipmaps, so a
/// 64 px icon drawn at 16 px skips three texels of four and its edges alias
/// (jagged). At twice the drawn size, on whole pixels, the bilinear sample at
/// scale 1 is the exact 2x2 average and at scale 2 the image is drawn 1:1.
pub const SHELL_ICON_PX: u32 = 32;

/// Straight-alpha RGBA pixels of any size made into a usable icon: returned
/// as they are when both sides already fit, otherwise shrunk so the longer
/// side is `MAX_ICON_PX`, keeping the aspect ratio. `None` only for an empty
/// image or a byte count that does not match the size; never for being large.
pub fn fit_rgba(width: u32, height: u32, rgba: Vec<u8>) -> Option<(u32, u32, Vec<u8>)> {
    shrink_rgba(width, height, rgba, MAX_ICON_PX)
}

/// Straight-alpha RGBA pixels returned as they are when both sides are at most
/// `longest`, otherwise shrunk so the longer side is `longest`, keeping the
/// aspect ratio. `None` only for an empty image or a byte count that does not
/// match the size.
///
/// The shrink is an area-average (box) filter over premultiplied alpha, so
/// every source pixel counts by the area it covers and transparent pixels do
/// not darken the edges.
pub fn shrink_rgba(
    width: u32,
    height: u32,
    rgba: Vec<u8>,
    longest_side: u32,
) -> Option<(u32, u32, Vec<u8>)> {
    let expected = (width as usize)
        .checked_mul(height as usize)?
        .checked_mul(4)?;
    if width == 0 || height == 0 || rgba.len() != expected {
        return None;
    }
    let longest = width.max(height);
    if longest_side == 0 {
        return None;
    }
    if longest <= longest_side {
        return Some((width, height, rgba));
    }
    let scaled = |side: u32| {
        ((u64::from(side) * u64::from(longest_side) + u64::from(longest) / 2) / u64::from(longest))
            .max(1) as u32
    };
    let (w, h) = (scaled(width), scaled(height));
    // Premultiplied, as f32: [r, g, b, a] with r, g, b scaled by a / 255.
    let premultiplied: Vec<[f32; 4]> = rgba
        .as_chunks::<4>()
        .0
        .iter()
        .map(|p| {
            let a = f32::from(p[3]) / 255.0;
            [
                f32::from(p[0]) * a,
                f32::from(p[1]) * a,
                f32::from(p[2]) * a,
                f32::from(p[3]),
            ]
        })
        .collect();
    let columns = box_weights(width, w);
    let rows = box_weights(height, h);
    // Rows first: width x h.
    let stride = width as usize;
    let mut tall = vec![[0f32; 4]; stride * h as usize];
    for (y, row) in rows.iter().enumerate() {
        for x in 0..stride {
            let mut sum = [0f32; 4];
            for &(sy, weight) in row {
                add(&mut sum, premultiplied[sy * stride + x], weight);
            }
            tall[y * stride + x] = sum;
        }
    }
    let mut out = Vec::with_capacity(w as usize * h as usize * 4);
    for y in 0..h as usize {
        for column in &columns {
            let mut sum = [0f32; 4];
            for &(sx, weight) in column {
                add(&mut sum, tall[y * stride + sx], weight);
            }
            let alpha = sum[3];
            let straight = |c: f32| {
                if alpha <= 0.0 {
                    0
                } else {
                    (c * 255.0 / alpha).round().clamp(0.0, 255.0) as u8
                }
            };
            out.extend([
                straight(sum[0]),
                straight(sum[1]),
                straight(sum[2]),
                alpha.round().clamp(0.0, 255.0) as u8,
            ]);
        }
    }
    Some((w, h, out))
}

fn add(sum: &mut [f32; 4], pixel: [f32; 4], weight: f32) {
    for (s, p) in sum.iter_mut().zip(pixel) {
        *s += p * weight;
    }
}

/// For each of `dst` output pixels, the source pixels it covers out of `src`
/// and the share of the output pixel each one covers (summing to 1).
fn box_weights(src: u32, dst: u32) -> Vec<Vec<(usize, f32)>> {
    let step = f64::from(src) / f64::from(dst);
    (0..dst)
        .map(|i| {
            let (start, end) = (f64::from(i) * step, f64::from(i + 1) * step);
            let first = start.floor() as u32;
            let last = (end.ceil() as u32).min(src);
            (first..last)
                .filter_map(|j| {
                    let overlap = end.min(f64::from(j + 1)) - start.max(f64::from(j));
                    (overlap > 0.0).then(|| (j as usize, (overlap / step) as f32))
                })
                .collect()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_and_lengths() {
        assert!(valid_rgba(2, 3, 24));
        assert!(valid_rgba(64, 64, 64 * 64 * 4));
        assert!(!valid_rgba(0, 3, 0));
        assert!(!valid_rgba(65, 1, 65 * 4));
        assert!(!valid_rgba(2, 3, 23));
    }

    fn solid(width: u32, height: u32, pixel: [u8; 4]) -> Vec<u8> {
        pixel.repeat((width * height) as usize)
    }

    #[test]
    fn an_icon_that_fits_is_left_alone() {
        let rgba: Vec<u8> = (0..2 * 3 * 4).map(|n| n as u8).collect();
        assert_eq!(fit_rgba(2, 3, rgba.clone()), Some((2, 3, rgba)));
        let full = solid(64, 64, [1, 2, 3, 4]);
        assert_eq!(fit_rgba(64, 64, full.clone()), Some((64, 64, full)));
    }

    #[test]
    fn a_large_icon_is_shrunk_to_the_bound_never_rejected() {
        // YouTube's favicon arrives at 128 x 128 (CEF lets the renderer hand
        // back up to 64 x the device scale).
        let (w, h, rgba) = fit_rgba(128, 128, solid(128, 128, [200, 10, 30, 255])).unwrap();
        assert_eq!((w, h), (64, 64));
        assert!(valid_rgba(w, h, rgba.len()));
        assert!(
            rgba.as_chunks::<4>()
                .0
                .iter()
                .all(|p| *p == [200, 10, 30, 255])
        );
        for side in [65, 144, 1024] {
            let (w, h, rgba) = fit_rgba(side, side, solid(side, side, [0; 4])).unwrap();
            assert_eq!((w, h), (64, 64), "{side}");
            assert!(valid_rgba(w, h, rgba.len()));
        }
    }

    #[test]
    fn the_aspect_ratio_is_kept() {
        let (w, h, _) = fit_rgba(256, 128, solid(256, 128, [9; 4])).unwrap();
        assert_eq!((w, h), (64, 32));
        let (w, h, _) = fit_rgba(1, 300, solid(1, 300, [9; 4])).unwrap();
        assert_eq!((w, h), (1, 64), "a side never shrinks to 0");
    }

    #[test]
    fn the_box_filter_averages_what_each_pixel_covers() {
        // 128 x 1 alternating 0 and 200 grey, opaque: every output pixel
        // covers one of each.
        let rgba: Vec<u8> = (0..128)
            .flat_map(|x| {
                let v = if x % 2 == 0 { 0 } else { 200 };
                [v, v, v, 255]
            })
            .collect();
        let (w, h, out) = fit_rgba(128, 1, rgba).unwrap();
        assert_eq!((w, h), (64, 1));
        assert!(
            out.as_chunks::<4>()
                .0
                .iter()
                .all(|p| *p == [100, 100, 100, 255]),
            "{out:?}"
        );
    }

    #[test]
    fn transparent_pixels_do_not_darken_the_edge() {
        // Opaque red beside fully transparent black: half-covered red, not
        // dark red.
        let rgba: Vec<u8> = (0..128)
            .flat_map(|x| {
                if x % 2 == 0 {
                    [255, 0, 0, 255]
                } else {
                    [0, 0, 0, 0]
                }
            })
            .collect();
        let (_, _, out) = fit_rgba(128, 1, rgba).unwrap();
        assert!(
            out.as_chunks::<4>()
                .0
                .iter()
                .all(|p| *p == [255, 0, 0, 128]),
            "{out:?}"
        );
    }

    #[test]
    fn the_shell_gets_twice_the_drawn_side() {
        // a 64 px stored icon comes down to SHELL_ICON_PX; a 16 px one stays as it is
        let (w, h, rgba) =
            shrink_rgba(64, 64, solid(64, 64, [1, 2, 3, 255]), SHELL_ICON_PX).unwrap();
        assert_eq!((w, h), (SHELL_ICON_PX, SHELL_ICON_PX));
        assert!(valid_rgba(w, h, rgba.len()));
        let (w, h, _) = shrink_rgba(64, 48, solid(64, 48, [9; 4]), SHELL_ICON_PX).unwrap();
        assert_eq!((w, h), (32, 24));
        let small = solid(16, 16, [9; 4]);
        assert_eq!(
            shrink_rgba(16, 16, small.clone(), SHELL_ICON_PX),
            Some((16, 16, small))
        );
        assert_eq!(shrink_rgba(16, 16, solid(16, 16, [9; 4]), 0), None);
    }

    #[test]
    fn malformed_pixels_are_refused() {
        assert_eq!(fit_rgba(0, 3, vec![]), None);
        assert_eq!(fit_rgba(2, 3, vec![0; 23]), None);
        assert_eq!(fit_rgba(128, 128, vec![0; 10]), None);
    }
}
