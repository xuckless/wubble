// Source: the owner's design repo (xuckless/the-wallpaper-project @ 598f814),
// `src/wubble/slint/minis.rs` (`Rng`, @source slint-rng), with the marked `wubble change`s.

//! `Math.random()` for the start page's machines. The web draws from one global stream; here the
//! draws go through one `Random` the `Living` owns, in the web's call order, so a run is
//! reproducible and the tests can replay the web's own draws.

/// A source of numbers in [0, 1), standing in for `Math.random()`.
pub trait Random {
    fn random(&mut self) -> f64;
}

// @source slint-rng
/// xorshift64* in [0, 1).
#[derive(Clone, Debug)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng(seed.max(1))
    }
}

// wubble change: the port's inherent `random` is the `Random` impl.
impl Random for Rng {
    fn random(&mut self) -> f64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        (x.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 11) as f64 / (1u64 << 53) as f64
    }
}

/// wubble change: a recorded list of draws, returned in order (the web's `Math.random()` values
/// stored in a fixture). Running out is a test bug: it panics.
#[cfg(test)]
#[derive(Clone, Debug)]
pub struct Replay {
    values: Vec<f64>,
    at: usize,
}

#[cfg(test)]
impl Replay {
    pub fn new(values: Vec<f64>) -> Self {
        Replay { values, at: 0 }
    }

    /// How many draws were taken.
    pub fn taken(&self) -> usize {
        self.at
    }
}

#[cfg(test)]
impl Random for Replay {
    fn random(&mut self) -> f64 {
        let v = *self
            .values
            .get(self.at)
            .unwrap_or_else(|| panic!("replay ran out after {} draws", self.at));
        self.at += 1;
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rng_is_deterministic_and_in_range() {
        let (mut a, mut b) = (Rng::new(42), Rng::new(42));
        for _ in 0..1000 {
            let v = a.random();
            assert_eq!(v, b.random());
            assert!((0.0..1.0).contains(&v));
        }
        assert_ne!(Rng::new(1).random(), Rng::new(2).random());
        // a zero seed would stick at zero: it is moved to 1
        assert_eq!(Rng::new(0).random(), Rng::new(1).random());
    }

    #[test]
    fn replay_returns_the_list_in_order() {
        let mut r = Replay::new(vec![0.25, 0.5, 0.75]);
        assert_eq!([r.random(), r.random(), r.random()], [0.25, 0.5, 0.75]);
        assert_eq!(r.taken(), 3);
    }

    #[test]
    #[should_panic(expected = "ran out")]
    fn replay_panics_when_empty() {
        Replay::new(vec![]).random();
    }
}
