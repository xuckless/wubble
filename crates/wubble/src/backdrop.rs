//! The window's glass: the design's wgpu scene under the shell, or Slint-only glass when it
//! cannot run. Chosen once at startup ([`choose`]) into one explicit [`Glass`], logged.
//!
//! With the scene, the shell driver's rendering notifier calls [`Glass::frame`] after its own
//! step (the wiring of the design repo's `render.rs::install`, the-wallpaper-project
//! `src/wubble/slint`, owner's code, moved into that one notifier): the scene renders under the
//! UI and its image is the window's bottom layer. The scene is fed from data only: the glass
//! list the panels report (`wubble_ui::glass`), the active Space's tint, the address pill's
//! beam (`address-beam`, the driver's), the pointer (from the host's winit filter) and, for the quality governor, each
//! frame's load ([`frame_load`]).

use std::time::{Duration, Instant};

use slint::ComponentHandle;
use slint::winit_030::WinitWindowAccessor;
use wubble_core::apply::GlassSetting;
use wubble_core::space::{Rgb, dye};
use wubble_gfx::scene::run::{RunEvent, RunState, scene_due};
use wubble_gfx::scene::{
    Beam, BeamRect, GlassRect, Governor, Quality, Rect, Renderer, SceneOptions, Signal, Tier,
};
use wubble_gfx::{GfxError, GpuContext};
use wubble_ui::{AppWindow, GlassItem, GlassKeys, GlassLayer, GlassTier};

/// Set to `slint` to force the Slint-only glass (by hand, to see the fallback).
pub const GLASS_ENV: &str = "WUBBLE_GLASS";

/// The frame budget the glass is held to: 120 Hz (AGENTS.md). Also the budget when the
/// display's refresh rate is unknown.
const DEFAULT_BUDGET: Duration = Duration::from_micros(8_333);

/// The scene's share of one frame: its GPU time is judged against this fraction of the frame
/// budget, so the glass steps down while Slint and the page still fit in the rest
/// (TODO.md Decided, "Glass cost").
pub const SCENE_SHARE: f64 = 0.5;

/// The frame budget for a display refreshing at `mhz`: one display period, but never less
/// than the 120 Hz budget. A faster display (300 Hz VRR: 3.33 ms) does not ask the glass for
/// more than 120 Hz: with one period as the budget, `Full` never fit there and the governor
/// cycled through every level (TODO.md Decided, "Glass cost").
pub fn frame_budget(mhz: Option<u32>) -> Duration {
    mhz.filter(|&mhz| mhz > 0)
        .map_or(DEFAULT_BUDGET, |mhz| {
            Duration::from_secs_f64(1000.0 / f64::from(mhz))
        })
        .max(DEFAULT_BUDGET)
}

/// One frame's load for the governor, as a cost against the frame `budget`
/// ([`frame_budget`]): the larger of the scene's GPU time over its share (when timestamps
/// came back) and the frame's lateness, its interval past the budget (the scene redraws every
/// frame, so a longer interval is a dropped frame, whatever the device measures). `None` when
/// there is neither.
pub fn frame_load(
    gpu: Option<Duration>,
    interval: Option<Duration>,
    budget: Duration,
) -> Option<Duration> {
    let scene = gpu.map(|c| c.div_f64(SCENE_SHARE));
    let late = interval.map(|i| i.saturating_sub(budget));
    scene.max(late)
}

/// The levels a glass setting lets the governor use, best and cheapest (`Quality`'s order:
/// greater is cheaper). `Off` is the flat glass; the Slint-only fallback is not a level.
pub fn levels(setting: GlassSetting) -> (Quality, Quality) {
    match setting {
        GlassSetting::Auto => (Quality::Full, Quality::Flat),
        GlassSetting::Full => (Quality::Full, Quality::Full),
        GlassSetting::Reduced => (Quality::HalfRes, Quality::Cached),
        GlassSetting::Off => (Quality::Flat, Quality::Flat),
    }
}

/// `quality` held inside the setting's levels.
pub fn clamp(quality: Quality, setting: GlassSetting) -> Quality {
    let (best, worst) = levels(setting);
    quality.clamp(best, worst)
}

/// Which glass the window has, for the whole run.
pub enum Glass {
    /// The wgpu scene draws the backdrop and the glass; Slint draws the content on top.
    Scene(Box<SceneHost>),
    /// No scene: the panels draw their own Slint glass on the plain ground.
    SlintOnly,
}

/// The scene and what the host keeps to feed it.
pub struct SceneHost {
    renderer: Renderer,
    governor: Governor,
    /// The levels the user allows (`settings::GLASS_QUALITY`); the governor stays inside them.
    setting: GlassSetting,
    /// The frame budget ([`frame_budget`]), once the window knows its monitor.
    budget: Option<Duration>,
    /// The tint the palette was last dyed from.
    tint: Option<Rgb>,
    /// The last logical pointer position, for taps.
    pointer: Option<(f32, f32)>,
    image_error_logged: bool,
    /// When the scene draws (`run_state`): full rate, idle drift or paused.
    run: crate::run_state::Runner,
}

/// Why the scene is off, or the scene. `forced` is the [`GLASS_ENV`] value, `scene` the
/// `settings::GLASS_SCENE` row.
pub fn choose_with<R>(
    forced: Option<&str>,
    scene: bool,
    build: impl FnOnce() -> Result<R, GfxError>,
) -> Result<R, String> {
    if forced == Some("slint") {
        return Err(format!("{GLASS_ENV}=slint"));
    }
    if !scene {
        return Err("the glass scene is off in settings".into());
    }
    build().map_err(|e| e.to_string())
}

/// The glass for this run, logged; with the scene, tells the shell so (before it shows).
pub fn choose(gpu: &GpuContext, window: &AppWindow, scene: bool) -> Glass {
    let forced = std::env::var(GLASS_ENV).ok();
    match choose_with(forced.as_deref(), scene, || {
        Renderer::try_new(gpu, SceneOptions::default())
    }) {
        Ok(renderer) => {
            let signal = if renderer.measures_cost() {
                Signal::SceneCost
            } else {
                eprintln!(
                    "wubble: glass: no GPU timestamps ({}), quality follows dropped frames, \
                     a step down kept only when it helps",
                    renderer.timestamp_features()
                );
                Signal::Lateness
            };
            eprintln!("wubble: glass: scene");
            window.global::<GlassLayer>().set_scene_active(true);
            Glass::Scene(Box::new(SceneHost {
                renderer,
                governor: Governor::new(Quality::Full, signal),
                setting: GlassSetting::Auto,
                budget: None,
                tint: None,
                pointer: None,
                image_error_logged: false,
                run: Default::default(),
            }))
        }
        Err(reason) => {
            eprintln!("wubble: glass: slint-only ({reason})");
            Glass::SlintOnly
        }
    }
}

/// The scene's rect for a panel's item (logical px either side).
pub fn to_rect(item: &GlassItem) -> GlassRect {
    GlassRect {
        rect: Rect::new(item.x, item.y, item.width, item.height),
        radius: item.radius,
        tier: tier(item.tier),
        pressed: item.pressed,
        hover: item.hover,
        focus: item.focus,
        // transparent: no tint (the scene's black)
        tint: if item.tint.alpha() == 0 {
            [0.0; 3]
        } else {
            [item.tint.red(), item.tint.green(), item.tint.blue()].map(|c| f32::from(c) / 255.0)
        },
    }
}

/// The scene's tier of the Slint mirror (`GlassTier`, model.slint). Exhaustive, and checked
/// both ways below: a variant added on either side fails to compile or the test.
pub fn tier(t: GlassTier) -> Tier {
    match t {
        GlassTier::Thin => Tier::Thin,
        GlassTier::Regular => Tier::Regular,
        GlassTier::Thick => Tier::Thick,
    }
}

/// RGB 0..1 for the scene.
fn rgb01(Rgb(r, g, b): Rgb) -> [f32; 3] {
    [r, g, b].map(|c| f32::from(c) / 255.0)
}

/// The active Space's tint, as the shell shows it (the model is its one owner).
fn active_tint(window: &AppWindow) -> Option<Rgb> {
    use slint::Model;
    let active = window.get_active_space_id();
    let space = window.get_spaces().iter().find(|s| s.id == active)?;
    let c = space.tint;
    Some(Rgb(c.red(), c.green(), c.blue()))
}

impl Glass {
    /// The scene's last frame as RGBA8 (width, height, pixels), for the frames smoke only: a
    /// readback that waits for the GPU.
    pub fn readback(&self) -> Option<(u32, u32, Vec<u8>)> {
        match self {
            Glass::Scene(s) => s.renderer.readback(),
            Glass::SlintOnly => None,
        }
    }
    /// The scene's current level, `None` without the scene.
    pub fn quality(&self) -> Option<Quality> {
        match self {
            Glass::Scene(s) => Some(s.governor.quality()),
            Glass::SlintOnly => None,
        }
    }

    pub fn is_scene(&self) -> bool {
        matches!(self, Glass::Scene(_))
    }

    /// The user's glass setting, applied at once: the governor starts over at the best level
    /// it allows (no measurements of other levels carry over).
    pub fn set_setting(&mut self, setting: GlassSetting) {
        let Glass::Scene(s) = self else {
            return;
        };
        if s.setting == setting {
            return;
        }
        s.setting = setting;
        let (best, _) = levels(setting);
        s.governor = Governor::new(best, s.governor.signal());
        s.renderer.set_quality(best);
    }

    /// The scene's output size, physical px, once it has rendered (`--measure`).
    pub fn scene_size(&self) -> Option<(u32, u32)> {
        match self {
            Glass::Scene(s) => s.renderer.output().map(|t| (t.width(), t.height())),
            Glass::SlintOnly => None,
        }
    }

    /// One rendered frame, after the shell driver's step. `glass` is the panels' new list
    /// when it changed. Steps and renders the scene only when it is due (`run::scene_due`: a
    /// page's frame alone does not make it due); returns whether it did.
    pub fn frame(&mut self, window: &AppWindow, now: Instant, glass: Option<&[GlassItem]>) -> bool {
        let Glass::Scene(s) = self else {
            return false;
        };
        // even while paused: the registry reports a changed list once, and the scene must have
        // the latest one when it resumes (cheap, no GPU work)
        if let Some(items) = glass {
            s.set_items(window, items);
        }
        let size = window.window().size();
        let resized = s
            .renderer
            .output()
            .is_none_or(|t| (t.width(), t.height()) != (size.width, size.height));
        if !scene_due(
            s.run.state(),
            window.get_page_fullscreen(),
            glass.is_some() || resized,
            s.renderer
                .last_frame()
                .map(|t| now.saturating_duration_since(t)),
        ) {
            return false;
        }
        let tint = active_tint(window);
        if tint != s.tint
            && let Some(t) = tint
        {
            s.renderer.scene.set_palette(dye(t).map(rgb01));
            s.tint = tint;
        }
        // the driver owns the beam's lap (wubble_core::beam); the scene draws this frame's shape
        let b = window.get_address_beam();
        s.renderer
            .scene
            .set_beam_shape((b.strength > 0.0).then_some(Beam {
                head: b.head.into(),
                length: b.length.into(),
                strength: b.strength.into(),
                hue: b.hue.into(),
            }));

        // Idle frames are slow on purpose, and the first frame after a change of run state
        // follows a gap (an idle tick, a pause): neither says anything about the frame's load.
        let steady = !s.run.take_changed();
        let full_rate = steady && s.run.state() == RunState::Running;
        let interval = s
            .renderer
            .last_frame()
            .map(|t| now.saturating_duration_since(t))
            .filter(|_| full_rate);
        match s
            .renderer
            .before_rendering(window.window(), now, s.run.max_dt())
        {
            // every frame: see `Renderer::before_rendering`
            Ok(Some(image)) => window.set_scene(image),
            Ok(None) => {}
            Err(e) => {
                if !std::mem::replace(&mut s.image_error_logged, true) {
                    eprintln!("wubble: glass: scene image: {e}");
                }
            }
        }
        s.govern(window, interval);
        let animating = s.renderer.animating();
        s.run.frame_done(window, now, animating, glass.is_some());
        true
    }

    /// A window event that changes when the scene draws (`run_state::event_of`).
    pub fn run_event(&mut self, event: RunEvent, window: &AppWindow) {
        if let Glass::Scene(s) = self {
            s.run.on(event, window);
        }
    }

    /// The pointer moved, in logical px.
    pub fn pointer_move(&mut self, x: f32, y: f32) {
        if let Glass::Scene(s) = self {
            s.pointer = Some((x, y));
            s.renderer.scene.pointer_move(x, y);
            s.run.pointer_moved(Instant::now());
        }
    }

    pub fn pointer_leave(&mut self) {
        if let Glass::Scene(s) = self {
            s.pointer = None;
            s.renderer.scene.pointer_leave();
        }
    }

    /// A press where the pointer is.
    pub fn tap(&mut self) {
        if let Glass::Scene(s) = self
            && let Some((x, y)) = s.pointer
        {
            s.renderer.scene.tap(x, y);
        }
    }
}

impl SceneHost {
    fn set_items(&mut self, window: &AppWindow, items: &[GlassItem]) {
        let rects: Vec<GlassRect> = items.iter().map(to_rect).collect();
        self.renderer.scene.set_glass(&rects);
        // the beam runs round the address pill's glass
        let address = window.global::<GlassKeys>().get_address();
        let beam = items.iter().find(|i| i.key == address).map(|i| BeamRect {
            rect: Rect::new(i.x, i.y, i.width, i.height),
            radius: i.radius,
        });
        self.renderer.scene.set_beam(beam);
    }

    /// Feeds the governor this frame's load ([`frame_load`]), at the edge, and applies its level.
    fn govern(&mut self, window: &AppWindow, interval: Option<Duration>) {
        let budget = *self.budget.get_or_insert_with(|| {
            frame_budget(
                window
                    .window()
                    .with_winit_window(crate::display_refresh_mhz)
                    .flatten(),
            )
        });
        let Some(cost) = frame_load(self.renderer.take_cost(), interval, budget) else {
            return;
        };
        let before = self.governor.quality();
        let next = self.governor.next(cost, budget);
        let after = clamp(next.quality(), self.setting);
        // A step outside the allowed levels is undone: the governor restarts at the bound.
        self.governor = if after == next.quality() {
            next
        } else {
            Governor::new(after, self.governor.signal())
        };
        if after != before {
            // The load that moved it: the smoothed load of the level it left.
            let load = self.governor.load_at(before).unwrap_or_default();
            eprintln!(
                "wubble: glass quality {before:?} -> {after:?} (load {:.2} ms of {:.2} ms)",
                load.as_secs_f64() * 1e3,
                budget.as_secs_f64() * 1e3,
            );
            self.renderer.set_quality(after);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_item_maps_field_by_field() {
        let item = GlassItem {
            key: 3,
            x: 1.0,
            y: 2.0,
            width: 30.0,
            height: 40.0,
            radius: 12.0,
            tier: GlassTier::Thick,
            pressed: true,
            hover: false,
            focus: true,
            tint: Default::default(),
            present: true,
        };
        assert_eq!(
            to_rect(&item),
            GlassRect {
                rect: Rect::new(1.0, 2.0, 30.0, 40.0),
                radius: 12.0,
                tier: Tier::Thick,
                pressed: true,
                hover: false,
                focus: true,
                tint: [0.0; 3],
            }
        );
        // a Space colour is mixed in (0..1); transparent is none
        let tinted = GlassItem {
            tint: slint::Color::from_rgb_u8(255, 0, 51),
            ..item
        };
        assert_eq!(to_rect(&tinted).tint, [1.0, 0.0, 0.2]);
    }

    #[test]
    fn the_scene_is_off_when_forced_or_when_it_fails_to_build() {
        let built = || Ok::<_, GfxError>(7);
        assert_eq!(choose_with(None, true, built), Ok(7));
        assert_eq!(choose_with(Some("scene"), true, built), Ok(7));
        assert_eq!(
            choose_with(Some("slint"), true, built),
            Err("WUBBLE_GLASS=slint".into())
        );
        // The settings row turns the scene off for the Slint-only glass.
        assert!(choose_with(None, false, built).is_err());
        let failed = || Err::<u8, _>(GfxError::Device("scene pipelines: bad".into()));
        let reason = choose_with(None, true, failed).unwrap_err();
        assert!(reason.contains("scene pipelines: bad"), "{reason}");
    }

    #[test]
    fn slint_glass_tier_mirrors_the_scene() {
        let to_ui = |t| match t {
            Tier::Thin => GlassTier::Thin,
            Tier::Regular => GlassTier::Regular,
            Tier::Thick => GlassTier::Thick,
        };
        for t in Tier::ALL {
            assert_eq!(tier(to_ui(t)), t);
        }
    }

    #[test]
    fn a_frame_is_judged_by_the_scene_share_and_by_lateness() {
        let ms = Duration::from_millis;
        let budget = ms(8);
        // the scene's GPU time counts against its share of the frame
        assert_eq!(frame_load(Some(ms(3)), None, budget), Some(ms(6)));
        // a frame on time is no load; one budget late (a dropped frame) is a full budget
        assert_eq!(frame_load(None, Some(ms(8)), budget), Some(ms(0)));
        assert_eq!(frame_load(None, Some(ms(16)), budget), Some(ms(8)));
        // the larger signal wins
        assert_eq!(frame_load(Some(ms(1)), Some(ms(16)), budget), Some(ms(8)));
        assert_eq!(frame_load(Some(ms(5)), Some(ms(8)), budget), Some(ms(10)));
        assert_eq!(frame_load(None, None, budget), None);
    }

    #[test]
    fn the_glass_is_never_asked_for_more_than_120_hz() {
        // the owner's 300 Hz display: 120 Hz, not one 3.33 ms period
        assert_eq!(frame_budget(Some(300_002)), DEFAULT_BUDGET);
        assert_eq!(frame_budget(Some(144_000)), DEFAULT_BUDGET);
        let at_120 = frame_budget(Some(120_000));
        assert!(
            at_120.abs_diff(DEFAULT_BUDGET) < Duration::from_micros(1),
            "{at_120:?}"
        );
        // a slower display gives its whole period
        let sixty = frame_budget(Some(60_000));
        assert!(sixty > Duration::from_micros(16_600) && sixty < Duration::from_micros(16_700));
        assert_eq!(frame_budget(None), DEFAULT_BUDGET);
        assert_eq!(frame_budget(Some(0)), DEFAULT_BUDGET);
    }

    #[test]
    fn frames_paced_at_300_hz_are_not_late() {
        // 3.33 ms apart, scene at 1 ms of GPU: well inside the budget at Full, held there
        let budget = frame_budget(Some(300_002));
        let mut g = Governor::new(Quality::Full, Signal::SceneCost);
        for _ in 0..10_000 {
            let load = frame_load(
                Some(Duration::from_millis(1)),
                Some(Duration::from_micros(3_333)),
                budget,
            );
            g = g.next(load.unwrap(), budget);
        }
        assert_eq!(g.quality(), Quality::Full);
    }

    #[test]
    fn a_scene_over_its_share_degrades_the_glass_before_the_frame_drops() {
        // 5 ms of scene in an 8.33 ms frame: the frame still fits, the glass steps down
        let (budget, scene) = (DEFAULT_BUDGET, Duration::from_millis(5));
        let mut g = Governor::new(Quality::Full, Signal::SceneCost);
        for _ in 0..wubble_gfx::scene::SETTLE_FRAMES {
            g = g.next(
                frame_load(Some(scene), Some(budget), budget).unwrap(),
                budget,
            );
        }
        assert_eq!(g.quality(), Quality::HalfRes);
    }

    #[test]
    fn dropped_frames_degrade_the_glass_without_timestamps() {
        let budget = DEFAULT_BUDGET;
        let mut g = Governor::new(Quality::Full, Signal::SceneCost);
        for _ in 0..wubble_gfx::scene::SETTLE_FRAMES {
            g = g.next(frame_load(None, Some(budget * 2), budget).unwrap(), budget);
        }
        assert_eq!(g.quality(), Quality::HalfRes);
    }

    #[test]
    fn the_glass_setting_bounds_the_level() {
        use Quality::*;
        let all = [Full, HalfRes, Cached, Flat];
        for q in all {
            assert_eq!(clamp(q, GlassSetting::Auto), q);
            assert_eq!(clamp(q, GlassSetting::Full), Full);
            assert_eq!(clamp(q, GlassSetting::Off), Flat);
        }
        assert_eq!(clamp(Full, GlassSetting::Reduced), HalfRes);
        assert_eq!(clamp(Cached, GlassSetting::Reduced), Cached);
        assert_eq!(clamp(Flat, GlassSetting::Reduced), Cached);
    }

    #[test]
    fn tints_go_to_the_scene_in_0_to_1() {
        assert_eq!(rgb01(Rgb(255, 0, 51)), [1.0, 0.0, 0.2]);
    }
}
