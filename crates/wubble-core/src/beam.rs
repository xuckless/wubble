//! The address bar's border beam: a head of light with a long tail that laps the bar when the
//! shown page changes (another tab selected, a navigation), and keeps lapping while the page
//! loads. Pure: one enum of states and one [`transition`]; the shell driver feeds it events and
//! frame time, the scene draws [`Beam::shape`] round the address pill's glass (and the Slint
//! `BorderBeam` draws the same shape where the scene does not).
//!
//! The curve is the design's (the owner's design repo, `src/wubble/browser/chrome.tsx`
//! `Address`, `// @source address-loading`): "the beam laps the bar on an ease-out and fades as
//! it slows: it brightens in 120 ms, then from 35% of the way its strength eases to zero
//! (smootherstep, squared so the last of it lingers) while its tail grows longer, like a comet
//! thinning out. 1500 ms in all." It starts whenever the address changes (the tab, its history
//! position, a reload: `sig` there), never on the first render, and not under reduced motion.
//!
//! wubble: the design's pages are snapshots that "load" within one lap. A real page can load for
//! longer, and the owner asked for the beam to show tab changes and loading, so while the page
//! loads the lap holds where its fade would begin (35%): full strength, the head running on at
//! the speed it had there. When the load ends the lap goes on from that point, so the fade is
//! the design's last 65% (975 ms), continuous in head, tail and strength.

/// One lap, seconds (chrome.tsx `D = 1500`).
pub const LAP: f64 = 1.5;
/// Where in the lap (0..1) the fade begins; a loading page holds the lap here.
pub const FADE_FROM: f64 = 0.35;

/// The head's speed at `k`, perimeters per second: d/dt of `0.08 + 1.35 · (1 − (1 − k)³)`.
fn head_speed(k: f64) -> f64 {
    1.35 * 3.0 * (1.0 - k).powi(2) / LAP
}

/// The beam as the shader reads it (`uBeam`): head and tail length as fractions of the
/// perimeter (clockwise from the top centre), strength 0..1, hue offset into the sheen.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shape {
    pub head: f64,
    pub length: f64,
    pub strength: f64,
    pub hue: f64,
}

// @source address-loading (browser/chrome.tsx)
/// The beam `k` (0..1) of the way through a lap.
pub fn shape_at(k: f64, hue: f64) -> Shape {
    let k = k.clamp(0.0, 1.0);
    let e = 1.0 - (1.0 - k).powi(3);
    let f = ((k - FADE_FROM) / (1.0 - FADE_FROM)).clamp(0.0, 1.0);
    let fade = 1.0 - f * f * f * (f * (f * 6.0 - 15.0) + 10.0);
    Shape {
        head: 0.08 + e * 1.35,
        length: 0.35 + 0.3 * f,
        strength: (k * 12.5).min(1.0) * fade * fade,
        hue,
    }
}

/// How far a lap is: `k` of the design's curve, plus `spin`, the head's extra travel
/// (perimeters) while a load held the lap at [`FADE_FROM`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Lap {
    pub k: f64,
    pub spin: f64,
    pub hue: f64,
}

impl Lap {
    pub fn new(hue: f64) -> Self {
        Self {
            k: 0.0,
            spin: 0.0,
            hue,
        }
    }

    pub fn shape(&self) -> Shape {
        let s = shape_at(self.k, self.hue);
        Shape {
            head: (s.head + self.spin).rem_euclid(1.0),
            ..s
        }
    }
}

/// The beam's lifecycle.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum Beam {
    /// Nothing drawn.
    #[default]
    Idle,
    /// One lap running out (no load, or the load ended).
    Lapping(Lap),
    /// The page is loading: the lap runs up to [`FADE_FROM`] and holds there, lapping.
    Loading(Lap),
}

/// What the shell tells the beam.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Event {
    /// Another page is shown (another tab selected): a new lap, holding if it is `loading`.
    /// `hue` is the new lap's offset into the sheen (the design's `Math.random()`).
    Shown { loading: bool, hue: f64 },
    /// The shown page started loading (a navigation, a reload).
    LoadStarted { hue: f64 },
    /// The shown page finished loading (or stopped).
    LoadEnded,
    /// `dt` seconds passed.
    Tick(f64),
    /// Reduced motion: no beam (chrome.tsx skips the lap under `reducedMotion()`).
    Reduce,
}

/// The one transition. The beam has no effects: its output is [`Beam::shape`].
pub fn transition(beam: Beam, event: Event) -> Beam {
    use Beam::*;
    match (beam, event) {
        (_, Event::Reduce) => Idle,
        (_, Event::Shown { loading, hue }) => {
            let lap = Lap::new(hue);
            if loading { Loading(lap) } else { Lapping(lap) }
        }
        // a navigation restarts the lap, as the design's `sig` change does; a load already
        // lapping keeps going
        (Loading(l), Event::LoadStarted { .. }) => Loading(l),
        (_, Event::LoadStarted { hue }) => Loading(Lap::new(hue)),
        (Loading(l), Event::LoadEnded) => Lapping(l),
        (b, Event::LoadEnded) => b,
        (Idle, Event::Tick(_)) => Idle,
        (Lapping(l), Event::Tick(dt)) => {
            let k = l.k + dt / LAP;
            if k >= 1.0 {
                Idle
            } else {
                Lapping(Lap { k, ..l })
            }
        }
        (Loading(l), Event::Tick(dt)) => {
            let k = l.k + dt / LAP;
            Loading(if k <= FADE_FROM {
                Lap { k, ..l }
            } else {
                // up to the hold point on the curve, the rest as extra travel at its speed
                let held = (k - FADE_FROM.max(l.k)) * LAP;
                Lap {
                    k: FADE_FROM,
                    spin: (l.spin + held * head_speed(FADE_FROM)).rem_euclid(1.0),
                    ..l
                }
            })
        }
    }
}

impl Beam {
    /// What to draw this frame, if anything.
    pub fn shape(&self) -> Option<Shape> {
        match self {
            Beam::Idle => None,
            Beam::Lapping(l) | Beam::Loading(l) => Some(l.shape()),
        }
    }

    /// Whether frames are still needed.
    pub fn running(&self) -> bool {
        !matches!(self, Beam::Idle)
    }
}

/// Watches the shown page and turns what changed into beam events. The shell keeps one and
/// calls [`Watch::observe`] once per frame with the shown tab and whether it loads.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Watch {
    /// The last frame's shown tab and its loading flag; `None` before the first frame.
    last: Option<(Option<i64>, bool)>,
}

impl Watch {
    /// The event for this frame's shown tab and loading flag, if any. The first tab shown (at
    /// launch, or after none) only records: the design starts no lap on the first render. A page
    /// already loading then still laps.
    pub fn observe(&mut self, tab: Option<i64>, loading: bool, hue: f64) -> Option<Event> {
        let (was, was_loading) = self.last.replace((tab, loading)).unwrap_or((None, false));
        match (was, tab) {
            // another tab shown
            (Some(a), Some(b)) if a != b => Some(Event::Shown { loading, hue }),
            // the same tab (or the first, or none): only its loading
            _ if loading && !was_loading => Some(Event::LoadStarted { hue }),
            _ if !loading && was_loading => Some(Event::LoadEnded),
            _ => None,
        }
    }
}

/// The `n`th lap's hue: the golden-ratio sequence, so consecutive laps never look alike (the
/// design draws `Math.random()`; this keeps the core deterministic).
pub fn hue(n: u64) -> f64 {
    (n as f64 * 0.618_033_988_749_895).fract()
}

// @source command-bar (browser/command.tsx, "thinking")
/// The command bar's beam while you type: every edit runs one 1 s lap round the bar, the head
/// from 10% to 100% of the perimeter, bright after 200 ms, fading over the second half
/// (command.tsx: "while you type, the beam laps the bar; it eases out once results settle").
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum Thinking {
    #[default]
    Idle,
    /// Seconds into the lap.
    Lapping(f64),
}

/// One thinking lap, seconds.
pub const THINK: f64 = 1.0;

impl Thinking {
    /// The query changed: a new lap (the TS effect restarts on `q`); an empty query ends it.
    pub fn typed(empty: bool) -> Self {
        if empty {
            Thinking::Idle
        } else {
            Thinking::Lapping(0.0)
        }
    }

    pub fn tick(self, dt: f64) -> Self {
        match self {
            Thinking::Lapping(t) if t + dt < THINK => Thinking::Lapping(t + dt),
            _ => Thinking::Idle,
        }
    }

    pub fn shape(&self) -> Option<Shape> {
        let Thinking::Lapping(k) = *self else {
            return None;
        };
        Some(Shape {
            head: 0.1 + k * 0.9,
            length: 0.3,
            strength: (k * 5.0).min(1.0) * (1.0 - (k - 0.5).max(0.0) / 0.5).max(0.0),
            hue: 0.2,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(mut b: Beam, secs: f64) -> Beam {
        let mut t = 0.0;
        while t < secs - 1e-9 {
            b = transition(b, Event::Tick(1.0 / 120.0));
            t += 1.0 / 120.0;
        }
        b
    }

    #[test]
    fn the_curve_matches_the_design() {
        // chrome.tsx: head 0.08 + e·1.35, length 0.35 + 0.3 f, strength min(1, 12.5 k) · fade²
        let start = shape_at(0.0, 0.3);
        assert_eq!(
            (start.head, start.length, start.strength),
            (0.08, 0.35, 0.0)
        );
        assert_eq!(shape_at(0.08, 0.0).strength, 1.0, "bright after 120 ms");
        assert_eq!(
            shape_at(FADE_FROM, 0.0).strength,
            1.0,
            "full until the fade"
        );
        let end = shape_at(1.0, 0.0);
        assert!((end.head - 1.43).abs() < 1e-12);
        assert!((end.length - 0.65).abs() < 1e-12);
        assert_eq!(end.strength, 0.0);
        let mid = shape_at(0.675, 0.0);
        assert!(
            (mid.strength - 0.25).abs() < 1e-12,
            "fade 1/2 at f = 1/2, squared"
        );
    }

    #[test]
    fn a_tab_switch_runs_one_lap_and_ends() {
        let b = transition(
            Beam::Idle,
            Event::Shown {
                loading: false,
                hue: 0.4,
            },
        );
        assert!(matches!(b, Beam::Lapping(_)));
        assert!(run(b, 0.2).shape().unwrap().strength > 0.99);
        let late = run(b, LAP * 0.9).shape().unwrap();
        assert!(
            late.strength > 0.0 && late.strength < 0.1,
            "fading: {}",
            late.strength
        );
        assert_eq!(run(b, LAP + 0.02), Beam::Idle);
    }

    #[test]
    fn loading_keeps_lapping_then_fades_over_the_rest_of_the_lap() {
        let b = transition(Beam::Idle, Event::LoadStarted { hue: 0.0 });
        let b = run(b, 10.0);
        let Beam::Loading(l) = b else { panic!("{b:?}") };
        assert_eq!(l.k, FADE_FROM);
        let s = b.shape().unwrap();
        assert_eq!((s.strength, s.length), (1.0, 0.35));
        // the head keeps moving, at the speed it had where the fade begins
        let s2 = run(b, 0.1).shape().unwrap();
        let moved = (s2.head - s.head).rem_euclid(1.0);
        assert!(
            (moved - 0.1 * head_speed(FADE_FROM)).abs() < 1e-6,
            "{moved}"
        );
        // load ends: the design's last 65% of a lap, then nothing
        let ended = transition(b, Event::LoadEnded);
        assert_eq!(ended.shape(), b.shape(), "continuous");
        let rest = LAP * (1.0 - FADE_FROM);
        assert!(run(ended, rest - 0.05).running());
        assert_eq!(run(ended, rest + 0.02), Beam::Idle);
    }

    #[test]
    fn a_short_load_finishes_its_lap_on_the_curve() {
        let b = transition(Beam::Idle, Event::LoadStarted { hue: 0.0 });
        let b = transition(run(b, 0.2), Event::LoadEnded);
        let Beam::Lapping(l) = b else { panic!() };
        assert!((l.k - 0.2 / LAP).abs() < 1e-9 && l.spin == 0.0);
        assert_eq!(run(b, LAP - 0.2 + 0.02), Beam::Idle);
    }

    #[test]
    fn showing_another_tab_restarts_the_lap() {
        let b = run(transition(Beam::Idle, Event::LoadStarted { hue: 0.0 }), 3.0);
        let b = transition(
            b,
            Event::Shown {
                loading: false,
                hue: 0.5,
            },
        );
        assert_eq!(b, Beam::Lapping(Lap::new(0.5)));
        let b = transition(
            b,
            Event::Shown {
                loading: true,
                hue: 0.7,
            },
        );
        assert_eq!(b, Beam::Loading(Lap::new(0.7)));
        assert_eq!(transition(b, Event::Reduce), Beam::Idle);
    }

    #[test]
    fn a_navigation_restarts_a_fading_lap() {
        let b = run(
            transition(
                Beam::Idle,
                Event::Shown {
                    loading: false,
                    hue: 0.0,
                },
            ),
            1.0,
        );
        assert_eq!(
            transition(b, Event::LoadStarted { hue: 0.9 }),
            Beam::Loading(Lap::new(0.9))
        );
        assert_eq!(transition(Beam::Idle, Event::LoadEnded), Beam::Idle);
    }

    #[test]
    fn thinking_laps_once_per_edit() {
        let t = Thinking::typed(false);
        assert_eq!(t.shape().unwrap().strength, 0.0);
        let s = t.tick(0.25).shape().unwrap();
        assert_eq!((s.strength, s.length, s.hue), (1.0, 0.3, 0.2));
        assert!((s.head - 0.325).abs() < 1e-12);
        assert!((t.tick(0.75).shape().unwrap().strength - 0.5).abs() < 1e-12);
        assert_eq!(t.tick(1.0), Thinking::Idle);
        assert_eq!(Thinking::typed(true), Thinking::Idle);
    }

    #[test]
    fn the_watch_turns_frames_into_events() {
        let mut w = Watch::default();
        assert_eq!(
            w.observe(Some(1), false, 0.1),
            None,
            "nothing on the first render"
        );
        assert_eq!(w.observe(Some(1), false, 0.1), None);
        assert_eq!(
            w.observe(Some(2), false, 0.2),
            Some(Event::Shown {
                loading: false,
                hue: 0.2
            })
        );
        assert_eq!(
            w.observe(Some(2), true, 0.3),
            Some(Event::LoadStarted { hue: 0.3 })
        );
        assert_eq!(w.observe(Some(2), true, 0.3), None);
        assert_eq!(w.observe(Some(2), false, 0.3), Some(Event::LoadEnded));
        assert_eq!(
            w.observe(Some(3), true, 0.4),
            Some(Event::Shown {
                loading: true,
                hue: 0.4
            })
        );
        assert_eq!(
            w.observe(None, true, 0.4),
            None,
            "no page: its load is still the one"
        );
        assert_eq!(
            w.observe(Some(5), false, 0.4),
            Some(Event::LoadEnded),
            "the first tab after none only ends the load"
        );
        let mut w = Watch::default();
        assert_eq!(w.observe(None, false, 0.5), None);
        assert_eq!(
            w.observe(Some(1), false, 0.5),
            None,
            "no lap for the first tab at launch"
        );
        let mut w = Watch::default();
        assert_eq!(
            w.observe(Some(1), true, 0.5),
            Some(Event::LoadStarted { hue: 0.5 }),
            "launch load"
        );
    }
}
