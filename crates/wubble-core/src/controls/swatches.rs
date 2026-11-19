//! The settings swatches' ring (`Swatches`, `pages/settings.tsx:263-300` @598f814, the owner's
//! design repo): a ring that travels between the swatches on the follow spring and stretches
//! along its travel (volume kept), its trailing side as the origin; the same body as the tab
//! indicator. No lifecycle beyond the spring: the chosen index is the state.

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
}

impl Ring {
    pub fn new(idx: usize) -> Self {
        Self {
            x: spring(idx as f64 * SWATCH::pitch),
            st: Stretch::new(),
            idx,
        }
    }

    /// The value chose a swatch (a click, a key, the setting): the ring travels there. The
    /// control is controlled, as the TSX's: only the value moves the ring; a key or a click
    /// only asks for a value (the neighbour is `segmented::key_select`).
    pub fn set(&mut self, idx: usize) {
        self.idx = idx;
    }

    /// The chosen swatch was laid out (the control appeared): the ring starts there, as the
    /// TSX's `spring(idx * SWATCH.pitch)`, without travelling.
    pub fn place(&mut self, idx: usize) {
        *self = Self::new(idx);
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
            origin: self.st.origin(),
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
    fn placing_starts_the_ring_at_its_swatch_without_travel() {
        let mut r = Ring::new(0);
        r.place(3);
        assert_eq!((r.idx(), r.view().x), (3, 3.0 * SWATCH::pitch));
        assert!(!r.step(DT), "already there");
        // a later value travels
        r.set(1);
        assert!(r.step(DT));
        assert!(r.view().x > SWATCH::pitch);
    }
}
