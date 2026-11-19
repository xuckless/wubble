//! `--smoke start [--shot-dir DIR]` (hand run only): the start page on screen. The bundled page
//! is pinned (a favourite, so the page has a tile), Ctrl+T's action opens a start tab, and the
//! scene must show its bubble: bright pixels (the W tube, the film, the speculars) inside the
//! body's circle on the dark liquid. A press and release on the body (real pointer events
//! through the page's stage) must poke it: a wave runs through it and its pixels change. A press
//! on a mini pops it. The search capsule is the page's address field (the command bar in the
//! page): the new tab opens with it holding the keys; after the stage took them, Ctrl+L gives them
//! back to it; typed keys land in it and its results show under it; Ctrl+L again keeps them
//! there; Enter navigates the start tab. The overlay bar never opens. With `--shot-dir`: one PNG
//! at rest (the capsule focused), mid-poke, after the pop and with the results. Every wait is
//! "until the host sees X", bounded by a count of CEF work runs, never by time.

use crate::nav_smoke::MAX_WORK_RUNS;

/// Scene pixels of the body that must be bright (any channel at least [`BRIGHT`]): the dark
/// liquid at its muted 0.3 never reaches it, the W tube and the film do.
pub const BRIGHT_MIN: usize = 24;
pub const BRIGHT: u8 = 150;
/// Work runs the bubble gets to settle before the rest check, and to move after a poke.
pub const SETTLE_RUNS: u32 = 40;

/// What the host sees after a run of CEF work.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Seen {
    /// The window can size views (the frame rate is known).
    pub ready: bool,
    /// The visible tab is pinned (a tile on the start page).
    pub pinned: bool,
    /// The visible tab is a start page, the scene's layer is up and has its W field.
    pub start: bool,
    /// Tiles the start page shows.
    pub sites: usize,
    /// The body's centre and radius, page-local px.
    pub body: Option<((f64, f64), f64)>,
    /// Waves running through the body.
    pub waves: usize,
    /// A mini's centre, page-local px.
    pub mini: Option<(f64, f64)>,
    /// Pops being drawn.
    pub pops: usize,
    /// Bright scene pixels inside the body (read only when [`StartSmoke::wants_pixels`]).
    pub bright: usize,
    /// A hash of the scene's pixels over the body (likewise).
    pub body_pixels: u64,
    /// The capsule holds the keys.
    pub search_focused: bool,
    /// The command bar is open in the page (its field the capsule).
    pub in_page: bool,
    /// The overlay command bar shows.
    pub overlay: bool,
    /// The bar's query (the capsule's text).
    pub query: String,
    /// The bar's rows.
    pub rows: usize,
    /// The visible tab left the start page for a site.
    pub navigated: bool,
}

/// What the smoke types into the capsule: an internal page, so Enter navigates without the
/// network.
pub const TYPED: &str = "wubble://history";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Start,
    /// The bundled page pinned; waiting for it to be a favourite.
    Pinning,
    /// A start tab asked for; waiting for its bubble.
    Opening,
    /// The bubble is drawn; letting it settle, then checking its pixels.
    Rest,
    /// Poked; waiting for the wave and the change.
    Poked {
        rest: u64,
    },
    /// A mini pressed; waiting for its pop.
    Popping,
    /// Ctrl+L pressed; waiting for the capsule to take the keys.
    Searching,
    /// Text typed; waiting for it in the capsule and its results.
    Typing,
    /// The results show; letting them grow, then Ctrl+L again.
    Results,
    /// Ctrl+L pressed again; the keys must stay in the capsule, the text with them.
    Reselect,
    /// Enter pressed; waiting for the tab to leave the start page.
    Navigating,
    Done,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StartSmoke {
    pub stage: Stage,
    runs: u32,
}

impl Default for StartSmoke {
    fn default() -> Self {
        Self {
            stage: Stage::Start,
            runs: 0,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    /// Pin the visible tab.
    Pin,
    /// Run `Action::NewTab` (Ctrl+T).
    OpenStart,
    /// Write the window as `NN-name.png` into the shot dir, when there is one.
    Shoot(&'static str),
    /// Press and release at this page-local point (the stage's own pointer events).
    Click(f64, f64),
    /// These chords through the host's key path (`parity::chords`).
    Keys(&'static str),
    /// This text typed through the host's key path.
    Type(&'static str),
    Pass,
    Fail(String),
}

impl StartSmoke {
    /// Whether this step needs the scene's pixels (a readback that waits for the GPU).
    pub fn wants_pixels(&self) -> bool {
        match self.stage {
            Stage::Rest => self.runs >= SETTLE_RUNS,
            Stage::Poked { .. } => self.runs >= SETTLE_RUNS / 4,
            _ => false,
        }
    }
}

pub fn next(s: StartSmoke, seen: &Seen) -> (StartSmoke, Vec<Action>) {
    use Stage::*;
    let to = |stage| StartSmoke { stage, runs: 0 };
    let fail = |why: String| Some((Done, vec![Action::Fail(why)]));
    // the capsule is the page's only address field: the overlay never opens on it
    let watching = !matches!(s.stage, Start | Pinning | Opening | Done);
    let focused = seen.search_focused && seen.in_page;
    let step: Option<(Stage, Vec<Action>)> = match s.stage {
        _ if watching && seen.overlay => fail(format!(
            "the overlay command bar opened on the start page at {:?}",
            s.stage
        )),
        Start if seen.ready => Some((Pinning, vec![Action::Pin])),
        Pinning if seen.pinned => Some((Opening, vec![Action::OpenStart])),
        // the new tab's capsule holds the keys
        Opening if seen.start && seen.body.is_some() && focused => Some((Rest, vec![])),
        Rest if s.wants_pixels() => match seen.body {
            Some((c, _)) if seen.bright >= BRIGHT_MIN && seen.sites >= 1 => Some((
                Poked {
                    rest: seen.body_pixels,
                },
                vec![Action::Shoot("rest"), Action::Click(c.0, c.1)],
            )),
            _ => fail(format!(
                "no bubble on the start page: {} bright pixels (want {BRIGHT_MIN}), {} tiles",
                seen.bright, seen.sites
            )),
        },
        Poked { rest } if s.wants_pixels() && seen.waves > 0 && seen.body_pixels != rest => {
            match seen.mini {
                Some(m) => Some((
                    Popping,
                    vec![Action::Shoot("poke"), Action::Click(m.0, m.1)],
                )),
                None => fail("the poke threw no minis".into()),
            }
        }
        // the stage took the keys from the capsule; Ctrl+L gives them back to it
        Popping if seen.pops > 0 && !seen.search_focused => Some((
            Searching,
            vec![Action::Shoot("pop"), Action::Keys("Ctrl+L")],
        )),
        Searching if focused => Some((Typing, vec![Action::Type(TYPED)])),
        Typing if focused && seen.query == TYPED && seen.rows > 0 => Some((Results, vec![])),
        Results if s.runs >= SETTLE_RUNS => Some((
            Reselect,
            vec![Action::Shoot("results"), Action::Keys("Ctrl+L")],
        )),
        Reselect if s.runs >= SETTLE_RUNS / 4 => {
            if focused && seen.query == TYPED {
                Some((Navigating, vec![Action::Keys("Enter")]))
            } else {
                fail(format!(
                    "Ctrl+L on the start page lost the capsule's keys or text: {seen:?}"
                ))
            }
        }
        Navigating if seen.navigated => Some((Done, vec![Action::Pass])),
        _ => None,
    };
    match step {
        Some((stage, actions)) => (to(stage), actions),
        None if s.stage == Done => (s, vec![]),
        None if s.runs >= MAX_WORK_RUNS => (
            to(Done),
            vec![Action::Fail(format!(
                "stuck at {:?} after {MAX_WORK_RUNS} runs of CEF work: {seen:?}",
                s.stage
            ))],
        ),
        None => (
            StartSmoke {
                runs: s.runs + 1,
                ..s
            },
            vec![],
        ),
    }
}

/// Bright pixels and a hash over a circle of `rgba` (`w` px wide, `scale` physical px per
/// logical px): centre `c` and radius `r` in window logical px.
pub fn body_stats(rgba: &[u8], w: u32, scale: f64, c: (f64, f64), r: f64) -> (usize, u64) {
    let h = (rgba.len() / 4) as u32 / w.max(1);
    let (cx, cy, rr) = (c.0 * scale, c.1 * scale, r * scale);
    let (mut bright, mut hash) = (0, 0xcbf2_9ce4_8422_2325u64);
    let lo = |v: f64| v.floor().max(0.0) as u32;
    for y in lo(cy - rr)..(lo(cy + rr) + 1).min(h) {
        for x in lo(cx - rr)..(lo(cx + rr) + 1).min(w) {
            let (dx, dy) = (f64::from(x) - cx, f64::from(y) - cy);
            if dx * dx + dy * dy > rr * rr {
                continue;
            }
            let o = ((y * w + x) * 4) as usize;
            let p = &rgba[o..o + 3];
            if p.iter().any(|&v| v >= BRIGHT) {
                bright += 1;
            }
            for &v in p {
                hash = (hash ^ u64::from(v)).wrapping_mul(0x0100_0000_01b3);
            }
        }
    }
    (bright, hash)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(mut s: StartSmoke, seen: &Seen, steps: u32) -> (StartSmoke, Vec<Action>) {
        let mut all = vec![];
        for _ in 0..steps {
            let (n, a) = next(s, seen);
            s = n;
            all.extend(a);
        }
        (s, all)
    }

    #[test]
    fn the_steps_in_order() {
        let mut seen = Seen {
            ready: true,
            ..Seen::default()
        };
        let (s, a) = next(StartSmoke::default(), &seen);
        assert_eq!(a, [Action::Pin]);
        seen.pinned = true;
        let (s, a) = next(s, &seen);
        assert_eq!(a, [Action::OpenStart]);
        let (s, a) = next(s, &seen);
        assert!(a.is_empty(), "no bubble yet");
        seen.start = true;
        seen.sites = 1;
        seen.body = Some(((400.0, 300.0), 80.0));
        let (s, _) = next(s, &seen);
        assert_eq!(s.stage, Stage::Opening, "the capsule must hold the keys");
        seen.search_focused = true;
        seen.in_page = true;
        let (s, _) = next(s, &seen);
        assert_eq!(s.stage, Stage::Rest);
        assert!(!s.wants_pixels());
        seen.bright = BRIGHT_MIN;
        seen.body_pixels = 1;
        let (s, a) = run(s, &seen, SETTLE_RUNS + 1);
        assert_eq!(a, [Action::Shoot("rest"), Action::Click(400.0, 300.0)]);
        assert_eq!(s.stage, Stage::Poked { rest: 1 });
        // the same pixels: not poked yet
        seen.waves = 1;
        let (s, a) = run(s, &seen, SETTLE_RUNS);
        assert!(a.is_empty());
        seen.body_pixels = 2;
        seen.mini = Some((10.0, 20.0));
        let (s, a) = next(s, &seen);
        assert_eq!(a, [Action::Shoot("poke"), Action::Click(10.0, 20.0)]);
        seen.pops = 1;
        let (s, a) = next(s, &seen);
        assert!(a.is_empty(), "the stage has not taken the keys yet");
        seen.search_focused = false;
        seen.in_page = false;
        let (s, a) = next(s, &seen);
        assert_eq!(a, [Action::Shoot("pop"), Action::Keys("Ctrl+L")]);
        seen.search_focused = true;
        seen.in_page = true;
        let (s, a) = next(s, &seen);
        assert_eq!(a, [Action::Type(TYPED)]);
        seen.query = TYPED.into();
        seen.rows = 3;
        let (s, a) = next(s, &seen);
        assert!(a.is_empty(), "the results grow first");
        let (s, a) = run(s, &seen, SETTLE_RUNS + 1);
        assert_eq!(a, [Action::Shoot("results"), Action::Keys("Ctrl+L")]);
        let (s, a) = run(s, &seen, SETTLE_RUNS / 4 + 1);
        assert_eq!(a, [Action::Keys("Enter")]);
        seen.navigated = true;
        let (s, a) = next(s, &seen);
        assert_eq!((s.stage, a), (Stage::Done, vec![Action::Pass]));
    }

    #[test]
    fn the_overlay_bar_fails_it_and_ctrl_l_must_keep_the_text() {
        let seen = Seen {
            overlay: true,
            ..Seen::default()
        };
        let s = StartSmoke {
            stage: Stage::Searching,
            runs: 0,
        };
        let (s, a) = next(s, &seen);
        assert_eq!(s.stage, Stage::Done);
        assert!(matches!(a.as_slice(), [Action::Fail(e)] if e.contains("overlay")));
        let seen = Seen {
            search_focused: true,
            in_page: true,
            query: String::new(),
            ..Seen::default()
        };
        let s = StartSmoke {
            stage: Stage::Reselect,
            runs: SETTLE_RUNS / 4,
        };
        let (_, a) = next(s, &seen);
        assert!(matches!(a.as_slice(), [Action::Fail(e)] if e.contains("Ctrl+L")));
    }

    #[test]
    fn a_dark_body_fails_and_a_wait_is_bounded() {
        let seen = Seen {
            ready: true,
            pinned: true,
            start: true,
            sites: 1,
            body: Some(((1.0, 1.0), 1.0)),
            ..Seen::default()
        };
        let s = StartSmoke {
            stage: Stage::Rest,
            runs: 0,
        };
        let (_, a) = run(s, &seen, SETTLE_RUNS + 1);
        assert!(matches!(a.as_slice(), [Action::Fail(e)] if e.contains("no bubble")));
        let s = StartSmoke {
            stage: Stage::Searching,
            runs: 0,
        };
        let (s, a) = run(s, &seen, MAX_WORK_RUNS + 1);
        assert_eq!(s.stage, Stage::Done);
        assert!(matches!(a.as_slice(), [Action::Fail(e)] if e.contains("Searching")));
    }

    #[test]
    fn body_stats_count_bright_pixels_in_the_circle_only() {
        // 8x8 logical at scale 1: bright everywhere but the circle's corners
        let (w, h) = (8u32, 8u32);
        let mut rgba = vec![0u8; (w * h * 4) as usize];
        for px in rgba.chunks_mut(4) {
            px.copy_from_slice(&[200, 10, 10, 255]);
        }
        let (bright, a) = body_stats(&rgba, w, 1.0, (4.0, 4.0), 2.0);
        assert!(bright > 0 && bright < 25, "{bright}");
        rgba[(4 * w + 4) as usize * 4] = 0;
        let (_, b) = body_stats(&rgba, w, 1.0, (4.0, 4.0), 2.0);
        assert_ne!(a, b);
        // off the image: nothing
        assert_eq!(body_stats(&rgba, w, 1.0, (100.0, 100.0), 2.0).0, 0);
    }
}
