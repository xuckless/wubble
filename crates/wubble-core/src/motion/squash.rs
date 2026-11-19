//! The squash token, the one keyframe table the controls share (toggle landing, buttons, check
//! boxes). Plain constants with no imports: `wubble-ui`'s build script includes this file to write
//! the same keys into the Slint squash (`squash-keys.slint`).

// wubble change: ported from `pages/touch-kit.tsx` `SQUASH` / `squashAt` (@source touch-squash).
/// The squash token as keys of `(t, sx, sy)` over [`SQUASH_MS`]:
/// 1.30×0.78 → 0.93×1.07 → 1.03×0.98 (README › Motion tokens › Squash).
pub const SQUASH_KEYS: [(f64, f64, f64); 5] = [
    (0.0, 1.0, 1.0),
    (0.28, 1.3, 0.78),
    (0.58, 0.93, 1.07),
    (0.8, 1.03, 0.98),
    (1.0, 1.0, 1.0),
];
/// The squash token's length.
pub const SQUASH_MS: f64 = 500.0;
