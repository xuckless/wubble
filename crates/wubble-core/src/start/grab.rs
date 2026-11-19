//! The pointer's hold on the soft body: free, held (a tap so far) or dragging. One enum and one
//! pure transition in place of the web's `held` / `dragging` booleans (`core/softbody.ts:30`,
//! `down`, `move`, `up`, `leave` at :66-100). The soft body owns the state and applies the
//! effects (the fling or the poke, forgetting the pointer).

use crate::kit_gen::softbody as K;

/// Where the press started and where on the body it caught (relative to the centre), px.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hold {
    pub start: (f64, f64),
    pub grab: (f64, f64),
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Grab {
    #[default]
    Free,
    /// Pressed on the body, moved at most `drag_threshold` (|dx| + |dy|) so far.
    Held(Hold),
    Dragging(Hold),
}

impl Grab {
    /// The web's `held`: pressed, dragging or not.
    pub fn held(&self) -> bool {
        !matches!(self, Grab::Free)
    }

    pub fn dragging(&self) -> Option<&Hold> {
        match self {
            Grab::Dragging(h) => Some(h),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum GrabEvent {
    /// A press on the body at (x, y); `centre` is the body's centre now.
    Down {
        x: f64,
        y: f64,
        centre: (f64, f64),
    },
    Move {
        x: f64,
        y: f64,
    },
    Up,
    Leave,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GrabEffect {
    /// A tap released: poke where the pointer is (`up()` returning 'poke').
    Poke,
    /// A drag released: throw the body with the pointer's velocity (`up()` returning 'fling').
    Fling,
    /// The pointer left while nothing is held: forget it (`leave()`).
    ForgetPointer,
}

/// `softbody.ts` `down` / `move` / `up` / `leave`, as a pure step.
pub fn transition(g: Grab, e: GrabEvent) -> (Grab, Option<GrabEffect>) {
    match (g, e) {
        (_, GrabEvent::Down { x, y, centre }) => (
            Grab::Held(Hold {
                start: (x, y),
                grab: (x - centre.0, y - centre.1),
            }),
            None,
        ),
        (Grab::Held(h), GrabEvent::Move { x, y })
            if (x - h.start.0).abs() + (y - h.start.1).abs() > K::drag_threshold =>
        {
            (Grab::Dragging(h), None)
        }
        (g, GrabEvent::Move { .. }) => (g, None),
        (Grab::Free, GrabEvent::Up) => (Grab::Free, None),
        (Grab::Held(_), GrabEvent::Up) => (Grab::Free, Some(GrabEffect::Poke)),
        (Grab::Dragging(_), GrabEvent::Up) => (Grab::Free, Some(GrabEffect::Fling)),
        (Grab::Free, GrabEvent::Leave) => (Grab::Free, Some(GrabEffect::ForgetPointer)),
        (g, GrabEvent::Leave) => (g, None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOWN: GrabEvent = GrabEvent::Down {
        x: 100.0,
        y: 50.0,
        centre: (90.0, 40.0),
    };
    const HOLD: Hold = Hold {
        start: (100.0, 50.0),
        grab: (10.0, 10.0),
    };

    #[test]
    fn the_transition_table() {
        let mv = |x, y| GrabEvent::Move { x, y };
        let cases = [
            (Grab::Free, DOWN, Grab::Held(HOLD), None),
            (Grab::Dragging(HOLD), DOWN, Grab::Held(HOLD), None),
            // 8 px (|dx| + |dy|) is still a tap; past it, a drag
            (Grab::Held(HOLD), mv(105.0, 53.0), Grab::Held(HOLD), None),
            (
                Grab::Held(HOLD),
                mv(105.0, 54.0),
                Grab::Dragging(HOLD),
                None,
            ),
            (Grab::Held(HOLD), mv(91.0, 50.0), Grab::Dragging(HOLD), None),
            (
                Grab::Dragging(HOLD),
                mv(100.0, 50.0),
                Grab::Dragging(HOLD),
                None,
            ),
            (Grab::Free, mv(500.0, 500.0), Grab::Free, None),
            (
                Grab::Held(HOLD),
                GrabEvent::Up,
                Grab::Free,
                Some(GrabEffect::Poke),
            ),
            (
                Grab::Dragging(HOLD),
                GrabEvent::Up,
                Grab::Free,
                Some(GrabEffect::Fling),
            ),
            (Grab::Free, GrabEvent::Up, Grab::Free, None),
            (
                Grab::Free,
                GrabEvent::Leave,
                Grab::Free,
                Some(GrabEffect::ForgetPointer),
            ),
            (Grab::Held(HOLD), GrabEvent::Leave, Grab::Held(HOLD), None),
            (
                Grab::Dragging(HOLD),
                GrabEvent::Leave,
                Grab::Dragging(HOLD),
                None,
            ),
        ];
        for (from, event, to, effect) in cases {
            assert_eq!(
                transition(from, event),
                (to, effect),
                "{from:?} + {event:?}"
            );
        }
    }

    #[test]
    fn held_covers_both_pressed_states() {
        assert!(!Grab::Free.held());
        assert!(Grab::Held(HOLD).held());
        assert!(Grab::Dragging(HOLD).held());
        assert_eq!(Grab::Dragging(HOLD).dragging(), Some(&HOLD));
        assert_eq!(Grab::Held(HOLD).dragging(), None);
    }
}
