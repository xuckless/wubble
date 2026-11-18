//! Pure maths for the glass pass: the tier table, the dual-Kawase level plan,
//! the saturate matrix and the uniform layout. No GPU here.

use crate::FrameSize;

/// A glass material. The blur and saturation numbers live only here; the
/// tint, shadow and rim tokens live in `wubble-ui/ui/tokens.slint`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Tier {
    Thin,
    Regular,
    Thick,
}

impl Tier {
    pub const ALL: [Tier; 3] = [Tier::Thin, Tier::Regular, Tier::Thick];

    /// CSS `blur()` radius of the tier in logical pixels (a standard
    /// deviation, as in CSS and Slint). Pack `Tokens.dc.html`: 16 / 30 / 44.
    pub const fn sigma_px(self) -> f32 {
        match self {
            Tier::Thin => 16.0,
            Tier::Regular => 30.0,
            Tier::Thick => 44.0,
        }
    }

    /// CSS `saturate()` amount, applied after the blur. Pack: 160 / 180 / 200%.
    pub const fn saturation(self) -> f32 {
        match self {
            Tier::Thin => 1.6,
            Tier::Regular => 1.8,
            Tier::Thick => 2.0,
        }
    }

    const fn bit(self) -> u8 {
        1 << self as u8
    }
}

/// The tiers on screen right now. Only these get a backdrop texture.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct TierSet(u8);

impl TierSet {
    pub const EMPTY: TierSet = TierSet(0);

    pub const fn with(self, tier: Tier) -> Self {
        Self(self.0 | tier.bit())
    }

    pub const fn contains(self, tier: Tier) -> bool {
        self.0 & tier.bit() != 0
    }

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub fn iter(self) -> impl Iterator<Item = Tier> {
        Tier::ALL.into_iter().filter(move |t| self.contains(*t))
    }
}

impl FromIterator<Tier> for TierSet {
    fn from_iter<I: IntoIterator<Item = Tier>>(iter: I) -> Self {
        iter.into_iter().fold(Self::EMPTY, Self::with)
    }
}

/// KWin's per-iteration offset table, `{minOffset, maxOffset, expandSize}`:
/// row `i` is `i + 1` down/up iterations. Outside `[min, max]` the dual
/// filter shows blocky (below) or diagonal-line (above) artefacts; `expand`
/// is how far, in pixels, the blur reaches.
/// Copied from KWin `src/plugins/blur/blur.cpp`, `initBlurStrengthValues`
/// (KDE/kwin, GPL-2.0-or-later).
const KWIN_OFFSETS: [(f32, f32, f32); 4] = [
    (1.0, 2.0, 10.0),
    (2.0, 3.0, 20.0),
    (2.0, 5.0, 50.0),
    (3.0, 8.0, 150.0),
];

/// Deepest pyramid level (frame size / 16), KWin's last enabled row.
pub const MAX_LEVELS: u32 = KWIN_OFFSETS.len() as u32;

/// How many standard deviations of a gaussian the Kawase reach must cover.
/// Starting point; the look is tuned by eye (unverified on screen).
const REACH_SIGMAS: f32 = 3.0;

/// Iteration count and sample offset of one dual-Kawase blur.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct KawasePlan {
    /// Down passes (and up passes) to run, `1..=MAX_LEVELS`.
    pub levels: u32,
    /// KWin's `offset` uniform, in half-texels.
    pub offset: f32,
}

/// The plan for a blur of `sigma_px` logical pixels at `scale` frame pixels
/// per logical pixel: the fewest levels whose KWin reach covers
/// `REACH_SIGMAS` sigmas, and an offset placed between that row's min and max
/// by where the reach falls between the previous row's reach and this one.
/// Larger blurs than the last row reaches use its maximum offset.
pub fn kawase_plan(sigma_px: f32, scale: f32) -> KawasePlan {
    let reach = (REACH_SIGMAS * sigma_px * scale).max(0.0);
    let mut previous = 0.0;
    for (i, &(min, max, expand)) in KWIN_OFFSETS.iter().enumerate() {
        let last = i + 1 == KWIN_OFFSETS.len();
        if reach <= expand || last {
            let t = ((reach - previous) / (expand - previous)).clamp(0.0, 1.0);
            return KawasePlan {
                levels: i as u32 + 1,
                offset: min + t * (max - min),
            };
        }
        previous = expand;
    }
    unreachable!("the table's last row always returns")
}

/// Size of pyramid level `level` (level 0 is the frame): halves per level,
/// never 0. Ported from backdrop-blur-wgpu 0.2.0 `kawase_level_size`
/// (abdu-benayad/backdrop-blur, MIT OR Apache-2.0).
pub fn kawase_level_size(size: FrameSize, level: u32) -> FrameSize {
    let half = |side: u32| side.checked_shr(level).unwrap_or(0).max(1);
    FrameSize::new(half(size.width), half(size.height))
}

/// Half a texel of the texture being sampled, in UV units (KWin's
/// `halfpixel` uniform: `0.5 / read->colorAttachment()->size()`).
pub fn kawase_halfpixel(sampled: FrameSize) -> [f32; 2] {
    [0.5 / sampled.width as f32, 0.5 / sampled.height as f32]
}

/// The `saturate` colour matrix as three RGB rows (fourth column zero, for
/// WGSL `vec4` alignment). W3C Filter Effects Module Level 1,
/// `feColorMatrix type="saturate"`, which CSS `saturate()` also uses.
/// Applied to gamma-encoded sRGB values, as Chromium applies the CSS
/// shorthand (the SVG primitive's linearRGB default does not apply there).
pub fn saturation_matrix(s: f32) -> [[f32; 4]; 3] {
    [
        [0.213 + 0.787 * s, 0.715 - 0.715 * s, 0.072 - 0.072 * s, 0.0],
        [0.213 - 0.213 * s, 0.715 + 0.285 * s, 0.072 - 0.072 * s, 0.0],
        [0.213 - 0.213 * s, 0.715 - 0.715 * s, 0.072 + 0.928 * s, 0.0],
    ]
}

/// Mirrors `Params` in `kawase.wgsl` (64 bytes). One layout for every pass;
/// passes that do not saturate carry the identity matrix.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct KawaseParams {
    pub halfpixel: [f32; 2],
    pub offset: f32,
    pub _pad: f32,
    pub saturate: [[f32; 4]; 3],
}

impl KawaseParams {
    pub(crate) fn new(sampled: FrameSize, offset: f32, saturation: f32) -> Self {
        Self {
            halfpixel: kawase_halfpixel(sampled),
            offset,
            _pad: 0.0,
            saturate: saturation_matrix(saturation),
        }
    }

    /// The uniform's bytes, field by field in declaration order.
    pub(crate) fn to_bytes(self) -> [u8; 64] {
        let words = [self.halfpixel[0], self.halfpixel[1], self.offset, self._pad];
        let mut bytes = [0; 64];
        let fields = words.into_iter().chain(self.saturate.into_iter().flatten());
        for (chunk, word) in bytes.as_chunks_mut::<4>().0.iter_mut().zip(fields) {
            *chunk = word.to_ne_bytes();
        }
        bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::{offset_of, size_of};

    #[test]
    fn tier_table_matches_the_pack() {
        let table: Vec<_> = Tier::ALL
            .iter()
            .map(|t| (t.sigma_px(), t.saturation()))
            .collect();
        assert_eq!(table, [(16.0, 1.6), (30.0, 1.8), (44.0, 2.0)]);
    }

    #[test]
    fn tier_set_holds_what_was_added() {
        let set: TierSet = [Tier::Thick, Tier::Thin].into_iter().collect();
        assert!(set.contains(Tier::Thin) && set.contains(Tier::Thick));
        assert!(!set.contains(Tier::Regular));
        assert_eq!(set.iter().collect::<Vec<_>>(), [Tier::Thin, Tier::Thick]);
        assert!(TierSet::EMPTY.is_empty());
        assert!(!set.is_empty());
    }

    #[test]
    fn plan_is_monotonic_in_sigma() {
        let mut previous = kawase_plan(0.0, 1.0);
        for step in 1..=400 {
            let plan = kawase_plan(step as f32 * 0.25, 1.0);
            assert!(
                plan.levels > previous.levels
                    || (plan.levels == previous.levels && plan.offset >= previous.offset),
                "{previous:?} -> {plan:?} at step {step}"
            );
            previous = plan;
        }
    }

    #[test]
    fn plan_levels_are_clamped() {
        assert_eq!(
            kawase_plan(0.0, 1.0),
            KawasePlan {
                levels: 1,
                offset: 1.0
            }
        );
        assert_eq!(kawase_plan(-5.0, 1.0).levels, 1);
        assert_eq!(
            kawase_plan(1000.0, 2.0),
            KawasePlan {
                levels: MAX_LEVELS,
                offset: 8.0
            }
        );
        for tier in Tier::ALL {
            let plan = kawase_plan(tier.sigma_px(), 1.0);
            assert!((1..=MAX_LEVELS).contains(&plan.levels));
        }
    }

    #[test]
    fn plan_is_scale_aware() {
        // Thin at scale 1 reaches 48 px (third row); at scale 2, 96 px (fourth).
        assert_eq!(kawase_plan(Tier::Thin.sigma_px(), 1.0).levels, 3);
        assert_eq!(kawase_plan(Tier::Thin.sigma_px(), 2.0).levels, 4);
        assert_eq!(kawase_plan(8.0, 2.0), kawase_plan(16.0, 1.0));
    }

    #[test]
    fn level_sizes_halve_and_never_vanish() {
        let frame = FrameSize::new(1001, 600);
        assert_eq!(kawase_level_size(frame, 0), frame);
        assert_eq!(kawase_level_size(frame, 1), FrameSize::new(500, 300));
        assert_eq!(kawase_level_size(frame, 4), FrameSize::new(62, 37));
        assert_eq!(
            kawase_level_size(FrameSize::new(3, 1), 5),
            FrameSize::new(1, 1)
        );
        assert_eq!(kawase_level_size(frame, 40), FrameSize::new(1, 1));
    }

    #[test]
    fn halfpixel_is_half_a_texel() {
        assert_eq!(kawase_halfpixel(FrameSize::new(100, 50)), [0.005, 0.01]);
    }

    #[test]
    fn saturation_matrix_identity_and_grey() {
        let close = |a: f32, b: f32| (a - b).abs() < 1e-6;
        let identity = saturation_matrix(1.0);
        for (r, row) in identity.iter().enumerate() {
            for (c, v) in row.iter().take(3).enumerate() {
                assert!(close(*v, if r == c { 1.0 } else { 0.0 }), "{identity:?}");
            }
        }
        for row in saturation_matrix(0.0) {
            assert_eq!(&row[..3], &[0.213, 0.715, 0.072]);
        }
        for s in [0.0, 0.5, 1.0, 1.6, 1.8, 2.0, 3.0] {
            for row in saturation_matrix(s) {
                assert!(close(row[0] + row[1] + row[2], 1.0), "s={s}");
                assert_eq!(row[3], 0.0);
            }
        }
    }

    #[test]
    fn params_layout_matches_wgsl() {
        assert_eq!(size_of::<KawaseParams>(), 64);
        assert_eq!(offset_of!(KawaseParams, halfpixel), 0);
        assert_eq!(offset_of!(KawaseParams, offset), 8);
        assert_eq!(offset_of!(KawaseParams, saturate), 16);
        let params = KawaseParams::new(FrameSize::new(4, 2), 3.0, 2.0);
        let bytes = params.to_bytes();
        assert_eq!(bytes.len(), size_of::<KawaseParams>());
        assert_eq!(bytes[8..12], 3.0f32.to_ne_bytes());
        assert_eq!(bytes[16..20], saturation_matrix(2.0)[0][0].to_ne_bytes());
    }
}
