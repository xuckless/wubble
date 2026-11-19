// Source: the owner's design repo (xuckless/the-wallpaper-project @ 598f814),
// `src/wubble/slint/wfield.rs` (a 1:1 port of `wTexture()` in `core/gl/scene.ts`), with the
// marked `wubble change`s: the geometry and sizes come from `kit_gen::scene`.

//! The logo W as a distance field. Each texel's alpha stores the distance (viewBox units,
//! 0–16 → 0–255) to the W's centre line `M32 48 Q38 82 46 82 Q53 82 60 60 Q67 82 74 82 Q82 82 88 48`,
//! measured against the four quadratic Béziers flattened into 48 segments each. The bubble pass
//! turns it into a round tube with per-pixel AA. Rows are bottom-first (row 0 is v = 0, matching
//! `wuv.y = 1 − y/120`); wgpu also maps v = 0 to row 0, so upload the bytes as they are.
//!
//! About 28 M inner iterations: compute it once (off the frame path) and keep the texture.

use crate::kit_gen::scene as S;

// @source slint-w-field
pub const W_SIZE: usize = S::w_size;

/// RGBA8 bytes, W_SIZE × W_SIZE, only alpha written (like the TS `Uint8Array`). Upload as
/// `Rgba8Unorm` (not sRGB: the alpha is data), linear filter, clamp-to-edge.
pub fn w_field_rgba() -> Vec<u8> {
    let s = W_SIZE;
    let segs = S::w_segments;
    let mut pts: Vec<f64> = Vec::with_capacity(S::w_quads.len() * (segs + 1) * 2);
    for [x0, y0, cx, cy, x1, y1] in S::w_quads {
        for i in 0..=segs {
            let t = i as f64 / segs as f64;
            let u = 1.0 - t;
            pts.push(u * u * x0 + 2.0 * u * t * cx + t * t * x1);
            pts.push(u * u * y0 + 2.0 * u * t * cy + t * t * y1);
        }
    }
    let mut data = vec![0u8; s * s * 4];
    for j in 0..s {
        let y = (1.0 - (j as f64 + 0.5) / s as f64) * S::w_view; // row 0 is the bottom (v = 0)
        for i in 0..s {
            let x = ((i as f64 + 0.5) / s as f64) * S::w_view;
            let mut best = 1e9_f64;
            let mut k = 0;
            while k + 3 < pts.len() {
                let (ax, ay, bx, by) = (pts[k], pts[k + 1], pts[k + 2], pts[k + 3]);
                let (dx, dy) = (bx - ax, by - ay);
                let l2 = dx * dx + dy * dy;
                let l2 = if l2 == 0.0 { 1.0 } else { l2 }; // `|| 1`
                let t = 0f64.max(1f64.min(((x - ax) * dx + (y - ay) * dy) / l2));
                let (ex, ey) = (x - ax - dx * t, y - ay - dy * t);
                let d2 = ex * ex + ey * ey;
                if d2 < best {
                    best = d2;
                }
                k += 2;
            }
            // Math.round is round-half-up; floor(x + 0.5) matches it exactly for x ≥ 0
            let v = ((best.sqrt() / S::w_maxd) * 255.0 + 0.5).floor().min(255.0);
            data[(j * s + i) * 4 + 3] = v as u8;
        }
    }
    data
}

#[cfg(test)]
mod tests {
    use super::*;

    /// FNV-1a 64 of the bytes (the fixture script's hash).
    fn fnv1a64(bytes: &[u8]) -> u64 {
        bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, b| {
            (h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3)
        })
    }

    /// The web's `wTexture()` bytes, evaluated by tools/fidelity/sample-start.mjs: exact.
    #[test]
    fn w_field_matches_the_web() {
        let fx: serde_json::Value =
            serde_json::from_str(include_str!("../../tests/fixtures/start_wfield.json")).unwrap();
        let data = w_field_rgba();
        assert_eq!(data.len() as u64, fx["bytes"].as_u64().unwrap());
        assert_eq!(
            format!("{:016x}", fnv1a64(&data)),
            fx["fnv1a64"].as_str().unwrap()
        );
        for t in fx["texels"].as_array().unwrap() {
            let (i, j) = (
                t[0].as_u64().unwrap() as usize,
                t[1].as_u64().unwrap() as usize,
            );
            assert_eq!(
                u64::from(data[(j * W_SIZE + i) * 4 + 3]),
                t[2].as_u64().unwrap(),
                "texel ({i}, {j})"
            );
        }
    }
}
