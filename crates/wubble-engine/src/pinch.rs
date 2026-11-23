//! A trackpad pinch on a page: Blink's own pinch zoom of the visual viewport, as Chrome and
//! Safari do it. The layout does not reflow, the zoom centres on the pointer, the page scale
//! never goes below 1 (Blink's limit), the page zoom (Ctrl + / Ctrl -) is left alone, and a page
//! that wants the gesture for itself (maps, design tools) gets a cancelable Ctrl+wheel first and
//! keeps the gesture by cancelling it, Chromium's contract (`input::TouchpadPinchEventQueue`:
//! `deltaY = -100·ln(scale)`, only the gesture's first wheel decides). Pure: inputs in, effects
//! out; [`crate::view::View`] executes them.
//!
//! Two roads behind [`Road`], one per engine:
//!
//! - [`Touchpad`]: Chromium's touchpad pinch through DevTools `Input.synthesizePinchGesture`
//!   (`content/browser/devtools/protocol/input_handler.cc`, BSD-3-Clause, called, nothing
//!   copied), the very path Chrome's trackpad pinch takes. It needs `patches/chromium/
//!   40-cef-touchpad-pinch.patch`: without it a windowless view sends the page the Ctrl+wheel and
//!   never the pinch (CEF's OSR view lacks `ForwardTouchpadZoomEventIfNecessary`). Each pinch
//!   step is one short synthetic gesture, anchored in the CSS pixels the protocol wants
//!   ([`css_anchor`]), measured just before it (`Page.getLayoutMetrics`).
//! - [`Touch`]: two synthetic fingers (`SendTouchEvent`) around the pointer, on a stock engine.
//!   Chromium's touch gesture detector turns their spread into a pinch. First the page gets the
//!   Ctrl+wheel the touchpad road would give it (its own `SendMouseWheelEvent`); `zoom.js` says
//!   whether the page cancelled it ([`Answer`]); only a wheel the page left alone lets the
//!   fingers down, and a page that kept it gets every further step as a Ctrl+wheel instead. The
//!   fingers go down and are cancelled (never lifted: no tap, no click) within one gesture, and
//!   the pointer is moved back where it is after them, so hover returns. What is left of touch:
//!   the page sees `touchstart`/`pointerdown` (`pointerType` touch) and moves, and a
//!   `touch-action: none` area does not zoom.
//!
//! [`Pinch`] wraps either road with what both share: which DevTools replies are the road's, and
//! how many `zoom.js` answers its wheels still owe, so an answer to the pinch's own wheel never
//! becomes a page-zoom step (a real Ctrl+wheel's answer does, as before).

use std::collections::BTreeMap;

use serde_json::{Value, json};

use crate::cdp::CdpId;
use crate::input::Point;

/// Chromium's wheel delta for a pinch step of `scale`: `100·ln(scale)` DIP up (the page reads
/// `deltaY = -100·ln(scale)`), `touchpad_pinch_event_queue.cc`.
pub fn wheel_delta(scale: f64) -> f64 {
    100.0 * scale.ln()
}

/// What the host reports of a pinch over this view.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Gesture {
    /// The fingers touched the trackpad with the pointer at `at` (view-relative DIP).
    Begin(Point),
    /// The scale changed by this factor since the last report (1.1 = 10 % larger).
    Scaled(f64),
    End,
}

/// [`Gesture`] with the view's size (DIP) on the begin.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Input {
    Begin { at: Point, size: (f32, f32) },
    Scaled(f64),
    End,
}

/// `zoom.js`'s word on a trusted Ctrl+wheel: left alone (`in` or `out`) or cancelled (`kept`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Answer {
    In,
    Out,
    Kept,
}

impl Answer {
    pub fn parse(body: &str) -> Option<Self> {
        match body {
            "in" => Some(Self::In),
            "out" => Some(Self::Out),
            "kept" => Some(Self::Kept),
            _ => None,
        }
    }
}

// The fingers go down, move and are cancelled (never released: no tap).
pub use wubble_core::touch::TouchKind;

#[derive(Clone, Debug, PartialEq)]
pub enum Effect {
    /// A Ctrl+wheel at `at`, `dy` whole DIP up (precise deltas); the page may cancel it.
    Wheel { at: Point, dy: i32 },
    /// The two fingers, ids 0 and 1.
    Touch { kind: TouchKind, points: [Point; 2] },
    /// The pointer moved to where it is, so the page's hover comes back after the fingers.
    Hover(Point),
    /// One DevTools call.
    Cdp {
        id: CdpId,
        method: &'static str,
        params: Value,
    },
}

/// One way of pinching a page.
pub trait Road {
    fn input(&mut self, input: Input, ids: &mut dyn FnMut() -> CdpId) -> Vec<Effect>;
    fn answer(&mut self, answer: Answer) -> Vec<Effect>;
    fn reply(
        &mut self,
        id: CdpId,
        result: &Result<Value, String>,
        ids: &mut dyn FnMut() -> CdpId,
    ) -> Vec<Effect>;
}

/// A factor that is a scale (winit says the delta may be NaN).
fn is_scale(f: f64) -> bool {
    f.is_finite() && f > 0.0
}

/// Close enough to 1 to send nothing.
fn is_one(f: f64) -> bool {
    wheel_delta(f).abs() < 0.5
}

// ── The touch road.

/// The fingers' distance when they go down, DIP; the pinch scales it. Wide, so a pinch out has
/// room before [`SPANS`]' floor and Chromium's span slop (2 × 15 DIP on Aura, eaten before the
/// first scale) is a small part of it; the fingers may lie outside the view (the gesture
/// detector runs in the browser on the points as given).
pub const SPAN: f64 = 1000.0;
/// The distances the fingers keep to. Under 125 DIP Chromium's detector ends the scale
/// (`GestureConfiguration::min_scaling_span_in_pixels`, `ScaleGestureDetector`): the floor
/// keeps above it, so one gesture pinches out by 1000/140 at most, past any page scale Blink
/// allows on a desktop; the ceiling is past any.
pub const SPANS: std::ops::RangeInclusive<f64> = 140.0..=40_000.0;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Touch {
    #[default]
    Idle,
    /// Begun; the scale so far is under one whole wheel DIP.
    Gathering { at: Point, scale: f64 },
    /// The first Ctrl+wheel is out (`sent` DIP); waiting for the page's answer. `scale` is all
    /// of the gesture so far; `ended`: the fingers left meanwhile.
    Asking {
        at: Point,
        scale: f64,
        sent: i32,
        ended: bool,
    },
    /// The page left the gesture to the browser: two fingers `span` DIP apart around `at`.
    Fingers { at: Point, span: f64 },
    /// The page kept the gesture: each step is its Ctrl+wheel; `rest` DIP not yet sent.
    Wheels { at: Point, rest: f64 },
}

fn fingers(at: Point, span: f64) -> [Point; 2] {
    let half = (span / 2.0) as f32;
    [
        Point {
            x: at.x - half,
            y: at.y,
        },
        Point {
            x: at.x + half,
            y: at.y,
        },
    ]
}

/// Whole DIP of `rest` to send now, and what stays.
fn whole(rest: f64) -> (i32, f64) {
    let w = rest.trunc();
    (w as i32, rest - w)
}

fn lift(at: Point, span: f64) -> Vec<Effect> {
    vec![
        Effect::Touch {
            kind: TouchKind::Cancel,
            points: fingers(at, span),
        },
        Effect::Hover(at),
    ]
}

/// The fingers down at [`SPAN`], then spread by `scale`.
fn press(at: Point, scale: f64) -> (Touch, Vec<Effect>) {
    let span = (SPAN * scale).clamp(*SPANS.start(), *SPANS.end());
    let mut effects = vec![Effect::Touch {
        kind: TouchKind::Press,
        points: fingers(at, SPAN),
    }];
    if span != SPAN {
        effects.push(Effect::Touch {
            kind: TouchKind::Move,
            points: fingers(at, span),
        });
    }
    (Touch::Fingers { at, span }, effects)
}

pub enum TouchEvent {
    Input(Input),
    Answer(Answer),
}

pub fn touch(state: Touch, event: TouchEvent) -> (Touch, Vec<Effect>) {
    use Touch as S;
    use TouchEvent as E;
    match (state, event) {
        (state, E::Input(Input::Begin { at, .. })) => {
            let effects = match state {
                S::Fingers { at, span } => lift(at, span),
                _ => vec![],
            };
            (S::Gathering { at, scale: 1.0 }, effects)
        }
        (state, E::Input(Input::Scaled(f))) if !is_scale(f) => (state, vec![]),
        (S::Gathering { at, scale }, E::Input(Input::Scaled(f))) => {
            let scale = scale * f;
            let (dy, _) = whole(wheel_delta(scale));
            if dy == 0 {
                (S::Gathering { at, scale }, vec![])
            } else {
                (
                    S::Asking {
                        at,
                        scale,
                        sent: dy,
                        ended: false,
                    },
                    vec![Effect::Wheel { at, dy }],
                )
            }
        }
        (
            S::Asking {
                at,
                scale,
                sent,
                ended,
            },
            E::Input(Input::Scaled(f)),
        ) => (
            S::Asking {
                at,
                scale: scale * f,
                sent,
                ended,
            },
            vec![],
        ),
        (S::Fingers { at, span }, E::Input(Input::Scaled(f))) => {
            let next = (span * f).clamp(*SPANS.start(), *SPANS.end());
            let effects = if next == span {
                vec![]
            } else {
                vec![Effect::Touch {
                    kind: TouchKind::Move,
                    points: fingers(at, next),
                }]
            };
            (S::Fingers { at, span: next }, effects)
        }
        (S::Wheels { at, rest }, E::Input(Input::Scaled(f))) => {
            let (dy, rest) = whole(rest + wheel_delta(f));
            let effects = if dy == 0 {
                vec![]
            } else {
                vec![Effect::Wheel { at, dy }]
            };
            (S::Wheels { at, rest }, effects)
        }
        (
            S::Asking {
                at,
                scale,
                sent,
                ended,
            },
            E::Input(Input::End),
        ) if !ended => (
            S::Asking {
                at,
                scale,
                sent,
                ended: true,
            },
            vec![],
        ),
        (S::Fingers { at, span }, E::Input(Input::End)) => (S::Idle, lift(at, span)),
        (_, E::Input(Input::End)) => (S::Idle, vec![]),
        (
            S::Asking {
                at, scale, ended, ..
            },
            E::Answer(Answer::In | Answer::Out),
        ) => {
            let (state, mut effects) = press(at, scale);
            if ended {
                let S::Fingers { at, span } = state else {
                    unreachable!("press puts the fingers down")
                };
                effects.extend(lift(at, span));
                return (S::Idle, effects);
            }
            (state, effects)
        }
        (
            S::Asking {
                at,
                scale,
                sent,
                ended,
            },
            E::Answer(Answer::Kept),
        ) => {
            if ended {
                return (S::Idle, vec![]);
            }
            // What the gesture grew by while the page decided goes to the page too.
            let (dy, rest) = whole(wheel_delta(scale) - f64::from(sent));
            let effects = if dy == 0 {
                vec![]
            } else {
                vec![Effect::Wheel { at, dy }]
            };
            (S::Wheels { at, rest }, effects)
        }
        (state, E::Answer(_)) => (state, vec![]),
        (state, E::Input(Input::Scaled(_))) => (state, vec![]),
    }
}

impl Road for Touch {
    fn input(&mut self, input: Input, _: &mut dyn FnMut() -> CdpId) -> Vec<Effect> {
        let (next, effects) = touch(*self, TouchEvent::Input(input));
        *self = next;
        effects
    }

    fn answer(&mut self, answer: Answer) -> Vec<Effect> {
        let (next, effects) = touch(*self, TouchEvent::Answer(answer));
        *self = next;
        effects
    }

    fn reply(
        &mut self,
        _: CdpId,
        _: &Result<Value, String>,
        _: &mut dyn FnMut() -> CdpId,
    ) -> Vec<Effect> {
        vec![]
    }
}

// ── The touchpad road (needs patch 40).

/// DevTools' pointer speed for a step, px/s: so fast the step is one update (`relativeSpeed`;
/// `SyntheticTouchpadPinchGesture::CalculateEndTime`).
const SPEED: i64 = 1_000_000;

/// A pinch over the page, between its begin and the last step sent.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Flight {
    pub at: Point,
    pub size: (f32, f32),
    /// The scale reported and not yet sent.
    pub pending: f64,
    pub ended: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Touchpad {
    #[default]
    Idle,
    /// Begun, nothing on its way.
    Ready(Flight),
    /// Asked for the page's scale and zoom (`Page.getLayoutMetrics`).
    Measuring { id: CdpId, flight: Flight },
    /// A step is on its way (`Input.synthesizePinchGesture`).
    Pinching { id: CdpId, flight: Flight },
}

/// The anchor `at` (view DIP, kept inside a view of `size`) in the CSS pixels
/// `Input.synthesizePinchGesture` takes: it multiplies them by the page zoom and the page scale
/// (`InputHandler::ScaleFactor`).
pub fn css_anchor(at: Point, size: (f32, f32), zoom: f64, scale: f64) -> (f64, f64) {
    let inside = |v: f32, max: f32| f64::from(v.clamp(0.0, (max - 1.0).max(0.0)));
    let by = (zoom * scale).max(f64::MIN_POSITIVE);
    (inside(at.x, size.0) / by, inside(at.y, size.1) / by)
}

/// `cssVisualViewport`'s scale and zoom in a `Page.getLayoutMetrics` result.
pub fn metrics(result: &Value) -> Option<(f64, f64)> {
    let v = result.get("cssVisualViewport")?;
    let scale = v.get("scale")?.as_f64()?;
    let zoom = v.get("zoom").and_then(Value::as_f64).unwrap_or(1.0);
    (is_scale(scale) && is_scale(zoom)).then_some((scale, zoom))
}

fn measure(id: CdpId, flight: Flight) -> (Touchpad, Vec<Effect>) {
    (
        Touchpad::Measuring { id, flight },
        vec![Effect::Cdp {
            id,
            method: "Page.getLayoutMetrics",
            params: json!({}),
        }],
    )
}

/// Nothing on its way: idle once the fingers left.
fn settled(flight: Flight) -> Touchpad {
    if flight.ended {
        Touchpad::Idle
    } else {
        Touchpad::Ready(Flight {
            pending: 1.0,
            ..flight
        })
    }
}

pub enum TouchpadEvent<'a> {
    Input(Input),
    Reply(CdpId, &'a Result<Value, String>),
}

pub fn touchpad(
    state: Touchpad,
    event: TouchpadEvent,
    ids: &mut dyn FnMut() -> CdpId,
) -> (Touchpad, Vec<Effect>) {
    use Touchpad as S;
    use TouchpadEvent as E;
    let add = |flight: Flight, f: f64| Flight {
        pending: flight.pending * f,
        ..flight
    };
    match (state, event) {
        (_, E::Input(Input::Begin { at, size })) => (
            S::Ready(Flight {
                at,
                size,
                pending: 1.0,
                ended: false,
            }),
            vec![],
        ),
        (state, E::Input(Input::Scaled(f))) if !is_scale(f) => (state, vec![]),
        (S::Ready(flight), E::Input(Input::Scaled(f))) if is_one(add(flight, f).pending) => {
            (S::Ready(add(flight, f)), vec![])
        }
        (S::Ready(flight), E::Input(Input::Scaled(f))) => measure(ids(), add(flight, f)),
        (S::Measuring { id, flight }, E::Input(Input::Scaled(f))) => (
            S::Measuring {
                id,
                flight: add(flight, f),
            },
            vec![],
        ),
        (S::Pinching { id, flight }, E::Input(Input::Scaled(f))) => (
            S::Pinching {
                id,
                flight: add(flight, f),
            },
            vec![],
        ),
        (S::Ready(_), E::Input(Input::End)) => (S::Idle, vec![]),
        (S::Measuring { id, flight }, E::Input(Input::End)) => (
            S::Measuring {
                id,
                flight: Flight {
                    ended: true,
                    ..flight
                },
            },
            vec![],
        ),
        (S::Pinching { id, flight }, E::Input(Input::End)) => (
            S::Pinching {
                id,
                flight: Flight {
                    ended: true,
                    ..flight
                },
            },
            vec![],
        ),
        (S::Measuring { id, flight }, E::Reply(got, result)) if got == id => {
            let Some((scale, zoom)) = result.as_ref().ok().and_then(metrics) else {
                return (settled(flight), vec![]);
            };
            if is_one(flight.pending) {
                return (settled(flight), vec![]);
            }
            let (x, y) = css_anchor(flight.at, flight.size, zoom, scale);
            let id = ids();
            (
                S::Pinching {
                    id,
                    flight: Flight {
                        pending: 1.0,
                        ..flight
                    },
                },
                vec![Effect::Cdp {
                    id,
                    method: "Input.synthesizePinchGesture",
                    params: json!({
                        "x": x,
                        "y": y,
                        "scaleFactor": flight.pending,
                        "relativeSpeed": SPEED,
                        "gestureSourceType": "mouse",
                    }),
                }],
            )
        }
        (S::Pinching { id, flight }, E::Reply(got, _)) if got == id => {
            if is_one(flight.pending) {
                (settled(flight), vec![])
            } else {
                measure(ids(), flight)
            }
        }
        (state, _) => (state, vec![]),
    }
}

impl Road for Touchpad {
    fn input(&mut self, input: Input, ids: &mut dyn FnMut() -> CdpId) -> Vec<Effect> {
        let (next, effects) = touchpad(*self, TouchpadEvent::Input(input), ids);
        *self = next;
        effects
    }

    /// The page's word on the pinch's wheel is Chromium's business on this road.
    fn answer(&mut self, _: Answer) -> Vec<Effect> {
        vec![]
    }

    fn reply(
        &mut self,
        id: CdpId,
        result: &Result<Value, String>,
        ids: &mut dyn FnMut() -> CdpId,
    ) -> Vec<Effect> {
        let (next, effects) = touchpad(*self, TouchpadEvent::Reply(id, result), ids);
        *self = next;
        effects
    }
}

// ── What both roads share.

/// Whether an effect makes the page's `zoom.js` answer once: a Ctrl+wheel, ours or the one a
/// synthetic touchpad pinch step sends.
fn owes_answer(effect: &Effect) -> bool {
    match effect {
        Effect::Wheel { .. } => true,
        Effect::Cdp { method, .. } => *method == "Input.synthesizePinchGesture",
        Effect::Touch { .. } | Effect::Hover(_) => false,
    }
}

pub struct Pinch {
    road: Box<dyn Road>,
    /// The road's DevTools calls on their way, and whether each owes an answer.
    calls: BTreeMap<CdpId, bool>,
    /// `zoom.js` answers the pinch's wheels still owe.
    owed: u32,
}

impl Pinch {
    /// The touchpad road on an engine with patch 40, the touch road on any other.
    pub fn new(touchpad: bool) -> Self {
        Self::on(if touchpad {
            Box::new(Touchpad::default())
        } else {
            Box::new(Touch::default())
        })
    }

    pub fn on(road: Box<dyn Road>) -> Self {
        Self {
            road,
            calls: BTreeMap::new(),
            owed: 0,
        }
    }

    fn sent(&mut self, effects: Vec<Effect>) -> Vec<Effect> {
        for effect in &effects {
            let owes = owes_answer(effect);
            if let Effect::Cdp { id, .. } = effect {
                self.calls.insert(*id, owes);
            }
            self.owed += u32::from(owes);
        }
        effects
    }

    pub fn input(&mut self, input: Input, ids: &mut dyn FnMut() -> CdpId) -> Vec<Effect> {
        // An answer still owed by an earlier gesture is not coming (no `zoom.js` where its
        // wheel landed): a real Ctrl+wheel's answer must not be taken for it.
        if matches!(input, Input::Begin { .. }) {
            self.owed = 0;
        }
        let effects = self.road.input(input, ids);
        self.sent(effects)
    }

    /// `zoom.js`'s answer: the pinch's (`Some`, with what follows) while one of its wheels owes
    /// one, else a real Ctrl+wheel's (`None`: the caller zooms the page as before).
    pub fn answer(&mut self, answer: Answer) -> Option<Vec<Effect>> {
        if self.owed == 0 {
            return None;
        }
        self.owed -= 1;
        let effects = self.road.answer(answer);
        Some(self.sent(effects))
    }

    /// A DevTools reply: the road's (`Some`) or another's (`None`). A step that failed sent no
    /// wheel, so its answer is no longer owed.
    pub fn reply(
        &mut self,
        id: CdpId,
        result: &Result<Value, String>,
        ids: &mut dyn FnMut() -> CdpId,
    ) -> Option<Vec<Effect>> {
        let owes = self.calls.remove(&id)?;
        if owes && result.is_err() {
            self.owed = self.owed.saturating_sub(1);
        }
        let effects = self.road.reply(id, result, ids);
        Some(self.sent(effects))
    }
}

/// The marker patch 40 exports: whether this engine applies a touchpad pinch. Looked up once.
pub fn touchpad_available() -> bool {
    static FOUND: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *FOUND.get_or_init(|| {
        // SAFETY: the symbol, when present, is `int cef_wubble_touchpad_pinch_v1(void)`.
        let get = crate::ext_api::exported::<unsafe extern "C" fn() -> std::ffi::c_int>(
            b"cef_wubble_touchpad_pinch_v1\0",
        );
        // SAFETY: as above; it only returns a constant.
        get.is_some_and(|f| unsafe { f() } == 1)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const AT: Point = Point { x: 300.0, y: 200.0 };
    const SIZE: (f32, f32) = (800.0, 600.0);

    fn counter() -> impl FnMut() -> CdpId {
        let mut n = 0;
        move || {
            n += 1;
            CdpId(n)
        }
    }

    fn begin() -> Input {
        Input::Begin { at: AT, size: SIZE }
    }

    fn run_touch(events: Vec<TouchEvent>) -> (Touch, Vec<Effect>) {
        events
            .into_iter()
            .fold((Touch::Idle, vec![]), |(s, mut all), e| {
                let (s, fx) = touch(s, e);
                all.extend(fx);
                (s, all)
            })
    }

    use TouchEvent::{Answer as A, Input as I};

    #[test]
    fn a_step_is_chromiums_wheel_delta() {
        assert!((wheel_delta(2.0) - 69.314_718).abs() < 1e-5);
        assert_eq!(wheel_delta(1.0), 0.0);
        assert!((wheel_delta(0.5) + wheel_delta(2.0)).abs() < 1e-12);
    }

    #[test]
    fn the_page_gets_the_wheel_first_and_then_the_fingers() {
        let (s, fx) = run_touch(vec![I(begin()), I(Input::Scaled(1.1))]);
        // 100·ln(1.1) = 9.53: the whole DIP go out as the gesture's first Ctrl+wheel
        assert_eq!(fx, [Effect::Wheel { at: AT, dy: 9 }]);
        assert!(matches!(s, Touch::Asking { sent: 9, .. }));
        // more while the page decides: kept, not sent
        let (s, fx) = touch(s, I(Input::Scaled(1.2)));
        assert!(fx.is_empty());
        // left alone: the fingers go down [`SPAN`] apart and spread by all of it
        let (s, fx) = touch(s, A(Answer::In));
        let span = SPAN * 1.1 * 1.2;
        assert_eq!(s, Touch::Fingers { at: AT, span });
        assert_eq!(
            fx,
            [
                Effect::Touch {
                    kind: TouchKind::Press,
                    points: fingers(AT, SPAN)
                },
                Effect::Touch {
                    kind: TouchKind::Move,
                    points: fingers(AT, span)
                }
            ]
        );
        assert_eq!(fingers(AT, 200.0)[0], Point { x: 200.0, y: 200.0 });
        // further steps spread them; the end cancels them (no lift: no tap) and puts the
        // pointer back for hover
        let (s, fx) = touch(s, I(Input::Scaled(0.5)));
        assert_eq!(fx.len(), 1);
        let (s, fx) = touch(s, I(Input::End));
        assert_eq!(s, Touch::Idle);
        assert!(matches!(
            fx.as_slice(),
            [
                Effect::Touch {
                    kind: TouchKind::Cancel,
                    ..
                },
                Effect::Hover(AT)
            ]
        ));
    }

    #[test]
    fn tiny_steps_gather_until_one_whole_wheel_dip() {
        let (s, fx) = run_touch(vec![I(begin()), I(Input::Scaled(1.004))]);
        assert!(fx.is_empty());
        let (s, fx) = touch(s, I(Input::Scaled(1.004)));
        assert!(fx.is_empty(), "0.8 DIP so far");
        let (_, fx) = touch(s, I(Input::Scaled(1.004)));
        assert_eq!(fx, [Effect::Wheel { at: AT, dy: 1 }]);
        // pinching out sends an upward-negative wheel
        let (_, fx) = run_touch(vec![I(begin()), I(Input::Scaled(0.9))]);
        assert_eq!(fx, [Effect::Wheel { at: AT, dy: -10 }]);
    }

    #[test]
    fn a_page_that_keeps_the_wheel_keeps_the_gesture() {
        let (s, fx) = run_touch(vec![
            I(begin()),
            I(Input::Scaled(1.1)),
            I(Input::Scaled(1.05)),
            A(Answer::Kept),
        ]);
        // the first wheel had 9 of 100·ln(1.155) = 14.4: the rest goes now
        assert_eq!(
            fx,
            [
                Effect::Wheel { at: AT, dy: 9 },
                Effect::Wheel { at: AT, dy: 5 }
            ]
        );
        let (s, fx) = touch(s, I(Input::Scaled(1.1)));
        assert_eq!(fx, [Effect::Wheel { at: AT, dy: 9 }]);
        // no fingers ever, and nothing to lift at the end
        let (s, fx) = touch(s, I(Input::End));
        assert_eq!(s, Touch::Idle);
        assert!(fx.is_empty());
    }

    #[test]
    fn a_quick_pinch_still_zooms_when_the_answer_comes_after_the_end() {
        let (s, fx) = run_touch(vec![
            I(begin()),
            I(Input::Scaled(1.3)),
            I(Input::End),
            A(Answer::In),
        ]);
        assert_eq!(s, Touch::Idle);
        let kinds: Vec<_> = fx
            .iter()
            .filter_map(|e| match e {
                Effect::Touch { kind, .. } => Some(*kind),
                _ => None,
            })
            .collect();
        assert_eq!(
            kinds,
            [TouchKind::Press, TouchKind::Move, TouchKind::Cancel]
        );
        assert_eq!(fx.last(), Some(&Effect::Hover(AT)));
        // kept after the end: nothing
        let (s, fx) = run_touch(vec![
            I(begin()),
            I(Input::Scaled(1.3)),
            I(Input::End),
            A(Answer::Kept),
        ]);
        assert_eq!((s, fx.len()), (Touch::Idle, 1));
    }

    #[test]
    fn the_fingers_keep_a_span_chromium_scales_with() {
        let (s, _) = run_touch(vec![I(begin()), I(Input::Scaled(1.1)), A(Answer::In)]);
        let (s, _) = touch(s, I(Input::Scaled(1e-6)));
        assert!(matches!(s, Touch::Fingers { span, .. } if span == *SPANS.start()));
        // at the limit, pinching further out sends nothing
        let (s, fx) = touch(s, I(Input::Scaled(0.5)));
        assert!(fx.is_empty());
        let (_, fx) = touch(s, I(Input::Scaled(f64::NAN)));
        assert!(fx.is_empty());
    }

    #[test]
    fn a_new_begin_lifts_fingers_left_down() {
        let (s, _) = run_touch(vec![I(begin()), I(Input::Scaled(1.1)), A(Answer::Out)]);
        let other = Point { x: 10.0, y: 10.0 };
        let (s, fx) = touch(
            s,
            I(Input::Begin {
                at: other,
                size: SIZE,
            }),
        );
        assert_eq!(
            s,
            Touch::Gathering {
                at: other,
                scale: 1.0
            }
        );
        assert!(matches!(
            fx.first(),
            Some(Effect::Touch {
                kind: TouchKind::Cancel,
                ..
            })
        ));
    }

    #[test]
    fn the_anchor_is_in_the_css_pixels_devtools_scales() {
        assert_eq!(css_anchor(AT, SIZE, 1.0, 1.0), (300.0, 200.0));
        // zoomed 2x by pinch and the page at 150 %: the protocol multiplies by 3
        assert_eq!(css_anchor(AT, SIZE, 1.5, 2.0), (100.0, 200.0 / 3.0));
        // kept inside the view (DevTools refuses a point outside it)
        let out = Point { x: -5.0, y: 900.0 };
        assert_eq!(css_anchor(out, SIZE, 1.0, 1.0), (0.0, 599.0));
    }

    fn layout(scale: f64, zoom: f64) -> Result<Value, String> {
        Ok(json!({ "cssVisualViewport": { "scale": scale, "zoom": zoom } }))
    }

    #[test]
    fn the_touchpad_road_measures_then_pinches_one_step_at_a_time() {
        let mut ids = counter();
        let (s, fx) = touchpad(Touchpad::Idle, TouchpadEvent::Input(begin()), &mut ids);
        assert!(fx.is_empty());
        let (s, fx) = touchpad(s, TouchpadEvent::Input(Input::Scaled(1.2)), &mut ids);
        assert!(matches!(
            fx.as_slice(),
            [Effect::Cdp {
                id: CdpId(1),
                method: "Page.getLayoutMetrics",
                ..
            }]
        ));
        // more while measuring: added up
        let (s, _) = touchpad(s, TouchpadEvent::Input(Input::Scaled(1.5)), &mut ids);
        let (s, fx) = touchpad(
            s,
            TouchpadEvent::Reply(CdpId(1), &layout(2.0, 1.0)),
            &mut ids,
        );
        let [Effect::Cdp { id, method, params }] = fx.as_slice() else {
            panic!("{fx:?}")
        };
        assert_eq!((*id, *method), (CdpId(2), "Input.synthesizePinchGesture"));
        assert!((params["scaleFactor"].as_f64().unwrap() - 1.8).abs() < 1e-12);
        assert_eq!(
            (params["x"].as_f64(), params["y"].as_f64()),
            (Some(150.0), Some(100.0))
        );
        // another's reply is not this one's
        let (s, fx) = touchpad(s, TouchpadEvent::Reply(CdpId(9), &Ok(json!({}))), &mut ids);
        assert!(fx.is_empty());
        // a step on its way: the next waits for it, then is measured again
        let (s, _) = touchpad(s, TouchpadEvent::Input(Input::Scaled(0.9)), &mut ids);
        let (s, _) = touchpad(s, TouchpadEvent::Input(Input::End), &mut ids);
        let (s, fx) = touchpad(s, TouchpadEvent::Reply(CdpId(2), &Ok(json!({}))), &mut ids);
        assert!(matches!(fx.as_slice(), [Effect::Cdp { id: CdpId(3), .. }]));
        let (s, fx) = touchpad(
            s,
            TouchpadEvent::Reply(CdpId(3), &layout(1.8, 1.0)),
            &mut ids,
        );
        assert!(
            matches!(&fx[0], Effect::Cdp { params, .. } if params["scaleFactor"] == json!(0.9))
        );
        // the last step done after the end: idle
        let (s, fx) = touchpad(s, TouchpadEvent::Reply(CdpId(4), &Ok(json!({}))), &mut ids);
        assert_eq!((s, fx.len()), (Touchpad::Idle, 0));
    }

    #[test]
    fn the_touchpad_road_sends_no_step_that_is_no_scale() {
        let mut ids = counter();
        let (s, _) = touchpad(Touchpad::Idle, TouchpadEvent::Input(begin()), &mut ids);
        for f in [f64::NAN, 0.0, -1.0, 1.0, 1.001] {
            let (next, fx) = touchpad(s, TouchpadEvent::Input(Input::Scaled(f)), &mut ids);
            assert!(fx.is_empty(), "{f}");
            assert!(matches!(next, Touchpad::Ready(_)), "{f}");
        }
        // a failed measurement drops the step
        let (s, _) = touchpad(s, TouchpadEvent::Input(Input::Scaled(2.0)), &mut ids);
        let (s, fx) = touchpad(
            s,
            TouchpadEvent::Reply(CdpId(1), &Err("gone".into())),
            &mut ids,
        );
        assert!(fx.is_empty());
        assert!(matches!(s, Touchpad::Ready(Flight { pending: 1.0, .. })));
    }

    #[test]
    fn the_pinchs_own_answers_never_reach_page_zoom() {
        let mut ids = counter();
        let mut pinch = Pinch::new(false);
        // a real Ctrl+wheel's answer, no pinch: the caller's
        assert_eq!(pinch.answer(Answer::In), None);
        pinch.input(begin(), &mut ids);
        pinch.input(Input::Scaled(1.1), &mut ids);
        // the pinch's wheel's answer: the pinch's, and the fingers follow
        let fx = pinch.answer(Answer::In).expect("the pinch's");
        assert!(matches!(
            fx.first(),
            Some(Effect::Touch {
                kind: TouchKind::Press,
                ..
            })
        ));
        assert_eq!(pinch.answer(Answer::Out), None);
        // an answer never given is forgotten at the next gesture
        pinch.input(Input::End, &mut ids);
        pinch.input(begin(), &mut ids);
        pinch.input(Input::Scaled(1.1), &mut ids);
        pinch.input(Input::End, &mut ids);
        pinch.input(begin(), &mut ids);
        assert_eq!(pinch.answer(Answer::In), None);
    }

    #[test]
    fn the_touchpad_road_owes_one_answer_per_step_that_went() {
        let mut ids = counter();
        let mut pinch = Pinch::new(true);
        pinch.input(begin(), &mut ids);
        pinch.input(Input::Scaled(1.5), &mut ids);
        assert_eq!(pinch.reply(CdpId(7), &Ok(json!({})), &mut ids), None);
        let fx = pinch.reply(CdpId(1), &layout(1.0, 1.0), &mut ids).unwrap();
        assert_eq!(fx.len(), 1);
        // the step's wheel: answered by zoom.js, the pinch's
        assert_eq!(pinch.answer(Answer::In), Some(vec![]));
        assert_eq!(pinch.answer(Answer::In), None);
        // a step DevTools refused sent no wheel
        pinch.input(Input::Scaled(1.5), &mut ids);
        pinch.reply(CdpId(2), &Ok(json!({})), &mut ids).unwrap();
        pinch.reply(CdpId(3), &layout(1.5, 1.0), &mut ids).unwrap();
        pinch
            .reply(CdpId(4), &Err("Position out of bounds".into()), &mut ids)
            .unwrap();
        assert_eq!(pinch.answer(Answer::In), None);
    }

    #[test]
    fn answers_parse() {
        assert_eq!(Answer::parse("in"), Some(Answer::In));
        assert_eq!(Answer::parse("out"), Some(Answer::Out));
        assert_eq!(Answer::parse("kept"), Some(Answer::Kept));
        assert_eq!(Answer::parse("sideways"), None);
    }
}
