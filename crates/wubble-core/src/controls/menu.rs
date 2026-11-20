//! The touch kit's menu (and the settings `Select`, which is the same menu): its lifecycle, the
//! one gliding highlight, the keys and the confirm flash, as one state and one pure
//! [`transition`]. Slint draws [`MenuState`] (kit `controls/menu.slint`) and runs the tweens; this
//! decides what is open, lit and chosen.
//!
//! Source: the owner's design repo (xuckless/the-wallpaper-project @ 598f814),
//! `src/wubble/pages/touch.tsx:458-552` (`MENU`, `Menu`; @source touch-menu) and
//! `pages/settings.tsx:200-260` (`Select`; @source settings-select), translated:
//! - presence: mounted while open and for `MENU.out` (176 ms) after it closes (`usePresence`);
//!   reopening mid-exit turns back from where it is (CSS transitions);
//! - the highlight (`hl`) follows the pointer and the focused row; `jump` when it arrives while
//!   no row was lit (it appears in place instead of gliding); pointer leave clears it; closing
//!   resets it;
//! - keys: Down/Up move the focus with wrap, Escape closes and refocuses the trigger, Tab closes
//!   without, Enter/Space choose the focused row; there is no type-ahead, Home/End or mnemonic;
//! - choosing closes the menu, refocusing the trigger only when the choice was made by key
//!   (`viaKey`: the click's `detail === 0`), and the trigger confirms the row for
//!   `MENU.confirmMs` (1400 ms).
//!
//! wubble (derived, docs/fidelity/derived-review.md): separators (wubble's menus have them, the
//! design's none) are skipped by the keys and never lit; disabled rows take the focus but are
//! never lit or chosen.
//!
//! Who owns `open`: the menu's owner (the host for the page's context menu, the trigger for a
//! menu button). So the phase changes only on [`Event::Open`] and [`Event::Close`]; the keys,
//! the pointer outside and a choice return intents ([`Effect`]) and the owner closes it. Time
//! is `dt` in ms passed in ([`Event::Tick`]), never read.

use crate::kit_gen::touch::MENU;

/// Where the menu is in its life.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Phase {
    Closed,
    Open,
    /// Leaving: still drawn for `left_ms` more.
    Closing {
        left_ms: f64,
    },
}

/// What a row is, for the keys and the highlight.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowKind {
    Item,
    Disabled,
    Separator,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Up,
    Down,
    Escape,
    Tab,
    /// Enter or Space on the focused row.
    Activate,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// The rows changed (or are given again with every event).
    Rows(Vec<RowKind>),
    /// The owner opened it; by key (the focus goes to `start`) or by pointer.
    Open {
        via_key: bool,
        start: Option<usize>,
    },
    /// The owner closed it.
    Close,
    /// The pointer entered row `i`.
    Enter(usize),
    /// The pointer left the menu.
    Leave,
    Key(Key),
    /// A pointer click (or the accessibility default action) on row `i`.
    Choose(usize),
    /// A pointer press outside the menu and its trigger.
    Outside,
    /// `dt` ms passed.
    Tick(f64),
}

/// Intents for the owner.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effect {
    Chosen(usize),
    /// Close it; give the keyboard back to the trigger when `refocus`.
    Dismiss {
        refocus: bool,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct MenuState {
    pub phase: Phase,
    /// The lit row (the highlight is on it); `None`: the highlight is off.
    pub hl: Option<usize>,
    /// The highlight appears in place (it arrived while no row was lit) instead of gliding.
    pub jump: bool,
    /// The row with the keyboard focus.
    pub focus: Option<usize>,
    /// The trigger's confirm flash: the chosen row, and how long it still shows.
    pub done: Option<(usize, f64)>,
    pub rows: Vec<RowKind>,
}

impl Default for MenuState {
    fn default() -> Self {
        Self {
            phase: Phase::Closed,
            hl: None,
            jump: true,
            focus: None,
            done: None,
            rows: vec![],
        }
    }
}

impl MenuState {
    pub fn is_open(&self) -> bool {
        self.phase == Phase::Open
    }

    /// Something still changes with time (the exit, the confirm flash).
    pub fn moving(&self) -> bool {
        matches!(self.phase, Phase::Closing { .. }) || self.done.is_some()
    }

    fn kind(&self, i: usize) -> Option<RowKind> {
        self.rows.get(i).copied()
    }

    /// The focus (or pointer) arrives on row `i`: it is lit unless it is not an item; the
    /// highlight jumps when no row was lit.
    fn light(&mut self, i: usize) {
        let lit = (self.kind(i) == Some(RowKind::Item)).then_some(i);
        if lit.is_some() {
            self.jump = self.hl.is_none();
        }
        self.hl = lit;
    }

    /// The next row from the focus by `delta` (±1), wrapping and stepping over separators;
    /// with no focus, Down starts at the first row and Up at the last (`(-1 ± 1) % n`).
    fn step(&self, delta: isize) -> Option<usize> {
        let n = self.rows.len() as isize;
        let mut i = match self.focus {
            Some(f) => f as isize,
            None if delta > 0 => -1,
            None => 0,
        };
        for _ in 0..n {
            i = (i + delta).rem_euclid(n);
            if self.rows[i as usize] != RowKind::Separator {
                return Some(i as usize);
            }
        }
        None
    }
}

/// One event: the next state and the intents it raises.
pub fn transition(state: &MenuState, event: Event) -> (MenuState, Vec<Effect>) {
    let mut s = state.clone();
    let mut fx = vec![];
    let open = s.is_open();
    match event {
        Event::Rows(rows) => {
            s.rows = rows;
            let n = s.rows.len();
            s.focus = s.focus.filter(|&f| f < n);
            s.hl = s.hl.filter(|&h| s.kind(h) == Some(RowKind::Item));
        }
        Event::Open { via_key, start } => {
            if !open {
                s.phase = Phase::Open;
                s.hl = None;
                s.jump = true;
                s.focus = None;
                // by key the start row takes the focus (and lights)
                if via_key && let Some(i) = start.or(Some(0)).filter(|&i| i < s.rows.len()) {
                    s.focus = Some(i);
                    s.light(i);
                }
            }
        }
        Event::Close => {
            if open {
                s.phase = Phase::Closing { left_ms: MENU::out };
                s.hl = None;
                s.jump = true;
                s.focus = None;
            }
        }
        Event::Enter(i) if open => s.light(i),
        Event::Leave if open => s.hl = None,
        Event::Key(key) if open => match key {
            Key::Down | Key::Up => {
                if let Some(i) = s.step(if key == Key::Down { 1 } else { -1 }) {
                    s.focus = Some(i);
                    s.light(i);
                }
            }
            Key::Escape => fx.push(Effect::Dismiss { refocus: true }),
            Key::Tab => fx.push(Effect::Dismiss { refocus: false }),
            Key::Activate => {
                if let Some(i) = s.focus {
                    return choose(s, i, true);
                }
            }
        },
        Event::Choose(i) if open => return choose(s, i, false),
        Event::Outside if open => fx.push(Effect::Dismiss { refocus: false }),
        Event::Tick(dt) => {
            if let Phase::Closing { left_ms } = s.phase {
                let left = left_ms - dt;
                s.phase = if left <= 0.0 {
                    Phase::Closed
                } else {
                    Phase::Closing { left_ms: left }
                };
            }
            if let Some((row, left)) = s.done {
                s.done = (left - dt > 0.0).then_some((row, left - dt));
            }
        }
        Event::Enter(_) | Event::Leave | Event::Key(_) | Event::Choose(_) | Event::Outside => {}
    }
    (s, fx)
}

/// A click on row `i` of the open menu (`by_key`: Enter or Space): an item is chosen (the owner
/// closes the menu), and the trigger confirms it; anything else does nothing.
fn choose(mut s: MenuState, i: usize, by_key: bool) -> (MenuState, Vec<Effect>) {
    if s.kind(i) != Some(RowKind::Item) {
        return (s, vec![]);
    }
    s.done = Some((i, MENU::confirmMs));
    (
        s,
        vec![Effect::Chosen(i), Effect::Dismiss { refocus: by_key }],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use RowKind::*;

    fn menu(rows: &[RowKind]) -> MenuState {
        MenuState {
            rows: rows.to_vec(),
            ..MenuState::default()
        }
    }

    fn run(s: MenuState, events: impl IntoIterator<Item = Event>) -> (MenuState, Vec<Effect>) {
        events.into_iter().fold((s, vec![]), |(s, mut all), e| {
            let (s, fx) = transition(&s, e);
            all.extend(fx);
            (s, all)
        })
    }

    fn open(rows: &[RowKind], via_key: bool) -> MenuState {
        run(
            menu(rows),
            [Event::Open {
                via_key,
                start: None,
            }],
        )
        .0
    }

    #[test]
    fn a_key_open_lights_the_start_row_and_a_pointer_open_none() {
        let s = open(&[Item, Item, Item], true);
        assert_eq!((s.focus, s.hl, s.jump), (Some(0), Some(0), true));
        // Select: the selected row
        let (s, _) = run(
            menu(&[Item, Item, Item]),
            [Event::Open {
                via_key: true,
                start: Some(2),
            }],
        );
        assert_eq!((s.focus, s.hl), (Some(2), Some(2)));
        let s = open(&[Item, Item], false);
        assert_eq!((s.phase, s.focus, s.hl), (Phase::Open, None, None));
    }

    #[test]
    fn down_and_up_wrap_and_skip_separators() {
        let s = open(&[Item, Separator, Item, Item], false);
        let (s, _) = run(s, [Event::Key(Key::Down)]);
        assert_eq!(s.focus, Some(0), "Down from nothing: the first row");
        let (s, _) = run(s, [Event::Key(Key::Down)]);
        assert_eq!(s.focus, Some(2), "the separator is skipped");
        let (s, _) = run(s, [Event::Key(Key::Down), Event::Key(Key::Down)]);
        assert_eq!(s.focus, Some(0), "wraps");
        let (s, _) = run(s, [Event::Key(Key::Up)]);
        assert_eq!(s.focus, Some(3), "wraps back");
        let s = open(&[Item, Item, Separator], false);
        let (s, _) = run(s, [Event::Key(Key::Up)]);
        assert_eq!(
            s.focus,
            Some(1),
            "Up from nothing: the last row, past the separator"
        );
    }

    #[test]
    fn disabled_rows_take_the_focus_but_are_never_lit_or_chosen() {
        let s = open(&[Item, Disabled, Item], false);
        let (s, _) = run(s, [Event::Key(Key::Down), Event::Key(Key::Down)]);
        assert_eq!((s.focus, s.hl), (Some(1), None));
        let (s, fx) = run(s, [Event::Key(Key::Activate), Event::Choose(1)]);
        assert!(fx.is_empty());
        assert_eq!(s.done, None);
        let (s, _) = run(s, [Event::Enter(1)]);
        assert_eq!(s.hl, None, "the pointer does not light it either");
    }

    #[test]
    fn the_highlight_jumps_only_when_no_row_was_lit() {
        let s = open(&[Item, Item, Item], false);
        let (s, _) = run(s, [Event::Enter(1)]);
        assert_eq!((s.hl, s.jump), (Some(1), true), "arrives in place");
        let (s, _) = run(s, [Event::Enter(2)]);
        assert_eq!((s.hl, s.jump), (Some(2), false), "glides");
        let (s, _) = run(s, [Event::Leave]);
        assert_eq!((s.hl, s.jump), (None, false), "leave keeps the last mode");
        let (s, _) = run(s, [Event::Enter(0)]);
        assert_eq!((s.hl, s.jump), (Some(0), true));
        let (s, _) = run(s, [Event::Key(Key::Down)]);
        assert_eq!((s.hl, s.jump, s.focus), (Some(0), false, Some(0)));
    }

    #[test]
    fn escape_tab_and_outside_ask_the_owner_to_close() {
        let s = open(&[Item], false);
        assert_eq!(
            run(s.clone(), [Event::Key(Key::Escape)]).1,
            [Effect::Dismiss { refocus: true }]
        );
        assert_eq!(
            run(s.clone(), [Event::Key(Key::Tab)]).1,
            [Effect::Dismiss { refocus: false }]
        );
        let (s, fx) = run(s, [Event::Outside]);
        assert_eq!(fx, [Effect::Dismiss { refocus: false }]);
        assert_eq!(s.phase, Phase::Open, "the owner closes it");
        // closed: no intents
        assert!(
            run(menu(&[Item]), [Event::Outside, Event::Choose(0)])
                .1
                .is_empty()
        );
    }

    #[test]
    fn choosing_refocuses_the_trigger_only_when_chosen_by_key() {
        let (_, fx) = run(open(&[Item, Item], true), [Event::Choose(1)]);
        assert_eq!(
            fx,
            [Effect::Chosen(1), Effect::Dismiss { refocus: false }],
            "a pointer click, even in a menu opened by key"
        );
        let (s, fx) = run(
            open(&[Item, Item], false),
            [
                Event::Key(Key::Down),
                Event::Key(Key::Down),
                Event::Key(Key::Activate),
            ],
        );
        assert_eq!(fx, [Effect::Chosen(1), Effect::Dismiss { refocus: true }]);
        assert_eq!(s.done, Some((1, MENU::confirmMs)));
    }

    #[test]
    fn closing_lasts_the_exit_and_reopening_turns_back() {
        let s = open(&[Item, Item], false);
        let (s, _) = run(s, [Event::Enter(1), Event::Close]);
        assert_eq!(
            (s.phase, s.hl, s.jump),
            (Phase::Closing { left_ms: 176.0 }, None, true)
        );
        assert!(s.moving());
        // 21 ticks of 8 ms: 168 ms, still leaving; the 22nd ends it
        let (s, _) = run(s, (0..21).map(|_| Event::Tick(8.0)));
        assert!(matches!(s.phase, Phase::Closing { .. }));
        let (closed, _) = run(s.clone(), [Event::Tick(8.0)]);
        assert_eq!(closed.phase, Phase::Closed);
        assert!(!closed.moving());
        let (back, _) = run(
            s,
            [Event::Open {
                via_key: false,
                start: None,
            }],
        );
        assert_eq!(back.phase, Phase::Open, "reopens from the exit");
        // keys and the pointer do nothing while leaving
        let (s, fx) = run(closed, [Event::Enter(0), Event::Key(Key::Escape)]);
        assert_eq!((s.hl, fx.len()), (None, 0));
    }

    #[test]
    fn the_confirm_flash_clears_after_its_time() {
        let (s, _) = run(open(&[Item], false), [Event::Choose(0), Event::Close]);
        let (s, _) = run(s, (0..174).map(|_| Event::Tick(8.0)));
        assert_eq!(s.done.map(|d| d.0), Some(0), "1392 ms: still confirming");
        let (s, _) = run(s, [Event::Tick(8.0)]);
        assert_eq!(s.done, None);
        assert!(!s.moving());
    }

    #[test]
    fn new_rows_drop_a_focus_or_highlight_past_them() {
        let (s, _) = run(open(&[Item, Item, Item], true), [Event::Key(Key::Up)]);
        assert_eq!(s.focus, Some(2));
        let (s, _) = run(s, [Event::Rows(vec![Item, Item])]);
        assert_eq!((s.focus, s.hl), (None, None));
    }
}
