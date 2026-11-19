//! The settings swatches' ring (`Swatches`, `pages/settings.tsx:263-300` @598f814, the owner's
//! design repo): a ring that travels between the swatches on the follow spring and stretches
//! along its travel (volume kept), its trailing side as the origin; the same body as the tab
//! indicator. No lifecycle beyond the spring: the chosen index is the state.

pub use super::segmented::key_select;
use crate::kit_gen::physics::K;
use crate::kit_gen::settings::SWATCH;
use crate::motion::{Spring, Stretch, spring, squash_of, step_k};

/// What the ring draws.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RingView {
    /// The ring's left, px from the first swatch's.
    pub x: f64,
    pub scale_x: f64,
    pub scale_y: f64,
    /// Transform origin across the ring: 0.3 moving right, 0.7 moving left.
    pub origin: f64,
}

#[derive(Clone, Debug)]
pub struct Ring {
    x: Spring,
    st: Stretch,
    idx: usize,
    /// How many swatches there are (for the keys).
    n: usize,
}

impl Ring {
    pub fn new(idx: usize) -> Self {
        Self {
            x: spring(idx as f64 * SWATCH::pitch),
            st: Stretch::new(),
            idx,
            n: idx + 1,
        }
    }

    pub fn set_count(&mut self, n: usize) {
        self.n = n;
    }

    /// ArrowLeft (`dir` < 0) or ArrowRight: the neighbour, clamped.
    pub fn key(&mut self, dir: i32) -> usize {
        self.idx = key_select(self.idx, self.n, dir);
        self.idx
    }

    /// A swatch chosen (a click, a key, the setting): the ring travels there.
    pub fn set(&mut self, idx: usize) {
        self.idx = idx;
    }

    pub fn idx(&self) -> usize {
        self.idx
    }

    pub fn step(&mut self, dt: f64) -> bool {
        let moving = step_k(&mut self.x, self.idx as f64 * SWATCH::pitch, K::follow, dt);
        self.st.update(self.x.x, dt) || moving
    }

    pub fn view(&self) -> RingView {
        let sx = self.st.x().min(1.0 + SWATCH::stretchMax);
        RingView {
            x: self.x.x,
            scale_x: sx,
            scale_y: squash_of(sx),
            origin: if self.st.dir > 0.0 { 0.3 } else { 0.7 },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DT: f64 = 1.0 / 120.0;

    #[test]
    fn the_ring_travels_and_stretches_from_its_trailing_side() {
        let mut r = Ring::new(0);
        assert_eq!(r.view().x, 0.0);
        r.set(4);
        let mut most: f64 = 1.0;
        let mut n = 0;
        let mut origin_right = None;
        while r.step(DT) {
            let v = r.view();
            most = most.max(v.scale_x);
            assert_eq!(v.scale_y, squash_of(v.scale_x));
            origin_right.get_or_insert(v.origin);
            n += 1;
            assert!(n < 20_000);
        }
        assert_eq!(origin_right, Some(0.3), "moving right: origin at 30%");
        assert!(most > 1.05 && most <= 1.0 + SWATCH::stretchMax, "{most}");
        assert_eq!(r.view().x, 4.0 * SWATCH::pitch);
        r.set(1);
        r.step(DT);
        r.step(DT);
        assert_eq!(r.view().origin, 0.7, "moving left: origin at 70%");
    }

    #[test]
    fn keys_clamp_at_both_ends() {
        assert_eq!(key_select(0, 6, -1), 0);
        assert_eq!(key_select(5, 6, 1), 5);
        assert_eq!(key_select(3, 6, 1), 4);
        let mut r = Ring::new(0);
        r.set_count(3);
        assert_eq!((r.key(1), r.key(1), r.key(1)), (1, 2, 2));
        assert_eq!(r.key(-1), 1);
    }
}
