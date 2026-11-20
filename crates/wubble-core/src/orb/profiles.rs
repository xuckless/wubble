// translates: ../ultra-reactivate/vendor/thinking-orbs/engine/profiles.ts:12-101
// Source: Ultra Reactivate's vendored thinking orbs (xuckless/the-wallpaper-project @ 598f814,
// src/ultra-reactivate/vendor/thinking-orbs), from Libraries.dev (MIT, (c) 2026 Jakub Antalik).
// The base profiles themselves are generated (`kit_gen::orb::BASE_PROFILES`).

//! The multipliers that scale a mode's profile: counts (grid pairs by the square root, so the
//! total scales by the factor) and radii (every radius key, so near/far falloff keeps its
//! proportion).

use super::{ModeOpts, js_round};

// 2-D lattices come in pairs: each side takes √scale so the total scales by `scale`.
const COUNT_PAIRS: &[(&str, &str)] = &[
    ("latRings", "lonDensity"),
    ("rings", "lonDensity"),
    ("lanes", "segs"),
];
const COUNT_KEYS: &[&str] = &["orbitN", "ghostN", "nodeN", "strandN", "signals"];
const ICON_DENSITY_KEYS: &[&str] = &["iconD"];

// Every key that sets a dot's rendered radius.
const RADIUS_KEYS: &[&str] = &[
    "rBase",
    "rDepth",
    "rActive",
    "rDot",
    "ghostR",
    "partR",
    "partRDepth",
    "nodeR",
    "nodeRDepth",
];

pub fn scale_counts(opts: &ModeOpts, scale: f64) -> ModeOpts {
    let mut out = opts.clone();
    let mut done: Vec<&str> = vec![];
    let rt = f64::sqrt(scale);
    for &(a, b) in COUNT_PAIRS {
        let va = out.get(a);
        let vb = out.get(b);
        if let (Some(va), Some(vb)) = (va, vb)
            && !done.contains(&a)
            && !done.contains(&b)
        {
            out.set(a, f64::max(2.0, js_round(va * rt)));
            out.set(b, f64::max(2.0, js_round(vb * rt)));
            done.push(a);
            done.push(b);
        }
    }
    for &k in COUNT_KEYS {
        let v = out.get(k);
        // 0 means the mode opted out of that layer entirely (ring has no ghost sphere)
        if let Some(v) = v
            && v != 0.0
            && !done.contains(&k)
        {
            out.set(k, f64::max(1.0, js_round(v * scale)));
        }
    }
    for &k in ICON_DENSITY_KEYS {
        let v = out.get(k);
        if let Some(v) = v {
            out.set(k, f64::max(0.02, v * scale));
        }
    }
    out
}

pub fn scale_radii(opts: &ModeOpts, scale: f64) -> ModeOpts {
    let mut out = opts.clone();
    for &k in RADIUS_KEYS {
        let v = out.get(k);
        if let Some(v) = v {
            out.set(k, v * scale);
        }
    }
    // remember the multiplier itself: spacing-derived radii (the morph outline) use it
    out.set("rSizeMul", out.get("rSizeMul").unwrap_or(1.0) * scale);
    out
}

/// How many dots a resolved profile draws: grid pairs as their product, the other counts as
/// themselves (exactly the knobs `scale_counts` treats as counts).
pub fn count_dots(opts: &ModeOpts) -> f64 {
    let mut total = 0.0;
    let mut done: Vec<&str> = vec![];
    for &(a, b) in COUNT_PAIRS {
        let va = opts.get(a);
        let vb = opts.get(b);
        if let (Some(va), Some(vb)) = (va, vb)
            && !done.contains(&a)
            && !done.contains(&b)
        {
            total += va * vb;
            done.push(a);
            done.push(b);
        }
    }
    for &k in COUNT_KEYS {
        let v = opts.get(k);
        if let Some(v) = v
            && v != 0.0
            && !done.contains(&k)
        {
            total += v;
        }
    }
    js_round(total)
}
