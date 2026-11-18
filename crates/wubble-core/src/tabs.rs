// Source: the owner's design repo (xuckless/the-wallpaper-project @ 6c840da),
// `src/wubble/slint/tabs.rs`, copied verbatim except the marked `wubble change`s.

//! Tab list physics, ported 1:1 from the loop and drag handlers in `browser/tabs.tsx` (without the
//! DOM). Every row sits on its own spring at slot·pitch, so closing, opening and reordering all
//! settle the same way. The active indicator follows on the follow spring and stretches along its
//! travel (volume kept on the cross axis). Dragging a row lifts it 1:1; the others make room.
//!
//! In Slint: push `rows()` into `[float]` models for each row's y and scale, and the indicator's
//! y / stretch / origin into the Sidebar's `indicator-*` properties (driver.rs).

use crate::motion::{K, Spring, Stretch, clamp, spring, squash_of, step};

// @source slint-tabs
#[allow(non_snake_case, non_upper_case_globals)]
pub mod TABS {
    /// 40: row 36 + gap 4
    pub const pitch: f64 = 36.0 + 4.0;
    /// a dragged row grows a little and casts a shadow
    pub const lift: f64 = 1.025;
    /// px before a press becomes a drag
    pub const dragStart: f64 = 6.0;
    /// closing row leaves 20% faster than rows enter (240)
    pub const exitMs: f64 = 190.0;
}

#[derive(Clone, Debug)]
pub struct RowAnim {
    pub id: u64,
    pub y: Spring,
    pub s: Spring,
}

#[derive(Clone, Copy, Debug)]
struct Drag {
    id: u64,
    from: usize,
    to: usize,
    y0: f64,
    start: f64,
    moved: bool,
}

/// What a pointer-up on a row means.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TabUp {
    None,
    Pick(usize),
    Moved(usize, usize),
}

/// The indicator as the Sidebar draws it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Indicator {
    pub y: f64,
    /// length along the travel (≥ 1 while moving)
    pub stretch: f64,
    /// cross axis, 1/√stretch
    pub squash: f64,
    /// transform origin along the row: 0.3 moving down, 0.7 moving up (the TS '50% 30%' / '50% 70%')
    pub origin: f64,
}

#[derive(Clone, Debug)]
pub struct TabRows {
    /// visible rows in order (closing rows are removed by the owner after `exitMs`)
    pub rows: Vec<RowAnim>,
    /// index into `rows` of the active tab
    /// wubble change: `None` when the active tab is not a row (a pinned tab, an empty Space);
    /// the indicator then rests at the top and the Sidebar hides it.
    pub active: Option<usize>,
    ind_y: Spring,
    stretch: Stretch,
    drag: Option<Drag>,
}

impl TabRows {
    pub fn new(ids: &[u64], active: Option<usize>) -> Self {
        let rows = ids
            .iter()
            .enumerate()
            .map(|(i, &id)| RowAnim {
                id,
                y: spring(i as f64 * TABS::pitch),
                s: spring(1.0),
            })
            .collect();
        TabRows {
            rows,
            active,
            ind_y: spring(active.unwrap_or(0) as f64 * TABS::pitch),
            stretch: Stretch::new(),
            drag: None,
        }
    }

    /// Sync with the model: new rows start at their slot (they appear in place and fade in).
    pub fn set_ids(&mut self, ids: &[u64], active: Option<usize>) {
        let mut next = Vec::with_capacity(ids.len());
        for (i, &id) in ids.iter().enumerate() {
            match self.rows.iter().position(|r| r.id == id) {
                Some(j) => next.push(self.rows[j].clone()),
                None => next.push(RowAnim {
                    id,
                    y: spring(i as f64 * TABS::pitch),
                    s: spring(1.0),
                }),
            }
        }
        self.rows = next;
        self.active = active;
    }

    // @source slint-tabs-slot
    /// Rows between the drag's origin and its current slot shift one pitch to make room.
    // wubble change: allowed rather than rewritten as a let-chain, to keep the port's text.
    #[allow(clippy::collapsible_if)]
    fn slot_of(&self, id: u64) -> Option<usize> {
        let mut i = self.rows.iter().position(|r| r.id == id)? as i64;
        if let Some(d) = self.drag {
            if d.moved && id != d.id {
                let (from, to) = (d.from as i64, d.to as i64);
                if from < to && i > from && i <= to {
                    i -= 1;
                } else if from > to && i >= to && i < from {
                    i += 1;
                }
            }
        }
        Some(i as usize)
    }

    // @source slint-tabs-drag
    /// Pointer down on row `i` (y in px). Returns nothing; a press only becomes a drag after 6 px.
    pub fn down(&mut self, i: usize, y: f64) {
        let id = self.rows[i].id;
        self.drag = Some(Drag {
            id,
            from: i,
            to: i,
            y0: i as f64 * TABS::pitch,
            start: y,
            moved: false,
        });
    }

    /// Returns true when this move turned the press into a drag (capture the pointer then).
    pub fn r#move(&mut self, y: f64) -> bool {
        let n = self.rows.len();
        let Some(d) = self.drag.as_mut() else {
            return false;
        };
        let dy = y - d.start;
        let mut began = false;
        if !d.moved && dy.abs() > TABS::dragStart {
            d.moved = true;
            began = true;
        }
        if !d.moved {
            return false;
        }
        // 1:1 inside the list, rubber-banded past its ends (k 18)
        let raw = d.from as f64 * TABS::pitch + dy;
        let max = (n as f64 - 1.0) * TABS::pitch;
        d.y0 = if raw < 0.0 {
            -18.0 * (1.0 - 1.0 / (-raw / 18.0 + 1.0))
        } else if raw > max {
            max + 18.0 * (1.0 - 1.0 / ((raw - max) / 18.0 + 1.0))
        } else {
            raw
        };
        d.to = clamp((d.y0 / TABS::pitch).round(), 0.0, n as f64 - 1.0) as usize;
        began
    }

    /// Pointer up on row `i`: a click picks the row, a drag may reorder (the owner applies
    /// `Moved(from, to)` to its model; the rows here are already in the new order).
    pub fn up(&mut self, i: usize) -> TabUp {
        let Some(d) = self.drag.take() else {
            return TabUp::None;
        };
        if !d.moved {
            return TabUp::Pick(i);
        }
        if d.to != d.from {
            let row = self.rows.remove(d.from);
            self.rows.insert(d.to, row);
            if let Some(a) = self.active.as_mut() {
                if *a == d.from {
                    *a = d.to;
                } else if d.from < *a && *a <= d.to {
                    *a -= 1;
                } else if d.to <= *a && *a < d.from {
                    *a += 1;
                }
            }
            return TabUp::Moved(d.from, d.to);
        }
        TabUp::None
    }

    pub fn cancel(&mut self) {
        self.drag = None;
    }

    pub fn is_dragging(&self, id: u64) -> bool {
        matches!(self.drag, Some(d) if d.moved && d.id == id)
    }

    // @source slint-tabs-step
    /// The Loop frame. Returns true while anything moves.
    pub fn step(&mut self, dt: f64) -> bool {
        let mut moving = false;
        let d = self.drag;
        let slots: Vec<Option<usize>> = self.rows.iter().map(|r| self.slot_of(r.id)).collect();
        for (r, slot) in self.rows.iter_mut().zip(slots) {
            let dragging = matches!(d, Some(d) if d.moved && d.id == r.id);
            if dragging {
                r.y.x = d.unwrap().y0;
                r.y.v = 0.0;
                moving = true;
            } else {
                moving = step(
                    &mut r.y,
                    slot.unwrap_or(0) as f64 * TABS::pitch,
                    K::follow[0],
                    26.0,
                    dt,
                ) || moving;
            }
            moving = step(
                &mut r.s,
                if dragging { TABS::lift } else { 1.0 },
                500.0,
                30.0,
                dt,
            ) || moving;
        }
        // indicator: follow + stretch; while dragging the active row it rides with it
        let act_id = self.active.and_then(|a| self.rows.get(a)).map(|r| r.id);
        let target = match (d, act_id) {
            (Some(d), Some(a)) if d.moved && d.id == a => d.y0,
            _ => act_id.and_then(|a| self.slot_of(a)).unwrap_or(0) as f64 * TABS::pitch,
        };
        moving = step(&mut self.ind_y, target, K::follow[0], K::follow[1], dt) || moving;
        moving = self.stretch.update(self.ind_y.x, dt) || moving;
        moving
    }

    pub fn indicator(&self) -> Indicator {
        let sx = self.stretch.x();
        Indicator {
            y: self.ind_y.x,
            stretch: sx,
            squash: squash_of(sx),
            origin: if self.stretch.dir > 0.0 { 0.3 } else { 0.7 }, // the trailing edge leads the stretch
        }
    }
}

// @source slint-spaces-switcher
/// The Space switcher's capsule (`SpaceSwitcher` in tabs.tsx): follows the active icon on the
/// follow spring and stretches between them along the switcher's axis.
#[derive(Clone, Debug)]
pub struct SpaceSwitcher {
    pos: Spring,
    stretch: Stretch,
    pub active: usize,
}

impl SpaceSwitcher {
    /// 36 tile + 2 gap
    pub const PITCH: f64 = 38.0;

    pub fn new(active: usize) -> Self {
        SpaceSwitcher {
            pos: spring(active as f64),
            stretch: Stretch::new(),
            active,
        }
    }

    pub fn step(&mut self, dt: f64) -> bool {
        let m = step(
            &mut self.pos,
            self.active as f64,
            K::follow[0],
            K::follow[1],
            dt,
        );
        self.stretch.update(self.pos.x * Self::PITCH, dt) || m
    }

    /// Offset along the axis (px), and the indicator stretch along it / squash across it.
    /// `origin` is the TS transform-origin along the axis: 0.3 moving forward, 0.7 back.
    pub fn indicator(&self) -> Indicator {
        let s = self.stretch.x();
        Indicator {
            y: self.pos.x * Self::PITCH,
            stretch: s,
            squash: squash_of(s),
            origin: if self.stretch.dir > 0.0 { 0.3 } else { 0.7 },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drag_reorders_and_settles() {
        let mut t = TabRows::new(&[10, 11, 12, 13], Some(0));
        t.down(0, 100.0);
        assert!(!t.r#move(103.0));
        assert!(t.r#move(110.0));
        t.r#move(100.0 + 2.0 * TABS::pitch + 3.0);
        for _ in 0..10 {
            t.step(1.0 / 60.0);
        }
        // rows 1 and 2 made room
        assert!(t.rows[1].y.x < TABS::pitch);
        assert_eq!(t.up(0), TabUp::Moved(0, 2));
        assert_eq!(t.active, Some(2));
        let mut n = 0;
        while t.step(1.0 / 60.0) {
            n += 1;
            assert!(n < 10_000);
        }
        for (i, r) in t.rows.iter().enumerate() {
            assert_eq!(r.y.x, i as f64 * TABS::pitch);
        }
        assert_eq!(t.indicator().y, 2.0 * TABS::pitch);
    }

    fn settle(t: &mut TabRows) {
        let mut n = 0;
        while t.step(1.0 / 60.0) {
            n += 1;
            assert!(n < 10_000);
        }
    }

    #[test]
    fn no_active_row() {
        let mut t = TabRows::new(&[1, 2, 3], None);
        settle(&mut t);
        assert_eq!(t.indicator().y, 0.0);
        // a reorder with no active row keeps it none
        t.down(0, 0.0);
        t.r#move(2.0 * TABS::pitch);
        assert_eq!(t.up(0), TabUp::Moved(0, 2));
        assert_eq!(t.active, None);
        t.set_ids(&[], None);
        settle(&mut t);
        assert!(t.rows.is_empty());
    }

    #[test]
    fn set_ids_keeps_springs_and_adds_rows_at_their_slot() {
        let mut t = TabRows::new(&[1, 2, 3], Some(0));
        settle(&mut t);
        // 3 moves to the top: it keeps its spring (still at slot 2) and springs up
        t.set_ids(&[3, 1, 9], Some(1));
        assert_eq!(t.rows.iter().map(|r| r.id).collect::<Vec<_>>(), [3, 1, 9]);
        assert_eq!(t.rows[0].y.x, 2.0 * TABS::pitch);
        assert_eq!(t.rows[1].y.x, 0.0);
        // a new row starts at its own slot
        assert_eq!(t.rows[2].y.x, 2.0 * TABS::pitch);
        assert!(t.step(1.0 / 60.0));
        settle(&mut t);
        for (i, r) in t.rows.iter().enumerate() {
            assert_eq!(r.y.x, i as f64 * TABS::pitch);
        }
        assert_eq!(t.indicator().y, TABS::pitch);
    }

    #[test]
    fn six_px_separates_a_pick_from_a_drag() {
        let mut t = TabRows::new(&[1, 2, 3], Some(0));
        t.down(1, 100.0);
        assert!(!t.r#move(100.0 + TABS::dragStart));
        assert_eq!(t.up(1), TabUp::Pick(1));
        t.down(1, 100.0);
        assert!(t.r#move(100.0 + TABS::dragStart + 0.5));
        assert!(t.is_dragging(2));
        // back over its own slot: a drag that lands where it started is no move
        t.r#move(100.0);
        assert_eq!(t.up(1), TabUp::None);
    }
}
