//! `--smoke pinch`: a trackpad pinch through the host's pinch path (`Host::pinch`, what the
//! Wayland and macOS gesture events call) on the loopback `assets/pinch.html`, checked by asking
//! the page (`pinchLook()`). On the engine in use: the touch road on a stock CEF, the touchpad
//! road with patch 40 (`wubble_engine::pinch`).
//!
//! 1. `?prevent` (the page keeps every Ctrl+wheel, as a map does): a pinch leaves the scale at
//!    1, the page saw its Ctrl+wheels, the layout width and the page zoom did not change.
//! 2. Ctrl + (the zoom keys' path): the page zoom grows, the visual scale stays 1.
//! 3. Back to 100 %, a plain page (a new document): scale 1.
//! 4. A second tab beside it (split; the first tab's pane lands on the right): a pinch over the
//!    first tab's pane magnifies it (scale above
//!    [`ZOOMED`]) with the same layout width and page zoom; no click, context menu, selection
//!    or pressed element is left, the hover is back, `pointer: coarse` never matched (asked
//!    during the gesture too); the other pane stays at 1.
//! 5. Pinched far out: back at exactly 1, never below.
//! 6. Pinched in again, for the window shot (`--shot`).
//!
//! Every wait is counted in runs of CEF work, never timed.

use std::collections::VecDeque;

use serde_json::Value;
use wubble_core::gesture::PinchInput;
use wubble_engine::input::Point;

use crate::nav_smoke::MAX_WORK_RUNS;

/// Where the pointer rests for a pinch, view-relative: over the page's `#hov` box.
pub const HOVER_AT: Point = Point { x: 200.0, y: 150.0 };
/// Runs of CEF work after the last pinch input before the page is asked.
pub const SETTLE: u32 = 120;
/// Asks a check may repeat while the page catches up before it fails.
const RETRIES: u32 = 15;
/// A pinch in must have magnified the page past this.
pub const ZOOMED: f64 = 1.5;

/// What `pinchLook()` answers.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Look {
    pub scale: f64,
    pub width: f64,
    pub dpr: f64,
    pub search: String,
    /// click, dblclick, auxclick and contextmenu events.
    pub clicks: u64,
    pub touches: u64,
    pub pointerdowns: u64,
    pub ctrl_wheels: u64,
    pub selection: String,
    pub hover: bool,
    pub active: bool,
    pub coarse: bool,
    pub any_coarse: bool,
    pub hover_none: bool,
}

impl Look {
    /// The eval's JSON text (the object, or the object as a JSON string).
    pub fn parse(text: &str) -> Option<Self> {
        let mut v: Value = serde_json::from_str(text).ok()?;
        if let Value::String(inner) = &v {
            v = serde_json::from_str(inner).ok()?;
        }
        let f = |k: &str| v.get(k).and_then(Value::as_f64);
        let b = |k: &str| v.get(k).and_then(Value::as_bool).unwrap_or(false);
        let seen = |k: &str| {
            v.get("seen")
                .and_then(|s| s.get(k))
                .and_then(Value::as_u64)
                .unwrap_or(0)
        };
        Some(Self {
            scale: f("scale")?,
            width: f("width")?,
            dpr: f("dpr")?,
            search: v.get("search")?.as_str()?.to_owned(),
            clicks: ["click", "dblclick", "auxclick", "contextmenu"]
                .iter()
                .map(|k| seen(k))
                .sum(),
            touches: seen("touchstart"),
            pointerdowns: seen("pointerdown"),
            ctrl_wheels: seen("ctrlWheels"),
            selection: v.get("selection")?.as_str()?.to_owned(),
            hover: b("hover"),
            active: b("active"),
            coarse: b("coarse"),
            any_coarse: b("anyCoarse"),
            hover_none: b("hoverNone"),
        })
    }
}

/// What the pages are asked after.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Check {
    Base,
    Prevented,
    PageZoomed,
    Fresh,
    SplitBase,
    Pinched,
    Out,
    Final,
}

impl Check {
    /// The panes asked.
    fn panes(self) -> usize {
        match self {
            Check::SplitBase | Check::Pinched => 2,
            _ => 1,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Load,
    /// Waiting for the pinch inputs to be fed and the pages to settle.
    Settle {
        check: Check,
        runs: u32,
    },
    /// Asked; waiting for the answers.
    Asked {
        check: Check,
    },
    /// The second tab opening, then split.
    Opening,
    Splitting,
    Done,
}

/// One item of a pinch the host feeds, one per run of CEF work.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Drip {
    Gesture(PinchInput),
    /// Ask the pinched pane while the fingers are down.
    Look,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    /// Ask pane `n` (`pinchLook()`).
    Look(usize),
    /// The pointer to [`HOVER_AT`] in pane `n`, then the pinch, fed by the host.
    Pinch(usize, Vec<Drip>),
    ZoomIn,
    ZoomReset,
    /// Load the plain fixture (no `?prevent`) in pane 0's tab.
    LoadPlain,
    OpenSecond,
    Split,
    /// A measured line to print.
    Note(String),
    Pass(String),
    Fail(String),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Input<'a> {
    /// A run of CEF work: tabs in the Space, panes on screen with a frame, and whether the
    /// host has fed every pinch input.
    Worked {
        tabs: usize,
        framed: usize,
        fed: bool,
    },
    /// Pane `n`'s answer; `during`: asked while the fingers were down.
    Looked {
        pane: usize,
        during: bool,
        look: Option<&'a Look>,
    },
}

/// A pinch of `steps` updates of `factor`, with a look halfway when `look`.
pub fn gesture(factor: f64, steps: usize, look: bool) -> Vec<Drip> {
    let mut drips = vec![Drip::Gesture(PinchInput::Begin)];
    for i in 0..steps {
        if look && i == steps / 2 {
            drips.push(Drip::Look);
        }
        drips.push(Drip::Gesture(PinchInput::Scaled(factor)));
    }
    drips.push(Drip::Gesture(PinchInput::End));
    drips
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PinchSmoke {
    pub stage: Option<Stage>,
    runs: u32,
    retries: u32,
    /// The plain page's first look, and each pane's once split.
    base: Option<Look>,
    split: [Option<Look>; 2],
    /// This check's answers so far.
    looks: [Option<Look>; 2],
    during: Option<Look>,
}

/// The host's side of the smoke: the machine and the pinch inputs it still has to feed.
#[derive(Clone, Debug, Default)]
pub struct Run {
    pub smoke: PinchSmoke,
    pub drips: VecDeque<(usize, Drip)>,
}

fn settle(check: Check) -> Stage {
    Stage::Settle { check, runs: 0 }
}

impl PinchSmoke {
    fn at(&mut self, stage: Stage) {
        if self.stage != Some(stage) {
            self.runs = 0;
        }
        self.stage = Some(stage);
    }

    fn ask(&mut self, check: Check) -> Vec<Action> {
        self.looks = [None, None];
        self.at(Stage::Asked { check });
        (0..check.panes()).map(Action::Look).collect()
    }

    fn fail(&mut self, why: String) -> Vec<Action> {
        self.stage = Some(Stage::Done);
        vec![Action::Fail(why)]
    }

    /// Not yet: ask again after another settle, a few times.
    fn again(&mut self, check: Check, why: String) -> Vec<Action> {
        self.retries += 1;
        if self.retries > RETRIES {
            return self.fail(why);
        }
        self.at(settle(check));
        vec![]
    }

    fn checked(&mut self, check: Check) -> Vec<Action> {
        self.retries = 0;
        self.at(settle(check));
        vec![]
    }

    pub fn next(&mut self, input: Input) -> Vec<Action> {
        let stage = *self.stage.get_or_insert(Stage::Load);
        match (stage, input) {
            (Stage::Done, _) => vec![],
            (_, Input::Worked { .. }) if self.runs >= MAX_WORK_RUNS => self.fail(format!(
                "pinch: stuck at {stage:?} for {MAX_WORK_RUNS} runs"
            )),
            (Stage::Load, Input::Worked { framed, .. }) => {
                self.runs += 1;
                if framed >= 1 {
                    return self.ask(Check::Base);
                }
                vec![]
            }
            (Stage::Settle { check, runs }, Input::Worked { fed, .. }) => {
                self.runs += 1;
                if !fed {
                    return vec![];
                }
                if runs + 1 < SETTLE {
                    self.stage = Some(Stage::Settle {
                        check,
                        runs: runs + 1,
                    });
                    return vec![];
                }
                self.ask(check)
            }
            (Stage::Opening, Input::Worked { tabs, .. }) => {
                self.runs += 1;
                if tabs == 2 {
                    self.at(Stage::Splitting);
                    return vec![Action::Split];
                }
                vec![]
            }
            (Stage::Splitting, Input::Worked { framed, .. }) => {
                self.runs += 1;
                if framed == 2 {
                    self.retries = 0;
                    self.at(settle(Check::SplitBase));
                }
                vec![]
            }
            (Stage::Asked { .. }, Input::Worked { .. }) => {
                self.runs += 1;
                vec![]
            }
            (
                _,
                Input::Looked {
                    during: true, look, ..
                },
            ) => {
                self.during = look.cloned();
                vec![]
            }
            (Stage::Asked { check }, Input::Looked { pane, look, .. }) => {
                let Some(look) = look else {
                    return self.again(check, format!("pinch: pane {pane} gave no look"));
                };
                if let Some(slot) = self.looks.get_mut(pane) {
                    *slot = Some(look.clone());
                }
                if self.looks[..check.panes()].iter().all(Option::is_some) {
                    let looks = self.looks.clone();
                    return self.judge(check, &looks);
                }
                vec![]
            }
            (_, Input::Looked { .. }) => vec![],
        }
    }

    fn judge(&mut self, check: Check, looks: &[Option<Look>; 2]) -> Vec<Action> {
        let a = looks[0].clone().unwrap_or_default();
        let b = looks[1].clone();
        let base = self.base.clone().unwrap_or_default();
        let inward = |look: bool| Action::Pinch(0, gesture(1.12, 12, look));
        match check {
            Check::Base => {
                if !a.search.contains("prevent") || a.scale != 1.0 {
                    return self.fail(format!("pinch: the first page is not as loaded: {a:?}"));
                }
                self.base = Some(a);
                let mut out = self.checked(Check::Prevented);
                out.push(inward(false));
                out
            }
            Check::Prevented => {
                if a.ctrl_wheels == 0 {
                    return self.again(check, "pinch: the page saw no Ctrl+wheel".into());
                }
                if a.scale != 1.0 || a.width != base.width || a.dpr != base.dpr {
                    return self.fail(format!(
                        "pinch: a page that keeps the Ctrl+wheel was zoomed: {a:?}, before {base:?}"
                    ));
                }
                let mut out = vec![Action::Note(format!(
                    "page kept the gesture: scale {} after {} Ctrl+wheels",
                    a.scale, a.ctrl_wheels
                ))];
                out.extend(self.checked(Check::PageZoomed));
                out.push(Action::ZoomIn);
                out
            }
            Check::PageZoomed => {
                if a.dpr <= base.dpr {
                    return self
                        .again(check, format!("pinch: Ctrl + did not zoom the page: {a:?}"));
                }
                if a.scale != 1.0 || a.width >= base.width {
                    return self.fail(format!(
                        "pinch: Ctrl + changed the visual scale or did not reflow: {a:?}"
                    ));
                }
                let mut out = vec![Action::Note(format!(
                    "Ctrl +: page zoom {:.2}x (width {} from {}), visual scale {}",
                    a.dpr / base.dpr,
                    a.width,
                    base.width,
                    a.scale
                ))];
                out.extend(self.checked(Check::Fresh));
                out.extend([Action::ZoomReset, Action::LoadPlain]);
                out
            }
            Check::Fresh => {
                if !a.search.is_empty() || a.dpr != base.dpr {
                    return self.again(check, format!("pinch: the plain page is not there: {a:?}"));
                }
                if a.scale != 1.0 {
                    return self.fail(format!("pinch: a new page starts zoomed: {a:?}"));
                }
                self.at(Stage::Opening);
                vec![Action::OpenSecond]
            }
            Check::SplitBase => {
                let b = b.unwrap_or_default();
                if a.scale != 1.0 || b.scale != 1.0 {
                    return self.fail(format!("pinch: split panes start zoomed: {a:?} {b:?}"));
                }
                self.split = [Some(a), Some(b)];
                let mut out = self.checked(Check::Pinched);
                out.push(inward(true));
                out
            }
            Check::Pinched => {
                let b = b.unwrap_or_default();
                let [Some(a0), Some(b0)] = self.split.clone() else {
                    return self.fail("pinch: no split base".into());
                };
                if a.scale <= ZOOMED {
                    return self.again(check, format!("pinch: the pane was not magnified: {a:?}"));
                }
                let during = self.during.clone().unwrap_or_default();
                let problems: Vec<String> = [
                    (a.width != a0.width, "the layout reflowed"),
                    (a.dpr != base.dpr, "the page zoom changed"),
                    (
                        a.clicks != a0.clicks,
                        "a click or context menu reached the page",
                    ),
                    (!a.selection.is_empty(), "text got selected"),
                    (a.active, "an element stayed pressed"),
                    (!a.hover, "the hover did not come back"),
                    (a.coarse || a.any_coarse, "`pointer: coarse` matched after"),
                    (
                        self.during.is_none(),
                        "the page gave no look during the gesture",
                    ),
                    (
                        during.coarse || during.any_coarse,
                        "`pointer: coarse` matched during",
                    ),
                    (b.scale != 1.0, "the other pane was zoomed"),
                    (b.width != b0.width, "the other pane reflowed"),
                ]
                .into_iter()
                .filter(|(bad, _)| *bad)
                .map(|(_, what)| what.to_owned())
                .collect();
                if !problems.is_empty() {
                    return self.fail(format!(
                        "pinch: {}: after {a:?}, during {during:?}, other {b:?}",
                        problems.join("; ")
                    ));
                }
                let mut out = vec![Action::Note(format!(
                    "pinched: scale {:.2}, width {} (unchanged), page zoom unchanged; left on the \
                     page: {} touchstart, {} pointerdown, 0 clicks; during: hover {} hover:none {} (before: {}); \
                     other pane scale {}",
                    a.scale,
                    a.width,
                    a.touches.saturating_sub(a0.touches),
                    a.pointerdowns.saturating_sub(a0.pointerdowns),
                    during.hover,
                    during.hover_none,
                    a0.hover_none,
                    b.scale
                ))];
                out.extend(self.checked(Check::Out));
                out.push(Action::Pinch(0, gesture(0.8, 20, false)));
                out
            }
            Check::Out => {
                if a.scale != 1.0 {
                    return self.again(check, format!("pinch: pinched out, not back at 1: {a:?}"));
                }
                let mut out = vec![Action::Note(format!("pinched out: scale {}", a.scale))];
                out.extend(self.checked(Check::Final));
                out.push(inward(false));
                out
            }
            Check::Final => {
                if a.scale <= ZOOMED {
                    return self.again(check, format!("pinch: not magnified again: {a:?}"));
                }
                self.stage = Some(Stage::Done);
                vec![Action::Pass(format!("scale {:.2}", a.scale))]
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn look(scale: f64, width: f64, dpr: f64, search: &str) -> Look {
        Look {
            scale,
            width,
            dpr,
            search: search.into(),
            hover: true,
            ..Look::default()
        }
    }

    fn worked(smoke: &mut PinchSmoke, tabs: usize, framed: usize) -> Vec<Action> {
        smoke.next(Input::Worked {
            tabs,
            framed,
            fed: true,
        })
    }

    fn answer(smoke: &mut PinchSmoke, pane: usize, l: &Look) -> Vec<Action> {
        smoke.next(Input::Looked {
            pane,
            during: false,
            look: Some(l),
        })
    }

    fn settle_out(smoke: &mut PinchSmoke, tabs: usize, framed: usize) -> Vec<Action> {
        (0..SETTLE)
            .flat_map(|_| worked(smoke, tabs, framed))
            .collect()
    }

    #[test]
    fn the_script_runs_to_its_end() {
        let mut s = PinchSmoke::default();
        assert_eq!(worked(&mut s, 1, 1), [Action::Look(0)]);
        let base = look(1.0, 1000.0, 1.0, "?prevent");
        let out = answer(&mut s, 0, &base);
        assert!(matches!(out.as_slice(), [Action::Pinch(0, _)]));
        assert_eq!(settle_out(&mut s, 1, 1), [Action::Look(0)]);
        let kept = Look {
            ctrl_wheels: 12,
            ..base.clone()
        };
        let out = answer(&mut s, 0, &kept);
        assert_eq!(out.last(), Some(&Action::ZoomIn));
        settle_out(&mut s, 1, 1);
        let out = answer(&mut s, 0, &look(1.0, 900.0, 1.1, "?prevent"));
        assert!(out.ends_with(&[Action::ZoomReset, Action::LoadPlain]));
        settle_out(&mut s, 1, 1);
        // not loaded yet: asked again later
        assert!(answer(&mut s, 0, &look(1.0, 900.0, 1.1, "?prevent")).is_empty());
        settle_out(&mut s, 1, 1);
        assert_eq!(
            answer(&mut s, 0, &look(1.0, 1000.0, 1.0, "")),
            [Action::OpenSecond]
        );
        assert_eq!(worked(&mut s, 2, 1), [Action::Split]);
        assert!(worked(&mut s, 2, 2).is_empty());
        assert_eq!(settle_out(&mut s, 2, 2), [Action::Look(0), Action::Look(1)]);
        let half = look(1.0, 500.0, 1.0, "");
        answer(&mut s, 0, &half);
        let out = answer(&mut s, 1, &half);
        assert!(matches!(out.as_slice(), [Action::Pinch(0, d)] if d.contains(&Drip::Look)));
        s.next(Input::Looked {
            pane: 0,
            during: true,
            look: Some(&half),
        });
        settle_out(&mut s, 2, 2);
        answer(&mut s, 0, &look(3.0, 500.0, 1.0, ""));
        let out = answer(&mut s, 1, &half);
        assert!(
            matches!(out.as_slice(), [Action::Note(_), Action::Pinch(0, _)]),
            "{out:?}"
        );
        settle_out(&mut s, 2, 2);
        let out = answer(&mut s, 0, &look(1.0, 500.0, 1.0, ""));
        assert!(matches!(out.last(), Some(Action::Pinch(0, _))));
        settle_out(&mut s, 2, 2);
        let out = answer(&mut s, 0, &look(2.5, 500.0, 1.0, ""));
        assert_eq!(out, [Action::Pass("scale 2.50".into())]);
    }

    #[test]
    fn a_reflow_or_a_click_fails_the_pinch() {
        let mut s = PinchSmoke {
            stage: Some(Stage::Asked {
                check: Check::Pinched,
            }),
            base: Some(look(1.0, 1000.0, 1.0, "")),
            split: [
                Some(look(1.0, 500.0, 1.0, "")),
                Some(look(1.0, 500.0, 1.0, "")),
            ],
            during: Some(look(2.0, 500.0, 1.0, "")),
            ..PinchSmoke::default()
        };
        let clicked = Look {
            clicks: 1,
            ..look(3.0, 400.0, 1.0, "")
        };
        answer(&mut s, 0, &clicked);
        let out = answer(&mut s, 1, &look(1.0, 500.0, 1.0, ""));
        let [Action::Fail(why)] = out.as_slice() else {
            panic!("{out:?}")
        };
        assert!(why.contains("reflowed") && why.contains("click"), "{why}");
    }

    #[test]
    fn a_gesture_begins_and_ends() {
        let g = gesture(1.1, 4, true);
        assert_eq!(g.first(), Some(&Drip::Gesture(PinchInput::Begin)));
        assert_eq!(g.last(), Some(&Drip::Gesture(PinchInput::End)));
        assert_eq!(g.iter().filter(|d| **d == Drip::Look).count(), 1);
        assert_eq!(g.len(), 7);
    }

    #[test]
    fn looks_parse_from_the_page() {
        let text = r#"{"scale":2,"width":800,"dpr":1,"search":"","seen":{"click":1,"contextmenu":1,"touchstart":2,"ctrlWheels":3},"selection":"","hover":true,"active":false,"coarse":false,"anyCoarse":false,"hoverNone":false}"#;
        let l = Look::parse(text).unwrap();
        assert_eq!(
            (l.scale, l.clicks, l.touches, l.ctrl_wheels),
            (2.0, 2, 2, 3)
        );
        assert!(l.hover);
        let quoted = serde_json::to_string(text).unwrap();
        assert_eq!(Look::parse(&quoted), Some(l));
        assert_eq!(Look::parse("null"), None);
    }
}
