//! Middle-click autoscroll's pointer: a dot while the page holds still, an
//! arrow toward the scroll while it moves, stretched with the speed (the
//! design's stretch spring, `motion::Stretch`).
//!
//! Chromium runs the autoscroll itself and says so through the cursor it
//! asks for (`autoscroll_controller.cc` `MiddleClickAutoscrollCursor`: middle
//! panning while still, a compass panning cursor while scrolling). Those
//! cursor changes drive [`transition`]; the pointer only moves the arrow.
//! The speed is Chromium's own curve ([`speed`]), so the stretch follows the
//! scroll the page actually does.

/// Pixels (CSS) around the anchor that do not scroll, per axis: Chromium's
/// `kNoMiddleClickAutoscrollRadius`.
pub const DEAD_ZONE: f64 = 15.0;
/// Chromium's curve: speed grows with the distance past the anchor to this
/// power (`kExponent`) ...
const EXPONENT: f64 = 2.2;
/// ... times this (`kMultiplier`, per millisecond; here per second).
const MULTIPLIER: f64 = 0.000_008 * 1000.0;

/// Where the page scrolls: the pointer's side of the anchor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    N,
    NE,
    E,
    SE,
    S,
    SW,
    W,
    NW,
}

impl Direction {
    /// Clockwise degrees from pointing down (south), for an arrow drawn
    /// pointing down.
    pub fn degrees(self) -> f32 {
        match self {
            Direction::S => 0.0,
            Direction::SW => 45.0,
            Direction::W => 90.0,
            Direction::NW => 135.0,
            Direction::N => 180.0,
            Direction::NE => 225.0,
            Direction::E => 270.0,
            Direction::SE => 315.0,
        }
    }
}

/// What the page's cursor says about autoscroll.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pan {
    /// Not a panning cursor: autoscroll is off.
    None,
    /// Middle panning: autoscroll is on, the page still.
    Still,
    /// A compass panning cursor: scrolling this way.
    Moving(Direction),
}

/// A point in window logical pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct At {
    pub x: f64,
    pub y: f64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Autoscroll {
    #[default]
    Off,
    /// On, the page still; `anchor` is where it started.
    Still {
        anchor: At,
    },
    Moving {
        anchor: At,
        direction: Direction,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Input {
    /// The page asked for this cursor.
    Cursor(Pan),
    /// A mouse button or a key went down. Chromium stops autoscroll on either
    /// (`EventHandler`), and asks for the page's cursor again only when the
    /// pointer next moves, so the press itself ends it here.
    Pressed,
}

/// `input` arrived with the pointer at `pointer`; the anchor is where the
/// pointer was when autoscroll began. Chromium starts every autoscroll still
/// (`MiddleClickAutoscrollCursor` is middle panning until the pointer leaves
/// the dead zone), so only that cursor starts one: a compass cursor while off
/// is a late answer to a move made before a press ended the last one.
pub fn transition(state: Autoscroll, input: Input, pointer: At) -> Autoscroll {
    let anchor = match state {
        Autoscroll::Off => pointer,
        Autoscroll::Still { anchor } | Autoscroll::Moving { anchor, .. } => anchor,
    };
    match (state, input) {
        (_, Input::Pressed | Input::Cursor(Pan::None)) => Autoscroll::Off,
        (_, Input::Cursor(Pan::Still)) => Autoscroll::Still { anchor },
        (Autoscroll::Off, Input::Cursor(Pan::Moving(_))) => Autoscroll::Off,
        (_, Input::Cursor(Pan::Moving(direction))) => Autoscroll::Moving { anchor, direction },
    }
}

impl Autoscroll {
    pub fn anchor(self) -> Option<At> {
        match self {
            Autoscroll::Off => None,
            Autoscroll::Still { anchor } | Autoscroll::Moving { anchor, .. } => Some(anchor),
        }
    }
}

/// The scroll speed (px/s, per axis, signed like the offset) for the pointer
/// `offset` from the anchor in CSS px: Chromium's curve, zero inside the dead
/// zone.
pub fn speed(offset: At) -> At {
    let axis = |d: f64| {
        if d.abs() <= DEAD_ZONE {
            0.0
        } else {
            d.signum() * d.abs().powf(EXPONENT) * MULTIPLIER
        }
    };
    At {
        x: axis(offset.x),
        y: axis(offset.y),
    }
}

/// How fast the page scrolls (px/s) with the pointer at `pointer`; zero
/// while autoscroll is off or still. `zoom` is CSS px per logical px.
pub fn scroll_speed(state: Autoscroll, pointer: At, zoom: f64) -> f64 {
    let Autoscroll::Moving { anchor, .. } = state else {
        return 0.0;
    };
    let v = speed(At {
        x: (pointer.x - anchor.x) / zoom,
        y: (pointer.y - anchor.y) / zoom,
    });
    v.x.hypot(v.y)
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: At = At { x: 100.0, y: 100.0 };

    #[test]
    fn the_anchor_is_where_autoscroll_began() {
        let s = transition(Autoscroll::Off, Input::Cursor(Pan::Still), A);
        assert_eq!(s, Autoscroll::Still { anchor: A });
        let below = At { x: 100.0, y: 300.0 };
        let s = transition(s, Input::Cursor(Pan::Moving(Direction::S)), below);
        assert_eq!(
            s,
            Autoscroll::Moving {
                anchor: A,
                direction: Direction::S
            }
        );
        // Back to still keeps the anchor; any other cursor ends it.
        assert_eq!(
            transition(s, Input::Cursor(Pan::Still), below),
            Autoscroll::Still { anchor: A }
        );
        assert_eq!(
            transition(s, Input::Cursor(Pan::None), below),
            Autoscroll::Off
        );
        assert_eq!(
            transition(Autoscroll::Off, Input::Cursor(Pan::None), A),
            Autoscroll::Off
        );
    }

    #[test]
    fn a_press_ends_autoscroll() {
        let s = transition(Autoscroll::Off, Input::Cursor(Pan::Still), A);
        assert_eq!(transition(s, Input::Pressed, A), Autoscroll::Off);
        assert_eq!(
            transition(Autoscroll::Off, Input::Pressed, A),
            Autoscroll::Off
        );
    }

    #[test]
    fn a_compass_cursor_after_the_end_does_not_start_it_again() {
        let s = transition(Autoscroll::Off, Input::Cursor(Pan::Still), A);
        let s = transition(s, Input::Pressed, A);
        let late = transition(s, Input::Cursor(Pan::Moving(Direction::SW)), A);
        assert_eq!(late, Autoscroll::Off);
    }

    #[test]
    fn the_dead_zone_does_not_scroll() {
        assert_eq!(speed(At { x: 15.0, y: -15.0 }), At { x: 0.0, y: 0.0 });
        let v = speed(At { x: 0.0, y: 16.0 });
        assert!(v.y > 0.0 && v.x == 0.0);
    }

    #[test]
    fn speed_follows_chromiums_curve() {
        // 0.008 px/s * d^2.2: about 200 px/s at 100 px, 1500 px/s near 250 px.
        let at = |d: f64| speed(At { x: 0.0, y: d }).y;
        assert!((at(100.0) - 0.008 * 100f64.powf(2.2)).abs() < 1e-9);
        assert!((195.0..210.0).contains(&at(100.0)), "{}", at(100.0));
        assert!((1400.0..1600.0).contains(&at(250.0)), "{}", at(250.0));
        assert_eq!(at(-100.0), -at(100.0));
    }

    #[test]
    fn scroll_speed_is_zero_unless_moving_and_scales_with_zoom() {
        let p = At { x: 100.0, y: 300.0 };
        assert_eq!(scroll_speed(Autoscroll::Off, p, 1.0), 0.0);
        assert_eq!(scroll_speed(Autoscroll::Still { anchor: A }, p, 1.0), 0.0);
        let moving = Autoscroll::Moving {
            anchor: A,
            direction: Direction::S,
        };
        let plain = scroll_speed(moving, p, 1.0);
        assert!((plain - speed(At { x: 0.0, y: 200.0 }).y).abs() < 1e-9);
        // At 200% the same logical distance is half as many CSS px.
        assert!(scroll_speed(moving, p, 2.0) < plain);
    }

    #[test]
    fn arrows_turn_clockwise_from_down() {
        assert_eq!(Direction::S.degrees(), 0.0);
        assert_eq!(Direction::N.degrees(), 180.0);
        assert_eq!(Direction::W.degrees(), 90.0);
        assert_eq!(Direction::E.degrees(), 270.0);
    }
}
