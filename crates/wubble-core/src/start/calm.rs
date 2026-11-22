//! When the start page's bubble and liquid come to rest. A wubble deviation from the web design,
//! whose loop never settles (`core/living.ts:115`); the owner's decision (2026-11-21): "Settle
//! after a few seconds, Just keep the orb reactive and settle the liquid. You can also make the
//! On click waves disappear."
//!
//! - **Live**: everything runs at its pace; seconds without input are counted.
//! - **Settling**: after [`IDLE_AFTER`] seconds without input, the time scale of the bubble, its
//!   minis and the liquid eases from 1 to 0 over [`SETTLE`] seconds ([`EASE`]): everything
//!   slows to a stop where it is, no snap. The click waves and pops age at the real rate, so
//!   they finish and are gone; rest waits for them.
//! - **Rest**: nothing moves, nothing is stepped: the renderer stops asking for frames.
//!
//! Any input (a pointer, a key, the page or window shown again, a resize) is [`Calm::Live`] at
//! once, with the time scale 1 on the same frame. Durations are the web's idle span
//! (`kit_gen::living::idle_after`, its `auto` mode's) for both; the curve is the design's
//! `inOut` ease (`kit_gen::tokens::motion::ease::inOut`): both owner-reviewable
//! (docs/fidelity/derived-review.md).

use crate::kit_gen::living as K;
use crate::kit_gen::tokens::motion::ease;

/// Seconds without input before the page settles.
pub const IDLE_AFTER: f64 = K::idle_after;
/// Seconds the settle takes.
pub const SETTLE: f64 = K::idle_after;
/// The settle's curve (CSS `cubic-bezier`): the time scale is `1 - EASE(t / SETTLE)`.
pub const EASE: [f64; 4] = ease::inOut;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Calm {
    /// `idle`: seconds since the last input.
    Live {
        idle: f64,
    },
    /// `t`: seconds into the settle.
    Settling {
        t: f64,
    },
    Rest,
}

impl Default for Calm {
    fn default() -> Self {
        Calm::Live { idle: 0.0 }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CalmEvent {
    /// The user touched the page or the window, or it was shown again.
    Input,
    /// `dt` seconds passed. `busy`: something still moves that must finish first (the body
    /// held under the pointer, a click wave, a pop).
    Tick { dt: f64, busy: bool },
}

/// One event. Pure.
pub fn transition(c: Calm, e: CalmEvent) -> Calm {
    match (c, e) {
        (_, CalmEvent::Input) => Calm::Live { idle: 0.0 },
        (Calm::Live { idle }, CalmEvent::Tick { dt, busy }) => {
            let idle = idle + dt.max(0.0);
            if idle >= IDLE_AFTER && !busy {
                Calm::Settling { t: 0.0 }
            } else {
                Calm::Live { idle }
            }
        }
        (Calm::Settling { t }, CalmEvent::Tick { dt, busy }) => {
            let t = (t + dt.max(0.0)).min(SETTLE);
            if t >= SETTLE && !busy {
                Calm::Rest
            } else {
                Calm::Settling { t }
            }
        }
        (Calm::Rest, CalmEvent::Tick { .. }) => Calm::Rest,
    }
}

impl Calm {
    /// The time scale of the bubble, its minis and the liquid: 1 live, easing to 0 settling,
    /// 0 at rest.
    pub fn scale(&self) -> f64 {
        match *self {
            Calm::Live { .. } => 1.0,
            Calm::Settling { t } => 1.0 - crate::motion::bezier(EASE, t / SETTLE),
            Calm::Rest => 0.0,
        }
    }

    pub fn resting(&self) -> bool {
        *self == Calm::Rest
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DT: f64 = 1.0 / 120.0;

    fn run(mut c: Calm, seconds: f64, busy: bool) -> Calm {
        for _ in 0..(seconds / DT).round() as usize {
            c = transition(c, CalmEvent::Tick { dt: DT, busy });
        }
        c
    }

    #[test]
    fn it_settles_after_the_idle_span_and_rests_after_the_settle() {
        let c = run(Calm::default(), IDLE_AFTER - 0.1, false);
        assert!(matches!(c, Calm::Live { .. }));
        assert_eq!(c.scale(), 1.0);
        let c = run(c, 0.2, false);
        assert!(matches!(c, Calm::Settling { .. }), "{c:?}");
        let c = run(c, SETTLE - 0.2, false);
        assert!(matches!(c, Calm::Settling { .. }), "{c:?}");
        let c = run(c, 0.3, false);
        assert_eq!(c, Calm::Rest);
        assert_eq!(c.scale(), 0.0);
        // at rest it stays at rest
        assert_eq!(run(c, 10.0, false), Calm::Rest);
    }

    #[test]
    fn the_settle_eases_down_without_a_step() {
        let mut c = Calm::Settling { t: 0.0 };
        let mut last = c.scale();
        assert_eq!(last, 1.0);
        while c != Calm::Rest {
            c = transition(
                c,
                CalmEvent::Tick {
                    dt: DT,
                    busy: false,
                },
            );
            let k = c.scale();
            assert!(k <= last, "never speeds up: {k} after {last}");
            // one frame never drops the scale by more than a small step (no snap)
            assert!(last - k < 0.02, "{last} -> {k}");
            last = k;
        }
        assert_eq!(last, 0.0);
    }

    #[test]
    fn input_wakes_it_at_once_from_every_state() {
        for c in [
            Calm::Live { idle: 1.5 },
            Calm::Settling { t: 0.7 },
            Calm::Rest,
        ] {
            let c = transition(c, CalmEvent::Input);
            assert_eq!(c, Calm::Live { idle: 0.0 });
            assert_eq!(c.scale(), 1.0);
        }
        // input that keeps coming keeps it live
        let mut c = Calm::default();
        for _ in 0..10 {
            c = run(c, IDLE_AFTER * 0.9, false);
            c = transition(c, CalmEvent::Input);
        }
        assert_eq!(c, Calm::Live { idle: 0.0 });
    }

    #[test]
    fn what_must_finish_holds_off_the_settle_and_the_rest() {
        // held, or a wave or pop in flight: not settling
        let c = run(Calm::default(), IDLE_AFTER * 3.0, true);
        assert!(matches!(c, Calm::Live { .. }));
        // it settles the frame they end
        assert!(matches!(run(c, DT, false), Calm::Settling { .. }));
        // a pop in flight at the end of the settle: it rests when the pop is gone
        let c = run(Calm::Settling { t: 0.0 }, SETTLE * 2.0, true);
        assert_eq!(c, Calm::Settling { t: SETTLE });
        assert_eq!(c.scale(), 0.0);
        assert_eq!(run(c, DT, false), Calm::Rest);
    }
}
