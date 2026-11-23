//! Touch: the sidebar's Space swipe, the input kind that sizes the chrome, and which page the
//! fingers on a touchscreen belong to ([`Fingers`]). Pure rules; the host feeds the events,
//! with time passed in.
//!
//! Not in the design bucket (it has no Space swipe): written new. The new-gesture gap is the
//! scroller's (`browser/scroller.tsx` `newGestureMs`, 260 ms).

use crate::tab::TabId;

/// px of horizontal travel before a swipe switches the Space.
pub const SWIPE_THRESHOLD: f64 = 60.0;
/// ms without a scroll event after which the next one starts a new gesture.
pub const SWIPE_GAP_MS: f64 = 260.0;
/// A swipe's horizontal travel must be this many times its vertical travel, so the sideways
/// drift of a long list scroll never adds up to a swipe.
pub const SWIPE_AXIS_RATIO: f64 = 2.0;

/// A two-finger horizontal swipe on the sidebar: one Space per gesture, ends do not wrap.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpaceSwipe {
    acc: f64,
    /// The gesture's vertical travel, px (sum of |dy|).
    travel_y: f64,
    last: f64,
    /// This gesture already switched: the rest of it is ignored.
    spent: bool,
}

impl Default for SpaceSwipe {
    fn default() -> Self {
        Self {
            acc: 0.0,
            travel_y: 0.0,
            last: f64::NEG_INFINITY,
            spent: false,
        }
    }
}

impl SpaceSwipe {
    /// A scroll delta (`dx`, `dy`; px, web sign: negative `dx` scrolls left) at `now` ms. With
    /// the Space at `active` of `count`, the index to switch to, if this delta completes a swipe:
    /// a negative `dx` goes to the next Space, a positive one to the previous. A gesture whose
    /// horizontal travel is not [`SWIPE_AXIS_RATIO`] times its vertical travel is a list scroll.
    pub fn scroll(
        &mut self,
        dx: f64,
        dy: f64,
        now: f64,
        active: usize,
        count: usize,
    ) -> Option<usize> {
        if now - self.last > SWIPE_GAP_MS {
            *self = SpaceSwipe::default();
        }
        self.last = now;
        if self.spent {
            return None;
        }
        self.acc += dx;
        self.travel_y += dy.abs();
        if self.acc.abs() < SWIPE_THRESHOLD || self.acc.abs() < SWIPE_AXIS_RATIO * self.travel_y {
            return None;
        }
        self.spent = true;
        if self.acc < 0.0 {
            (active + 1 < count).then_some(active + 1)
        } else {
            active.checked_sub(1)
        }
    }
}

/// The kind of pointer the user is on: rows grow to 44 px under a finger (`Chrome.rowTouch`).
/// Not in the design bucket (it has the token only): written new.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum InputKind {
    #[default]
    Mouse,
    /// Last touched at this window position (logical px).
    Touch { x: f64, y: f64 },
}

/// A pointer event as the input kind sees it (window position, logical px).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum InputEvent {
    Touch { x: f64, y: f64 },
    MouseMove { x: f64, y: f64 },
}

/// A mouse move within this many px of the last touch point is the touch's own emulated mouse
/// event (Windows pen/touch compatibility, some X11 setups), not a real mouse.
pub const EMULATED_SLOP: f64 = 2.0;

/// The next input kind: a touch flips to touch, a real mouse move flips back.
pub fn next(kind: InputKind, event: InputEvent) -> InputKind {
    match (kind, event) {
        (_, InputEvent::Touch { x, y }) => InputKind::Touch { x, y },
        (InputKind::Touch { x: tx, y: ty }, InputEvent::MouseMove { x, y })
            if (x - tx).abs() <= EMULATED_SLOP && (y - ty).abs() <= EMULATED_SLOP =>
        {
            kind
        }
        (_, InputEvent::MouseMove { .. }) => InputKind::Mouse,
    }
}

/// The `ui.touch_sizing` setting: follow the input, or force one size.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Sizing {
    #[default]
    Auto,
    Touch,
    Mouse,
}

impl Sizing {
    /// From the setting's stored choice; unknown values are `Auto`.
    pub fn parse(s: &str) -> Self {
        match s {
            "touch" => Sizing::Touch,
            "mouse" => Sizing::Mouse,
            _ => Sizing::Auto,
        }
    }
}

/// Whether the chrome uses touch sizing (44 px rows).
pub fn is_touch(sizing: Sizing, kind: InputKind) -> bool {
    match sizing {
        Sizing::Touch => true,
        Sizing::Mouse => false,
        Sizing::Auto => matches!(kind, InputKind::Touch { .. }),
    }
}

/// One touch event's kind as a page gets it (CEF's `cef_touch_event_type_t`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TouchKind {
    Press,
    Move,
    Release,
    Cancel,
}

/// The pressure a page gets from a screen that reports none: the Pointer Events spec's 0.5 for
/// an active pointer without pressure hardware (W3C Pointer Events, `pressure`).
pub const UNKNOWN_PRESSURE: f32 = 0.5;

/// A finger down on a page.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Finger {
    /// The platform's id for the finger (winit's `Touch::id`).
    pub id: u64,
    /// The page's touch id: the lowest the other fingers leave free, so ids stay small and
    /// stable for the whole touch, as a touchscreen's are.
    pub slot: i32,
    /// Where it last was, window logical px: a cancel is sent there.
    pub at: (f32, f32),
}

/// The touchscreen over a window, as the pages see it. Slint hit-tests the first finger (its
/// press lands on a page's pane or on the shell above it: a menu, a sheet, the sidebar); a
/// finger that landed on a page is that page's until it lifts, wherever it moves (Chrome's
/// touch capture), and so is every finger that goes down while it is. A first finger that
/// landed on the shell is the shell's and is not listed.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Fingers {
    #[default]
    None,
    /// A first finger went down; Slint is placing it (for the length of its own event).
    Placing {
        id: u64,
        at: (f32, f32),
        pressure: f32,
    },
    /// Fingers (never none) on `pane`'s page, whose view's top left sits at `origin` in the
    /// window (logical px).
    Page {
        pane: TabId,
        origin: (f32, f32),
        down: Vec<Finger>,
    },
}

impl Fingers {
    /// A page holds fingers.
    pub fn on_page(&self) -> bool {
        matches!(self, Fingers::Page { .. })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum FingerInput {
    /// A finger went down at `at` (window logical px).
    Down {
        id: u64,
        at: (f32, f32),
        pressure: f32,
    },
    /// Slint's press for the finger it is placing reached `pane`'s page, at `point` in it.
    Landed {
        pane: TabId,
        point: (f32, f32),
    },
    Moved {
        id: u64,
        at: (f32, f32),
        pressure: f32,
    },
    Up {
        id: u64,
        at: (f32, f32),
    },
    /// The platform took the finger back (Wayland's `wl_touch.cancel`).
    Cancelled {
        id: u64,
    },
    /// The pages on screen now: fingers on any other page (closed, switched away, the window
    /// lost) are cancelled there, so no page keeps a finger that will never lift.
    OnScreen(Vec<TabId>),
}

/// One touch event for `pane`'s page: finger `slot`, at `point` in its view.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PageTouch {
    pub pane: TabId,
    pub slot: i32,
    pub kind: TouchKind,
    pub point: (f32, f32),
    pub pressure: f32,
}

/// Every row is an arm here, one per (state, input); add behaviour as an arm, not as a check
/// in the host before it.
pub fn transition(state: Fingers, input: FingerInput) -> (Fingers, Vec<PageTouch>) {
    use FingerInput as I;
    let send = |pane: TabId, origin: (f32, f32), f: &Finger, kind, pressure| PageTouch {
        pane,
        slot: f.slot,
        kind,
        point: (f.at.0 - origin.0, f.at.1 - origin.1),
        pressure,
    };
    match (state, input) {
        // The first finger: Slint places it.
        (Fingers::None | Fingers::Placing { .. }, I::Down { id, at, pressure }) => {
            (Fingers::Placing { id, at, pressure }, vec![])
        }
        (Fingers::Placing { id, at, pressure }, I::Landed { pane, point }) => {
            let origin = (at.0 - point.0, at.1 - point.1);
            let f = Finger { id, slot: 0, at };
            let press = send(pane, origin, &f, TouchKind::Press, pressure);
            (
                Fingers::Page {
                    pane,
                    origin,
                    down: vec![f],
                },
                vec![press],
            )
        }
        // Not placed on a page by the next event: the finger is the shell's (Slint's alone).
        (Fingers::None | Fingers::Placing { .. }, _) => (Fingers::None, vec![]),
        (
            Fingers::Page {
                pane,
                origin,
                mut down,
            },
            input,
        ) => {
            let held = down.iter().position(|f| match input {
                I::Down { id, .. }
                | I::Moved { id, .. }
                | I::Up { id, .. }
                | I::Cancelled { id } => f.id == id,
                I::Landed { .. } | I::OnScreen(_) => false,
            });
            let mut effects = vec![];
            match (input, held) {
                // A down for a finger still held (a lift the platform lost): it ends first.
                (I::Down { id, at, pressure }, held) => {
                    if let Some(i) = held {
                        let f = down.remove(i);
                        effects.push(send(pane, origin, &f, TouchKind::Cancel, 0.0));
                    }
                    let slot = (0..)
                        .find(|s| !down.iter().any(|f| f.slot == *s))
                        .unwrap_or(0);
                    let f = Finger { id, slot, at };
                    effects.push(send(pane, origin, &f, TouchKind::Press, pressure));
                    down.push(f);
                }
                (I::Moved { at, pressure, .. }, Some(i)) => {
                    down[i].at = at;
                    effects.push(send(pane, origin, &down[i], TouchKind::Move, pressure));
                }
                (I::Up { at, .. }, Some(i)) => {
                    let f = Finger {
                        at,
                        ..down.remove(i)
                    };
                    effects.push(send(pane, origin, &f, TouchKind::Release, 0.0));
                }
                (I::Cancelled { .. }, Some(i)) => {
                    let f = down.remove(i);
                    effects.push(send(pane, origin, &f, TouchKind::Cancel, 0.0));
                }
                (I::OnScreen(shown), _) if !shown.contains(&pane) => {
                    for f in down.drain(..) {
                        effects.push(send(pane, origin, &f, TouchKind::Cancel, 0.0));
                    }
                }
                // A finger that is not the page's (one the shell had before); Slint's press for
                // the finger left after a two-finger touch; the page still on screen.
                (I::Moved { .. } | I::Up { .. } | I::Cancelled { .. }, None)
                | (I::Landed { .. } | I::OnScreen(_), _) => {}
            }
            let next = if down.is_empty() {
                Fingers::None
            } else {
                Fingers::Page { pane, origin, down }
            };
            (next, effects)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingers_follow_their_page_until_they_lift() {
        use FingerInput::*;
        use TouchKind::*;
        let (a, b) = (TabId(1), TabId(2));
        // page a's view sits at (100, 10) in the window
        let origin = (100.0, 10.0);
        let touch = |slot, kind, point, pressure| PageTouch {
            pane: a,
            slot,
            kind,
            point,
            pressure,
        };
        let down = |id, at| Down {
            id,
            at,
            pressure: 0.5,
        };
        let placing = Fingers::Placing {
            id: 7,
            at: (110.0, 20.0),
            pressure: 0.5,
        };
        let page = |down: Vec<Finger>| Fingers::Page {
            pane: a,
            origin,
            down,
        };
        let f = |id, slot, at| Finger { id, slot, at };
        let one = page(vec![f(7, 0, (110.0, 20.0))]);
        let two = page(vec![f(7, 0, (110.0, 20.0)), f(9, 1, (130.0, 20.0))]);
        // (state, input, next, effects): one row per (state, input).
        let rows = vec![
            // none: a down is placed by Slint; anything else is the shell's
            (
                Fingers::None,
                down(7, (110.0, 20.0)),
                placing.clone(),
                vec![],
            ),
            (
                Fingers::None,
                Landed {
                    pane: a,
                    point: (10.0, 10.0),
                },
                Fingers::None,
                vec![],
            ),
            (
                Fingers::None,
                Moved {
                    id: 7,
                    at: (1.0, 1.0),
                    pressure: 0.5,
                },
                Fingers::None,
                vec![],
            ),
            (
                Fingers::None,
                Up {
                    id: 7,
                    at: (1.0, 1.0),
                },
                Fingers::None,
                vec![],
            ),
            (Fingers::None, Cancelled { id: 7 }, Fingers::None, vec![]),
            (Fingers::None, OnScreen(vec![]), Fingers::None, vec![]),
            // placing: landing on a page presses there; the next event otherwise leaves it the shell's
            (
                placing.clone(),
                Landed {
                    pane: a,
                    point: (10.0, 10.0),
                },
                one.clone(),
                vec![touch(0, Press, (10.0, 10.0), 0.5)],
            ),
            (
                placing.clone(),
                down(8, (5.0, 5.0)),
                Fingers::Placing {
                    id: 8,
                    at: (5.0, 5.0),
                    pressure: 0.5,
                },
                vec![],
            ),
            (
                placing.clone(),
                Moved {
                    id: 7,
                    at: (1.0, 1.0),
                    pressure: 0.5,
                },
                Fingers::None,
                vec![],
            ),
            (
                placing.clone(),
                Up {
                    id: 7,
                    at: (1.0, 1.0),
                },
                Fingers::None,
                vec![],
            ),
            (placing.clone(), Cancelled { id: 7 }, Fingers::None, vec![]),
            (placing.clone(), OnScreen(vec![a]), Fingers::None, vec![]),
            // page: a later finger is the page's wherever it lands, in the next free slot
            (
                one.clone(),
                down(9, (130.0, 20.0)),
                two.clone(),
                vec![touch(1, Press, (30.0, 10.0), 0.5)],
            ),
            (
                page(vec![f(9, 1, (130.0, 20.0))]),
                down(7, (2000.0, 20.0)),
                page(vec![f(9, 1, (130.0, 20.0)), f(7, 0, (2000.0, 20.0))]),
                vec![touch(0, Press, (1900.0, 10.0), 0.5)],
            ),
            // a down for a held finger cancels its old touch first
            (
                one.clone(),
                down(7, (120.0, 20.0)),
                page(vec![f(7, 0, (120.0, 20.0))]),
                vec![
                    touch(0, Cancel, (10.0, 10.0), 0.0),
                    touch(0, Press, (20.0, 10.0), 0.5),
                ],
            ),
            // Slint's press for the finger left after a two-finger touch is nothing new
            (
                two.clone(),
                Landed {
                    pane: a,
                    point: (30.0, 10.0),
                },
                two.clone(),
                vec![],
            ),
            // a move stays with the page, even off it
            (
                one.clone(),
                Moved {
                    id: 7,
                    at: (50.0, 5.0),
                    pressure: 0.8,
                },
                page(vec![f(7, 0, (50.0, 5.0))]),
                vec![touch(0, Move, (-50.0, -5.0), 0.8)],
            ),
            (
                one.clone(),
                Moved {
                    id: 8,
                    at: (1.0, 1.0),
                    pressure: 0.5,
                },
                one.clone(),
                vec![],
            ),
            // a lift releases where it lifts; the last one ends the page's touch
            (
                two.clone(),
                Up {
                    id: 7,
                    at: (115.0, 25.0),
                },
                page(vec![f(9, 1, (130.0, 20.0))]),
                vec![touch(0, Release, (15.0, 15.0), 0.0)],
            ),
            (
                one.clone(),
                Up {
                    id: 7,
                    at: (115.0, 25.0),
                },
                Fingers::None,
                vec![touch(0, Release, (15.0, 15.0), 0.0)],
            ),
            (
                one.clone(),
                Up {
                    id: 8,
                    at: (1.0, 1.0),
                },
                one.clone(),
                vec![],
            ),
            // the platform's cancel cancels where the finger last was
            (
                two.clone(),
                Cancelled { id: 9 },
                one.clone(),
                vec![touch(1, Cancel, (30.0, 10.0), 0.0)],
            ),
            (one.clone(), Cancelled { id: 8 }, one.clone(), vec![]),
            // the page off screen loses every finger; on screen keeps them
            (
                two.clone(),
                OnScreen(vec![b]),
                Fingers::None,
                vec![
                    touch(0, Cancel, (10.0, 10.0), 0.0),
                    touch(1, Cancel, (30.0, 10.0), 0.0),
                ],
            ),
            (two.clone(), OnScreen(vec![a, b]), two.clone(), vec![]),
        ];
        for (i, (state, input, next, effects)) in rows.into_iter().enumerate() {
            assert_eq!(transition(state, input), (next, effects), "row {i}");
        }
    }

    #[test]
    fn a_touch_flips_to_touch_and_a_real_mouse_move_flips_back() {
        let k = InputKind::Mouse;
        let k = next(k, InputEvent::Touch { x: 10.0, y: 10.0 });
        assert!(is_touch(Sizing::Auto, k));
        // the touch's emulated mouse move, at the touch point: still touch
        let k = next(k, InputEvent::MouseMove { x: 11.0, y: 9.0 });
        assert!(is_touch(Sizing::Auto, k));
        // a real move away
        let k = next(k, InputEvent::MouseMove { x: 40.0, y: 10.0 });
        assert_eq!(k, InputKind::Mouse);
        assert!(!is_touch(Sizing::Auto, k));
        // mouse moves stay mouse
        assert_eq!(
            next(k, InputEvent::MouseMove { x: 10.0, y: 10.0 }),
            InputKind::Mouse
        );
    }

    #[test]
    fn the_setting_forces_a_size() {
        let touch = InputKind::Touch { x: 0.0, y: 0.0 };
        for (sizing, mouse, touched) in [
            (Sizing::Auto, false, true),
            (Sizing::Touch, true, true),
            (Sizing::Mouse, false, false),
        ] {
            assert_eq!(is_touch(sizing, InputKind::Mouse), mouse);
            assert_eq!(is_touch(sizing, touch), touched);
        }
        assert_eq!(Sizing::parse("touch"), Sizing::Touch);
        assert_eq!(Sizing::parse("mouse"), Sizing::Mouse);
        assert_eq!(Sizing::parse("x"), Sizing::Auto);
    }

    #[test]
    fn a_swipe_switches_once_past_the_threshold() {
        let mut s = SpaceSwipe::default();
        assert_eq!(s.scroll(-30.0, 0.0, 0.0, 0, 3), None);
        assert_eq!(s.scroll(-40.0, 0.0, 16.0, 0, 3), Some(1));
        // the rest of the same gesture does nothing
        assert_eq!(s.scroll(-200.0, 0.0, 32.0, 0, 3), None);
        // a pause starts a new gesture
        assert_eq!(
            s.scroll(70.0, 0.0, 32.0 + SWIPE_GAP_MS + 1.0, 1, 3),
            Some(0)
        );
    }

    #[test]
    fn the_ends_do_not_wrap() {
        let mut s = SpaceSwipe::default();
        assert_eq!(s.scroll(100.0, 0.0, 0.0, 0, 3), None);
        let mut s = SpaceSwipe::default();
        assert_eq!(s.scroll(-100.0, 0.0, 0.0, 2, 3), None);
    }

    #[test]
    fn the_drift_of_a_list_scroll_is_not_a_swipe() {
        // 1200 px down with 5 % sideways drift, one continuous gesture
        let mut s = SpaceSwipe::default();
        for i in 0..120 {
            assert_eq!(s.scroll(-0.5, 10.0, f64::from(i) * 16.0, 1, 3), None, "{i}");
        }
        // a mostly horizontal swipe with a little vertical wobble still switches
        let mut s = SpaceSwipe::default();
        assert_eq!(s.scroll(-30.0, 4.0, 0.0, 1, 3), None);
        assert_eq!(s.scroll(-40.0, -3.0, 16.0, 1, 3), Some(2));
    }

    #[test]
    fn back_and_forth_within_the_threshold_is_not_a_swipe() {
        let mut s = SpaceSwipe::default();
        for i in 0..20 {
            let dx = if i % 2 == 0 { 40.0 } else { -40.0 };
            assert_eq!(s.scroll(dx, 0.0, f64::from(i) * 10.0, 1, 3), None);
        }
    }
}
