// Source: the owner's design repo (xuckless/the-wallpaper-project @ 6c840da),
// `src/wubble/slint/tabs.rs`, copied verbatim except the marked `wubble change`s.

//! Tab list physics, ported 1:1 from the loop and drag handlers in `browser/tabs.tsx` (without the
//! DOM). Every row sits on its own spring at slot·pitch, so closing, opening and reordering all
//! settle the same way. The active indicator follows on the follow spring and stretches along its
//! travel (volume kept on the cross axis). Dragging a row lifts it 1:1; the others make room.
//!
//! In Slint: push `rows()` into `[float]` models for each row's y and scale, and the indicator's
//! y / stretch / origin into the Sidebar's `indicator-*` properties (driver.rs).

use crate::motion::{
    K, OVERLAP_IN, Spring, Stretch, clamp, ease, overlap_in, spring, squash_of, step, tween,
};

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
    /// wubble change: at most this many rows run their entrance at once, and at most this many
    /// their exit; rows added past it appear in place, rows dropped past it go at once. Each
    /// entering or leaving row is an opacity layer in Slint's Skia renderer, a render target
    /// Skia keeps cached until its 256 MiB budget is full (Slint 1.19 sets no lower limit and
    /// runs no cleanup), so 40 rows entering together (session restore, a burst of opens) left
    /// 40 MiB of VRAM behind (TODO Decided "GPU memory: Skia scratch layers"). A whole new list
    /// (a Space switch) enters as one layer: [`TabRows::list_entrance`].
    pub const maxAnimating: usize = 4;
}

#[derive(Clone, Debug)]
pub struct RowAnim {
    pub id: u64,
    pub y: Spring,
    pub s: Spring,
    /// wubble: seconds since the row was inserted, for its Overlap entrance (`.wb-tab` runs
    /// `wb-in`, browser.css); `OVERLAP_IN` or more once it has arrived. The TS has CSS run it.
    pub enter: f64,
}

impl RowAnim {
    fn arrived(id: u64, slot: usize, pitch: f64) -> Self {
        RowAnim {
            id,
            y: spring(slot as f64 * pitch),
            s: spring(1.0),
            enter: OVERLAP_IN,
        }
    }

    /// How far its entrance is, 0..1 (1 once arrived).
    pub fn entrance(&self) -> f64 {
        overlap_in(self.enter)
    }
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

/// wubble: a row the model dropped, kept while it leaves (`.wb-tab-closing` runs `wb-tab-out`:
/// 190 ms in-out to clear, blurred and 0.94; tabs.tsx keeps the row until then). The TS delays
/// the close itself; here the host has already closed the tab, so the row leaves where it was
/// while the others close the gap (Overlap: old and new share the screen).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Leaving {
    pub id: u64,
    /// where it was, px
    pub y: f64,
    /// seconds since it left
    pub t: f64,
}

impl Leaving {
    /// How far it has gone, 0..1, on the in-out curve.
    pub fn progress(&self) -> f64 {
        tween(ease::inOut, self.t, TABS::exitMs / 1000.0)
    }
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
    /// rows on their way out (wubble)
    pub leaving: Vec<Leaving>,
    /// wubble: seconds since the whole list was replaced (a Space switch); `OVERLAP_IN` or more
    /// once it has arrived. The list enters as one, so its rows need no layer each.
    list_enter: f64,
    ind_y: Spring,
    stretch: Stretch,
    drag: Option<Drag>,
    /// wubble change: the row pitch, px ([`TABS::pitch`] by default; 44 + 4 on touch). Set by
    /// [`TabRows::set_pitch`]: the row springs then carry every row to its new slot.
    pub pitch: f64,
}

impl TabRows {
    pub fn new(ids: &[u64], active: Option<usize>) -> Self {
        let rows = ids
            .iter()
            .enumerate()
            .map(|(i, &id)| RowAnim::arrived(id, i, TABS::pitch))
            .collect();
        TabRows {
            rows,
            active,
            leaving: vec![],
            list_enter: OVERLAP_IN,
            ind_y: spring(active.unwrap_or(0) as f64 * TABS::pitch),
            stretch: Stretch::new(),
            drag: None,
            pitch: TABS::pitch,
        }
    }

    /// wubble change: a new row pitch (touch sizing). Nothing jumps: the springs carry each row
    /// and the indicator from where they are to their slots at the new pitch.
    pub fn set_pitch(&mut self, pitch: f64) {
        self.pitch = pitch;
    }

    /// Sync with the model: new rows start at their slot (they appear in place and fade in, at
    /// most [`TABS::maxAnimating`] at once). A Space switch replaces every row, so the whole
    /// list enters as one ([`TabRows::list_entrance`]; tabs.tsx remounts the list per Space:
    /// `key={sp.id}`).
    pub fn set_ids(&mut self, ids: &[u64], active: Option<usize>) {
        // Rows the model dropped leave where they were, unless nothing stayed (another Space: the
        // list is a new one, as tabs.tsx remounts it per Space).
        let kept = self.rows.iter().any(|r| ids.contains(&r.id));
        if !kept {
            self.list_enter = 0.0;
        }
        let entering = self
            .rows
            .iter()
            .filter(|r| r.enter < OVERLAP_IN && ids.contains(&r.id))
            .count();
        let mut room = if kept {
            TABS::maxAnimating.saturating_sub(entering)
        } else {
            0
        };
        let mut next = Vec::with_capacity(ids.len());
        for (i, &id) in ids.iter().enumerate() {
            match self.rows.iter().position(|r| r.id == id) {
                Some(j) => next.push(self.rows[j].clone()),
                None => {
                    let mut row = RowAnim::arrived(id, i, self.pitch);
                    if room > 0 {
                        room -= 1;
                        row.enter = 0.0;
                    }
                    next.push(row);
                }
            }
        }
        self.leaving.retain(|l| kept && !ids.contains(&l.id));
        let room = TABS::maxAnimating.saturating_sub(self.leaving.len());
        let dropped = self.rows.iter().filter(|r| !ids.contains(&r.id));
        self.leaving
            .extend(dropped.filter(|_| kept).take(room).map(|r| Leaving {
                id: r.id,
                y: r.y.x,
                t: 0.0,
            }));
        self.rows = next;
        self.active = active;
        // wubble change: the host's model can change under a held press (a tab closes itself,
        // a popup opens). Keep the drag on its row by id, or drop it when the row is gone, so
        // `up` never reorders by stale indices.
        self.drag = self.drag.and_then(|mut d| {
            d.from = self.rows.iter().position(|r| r.id == d.id)?;
            d.to = d.to.min(self.rows.len() - 1);
            Some(d)
        });
    }

    /// wubble: how far the whole list's entrance is after a Space switch, 0..1 (1 once arrived).
    pub fn list_entrance(&self) -> f64 {
        overlap_in(self.list_enter)
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
            y0: i as f64 * self.pitch,
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
        let raw = d.from as f64 * self.pitch + dy;
        let max = (n as f64 - 1.0) * self.pitch;
        d.y0 = if raw < 0.0 {
            -18.0 * (1.0 - 1.0 / (-raw / 18.0 + 1.0))
        } else if raw > max {
            max + 18.0 * (1.0 - 1.0 / ((raw - max) / 18.0 + 1.0))
        } else {
            raw
        };
        d.to = clamp((d.y0 / self.pitch).round(), 0.0, n as f64 - 1.0) as usize;
        began
    }

    /// Pointer up: a click picks the row, a drag may reorder (the owner applies
    /// `Moved(from, to)` to its model; the rows here are already in the new order).
    pub fn up(&mut self) -> TabUp {
        let Some(d) = self.drag.take() else {
            return TabUp::None;
        };
        // wubble change: picks the press's own row (kept by id across `set_ids`); the port takes
        // the pointer-up's row index, which names another tab once the list has changed.
        if !d.moved {
            return TabUp::Pick(d.from);
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
        let exit = TABS::exitMs / 1000.0;
        for l in &mut self.leaving {
            l.t += dt;
        }
        self.leaving.retain(|l| l.t < exit);
        moving |= !self.leaving.is_empty();
        if self.list_enter < OVERLAP_IN {
            self.list_enter += dt;
            moving = true;
        }
        let slots: Vec<Option<usize>> = self.rows.iter().map(|r| self.slot_of(r.id)).collect();
        for (r, slot) in self.rows.iter_mut().zip(slots) {
            if r.enter < OVERLAP_IN {
                r.enter += dt;
                moving = true;
            }
            let dragging = matches!(d, Some(d) if d.moved && d.id == r.id);
            if dragging {
                r.y.x = d.unwrap().y0;
                r.y.v = 0.0;
                moving = true;
            } else {
                moving = step(
                    &mut r.y,
                    slot.unwrap_or(0) as f64 * self.pitch,
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
            _ => act_id.and_then(|a| self.slot_of(a)).unwrap_or(0) as f64 * self.pitch,
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
            origin: self.stretch.origin(), // the trailing edge leads the stretch
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
            origin: self.stretch.origin(),
        }
    }
}

/// wubble change: the tab list's Up / Down keys (`tabs.tsx` row `onKeyDown`:
/// `clamp(i ± 1, 0, n - 1)`, no wrap), over wubble's rows, where folder headers sit between the
/// tabs: from row `from`, the nearest tab row (`is_tab`) one way, or `from` itself at the end.
pub fn step_focus(is_tab: &[bool], from: usize, down: bool) -> usize {
    let next = if down {
        is_tab.iter().enumerate().skip(from + 1).find(|(_, t)| **t)
    } else {
        is_tab
            .iter()
            .enumerate()
            .take(from.min(is_tab.len()))
            .rev()
            .find(|(_, t)| **t)
    };
    next.map_or(from, |(i, _)| i)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn step_focus_clamps_and_skips_folders() {
        let rows = [true, true, true];
        assert_eq!(step_focus(&rows, 0, true), 1);
        assert_eq!(step_focus(&rows, 2, true), 2, "no wrap at the end");
        assert_eq!(step_focus(&rows, 0, false), 0, "no wrap at the start");
        assert_eq!(step_focus(&rows, 2, false), 1);
        let folders = [false, true, false, true];
        assert_eq!(step_focus(&folders, 1, true), 3, "past a folder header");
        assert_eq!(step_focus(&folders, 1, false), 1, "a header is not a stop");
        assert_eq!(step_focus(&[true], 0, true), 0);
        assert_eq!(step_focus(&[true], 0, false), 0);
        assert_eq!(step_focus(&[], 0, true), 0, "empty list");
        assert_eq!(step_focus(&[], 0, false), 0);
    }

    #[test]
    fn inserted_rows_enter_and_first_rows_do_not() {
        let mut t = TabRows::new(&[1, 2], Some(0));
        let arrived = overlap_in(OVERLAP_IN);
        assert!(
            t.rows.iter().all(|r| r.entrance() == arrived),
            "at launch: in place"
        );
        t.set_ids(&[1, 3, 2], Some(0));
        assert_eq!(t.rows[1].entrance(), 0.0, "a new tab starts from nothing");
        assert_eq!(t.rows[0].entrance(), arrived);
        assert!(t.step(0.016));
        let mid = t.rows[1].entrance();
        assert!(mid > 0.0 && mid < 1.0);
        for _ in 0..20 {
            t.step(0.016);
        }
        assert_eq!(t.rows[1].entrance(), arrived);
        // a closed row leaves where it was, over 190 ms
        t.set_ids(&[1, 2], Some(0));
        assert_eq!(t.leaving.len(), 1);
        assert_eq!((t.leaving[0].id, t.leaving[0].y), (3, TABS::pitch));
        t.step(0.1);
        let p = t.leaving[0].progress();
        assert!(p > 0.0 && p < 1.0);
        for _ in 0..10 {
            t.step(0.016);
        }
        assert!(t.leaving.is_empty(), "gone after exitMs");
        // another Space: every row is new, so the whole list enters as one (and none leaves)
        assert_eq!(t.list_entrance(), arrived);
        t.set_ids(&[7, 8], None);
        assert!(t.leaving.is_empty());
        assert!(t.rows.iter().all(|r| r.entrance() == arrived));
        assert_eq!(t.list_entrance(), 0.0);
        assert!(t.step(0.016));
        settle(&mut t);
        assert_eq!(t.list_entrance(), arrived);
    }

    #[test]
    fn a_new_pitch_springs_rows_to_their_slots() {
        let mut t = TabRows::new(&[1, 2, 3], Some(2));
        t.set_pitch(48.0);
        // nothing jumps: the rows are still where they were
        assert_eq!(t.rows[2].y.x, 2.0 * TABS::pitch);
        let mut n = 0;
        while t.step(1.0 / 60.0) {
            n += 1;
            assert!(n < 10_000);
        }
        for (i, r) in t.rows.iter().enumerate() {
            assert_eq!(r.y.x, i as f64 * 48.0);
        }
        assert_eq!(t.indicator().y, 96.0);
    }

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
        assert_eq!(t.up(), TabUp::Moved(0, 2));
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
        assert_eq!(t.up(), TabUp::Moved(0, 2));
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
    fn model_change_under_a_drag_follows_the_row_by_id() {
        // A, B, C; A dragged to slot 2, then C closes before the release.
        let mut t = TabRows::new(&[1, 2, 3], Some(2));
        t.down(0, 0.0);
        t.r#move(2.0 * TABS::pitch);
        t.set_ids(&[1, 2], Some(0));
        assert_eq!(t.up(), TabUp::Moved(0, 1));
        assert_eq!(t.rows.iter().map(|r| r.id).collect::<Vec<_>>(), [2, 1]);
        assert_eq!(t.active, Some(1));

        // A row opens above the pressed one: the click still picks the pressed row.
        let mut t = TabRows::new(&[1, 2], Some(0));
        t.down(1, 0.0);
        t.set_ids(&[9, 1, 2], Some(1));
        assert_eq!(t.up(), TabUp::Pick(2));

        // The dragged row itself goes away: the drag ends with nothing to report.
        let mut t = TabRows::new(&[1, 2, 3], Some(0));
        t.down(2, 0.0);
        t.r#move(-2.0 * TABS::pitch);
        t.set_ids(&[1, 2], Some(0));
        assert!(!t.is_dragging(3));
        assert_eq!(t.up(), TabUp::None);
    }

    #[test]
    fn six_px_separates_a_pick_from_a_drag() {
        let mut t = TabRows::new(&[1, 2, 3], Some(0));
        t.down(1, 100.0);
        assert!(!t.r#move(100.0 + TABS::dragStart));
        assert_eq!(t.up(), TabUp::Pick(1));
        t.down(1, 100.0);
        assert!(t.r#move(100.0 + TABS::dragStart + 0.5));
        assert!(t.is_dragging(2));
        // back over its own slot: a drag that lands where it started is no move
        t.r#move(100.0);
        assert_eq!(t.up(), TabUp::None);
    }

    fn entering(t: &TabRows) -> Vec<u64> {
        t.rows
            .iter()
            .filter(|r| r.entrance() < 1.0)
            .map(|r| r.id)
            .collect()
    }

    #[test]
    fn a_burst_of_new_rows_animates_at_most_the_cap() {
        // 40 tabs open in one tick (session restore, the memory harness)
        let mut t = TabRows::new(&[1], Some(0));
        let ids: Vec<u64> = (1..=41).collect();
        t.set_ids(&ids, Some(0));
        assert_eq!(t.rows.iter().map(|r| r.id).collect::<Vec<_>>(), ids);
        assert_eq!(entering(&t), [2, 3, 4, 5], "the top new rows animate");
        assert!(t.rows[5..].iter().all(|r| r.entrance() == 1.0));
        // a whole new list (another Space) enters as one: no row on its own
        let other: Vec<u64> = (100..140).collect();
        t.set_ids(&other, None);
        assert!(entering(&t).is_empty());
        assert_eq!(t.list_entrance(), 0.0);
    }

    #[test]
    fn a_burst_of_closes_leaves_at_most_the_cap() {
        // "Close Other Tabs" on 41 rows: one stays, 40 go in one tick
        let ids: Vec<u64> = (1..=41).collect();
        let mut t = TabRows::new(&ids, Some(0));
        t.set_ids(&[1], Some(0));
        assert_eq!(
            t.leaving.iter().map(|l| l.id).collect::<Vec<_>>(),
            [2, 3, 4, 5],
            "the top dropped rows animate out, the rest go at once"
        );
        // rows already leaving use up the cap
        let mut t = TabRows::new(&[1, 2, 3, 4, 5, 6], Some(0));
        t.set_ids(&[1, 2, 3, 4, 5], Some(0));
        t.step(0.016);
        t.set_ids(&[1], Some(0));
        assert_eq!(
            t.leaving.iter().map(|l| l.id).collect::<Vec<_>>(),
            [6, 2, 3, 4]
        );
    }

    #[test]
    fn rows_already_entering_keep_running_and_use_up_the_cap() {
        let mut t = TabRows::new(&[1], Some(0));
        // one tab per tick: the first four animate, later ones appear in place
        let mut ids = vec![1];
        for id in 2..=8 {
            ids.push(id);
            t.set_ids(&ids, Some(0));
            t.step(0.016);
        }
        assert_eq!(entering(&t), [2, 3, 4, 5]);
        // once they have arrived, a new tab animates again
        settle(&mut t);
        ids.push(9);
        t.set_ids(&ids, Some(0));
        assert_eq!(entering(&t), [9]);
        assert_eq!(t.rows.last().unwrap().entrance(), 0.0);
    }
}
