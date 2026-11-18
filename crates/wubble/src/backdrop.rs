//! The window's glass: the design's wgpu scene under the shell, or Slint-only glass when it
//! cannot run. Chosen once at startup ([`choose`]) into one explicit [`Glass`], logged.
//!
//! With the scene, the shell driver's rendering notifier calls [`Glass::frame`] after its own
//! step (the wiring of the design repo's `render.rs::install`, the-wallpaper-project
//! `src/wubble/slint`, owner's code, moved into that one notifier): the scene renders under the
//! UI and its image is the window's bottom layer. The scene is fed from data only: the glass
//! list the panels report (`wubble_ui::glass`), the active Space's tint, the address pill's
//! `loading`, the pointer (from the host's winit filter) and, for the quality governor, each
//! frame's load ([`frame_load`]).

use std::time::{Duration, Instant};

use slint::ComponentHandle;
use slint::winit_030::WinitWindowAccessor;
use wubble_core::space::{Rgb, dye};
use wubble_gfx::scene::{
    BeamRect, GlassRect, Governor, Quality, Rect, Renderer, SceneOptions, Tier,
};
use wubble_gfx::{GfxError, GpuContext};
use wubble_ui::{AppWindow, GlassItem, GlassKeys, GlassLayer, GlassTier};

/// Set to `slint` to force the Slint-only glass (by hand, to see the fallback).
pub const GLASS_ENV: &str = "WUBBLE_GLASS";

/// The frame budget when the display's refresh rate is unknown: 120 Hz.
const DEFAULT_BUDGET: Duration = Duration::from_micros(8_333);

/// The scene's share of one display period: its GPU time is judged against this fraction of
/// the frame, so the glass steps down while Slint and the page still fit in the rest
/// (TODO.md Decided, "Glass cost").
pub const SCENE_SHARE: f64 = 0.5;

/// One frame's load for the governor, as a cost against one display `period`: the larger of
/// the scene's GPU time over its share (when timestamps came back) and the frame's lateness,
/// its interval past one period (the scene redraws every frame, so a longer interval is a
/// dropped frame, whatever the device measures). `None` when there is neither.
pub fn frame_load(
    gpu: Option<Duration>,
    interval: Option<Duration>,
    period: Duration,
) -> Option<Duration> {
    let scene = gpu.map(|c| c.div_f64(SCENE_SHARE));
    let late = interval.map(|i| i.saturating_sub(period));
    scene.max(late)
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
    /// One display period, once the window knows its monitor.
    budget: Option<Duration>,
    /// The tint the palette was last dyed from.
    tint: Option<Rgb>,
    /// The address pill's `loading` last frame (the beam laps on its rising edge).
    loading: bool,
    /// The last logical pointer position, for taps.
    pointer: Option<(f32, f32)>,
    image_error_logged: bool,
}

/// Why the scene is off, or the scene. `forced` is the [`GLASS_ENV`] value.
pub fn choose_with<R>(
    forced: Option<&str>,
    build: impl FnOnce() -> Result<R, GfxError>,
) -> Result<R, String> {
    if forced == Some("slint") {
        return Err(format!("{GLASS_ENV}=slint"));
    }
    build().map_err(|e| e.to_string())
}

/// The glass for this run, logged; with the scene, tells the shell so (before it shows).
pub fn choose(gpu: &GpuContext, window: &AppWindow) -> Glass {
    let forced = std::env::var(GLASS_ENV).ok();
    match choose_with(forced.as_deref(), || {
        Renderer::try_new(gpu, SceneOptions::default())
    }) {
        Ok(renderer) => {
            if !renderer.measures_cost() {
                eprintln!("wubble: glass: no GPU timestamps, quality follows dropped frames only");
            }
            eprintln!("wubble: glass: scene");
            window.global::<GlassLayer>().set_scene_active(true);
            Glass::Scene(Box::new(SceneHost {
                renderer,
                governor: Governor::new(Quality::Full),
                budget: None,
                tint: None,
                loading: false,
                pointer: None,
                image_error_logged: false,
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
    pub fn is_scene(&self) -> bool {
        matches!(self, Glass::Scene(_))
    }

    /// One rendered frame, after the shell driver's step. `glass` is the panels' new list
    /// when it changed.
    pub fn frame(&mut self, window: &AppWindow, now: Instant, glass: Option<&[GlassItem]>) {
        let Glass::Scene(s) = self else {
            return;
        };
        if let Some(items) = glass {
            s.set_items(window, items);
        }
        let tint = active_tint(window);
        if tint != s.tint
            && let Some(t) = tint
        {
            s.renderer.scene.set_palette(dye(t).map(rgb01));
            s.tint = tint;
        }
        let loading = window.get_address_loading();
        if loading && !s.loading {
            s.renderer.scene.lap_beam();
        }
        s.loading = loading;

        let interval = s
            .renderer
            .last_frame()
            .map(|t| now.saturating_duration_since(t));
        match s.renderer.before_rendering(window.window(), now) {
            Ok(Some(image)) => window.set_scene(image),
            Ok(None) => {}
            Err(e) => {
                if !std::mem::replace(&mut s.image_error_logged, true) {
                    eprintln!("wubble: glass: scene image: {e}");
                }
            }
        }
        s.govern(window, interval);
    }

    /// The pointer moved, in logical px.
    pub fn pointer_move(&mut self, x: f32, y: f32) {
        if let Glass::Scene(s) = self {
            s.pointer = Some((x, y));
            s.renderer.scene.pointer_move(x, y);
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
            window
                .window()
                .with_winit_window(crate::display_refresh_mhz)
                .flatten()
                .filter(|&mhz| mhz > 0)
                .map_or(DEFAULT_BUDGET, |mhz| {
                    Duration::from_secs_f64(1000.0 / f64::from(mhz))
                })
        });
        let Some(cost) = frame_load(self.renderer.take_cost(), interval, budget) else {
            return;
        };
        let before = self.governor.quality();
        self.governor = self.governor.next(cost, budget);
        let after = self.governor.quality();
        if after != before {
            eprintln!("wubble: glass quality {before:?} -> {after:?}");
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
            }
        );
    }

    #[test]
    fn the_scene_is_off_when_forced_or_when_it_fails_to_build() {
        let built = || Ok::<_, GfxError>(7);
        assert_eq!(choose_with(None, built), Ok(7));
        assert_eq!(choose_with(Some("scene"), built), Ok(7));
        assert_eq!(
            choose_with(Some("slint"), built),
            Err("WUBBLE_GLASS=slint".into())
        );
        let failed = || Err::<u8, _>(GfxError::Device("scene pipelines: bad".into()));
        let reason = choose_with(None, failed).unwrap_err();
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
        let period = ms(8);
        // the scene's GPU time counts against its share of the frame
        assert_eq!(frame_load(Some(ms(3)), None, period), Some(ms(6)));
        // a frame on time is no load; one period late (a dropped frame) is a full period
        assert_eq!(frame_load(None, Some(ms(8)), period), Some(ms(0)));
        assert_eq!(frame_load(None, Some(ms(16)), period), Some(ms(8)));
        // the larger signal wins
        assert_eq!(frame_load(Some(ms(1)), Some(ms(16)), period), Some(ms(8)));
        assert_eq!(frame_load(Some(ms(5)), Some(ms(8)), period), Some(ms(10)));
        assert_eq!(frame_load(None, None, period), None);
    }

    #[test]
    fn a_scene_over_its_share_degrades_the_glass_before_the_frame_drops() {
        // 5 ms of scene in an 8.33 ms frame: the frame still fits, the glass steps down
        let (period, scene) = (DEFAULT_BUDGET, Duration::from_millis(5));
        let mut g = Governor::new(Quality::Full);
        for _ in 0..wubble_gfx::scene::DOWN_COUNT {
            g = g.next(
                frame_load(Some(scene), Some(period), period).unwrap(),
                period,
            );
        }
        assert_eq!(g.quality(), Quality::HalfRes);
    }

    #[test]
    fn dropped_frames_degrade_the_glass_without_timestamps() {
        let period = DEFAULT_BUDGET;
        let mut g = Governor::new(Quality::Full);
        for _ in 0..wubble_gfx::scene::DOWN_COUNT {
            g = g.next(frame_load(None, Some(period * 2), period).unwrap(), period);
        }
        assert_eq!(g.quality(), Quality::HalfRes);
    }

    #[test]
    fn tints_go_to_the_scene_in_0_to_1() {
        assert_eq!(rgb01(Rgb(255, 0, 51)), [1.0, 0.0, 0.2]);
    }
}
