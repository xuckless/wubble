// Source: the owner's design repo (xuckless/the-wallpaper-project @ 6c840da),
// `src/wubble/slint/scroller.rs`, copied verbatim (tests added below the port's), except the
// marked `wubble change`: the glide of `browser/scroller.tsx` @ 21838b8 (`to(y)`), which the
// port does not have yet.

//! Scroll physics, ported 1:1 from `browser/scroller.tsx` (the Preact component's loop and event
//! handlers, without the DOM). Wheel and keys move a target that the content follows on a
//! near-critical spring, touch drags follow 1:1 and fling, and both ends rubber-band (k 34 → the
//! content stretches past the edge and springs back once input stops). An overlay thumb shows
//! while moving and can be dragged.
//!
//! Slint's `Flickable` has its own (non-spring) physics, so a Wubble scroll view is a clipping
//! `Rectangle` whose content `y` is `-render().shown`, fed by a `TouchArea`'s `scroll-event` and
//! pointer events (see `WubbleScroller` in components.slint and `driver.rs`).
//! Times are in milliseconds (the TS uses `performance.now()`), dt in seconds.

use crate::motion::{Spring, Velocity, band, clamp, spring, step};

// @source slint-scroll
#[allow(non_snake_case, non_upper_case_globals)]
pub mod SCROLL {
    /// wheel/keys: ζ ≈ 1, no overshoot inside the page
    pub const follow: [f64; 2] = [520.0, 46.0];
    /// back from overscroll: ζ ≈ 0.93, home in ~350 ms
    pub const release: [f64; 2] = [420.0, 38.0];
    /// wubble change (scroller.tsx @ 21838b8): `to()`, a programmatic jump (a settings section),
    /// ζ = 1, so long trips read as a move, not a cut
    pub const glide: [f64; 2] = [170.0, 26.0];
    /// rubber band past the ends
    pub const bandK: f64 = 34.0;
    /// wheel quiet for this long → clamp the target
    pub const idleMs: f64 = 90.0;
    /// past an end, outward wheel input counts only this long; after that it's trackpad momentum
    /// (macOS keeps sending it for seconds) and is ignored, so the page comes home at once
    pub const outBudgetMs: f64 = 160.0;
    /// a wheel gap this long starts a new gesture (and a new budget)
    pub const newGestureMs: f64 = 260.0;
    /// touch fling: v *= e^(−3.2 t)
    pub const flingDecay: f64 = 3.2;
    pub const thumbFade: f64 = 900.0;
}

/// `WheelEvent.deltaMode`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeltaMode {
    Pixel,
    Line,
    Page,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScrollKey {
    ArrowDown,
    ArrowUp,
    PageDown,
    PageUp,
    Space,
    Home,
    End,
}

/// What the view shows this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScrollRender {
    /// content offset (translate by −shown); rubber-banded past the ends
    pub shown: f64,
    /// thumb top and height in px, `None` when the content fits
    pub thumb: Option<(f64, f64)>,
    pub thumb_visible: bool,
}

#[derive(Clone, Debug)]
pub struct Scroller {
    pub pos: Spring,
    pub target: f64,
    pub max: f64,
    /// viewport height
    pub h: f64,
    /// content height
    pub ch: f64,
    last_wheel: f64,
    out_at: f64,
    out_dir: f64,
    pub fling: f64,
    thumb_drag: Option<(f64, f64)>, // (y0, t0)
    shown_at: f64,
    vel: Velocity,
    touch_y: f64,
    pub held: bool,
    /// wubble change: gliding to a `to()` target, until it arrives or input takes over
    pub glide: bool,
}

impl Default for Scroller {
    fn default() -> Self {
        Self::new()
    }
}

impl Scroller {
    pub fn new() -> Self {
        Scroller {
            pos: spring(0.0),
            target: 0.0,
            max: 0.0,
            h: 0.0,
            ch: 0.0,
            last_wheel: 0.0,
            out_at: 0.0,
            out_dir: 0.0,
            fling: 0.0,
            thumb_drag: None,
            shown_at: 0.0,
            vel: Velocity::new(),
            touch_y: 0.0,
            held: false,
            glide: false,
        }
    }

    /// The ResizeObserver callback: viewport and content heights.
    pub fn measure(&mut self, h: f64, ch: f64) {
        self.h = h;
        self.ch = ch;
        self.max = 0f64.max(ch - h);
    }

    /// A new page starts at the top (`resetKey`).
    pub fn reset(&mut self) {
        self.pos.x = 0.0;
        self.pos.v = 0.0;
        self.target = 0.0;
        self.fling = 0.0;
        self.glide = false;
    }

    /// wubble change (scroller.tsx @ 21838b8 `to`): glide to `y`, kept inside the page.
    pub fn to(&mut self, y: f64, now: f64) {
        self.target = clamp(y, 0.0, self.max);
        self.glide = true;
        self.fling = 0.0;
        self.shown_at = now;
    }

    // @source slint-scroll-input
    pub fn wheel(&mut self, delta_y: f64, mode: DeltaMode, now: f64) {
        let d = match mode {
            DeltaMode::Line => delta_y * 32.0,
            DeltaMode::Page => delta_y * self.h,
            DeltaMode::Pixel => delta_y,
        };
        // a wheel gap or a reversal starts a new gesture
        if now - self.last_wheel > SCROLL::newGestureMs
            || (self.out_at != 0.0 && js_sign(d) != self.out_dir)
        {
            self.out_at = 0.0;
        }
        // past an end, input itself meets resistance
        let out = if self.target < 0.0 {
            -self.target
        } else if self.target > self.max {
            self.target - self.max
        } else {
            0.0
        };
        let outward = out > 0.0 && js_sign(d) == js_sign(self.target - self.max / 2.0);
        if outward && self.out_at == 0.0 {
            self.out_at = now;
            self.out_dir = js_sign(d);
        }
        // the same gesture still pushing toward an end it has already hit, past the budget: that's
        // momentum. Drop it, and don't let it count as input, so the release spring runs now.
        if self.out_at != 0.0
            && js_sign(d) == self.out_dir
            && now - self.out_at > SCROLL::outBudgetMs
        {
            self.shown_at = now;
            self.last_wheel = now - SCROLL::idleMs - 1.0;
            return;
        }
        self.glide = false;
        self.target += d
            * (if outward {
                1.0 / (1.0 + out / 40.0)
            } else {
                1.0
            });
        self.target = clamp(self.target, -140.0, self.max + 140.0);
        self.fling = 0.0;
        self.last_wheel = now;
        self.shown_at = now;
    }

    pub fn key(&mut self, key: ScrollKey, shift: bool, now: f64) {
        let page = self.h * 0.88;
        let m = match key {
            ScrollKey::ArrowDown => 64.0,
            ScrollKey::ArrowUp => -64.0,
            ScrollKey::PageDown => page,
            ScrollKey::PageUp => -page,
            ScrollKey::Space => {
                if shift {
                    -page
                } else {
                    page
                }
            }
            ScrollKey::Home => -1e9,
            ScrollKey::End => 1e9,
        };
        self.glide = false;
        self.target = clamp(self.target + m, 0.0, self.max);
        self.shown_at = now;
        self.last_wheel = now;
    }

    /// Touch and pen drag the page directly (mouse keeps text selection in the web version).
    pub fn touch_down(&mut self, y: f64, now: f64) {
        self.held = true;
        self.glide = false;
        self.touch_y = y;
        self.vel.reset();
        self.vel.push(0.0, y, now);
        self.fling = 0.0;
        self.target = self.pos.x;
        self.pos.v = 0.0;
    }

    pub fn touch_move(&mut self, y: f64, now: f64) {
        if !self.held {
            return;
        }
        let dy = self.touch_y - y;
        self.touch_y = y;
        let out = self.pos.x < 0.0 || self.pos.x > self.max;
        self.pos.x += if out { dy * 0.5 } else { dy }; // 1:1 inside, resisted past the ends
        self.target = self.pos.x;
        self.vel.push(0.0, y, now);
        self.shown_at = now;
    }

    pub fn touch_up(&mut self) {
        if !self.held {
            return;
        }
        self.held = false;
        self.fling = -self.vel.get().1;
        self.last_wheel = 0.0;
    }

    fn thumb_h(&self) -> f64 {
        36f64.max(self.h * (self.h / self.ch))
    }

    /// The thumb drags 1:1 along the track.
    pub fn thumb_down(&mut self, y: f64) {
        self.thumb_drag = Some((y, self.pos.x));
    }

    pub fn thumb_move(&mut self, y: f64) {
        let Some((y0, t0)) = self.thumb_drag else {
            return;
        };
        let k = self.max / 1f64.max(self.h - self.thumb_h() - 8.0);
        self.pos.x = clamp(t0 + (y - y0) * k, 0.0, self.max);
        self.target = self.pos.x;
        self.pos.v = 0.0;
    }

    pub fn thumb_up(&mut self, now: f64) {
        self.thumb_drag = None;
        self.shown_at = now;
    }

    // @source slint-scroll-step
    /// The Loop frame. Returns true while anything moves or the thumb is still fading.
    pub fn step(&mut self, dt: f64, now: f64) -> bool {
        if !self.held && self.thumb_drag.is_none() {
            if self.fling.abs() > 5.0 {
                self.target += self.fling * dt;
                self.fling *= (-SCROLL::flingDecay * dt).exp();
            } else {
                self.fling = 0.0;
            }
            // past an end and no fresh input: the target comes home
            // (a touch fling may carry past an end first; it's stopped 60 px out, below)
            if now - self.last_wheel > SCROLL::idleMs && self.fling == 0.0 {
                self.target = clamp(self.target, 0.0, self.max);
            }
            let out = self.pos.x < 0.0 || self.pos.x > self.max;
            let [k, c] = if out && now - self.last_wheel > SCROLL::idleMs {
                SCROLL::release
            } else if self.glide {
                SCROLL::glide
            } else {
                SCROLL::follow
            };
            step(&mut self.pos, self.target, k, c, dt);
            if self.glide && self.pos.x == self.target {
                self.glide = false;
            }
            if self.fling != 0.0 && (self.target < -60.0 || self.target > self.max + 60.0) {
                self.fling = 0.0;
            }
        }
        self.held
            || self.thumb_drag.is_some()
            || self.pos.v != 0.0
            || self.pos.x != self.target
            || self.fling.abs() > 5.0
            || now - self.shown_at < SCROLL::thumbFade + 200.0
    }

    // @source slint-scroll-render
    /// The TS `render()`: content offset and the thumb, which squashes at the ends like the content.
    pub fn render(&self, now: f64) -> ScrollRender {
        let y = self.pos.x;
        let shown = if y < 0.0 {
            band(y, SCROLL::bandK * 3.0)
        } else if y > self.max {
            self.max + band(y - self.max, SCROLL::bandK * 3.0)
        } else {
            y
        };
        let mut thumb = None;
        let mut vis = false;
        if self.max > 0.0 {
            let t_h = self.thumb_h();
            let over = if y < 0.0 {
                -band(y, 40.0)
            } else if y > self.max {
                band(y - self.max, 40.0)
            } else {
                0.0
            };
            let t_y = (clamp(y, 0.0, self.max) / self.max) * (self.h - t_h - 8.0) + 4.0;
            let sq = 16f64.max(t_h - over.abs());
            let top = if y < 0.0 {
                4.0
            } else if y > self.max {
                t_y + (t_h - sq)
            } else {
                t_y
            };
            thumb = Some((top, sq));
            vis = self.thumb_drag.is_some() || self.held || now - self.shown_at < SCROLL::thumbFade;
        }
        ScrollRender {
            shown,
            thumb,
            thumb_visible: vis,
        }
    }
}

fn js_sign(v: f64) -> f64 {
    crate::motion::js_sign(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wheel_past_the_end_bands_and_returns() {
        let mut s = Scroller::new();
        s.measure(600.0, 2000.0);
        let mut now = 0.0;
        for _ in 0..10 {
            s.wheel(-80.0, DeltaMode::Pixel, now);
            now += 16.0;
            s.step(0.016, now);
        }
        assert!(s.render(now).shown < 0.0);
        while s.step(0.016, now) {
            now += 16.0;
        }
        assert_eq!(s.pos.x, 0.0);
    }

    #[test]
    fn fling_carries_and_stops() {
        let mut s = Scroller::new();
        s.measure(600.0, 4000.0);
        s.touch_down(500.0, 0.0);
        for i in 1..=6 {
            s.touch_move(500.0 - i as f64 * 30.0, i as f64 * 16.0);
        }
        s.touch_up();
        assert!(s.fling > 1000.0);
        let mut now = 100.0;
        while s.step(0.016, now) {
            now += 16.0;
        }
        assert!(s.pos.x > 180.0 && s.pos.x <= s.max);
    }

    /// Past an end, outward wheel input counts for 160 ms of one gesture; after that it is
    /// momentum and is dropped.
    #[test]
    fn outward_budget_drops_momentum() {
        let mut s = Scroller::new();
        s.measure(600.0, 2000.0);
        // already past the top
        s.wheel(-50.0, DeltaMode::Pixel, 1000.0);
        let past = s.target;
        assert!(past < 0.0);
        // the same gesture pushes outward at 1100 ms: the budget starts, the push is resisted
        s.wheel(-50.0, DeltaMode::Pixel, 1100.0);
        assert!(s.target < past && s.target > past - 50.0);
        s.wheel(-50.0, DeltaMode::Pixel, 1100.0 + 160.0);
        let at = s.target;
        assert!(at < past);
        // more than 160 ms after the first outward push: momentum, dropped
        s.wheel(-50.0, DeltaMode::Pixel, 1100.0 + 161.0);
        assert_eq!(s.target, at);
        // a wheel gap over 260 ms starts a new gesture with a fresh budget
        s.wheel(-50.0, DeltaMode::Pixel, 1100.0 + 161.0 + 261.0);
        assert!(s.target < at);
    }

    /// The target is clamped home only once the wheel has been quiet for 90 ms.
    #[test]
    fn idle_clamps_the_target_after_90_ms() {
        let mut s = Scroller::new();
        s.measure(600.0, 2000.0);
        s.wheel(-60.0, DeltaMode::Pixel, 0.0);
        assert!(s.target < 0.0);
        s.step(0.016, 90.0);
        assert!(s.target < 0.0);
        s.step(0.016, 91.0);
        assert_eq!(s.target, 0.0);
    }

    /// `to()` glides on its own spring (no overshoot, ζ = 1), inside the page, and stops gliding
    /// when it arrives; a wheel takes over at once.
    #[test]
    fn to_glides_inside_the_page() {
        let mut s = Scroller::new();
        s.measure(600.0, 2000.0);
        s.to(5000.0, 0.0);
        assert_eq!(s.target, s.max);
        assert!(s.glide);
        let mut now = 0.0;
        let mut top = 0f64;
        for _ in 0..600 {
            now += 16.0;
            s.step(0.016, now);
            top = top.max(s.pos.x);
        }
        assert!(top <= s.max + 1e-9, "no overshoot: {top}");
        assert_eq!(s.pos.x, s.max);
        assert!(!s.glide, "arrived");
        s.to(300.0, now);
        s.wheel(10.0, DeltaMode::Pixel, now);
        assert!(!s.glide);
        assert_eq!(s.target, 310.0);
    }
}
