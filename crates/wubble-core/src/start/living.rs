//! The living bubble on the start page: the soft body, its minis and their pops, driven by the
//! pointer and a fixed-step tick. A translation of `mountLiving` in the design's
//! `core/living.ts` (the loop at :88-116, the pointer handlers at :53-73, the code poke at :76-85)
//! into one state and one pure `transition(state, event) -> (state, effects)`; the scene (the
//! liquid, the light, the ripples, the bubble and pop passes) stays the renderer's, which gets
//! what it must do as `Effect`s and what it must draw from `Living::frame`.
//!
//! Not ported (the web has them, the start page does not use them): the `auto` demo mode
//! (gallery and wallpaper hosts without input) and the voice bands (`SoftBody::voice` stays
//! `None`). The scene's own random draws (the liquid's seeding, `stir`) are the scene's: this
//! machine's `Random` carries only the body's and the minis' draws, in the web's order.

use super::minis::{Circle, Minis, Throw};
use super::rand::{Random, Rng};
use super::softbody::{PopState, Release, SoftBody, age_waves, push_wave};
use crate::kit_gen::{living as K, scene as S, softbody as KB, start as KS};

/// `LivingOptions` in living.ts (the ones the start page sets).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Options {
    /// bubble radius as a fraction of the shorter side
    pub size: f64,
    /// where the bubble sits, 0..1
    pub at: (f64, f64),
    /// ambient minis rising from the bottom
    pub ambient: bool,
    pub wobble: f64,
}

impl Options {
    /// The start page's (`pages/start.tsx`): size .1, at (.5, .4), ambient on.
    pub fn start_page() -> Self {
        Options {
            size: KS::size,
            at: (KS::at_x, KS::at_y),
            ambient: true,
            wobble: K::wobble,
        }
    }

    /// The new-tab page's: the start page's, with the bubble higher. Owner override of the web
    /// design (2026-11-19): the search capsule sits at 60% of the page, so the bubble goes up
    /// with it (the web has it at .4, the capsule near 70%).
    pub fn new_tab() -> Self {
        Options {
            at: (KS::at_x, NEW_TAB_AT_Y),
            ..Self::start_page()
        }
    }
}

/// Where the new-tab page's bubble rests, as a fraction of the page's height (owner, 2026-11-19).
pub const NEW_TAB_AT_Y: f64 = 0.31;

/// Pointer events in page-local logical px; `t_ms` is the event's timestamp (for the fling's
/// velocity), any monotonic ms clock the host has.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Event {
    /// The page's size changed (the web's `ResizeObserver`).
    Resize {
        w: f64,
        h: f64,
    },
    Down {
        x: f64,
        y: f64,
        t_ms: f64,
    },
    Move {
        x: f64,
        y: f64,
        t_ms: f64,
    },
    /// `pointerup` and `pointercancel`.
    Up,
    Leave,
    /// A poke from code (keyboard, preview): on the rim, in the middle, or `None` to let the
    /// random draw choose as the web does.
    Poke {
        rim: Option<bool>,
    },
    /// The page is shown (`setActive`): while false, `Tick` does nothing.
    Active(bool),
    /// Reduced motion (`reducedMotion()`): no wobble, no ambient minis, the scene at 0.3 speed.
    Reduced(bool),
    /// One step of `dt` seconds, capped at the web loop's `DT_CAP`; a non-finite or negative
    /// dt is ignored.
    Tick {
        dt: f64,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cursor {
    Pointer,
    Grabbing,
}

/// What the scene and the host do; the drawing state itself is in `Living::frame`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Effect {
    /// `scene.ripple(x, y, reach, strength)`: a ring through the backdrop and a push in the
    /// liquid (`fluid.ripple(x / w, 1 - y / h, 1100 * strength, 0.01)`).
    Ripple {
        x: f64,
        y: f64,
        reach: f64,
        strength: f64,
    },
    /// `scene.move(x, y)`: the pointer pushes the liquid and the light follows it.
    Light {
        x: f64,
        y: f64,
    },
    /// `scene.leave()`.
    LightLeave,
    /// `scene.pop`'s liquid push (`fluid.ripple(x / w, 1 - y / h, 500, max(.002, r / w * .6))`);
    /// the pop itself is drawn from `Living::frame().pops`.
    Pop(PopState),
    /// Capture the pointer (the press is on the body).
    Capture,
    Cursor(Cursor),
    /// `onPoke`: the bubble was poked.
    Poked,
    /// `onPop`: a mini was popped by the pointer.
    Popped,
}

/// The living bubble's whole state. `R` is the random source (the web's `Math.random()`).
#[derive(Clone, Debug)]
pub struct Living<R: Random = Rng> {
    pub body: SoftBody,
    pub minis: Minis,
    /// pops being drawn, aged by the scene's frame time
    pub pops: Vec<PopState>,
    pub opts: Options,
    /// the page's size in logical px; (0, 0) until the first `Resize` (the scene draws nothing)
    pub size: (f64, f64),
    /// `scene.time`: seconds of scene time (slowed when reduced)
    pub time: f64,
    spawn_t: f64,
    pub active: bool,
    pub still: bool,
    rng: R,
}

impl<R: Random> Living<R> {
    /// `mountLiving`: the soft body draws its seed first.
    pub fn new(opts: Options, mut rng: R) -> Self {
        let seed = rng.random() * KB::seed;
        Living {
            body: SoftBody::new(None, Some(opts.wobble), seed),
            minis: Minis::new(),
            pops: Vec::new(),
            opts,
            size: (0.0, 0.0),
            time: 0.0,
            spawn_t: K::spawn_first,
            active: true,
            still: false,
            rng,
        }
    }

    /// Pops the minis made since the last call: kept for drawing, sent to the scene's liquid.
    fn take_pops(&mut self, fx: &mut Vec<Effect>) {
        for p in self.minis.pops.drain(..) {
            fx.push(Effect::Pop(p));
            self.pops.push(p);
        }
    }

    /// `poke()` / the poke half of `onUp`: ripple, wave, three minis.
    fn poked(&mut self, x: f64, y: f64, fx: &mut Vec<Effect>) {
        fx.push(Effect::Ripple {
            x,
            y,
            reach: K::poke_ripple,
            strength: S::ripple_strength,
        });
        push_wave(
            &mut self.body.state.waves,
            x,
            y,
            self.body.R * K::poke_wave,
            S::wave_life,
        );
        self.minis
            .spawn(x, y, K::poke_minis, Throw::Burst, 1.0, &mut self.rng);
        fx.push(Effect::Poked);
    }

    fn tick(&mut self, dt: f64, fx: &mut Vec<Effect>) {
        // physics.ts `Loop` caps every frame's dt before it reaches the living loop
        if !self.active || !dt.is_finite() || dt < 0.0 {
            return;
        }
        let dt = dt.min(crate::kit_gen::physics::DT_CAP);
        let still = self.still;
        self.body.wobble = if still { 0.0 } else { self.opts.wobble };
        self.body.step(dt);
        if self.opts.ambient && !still {
            self.spawn_t -= dt;
            if self.spawn_t <= 0.0 {
                self.spawn_t = K::spawn_every + self.rng.random() * K::spawn_every_range;
                let x = self.size.0 * (K::spawn_x + self.rng.random() * K::spawn_x_range);
                self.minis.spawn(
                    x,
                    self.size.1 + K::spawn_below,
                    1,
                    Throw::Gentle,
                    1.0,
                    &mut self.rng,
                );
            }
        }
        let c = self.body.center();
        let avoid = [Circle {
            x: c.0,
            y: c.1,
            r: self.body.R,
        }];
        // the minis see the scene time before this frame's `scene.frame` adds to it
        self.minis.step(dt, self.time, &avoid, &mut self.rng);
        self.take_pops(fx);
        // `scene.frame(still ? dt * .3 : dt)`; a scene with no size yet does nothing
        let frame_dt = if still { dt * K::still_time } else { dt };
        if self.size.0 > 0.0 {
            self.time += frame_dt;
            age_waves(&mut self.body.state.waves, frame_dt);
            // the pops are aged as they are drawn (`age += lastDt`, then dropped at `life`)
            self.pops.retain_mut(|p| {
                p.age += frame_dt;
                p.age < p.life
            });
        }
    }
}

/// One event. Pure: the state in, the next state and the effects out.
pub fn transition<R: Random>(mut s: Living<R>, e: Event) -> (Living<R>, Vec<Effect>) {
    let mut fx = Vec::new();
    match e {
        Event::Resize { w, h } => {
            s.size = (w, h);
            s.body.R = K::r_min.max(w.min(h) * s.opts.size);
            s.body.home = (w * s.opts.at.0, h * s.opts.at.1);
        }
        Event::Down { x, y, t_ms } => {
            if s.minis.hit(x, y, &mut s.rng) {
                s.take_pops(&mut fx);
                fx.push(Effect::Popped);
            } else if s.body.hit(x, y) {
                s.body.down(x, y, t_ms);
                fx.push(Effect::Capture);
                fx.push(Effect::Cursor(Cursor::Grabbing));
            } else {
                fx.push(Effect::Ripple {
                    x,
                    y,
                    reach: S::ripple_reach,
                    strength: S::ripple_strength,
                });
                s.minis
                    .spawn(x, y, K::desk_minis, Throw::Burst, K::desk_size, &mut s.rng);
            }
        }
        Event::Move { x, y, t_ms } => {
            s.body.r#move(x, y, t_ms);
            fx.push(Effect::Light { x, y });
        }
        Event::Up => {
            let c = s.body.center();
            let r = s.body.up();
            fx.push(Effect::Cursor(Cursor::Pointer));
            match (r, s.body.ptr) {
                (Some(Release::Poke), Some((px, py))) => s.poked(px, py, &mut fx),
                (Some(Release::Fling), _) => {
                    fx.push(Effect::Ripple {
                        x: c.0,
                        y: c.1,
                        reach: s.body.R * K::fling_ripple,
                        strength: K::fling_ripple_strength,
                    });
                    push_wave(
                        &mut s.body.state.waves,
                        c.0,
                        c.1,
                        s.body.R * K::fling_wave,
                        K::fling_wave_life,
                    );
                }
                _ => {}
            }
        }
        Event::Leave => {
            s.body.leave();
            fx.push(Effect::LightLeave);
        }
        Event::Poke { rim } => {
            // the web draws `rim` only when it is not given, then the angle, then (in the
            // middle) x and y
            let rim = rim.unwrap_or_else(|| s.rng.random() > K::code_rim);
            let c = s.body.center();
            let a = s.rng.random() * std::f64::consts::PI * 2.0;
            let (x, y) = if rim {
                (c.0 + a.cos() * s.body.R, c.1 + a.sin() * s.body.R)
            } else {
                let x = c.0 + (s.rng.random() - 0.5) * s.body.R * K::code_mid;
                let y = c.1 + (s.rng.random() - 0.5) * s.body.R * K::code_mid;
                (x, y)
            };
            s.body.poke(x, y, 1.0);
            s.poked(x, y, &mut fx);
        }
        Event::Active(on) => s.active = on,
        Event::Reduced(on) => s.still = on,
        Event::Tick { dt } => s.tick(dt, &mut fx),
    }
    (s, fx)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::start::rand::Replay;

    const DT: f64 = 1.0 / 120.0;

    fn living(draws: Vec<f64>) -> Living<Replay> {
        let mut v = vec![0.5];
        v.extend(draws);
        let (l, _) = transition(
            Living::new(Options::start_page(), Replay::new(v)),
            Event::Resize {
                w: 1000.0,
                h: 800.0,
            },
        );
        l
    }

    #[test]
    fn resize_sets_the_radius_and_home() {
        let l = living(vec![]);
        assert_eq!(l.body.R, 800.0 * KS::size);
        assert_eq!(l.body.home, (1000.0 * KS::at_x, 800.0 * KS::at_y));
        assert_eq!(l.body.state.seed, 0.5 * KB::seed);
        // never smaller than the floor
        let (l, _) = transition(l, Event::Resize { w: 200.0, h: 100.0 });
        assert_eq!(l.body.R, K::r_min);
    }

    #[test]
    fn a_huge_dt_steps_like_the_cap_and_a_bad_one_is_ignored() {
        let cap = crate::kit_gen::physics::DT_CAP;
        let (a, _) = transition(living(vec![]), Event::Tick { dt: 0.25 });
        let (b, _) = transition(living(vec![]), Event::Tick { dt: cap });
        assert_eq!((a.body.t, a.time), (b.body.t, b.time));
        assert_eq!(a.body.center(), b.body.center());
        for dt in [f64::NAN, f64::INFINITY, -1.0] {
            let l = living(vec![]);
            let before = (l.body.t, l.time, l.body.center());
            let (l, _) = transition(l, Event::Tick { dt });
            assert_eq!((l.body.t, l.time, l.body.center()), before);
        }
    }

    #[test]
    fn an_inactive_page_ignores_ticks() {
        let l = living(vec![]);
        let before = (l.body.t, l.time);
        let (l, _) = transition(l, Event::Active(false));
        let (l, fx) = transition(l, Event::Tick { dt: DT });
        assert_eq!((l.body.t, l.time), before);
        assert!(fx.is_empty());
        let (l, _) = transition(l, Event::Active(true));
        let (l, _) = transition(l, Event::Tick { dt: DT });
        assert_eq!(l.body.t, DT);
    }

    #[test]
    fn ambient_spawn_draws_the_interval_then_x() {
        // one tick past the first spawn time; then six draws for the mini
        let mut l = living(vec![0.0, 1.0, 0.1, 0.2, 0.3, 0.4, 0.5, 0.6]);
        l.spawn_t = DT / 2.0;
        let (l, _) = transition(l, Event::Tick { dt: DT });
        assert_eq!(l.rng.taken(), 9);
        assert_eq!(l.spawn_t, K::spawn_every);
        let m = &l.minis.list[0];
        // x from the second draw; spawned below the page, then stepped once
        assert!((m.x - 1000.0 * (K::spawn_x + K::spawn_x_range)).abs() < 1.0);
        assert!(m.y > 800.0);
    }

    #[test]
    fn scene_time_runs_slow_when_reduced_and_minis_see_the_time_before() {
        let l = living(vec![]);
        let (l, _) = transition(l, Event::Reduced(true));
        let (l, _) = transition(l, Event::Tick { dt: DT });
        assert_eq!(l.time, DT * K::still_time);
        assert_eq!(l.body.wobble, 0.0);
        // no ambient spawn while still: no draws beyond the seed
        assert_eq!(l.rng.taken(), 1);
    }

    #[test]
    fn a_press_on_the_bare_desk_ripples_and_throws_four_small_minis() {
        let l = living(vec![0.5; 6 * K::desk_minis]);
        let (l, fx) = transition(
            l,
            Event::Down {
                x: 10.0,
                y: 10.0,
                t_ms: 0.0,
            },
        );
        assert_eq!(
            fx,
            [Effect::Ripple {
                x: 10.0,
                y: 10.0,
                reach: S::ripple_reach,
                strength: S::ripple_strength
            }]
        );
        assert_eq!(l.minis.list.len(), K::desk_minis);
        assert_eq!(
            l.minis.list[0].r,
            (crate::kit_gen::minis::r_min + 0.5 * crate::kit_gen::minis::r_range) * K::desk_size
        );
    }

    fn tap_body(l: Living<Replay>) -> (Living<Replay>, Vec<Effect>) {
        let (cx, cy) = l.body.center();
        let (l, fx) = transition(
            l,
            Event::Down {
                x: cx + 5.0,
                y: cy,
                t_ms: 0.0,
            },
        );
        assert_eq!(fx, [Effect::Capture, Effect::Cursor(Cursor::Grabbing)]);
        transition(l, Event::Up)
    }

    #[test]
    fn a_tap_on_the_body_pokes_it() {
        let l = living(vec![0.5; 6 * K::poke_minis]);
        let (cx, cy) = l.body.center();
        let (l, fx) = tap_body(l);
        assert_eq!(
            fx,
            [
                Effect::Cursor(Cursor::Pointer),
                Effect::Ripple {
                    x: cx + 5.0,
                    y: cy,
                    reach: K::poke_ripple,
                    strength: S::ripple_strength
                },
                Effect::Poked
            ]
        );
        let w = l.body.state.waves[0];
        assert_eq!((w.reach, w.life), (l.body.R * K::poke_wave, S::wave_life));
        assert_eq!(l.minis.list.len(), K::poke_minis);
    }

    #[test]
    fn a_drag_flings_with_a_ripple_and_a_wave_at_the_centre() {
        let l = living(vec![]);
        let (cx, cy) = l.body.center();
        let (l, _) = transition(
            l,
            Event::Down {
                x: cx,
                y: cy,
                t_ms: 0.0,
            },
        );
        let (l, _) = transition(
            l,
            Event::Move {
                x: cx + 40.0,
                y: cy,
                t_ms: 16.0,
            },
        );
        let c = l.body.center();
        let (l, fx) = transition(l, Event::Up);
        assert_eq!(
            fx,
            [
                Effect::Cursor(Cursor::Pointer),
                Effect::Ripple {
                    x: c.0,
                    y: c.1,
                    reach: l.body.R * K::fling_ripple,
                    strength: K::fling_ripple_strength
                }
            ]
        );
        let w = l.body.state.waves[0];
        assert_eq!(
            (w.reach, w.life),
            (l.body.R * K::fling_wave, K::fling_wave_life)
        );
        assert!(l.minis.list.is_empty());
    }

    #[test]
    fn the_shell_reduced_motion_flag_does_not_snap_the_body() {
        // `Event::Reduced` is the only reduced-motion input: the web keeps the body's physics
        fn fling_travel() -> f64 {
            let l = living(vec![]);
            let (cx, cy) = l.body.center();
            let mut l = transition(
                l,
                Event::Down {
                    x: cx,
                    y: cy,
                    t_ms: 0.0,
                },
            )
            .0;
            for i in 1..=4 {
                let x = cx + 10.0 * i as f64;
                l = transition(
                    l,
                    Event::Move {
                        x,
                        y: cy,
                        t_ms: 4.0 * i as f64,
                    },
                )
                .0;
                l = transition(l, Event::Tick { dt: DT }).0;
            }
            l = transition(l, Event::Up).0;
            let mut most: f64 = 0.0;
            for _ in 0..60 {
                l = transition(l, Event::Tick { dt: DT }).0;
                most = most.max((l.body.center().0 - cx).abs());
            }
            most
        }
        let free = fling_travel();
        crate::motion::set_reduced(true);
        let flagged = fling_travel();
        crate::motion::set_reduced(false);
        assert!(free > 10.0);
        assert_eq!(flagged, free);
    }

    #[test]
    fn a_press_on_a_mini_pops_it_and_the_pop_ages_out() {
        let l = living(vec![0.5; 6 * K::desk_minis]);
        let (l, _) = transition(
            l,
            Event::Down {
                x: 10.0,
                y: 10.0,
                t_ms: 0.0,
            },
        );
        let newest = l.minis.list.last().unwrap().clone();
        let mut l = l;
        l.rng = Replay::new(vec![0.75]);
        let (l, fx) = transition(
            l,
            Event::Down {
                x: newest.x,
                y: newest.y,
                t_ms: 0.0,
            },
        );
        let pop = l.pops[0];
        assert_eq!(fx, [Effect::Pop(pop), Effect::Popped]);
        assert_eq!((pop.life, pop.seed), (S::pop_life, 0.75 * S::pop_seed));
        assert_eq!(l.minis.list.len(), K::desk_minis - 1);
        // aged by the frame time, gone at its life
        let mut l = l;
        l.opts.ambient = false;
        let mut n = 0;
        while !l.pops.is_empty() {
            l = transition(l, Event::Tick { dt: DT }).0;
            n += 1;
        }
        assert_eq!(n, (S::pop_life / DT).ceil() as usize);
    }

    #[test]
    fn waves_cap_and_age_out() {
        let mut l = living(vec![0.5; 6 * K::poke_minis * 6]);
        for _ in 0..6 {
            l.minis.list.clear();
            l = tap_body(l).0;
        }
        assert_eq!(l.body.state.waves.len(), S::wave_max);
        l.opts.ambient = false;
        for _ in 0..((S::wave_life / DT) as usize + 2) {
            l = transition(l, Event::Tick { dt: DT }).0;
        }
        assert!(l.body.state.waves.is_empty());
    }

    #[test]
    fn a_code_poke_draws_rim_angle_then_the_middle() {
        // rim draw .3 (not > .4: the middle), angle, x, y; then 3 minis
        let l = living([vec![0.3, 0.0, 0.5, 0.5], vec![0.5; 6 * K::poke_minis]].concat());
        let c = l.body.center();
        let (l, fx) = transition(l, Event::Poke { rim: None });
        assert_eq!(l.rng.taken(), 5 + 6 * K::poke_minis);
        assert_eq!(
            fx[0],
            Effect::Ripple {
                x: c.0,
                y: c.1,
                reach: K::poke_ripple,
                strength: S::ripple_strength
            }
        );
        // a given rim skips the rim draw: angle 0 is to the right
        let l = living([vec![0.0], vec![0.5; 6 * K::poke_minis]].concat());
        let c = l.body.center();
        let (_, fx) = transition(l, Event::Poke { rim: Some(true) });
        assert_eq!(
            fx[0],
            Effect::Ripple {
                x: c.0 + 800.0 * KS::size,
                y: c.1,
                reach: K::poke_ripple,
                strength: S::ripple_strength
            }
        );
    }

    #[test]
    fn leaving_forgets_the_pointer_unless_held() {
        let l = living(vec![]);
        let (l, fx) = transition(
            l,
            Event::Move {
                x: 1.0,
                y: 1.0,
                t_ms: 0.0,
            },
        );
        assert_eq!(fx, [Effect::Light { x: 1.0, y: 1.0 }]);
        let (l, fx) = transition(l, Event::Leave);
        assert_eq!(fx, [Effect::LightLeave]);
        assert_eq!(l.body.ptr, None);
        let (cx, cy) = l.body.center();
        let (l, _) = transition(
            l,
            Event::Down {
                x: cx,
                y: cy,
                t_ms: 0.0,
            },
        );
        let (l, _) = transition(l, Event::Leave);
        assert_eq!(l.body.ptr, Some((cx, cy)));
    }
}
