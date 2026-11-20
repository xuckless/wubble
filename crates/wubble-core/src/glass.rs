//! The glass's material over web pages, as data for the scene (`wubble_gfx::scene::Material`).
//!
//! Over a page the design draws its glass with CSS (`core/wubble.css` `.wb-thin/.wb-regular/
//! .wb-thick`: backdrop blur, saturate, brightness, a translucent fill), not the WebGL frost it
//! uses over its own dark liquid; [`css_tiers`] hands the scene those tiers.
//!
//! The contrast ceiling is wubble's (derived, `docs/fidelity/derived-review.md`): the design
//! never puts its thin or regular glass over a white page, and even its thick tier over pure
//! white leaves the kit's secondary ink under 3:1. So no glass may get brighter than the
//! luminance at which the kit's own inks keep WCAG 2's minimums: the primary ink 4.5:1, the
//! secondary ink 3:1 ([`ceiling`]). The scene compresses the glass's light toward it above
//! [`knee`] (hue and chroma kept: it darkens, it does not grey), so the design's glass over its
//! own liquid, which stays under the knee, is untouched.

use crate::kit_gen::tokens::{color, glass};

/// WCAG 2 contrast minimums: normal text (1.4.3 AA) and secondary text held to the large-text
/// and non-text minimum (1.4.3 large, 1.4.11).
pub const PRIMARY_MIN: f64 = 4.5;
pub const SECONDARY_MIN: f64 = 3.0;

/// The kit's inks the ceiling keeps legible: `ink` (titles, menu rows) at [`PRIMARY_MIN`],
/// `mist` (secondary text) at [`SECONDARY_MIN`].
pub const INKS: [([u8; 3], f64); 2] = [(color::ink, PRIMARY_MIN), (color::mist, SECONDARY_MIN)];

/// The relative luminance of an sRGB colour (WCAG 2).
pub fn luminance(rgb: [u8; 3]) -> f64 {
    let lin = |c: u8| {
        let c = f64::from(c) / 255.0;
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * lin(rgb[0]) + 0.7152 * lin(rgb[1]) + 0.0722 * lin(rgb[2])
}

/// The WCAG contrast ratio of two relative luminances.
pub fn contrast(a: f64, b: f64) -> f64 {
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

/// The brightest a glass surface may be (relative luminance): every ink of [`INKS`] keeps its
/// minimum over it.
pub fn ceiling() -> f64 {
    INKS.iter()
        .map(|&(ink, min)| (luminance(ink) + 0.05) / min - 0.05)
        .fold(1.0, f64::min)
}

/// Where the easing toward [`ceiling`] starts: half of it. The design's glass over its own liquid
/// sits about here (its sidebar measures 0.050), so it keeps its look; above it a white page and
/// a mid-grey one still differ.
pub fn knee() -> f64 {
    ceiling() / 2.0
}

/// A glass tier as CSS draws it over a page: `[saturate, brightness, fill r, g, b (0-1), fill
/// alpha]`, thin, regular, thick (`GlassTier`'s order).
pub fn css_tiers() -> [[f32; 6]; 3] {
    [glass::thin, glass::regular, glass::thick].map(|t| {
        [
            t[0] as f32,
            t[1] as f32,
            (t[2] / 255.0) as f32,
            (t[3] / 255.0) as f32,
            (t[4] / 255.0) as f32,
            t[5] as f32,
        ]
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const WHITE: [u8; 3] = [255, 255, 255];

    #[test]
    fn luminance_and_contrast_match_wcag() {
        assert!((luminance(WHITE) - 1.0).abs() < 1e-9);
        assert_eq!(luminance([0, 0, 0]), 0.0);
        assert!((contrast(1.0, 0.0) - 21.0).abs() < 1e-9);
        assert_eq!(contrast(0.2, 0.5), contrast(0.5, 0.2));
    }

    #[test]
    fn every_ink_keeps_its_minimum_at_the_ceiling() {
        let c = ceiling();
        for (ink, min) in INKS {
            assert!(contrast(luminance(ink), c) >= min - 1e-9, "{ink:?}");
        }
        // and one of them sits right on it: the ceiling is no darker than it must be
        assert!(
            INKS.iter()
                .any(|&(ink, min)| (contrast(luminance(ink), c) - min).abs() < 1e-9)
        );
        assert!(knee() > 0.0 && knee() < c);
    }

    #[test]
    fn the_design_thick_glass_over_white_is_above_the_ceiling() {
        // .wb-thick over a white page: brightness .55, then the fill at .42
        let t = css_tiers()[2];
        let over = |c: f32| (c * t[1]) * (1.0 - t[5]);
        let rgb = [0, 1, 2].map(|i| ((over(1.0) + t[2 + i] * t[5]) * 255.0).round() as u8);
        assert!(luminance(rgb) > ceiling(), "{rgb:?}");
        // where its primary ink only just reaches 4.5:1 and the secondary falls short
        assert!(contrast(luminance(color::mist), luminance(rgb)) < SECONDARY_MIN);
    }

    #[test]
    fn the_tiers_are_the_design_s() {
        let [thin, regular, thick] = css_tiers();
        assert_eq!(thin[..2], [1.7, 1.08]);
        assert_eq!(regular[1], 1.0);
        assert_eq!(thick[1], 0.55);
        assert!(thick[2..5].iter().all(|c| *c < 0.4));
    }
}
