// translates: ../ultra-reactivate/vendor/thinking-orbs/presets.ts:99-114
// Source: Ultra Reactivate's vendored thinking orbs (xuckless/the-wallpaper-project @ 598f814,
// src/ultra-reactivate/vendor/thinking-orbs), from Libraries.dev (MIT, (c) 2026 Jakub Antalik).
// The tables are generated (`kit_gen::orb::{STATE_TO_MODE, PRESETS, BASE_PROFILES}`).

//! A (state, size) pair resolved to its mode, its speed and its fully scaled draw options.

use super::profiles::{scale_counts, scale_radii};
use super::{ModeKey, ModeOpts, OrbSize, OrbState};
use crate::kit_gen::orb::{BASE_PROFILES, PRESETS};

/// What a (state, size) pair draws: the mode, its baked speed and its options.
#[derive(Clone, Debug, PartialEq)]
pub struct Resolved {
    pub mode: ModeKey,
    pub speed: f64,
    pub opts: ModeOpts,
}

/// Resolve a (state, size) pair to its mode + fully scaled draw options.
pub fn resolve_preset(state: OrbState, size: OrbSize) -> Resolved {
    let mode = state.mode();
    let (_, _, speed, count, size_mul, extra) = PRESETS
        .iter()
        .find(|&&(m, px, ..)| m == mode.name() && px == size.px())
        .copied()
        .expect("every mode has a preset per size (kit_gen::orb::PRESETS)");
    let (_, base) = BASE_PROFILES
        .iter()
        .find(|&&(m, _)| m == mode.name())
        .copied()
        .expect("every mode has a base profile (kit_gen::orb::BASE_PROFILES)");
    let mut opts = ModeOpts::of(base);
    if count != 1.0 {
        opts = scale_counts(&opts, count);
    }
    if size_mul != 1.0 {
        opts = scale_radii(&opts, size_mul);
    }
    opts.merge(extra);

    Resolved { mode, speed, opts }
}
