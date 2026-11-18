//! Pure state of the spring-driven shell: compact mode, the resizable
//! sidebar seam, the Space indicator, and the spring bank that animates them.
//!
//! Seam behaviour is ported from the design pack `ux/pack/project/RoomToPlay.dc.html`
//! lines 91, 99-106, 118-132, 148-157 (owner-authored, same repo): held = the
//! `SEAM_HELD` spring toward the rubber-banded pointer, release = the `SEAM`
//! spring toward the clamped width.
//!
//! One state struct, one pure [`transition`], one pure [`targets`]. Every
//! animated value is a unit fraction or an index, so no pixel literal lives
//! here: the UI multiplies by its own sizes. No clocks, no effects.

use crate::motion::{
    self, BAND_SEAM, CONTENTS, FOLLOW, Params, SEAM, SEAM_HELD, Spring, rubber_band_clamp,
};

/// Every animated shell value. The discriminant is the index in [`Bank`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Id {
    /// Sidebar width, in logical pixels.
    SidebarWidth,
    /// Sidebar reveal, 0 (off screen) to 1 (in place).
    Slide,
    /// Fraction of the sidebar (plus gutter) the content area gives up, 0 to 1.
    ContentInset,
    /// Position of the active-Space ring, in dot pitches.
    SpaceIndicator,
}

/// Number of animated values.
pub const N: usize = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompactMode {
    Expanded,
    Hidden,
    Peeking,
}

/// The seam drag. `x` is the last pointer x in window coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Seam {
    Idle,
    Held {
        press_x: f32,
        start_width: f32,
        x: f32,
    },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shell {
    pub compact: CompactMode,
    pub seam: Seam,
    /// Committed sidebar width, within the limits.
    pub width: f32,
    pub space_index: usize,
}

/// Sidebar width limits, in logical pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Limits {
    pub min: f32,
    pub max: f32,
}

impl Limits {
    fn clamp(self, v: f32) -> f32 {
        v.max(self.min).min(self.max)
    }
}

impl Shell {
    pub fn new(width: f32, limits: Limits) -> Self {
        Self {
            compact: CompactMode::Expanded,
            seam: Seam::Idle,
            width: limits.clamp(width),
            space_index: 0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Event {
    ToggleCompact,
    /// The pointer entered the sidebar's hover zone.
    ZoneEntered,
    ZoneLeft,
    /// `width` is the width currently displayed, which differs from the
    /// committed one while a spring-back is still running.
    SeamPressed {
        x: f32,
        width: f32,
    },
    SeamDragged {
        x: f32,
    },
    SeamReleased,
    SpaceChanged(usize),
}

/// The whole shell lifecycle. Seam events only act while expanded.
pub fn transition(s: Shell, e: Event, l: Limits) -> Shell {
    use CompactMode::{Expanded, Hidden, Peeking};
    match e {
        Event::ToggleCompact => Shell {
            compact: if s.compact == Expanded {
                Hidden
            } else {
                Expanded
            },
            seam: Seam::Idle,
            ..s
        },
        Event::ZoneEntered => Shell {
            compact: if s.compact == Hidden {
                Peeking
            } else {
                s.compact
            },
            ..s
        },
        Event::ZoneLeft => Shell {
            compact: if s.compact == Peeking {
                Hidden
            } else {
                s.compact
            },
            ..s
        },
        Event::SeamPressed { x, width }
            if s.compact == Expanded && x.is_finite() && width.is_finite() =>
        {
            Shell {
                seam: Seam::Held {
                    press_x: x,
                    start_width: width,
                    x,
                },
                ..s
            }
        }
        Event::SeamDragged { x } if x.is_finite() => match s.seam {
            Seam::Held {
                press_x,
                start_width,
                ..
            } => Shell {
                seam: Seam::Held {
                    press_x,
                    start_width,
                    x,
                },
                ..s
            },
            Seam::Idle => s,
        },
        Event::SeamReleased => match s.seam {
            Seam::Held {
                press_x,
                start_width,
                x,
            } => Shell {
                seam: Seam::Idle,
                width: l.clamp(start_width + x - press_x),
                ..s
            },
            Seam::Idle => s,
        },
        Event::SpaceChanged(index) => Shell {
            space_index: index,
            ..s
        },
        Event::SeamPressed { .. } | Event::SeamDragged { .. } => s,
    }
}

/// Where one spring is headed and how.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Target {
    pub value: f32,
    pub params: Params,
}

/// The declared table of targets, one per [`Id`], derived from the state.
pub fn targets(s: &Shell, l: Limits) -> [Target; N] {
    let shown = s.compact != CompactMode::Hidden;
    let unit = |on: bool| if on { 1.0 } else { 0.0 };
    let width = match s.seam {
        Seam::Held {
            press_x,
            start_width,
            x,
        } => Target {
            value: rubber_band_clamp(start_width + x - press_x, l.min, l.max, BAND_SEAM),
            params: SEAM_HELD,
        },
        Seam::Idle => Target {
            value: s.width,
            params: SEAM,
        },
    };
    let mut out = [width; N];
    out[Id::Slide as usize] = Target {
        value: unit(shown),
        params: SEAM,
    };
    out[Id::ContentInset as usize] = Target {
        value: unit(s.compact == CompactMode::Expanded),
        params: CONTENTS,
    };
    out[Id::SpaceIndicator as usize] = Target {
        value: s.space_index as f32,
        params: FOLLOW,
    };
    out
}

/// Position of `id` in `ids`, or 0 when absent (the first Space is the default).
pub fn index_of<T: PartialEq>(ids: impl IntoIterator<Item = T>, id: T) -> usize {
    ids.into_iter().position(|x| x == id).unwrap_or(0)
}

/// One spring per [`Id`].
#[derive(Clone, Copy, Debug)]
pub struct Bank {
    springs: [Spring; N],
}

impl Bank {
    /// Springs resting at their first targets.
    pub fn new(initial: &[Target; N]) -> Self {
        Self {
            springs: initial.map(|t| Spring::at(t.value, t.params)),
        }
    }

    /// Swap targets and presets, keeping value and velocity.
    pub fn retarget(&mut self, t: &[Target; N]) {
        for (s, t) in self.springs.iter_mut().zip(t) {
            *s = Spring {
                params: t.params,
                ..s.retarget(t.value)
            };
        }
    }

    /// Advance by `dt` seconds. Returns true while another frame is needed.
    /// An idle bank is left untouched.
    pub fn step(&mut self, dt: f32) -> bool {
        if self.is_idle() {
            return false;
        }
        for s in &mut self.springs {
            *s = motion::step(*s, dt);
        }
        !self.is_idle()
    }

    pub fn values(&self) -> [f32; N] {
        self.springs.map(|s| s.value)
    }

    pub fn is_idle(&self) -> bool {
        self.springs
            .iter()
            .all(|s| s.is_at_rest() && s.value == s.target)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use CompactMode::{Expanded, Hidden, Peeking};

    const L: Limits = Limits {
        min: 200.0,
        max: 360.0,
    };

    fn shell(mode: CompactMode) -> Shell {
        Shell {
            compact: mode,
            ..Shell::new(260.0, L)
        }
    }

    #[test]
    fn compact_transition_table() {
        let table = [
            (Expanded, Event::ToggleCompact, Hidden),
            (Expanded, Event::ZoneEntered, Expanded),
            (Expanded, Event::ZoneLeft, Expanded),
            (Hidden, Event::ToggleCompact, Expanded),
            (Hidden, Event::ZoneEntered, Peeking),
            (Hidden, Event::ZoneLeft, Hidden),
            (Peeking, Event::ToggleCompact, Expanded),
            (Peeking, Event::ZoneEntered, Peeking),
            (Peeking, Event::ZoneLeft, Hidden),
        ];
        for (from, event, to) in table {
            assert_eq!(
                transition(shell(from), event, L).compact,
                to,
                "{from:?} {event:?}"
            );
        }
        for mode in [Hidden, Peeking] {
            for e in [
                Event::SeamPressed {
                    x: 10.0,
                    width: 260.0,
                },
                Event::SeamDragged { x: 20.0 },
                Event::SeamReleased,
            ] {
                assert_eq!(transition(shell(mode), e, L), shell(mode), "{mode:?} {e:?}");
            }
        }
    }

    #[test]
    fn peek_cannot_flicker() {
        let mut s = shell(Hidden);
        s = transition(s, Event::ZoneEntered, L);
        s = transition(s, Event::ZoneEntered, L);
        assert_eq!(s.compact, Peeking);
        s = transition(s, Event::ZoneLeft, L);
        assert_eq!(s.compact, Hidden);
        s = transition(s, Event::ZoneEntered, L);
        s = transition(s, Event::ToggleCompact, L);
        assert_eq!(s.compact, Expanded);
    }

    #[test]
    fn toggle_drops_a_drag() {
        let mut s = transition(
            shell(Expanded),
            Event::SeamPressed {
                x: 5.0,
                width: 260.0,
            },
            L,
        );
        assert_ne!(s.seam, Seam::Idle);
        s = transition(s, Event::ToggleCompact, L);
        assert_eq!(s.seam, Seam::Idle);
        assert_eq!(s.width, 260.0);
    }

    #[test]
    fn targets_by_mode() {
        let t = |m| targets(&shell(m), L);
        let at = |m, id: Id| t(m)[id as usize];
        assert_eq!(
            at(Expanded, Id::Slide),
            Target {
                value: 1.0,
                params: SEAM
            }
        );
        assert_eq!(
            at(Peeking, Id::Slide),
            Target {
                value: 1.0,
                params: SEAM
            }
        );
        assert_eq!(
            at(Hidden, Id::Slide),
            Target {
                value: 0.0,
                params: SEAM
            }
        );
        assert_eq!(
            at(Expanded, Id::ContentInset),
            Target {
                value: 1.0,
                params: CONTENTS
            }
        );
        assert_eq!(at(Peeking, Id::ContentInset).value, 0.0);
        assert_eq!(
            at(Hidden, Id::ContentInset),
            Target {
                value: 0.0,
                params: CONTENTS
            }
        );
        let mut s = shell(Expanded);
        s.space_index = 2;
        assert_eq!(
            targets(&s, L)[Id::SpaceIndicator as usize],
            Target {
                value: 2.0,
                params: FOLLOW
            }
        );
        assert_eq!(
            targets(&s, L)[Id::SidebarWidth as usize],
            Target {
                value: 260.0,
                params: SEAM
            }
        );
    }

    fn held_target(drag_to: f32) -> Target {
        let mut s = transition(
            shell(Expanded),
            Event::SeamPressed {
                x: 100.0,
                width: 260.0,
            },
            L,
        );
        s = transition(s, Event::SeamDragged { x: drag_to }, L);
        targets(&s, L)[Id::SidebarWidth as usize]
    }

    #[test]
    fn seam_held_is_banded() {
        // start width 260, press at 100: raw = 160 + x
        let over = held_target(100.0 + 5000.0);
        assert!(over.value > L.max && over.value < L.max + BAND_SEAM);
        let under = held_target(100.0 - 5000.0);
        assert!(under.value < L.min && under.value > L.min - BAND_SEAM);
        let inside = held_target(100.0 + 40.0);
        assert_eq!(inside.value, 300.0);
        assert_eq!(over.params, SEAM_HELD);
    }

    #[test]
    fn seam_release_commits_clamped() {
        let mut s = transition(
            shell(Expanded),
            Event::SeamPressed {
                x: 100.0,
                width: 260.0,
            },
            L,
        );
        s = transition(s, Event::SeamDragged { x: 5000.0 }, L);
        s = transition(s, Event::SeamReleased, L);
        assert_eq!(s.width, L.max);
        assert_eq!(s.seam, Seam::Idle);
        let t = targets(&s, L)[Id::SidebarWidth as usize];
        assert_eq!(
            t,
            Target {
                value: L.max,
                params: SEAM
            }
        );
        let mut bank = Bank::new(&targets(&shell(Expanded), L));
        bank.retarget(&targets(&s, L));
        for _ in 0..4000 {
            if !bank.step(0.016) {
                break;
            }
        }
        assert_eq!(bank.values()[Id::SidebarWidth as usize], L.max);
    }

    #[test]
    fn seam_press_starts_from_displayed_width() {
        // Mid spring-back from a banded 394 toward 360: the press must not
        // snap the target to the committed width.
        let s = transition(
            shell(Expanded),
            Event::SeamPressed {
                x: 100.0,
                width: 394.0,
            },
            L,
        );
        let t = targets(&s, L)[Id::SidebarWidth as usize];
        assert_eq!(t.value, rubber_band_clamp(394.0, L.min, L.max, BAND_SEAM));
        let s = transition(s, Event::SeamReleased, L);
        assert_eq!(s.width, L.max);
    }

    #[test]
    fn non_finite_pointer_is_ignored() {
        let s = shell(Expanded);
        assert_eq!(
            transition(
                s,
                Event::SeamPressed {
                    x: f32::NAN,
                    width: 260.0
                },
                L
            ),
            s
        );
    }

    #[test]
    fn bank_idle_detection() {
        let mut bank = Bank::new(&targets(&shell(Expanded), L));
        assert!(bank.is_idle());
        let before = bank.values();
        assert!(!bank.step(0.016));
        assert_eq!(bank.values(), before);

        let hidden = targets(&shell(Hidden), L);
        bank.retarget(&hidden);
        assert!(!bank.is_idle());
        let mut steps = 0;
        while bank.step(0.016) {
            steps += 1;
            assert!(steps < 4000, "bank never settled");
        }
        assert!(bank.is_idle());
        let want = hidden.map(|t| t.value);
        assert_eq!(bank.values(), want);
    }

    #[test]
    fn bank_params_swap_keeps_velocity() {
        let mut bank = Bank::new(&targets(&shell(Expanded), L));
        bank.retarget(&targets(&shell(Hidden), L));
        for _ in 0..5 {
            bank.step(0.016);
        }
        let slide = Id::Slide as usize;
        let (value, velocity) = (bank.springs[slide].value, bank.springs[slide].velocity);
        assert!(velocity != 0.0);
        let mut t = targets(&shell(Hidden), L);
        t[slide].params = SEAM_HELD;
        bank.retarget(&t);
        assert_eq!(bank.springs[slide].value, value);
        assert_eq!(bank.springs[slide].velocity, velocity);
        assert_eq!(bank.springs[slide].params, SEAM_HELD);
    }

    #[test]
    fn index_of_finds_or_defaults() {
        assert_eq!(index_of([4, 5, 6], 6), 2);
        assert_eq!(index_of([4, 5, 6], 9), 0);
        assert_eq!(index_of(Vec::<i32>::new(), 1), 0);
    }
}
