//! The window's glass: the design's wgpu scene under the shell, or Slint-only glass when it
//! cannot run. Chosen once at startup ([`choose`]) into one explicit [`Glass`], logged.
//!
//! With the scene, the shell driver's rendering notifier calls [`Glass::frame`] after its own
//! step (the wiring of the design repo's `render.rs::install`, the-wallpaper-project
//! `src/wubble/slint`, owner's code, moved into that one notifier): the scene renders under the
//! UI and its image is the window's bottom layer. The scene is fed from data only: the glass
//! list the panels report (`wubble_ui::glass`), the active Space's tint, the address pill's
//! beam (`address-beam`, the driver's), the pointer (from the host's winit filter) and, for the quality governor, each
//! frame's load ([`frame_load`]). The panes' latest frames go to it too ([`Glass::set_pages`]):
//! overlay glass over a pane frosts the page, and the window's glass picks up the page's light
//! (`wubble_gfx::scene::pages`). Overlay glass comes back as one image per layer, shown above
//! the page (`over-card`, `over-peek`, `over-top`).

use std::time::{Duration, Instant};

use slint::ComponentHandle;
use slint::winit_030::WinitWindowAccessor;
use wubble_core::apply::GlassSetting;
use wubble_core::space::{Rgb, dye};
use wubble_core::start::living::Living;
use wubble_core::start::rand::Rng;
use wubble_core::tab::TabId;
use wubble_gfx::scene::run::{RunEvent, RunState, scene_due};
use wubble_gfx::scene::{
    Backdrop, Beam, Damage, GlassRect, Governor, OverLayer, PageBlur, PagePane, PageRect, Quality,
    Rect, Renderer, SceneOptions, Signal, StartCard, Tier, out_size, view_of,
};
use wubble_gfx::{GfxError, GpuContext};
use wubble_ui::{
    AppWindow, GlassItem, GlassKeys, GlassLayer, GlassOver, GlassTier, Radius, StartBridge,
};

use crate::host::start::{self, StagePointer, StartHost};
use crate::measure::Count;

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
    /// When the scene draws (`run_state`): running, still or paused.
    run: crate::run_state::Runner,
    /// Wakes the window when the page blur is due later (`PageBlur::After`: the ambience's rate).
    page_wake: slint::Timer,
    /// The start pages' bubbles (host/start.rs); the scene's living layer while one shows.
    start: StartHost,
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
        // the browser window's backdrop is the still dye (owner, 2026-11-19: no moving liquid)
        Renderer::try_new(
            gpu,
            SceneOptions {
                backdrop: Backdrop::Still,
                ..SceneOptions::default()
            },
        )
    }) {
        Ok(mut renderer) => {
            // over a page the design's CSS glass, and the contrast ceiling (wubble_core::glass)
            renderer.set_material(material());
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
                page_wake: slint::Timer::default(),
                // `--frozen-backdrop`: the same bubble on every run
                start: StartHost::new(if crate::shot::frozen_backdrop() {
                    0
                } else {
                    start_nonce()
                }),
            }))
        }
        Err(reason) => {
            eprintln!("wubble: glass: slint-only ({reason})");
            Glass::SlintOnly
        }
    }
}

/// The glass's material over pages and its contrast ceiling, from the design's tokens.
pub fn material() -> wubble_gfx::scene::Material {
    use wubble_core::glass;
    wubble_gfx::scene::Material {
        css: glass::css_tiers(),
        ceiling: glass::ceiling() as f32,
        knee: glass::knee() as f32,
    }
}

/// The shell's glass keys that the scene treats apart (`GlassKeys`, model.slint).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Keys {
    /// The window's own glass (`GlassKeys.sidebar`).
    pub window: i32,
    pub address: i32,
    /// The first favourite tile; a tile's key is this plus its tab id.
    pub favourites: i32,
    /// The first overlay's key: every overlay's is at least this.
    pub overlays: i32,
    /// `GlassItem.layer` of each [`OverLayer`], by [`OverLayer::index`] (`GlassOver`): Slint
    /// holds the numbers.
    pub layers: [i32; 3],
}

impl Keys {
    fn of(window: &AppWindow) -> Self {
        let k = window.global::<GlassKeys>();
        Keys {
            window: k.get_sidebar(),
            address: k.get_address(),
            favourites: k.get_favourites(),
            overlays: k.get_overlays(),
            layers: {
                let o = window.global::<GlassOver>();
                OverLayer::ALL.map(|l| match l {
                    OverLayer::Card => o.get_card(),
                    OverLayer::Peek => o.get_peek(),
                    OverLayer::Top => o.get_top(),
                })
            },
        }
    }

    /// The design's drop-shadow weight of a panel: the window's glass 1 (desktop.tsx:50), the
    /// address bar 0 (chrome.tsx:35), a favourite 0.3 (chrome.tsx:72), any other `useGlass`'s
    /// 0.6 (browser/layer.ts:40).
    pub fn shadow(&self, key: i32) -> f32 {
        match key {
            k if k == self.window => 1.0,
            k if k == self.address => 0.0,
            k if k >= self.favourites && k < self.overlays => 0.3,
            _ => 0.6,
        }
    }
}

/// The scene's rect for a panel's item (logical px either side).
pub fn to_rect(item: &GlassItem, keys: &Keys) -> GlassRect {
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
        shadow: keys.shadow(item.key),
        // the beam runs round the address pill's glass
        beam: item.key == keys.address,
        layer: OverLayer::ALL
            .into_iter()
            .find(|l| keys.layers[l.index()] == item.layer),
        alpha: item.alpha,
    }
}

/// A start page nonce for this run: its bubbles differ from the last run's.
fn start_nonce() -> u64 {
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    u64::from(std::process::id()).rotate_left(32) ^ t.as_nanos() as u64
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
        // against the texture `before_rendering` sizes, which is smaller than the window above
        // `DPR_MAX`
        let size = window.window().size();
        let (w, h, dpr) = view_of(size.width, size.height, window.window().scale_factor());
        let resized = s
            .renderer
            .output()
            .is_none_or(|t| (t.width(), t.height()) != out_size(w, h, dpr));
        // the scene's own motion starting (a beam lap, a Space's palette) wakes a still scene,
        // as input does; a page's frames do not
        let tint = active_tint(window);
        if window.get_address_beam().strength > 0.0 || (tint.is_some() && tint != s.tint) {
            s.run.wake_if_still(window, Count::WakeBeam);
        }
        let covered = window.get_page_fullscreen();
        let page_due = s.schedule_pages(window, now, covered);
        if !scene_due(s.run.state(), covered, glass.is_some() || resized, page_due) {
            return false;
        }
        let since = s
            .renderer
            .last_frame()
            .map(|t| now.saturating_duration_since(t));
        crate::measure::count(crate::measure::Count::SceneRender);
        if tint != s.tint
            && let Some(t) = tint
        {
            s.renderer.scene.set_palette(dye(t).map(rgb01));
            // the start page's liquid takes the Space's dye too (pages/start.tsx:29-33)
            s.renderer.set_start_palette(dye(t).map(rgb01));
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

        // The first frame after a change of run state follows a gap (a still spell, a pause):
        // it says nothing about the frame's load.
        let steady = !s.run.take_changed();
        // `--frozen-backdrop` (fidelity harness): the scene's clock never advances.
        let pace = if crate::shot::frozen_backdrop() {
            0.0
        } else {
            1.0
        };
        let full_rate = steady && s.run.state() == RunState::Running;
        let interval = since.filter(|_| full_rate);
        s.start_frame(window, since);
        // the scene drew into the textures Slint already shows: Slint cannot see that, so the
        // frame's damage is marked (the start page's card alone while nothing else moved)
        let mut damage = Damage::Whole;
        let mut new_textures = false;
        match s.renderer.before_rendering(window.window(), now, pace) {
            // new textures only (first frame, resize, a layer's glass appearing or gone): see
            // `Renderer::before_rendering`
            Ok(frame) => {
                damage = frame.damage;
                if let Some(image) = frame.scene {
                    new_textures = true;
                    window.set_scene(image);
                }
                for (layer, image) in OverLayer::ALL.into_iter().zip(frame.over) {
                    if let Some(image) = image {
                        new_textures = true;
                        match layer {
                            OverLayer::Card => window.set_over_card(image),
                            OverLayer::Peek => window.set_over_peek(image),
                            OverLayer::Top => window.set_over_top(image),
                        }
                    }
                }
            }
            Err(e) => {
                if !std::mem::replace(&mut s.image_error_logged, true) {
                    eprintln!("wubble: glass: scene image: {e}");
                }
            }
        }
        s.govern(window, interval);
        let animating = s.renderer.animating();
        // a changed window size is a change like a changed glass list: it wakes a still scene
        s.run
            .frame_done(window, animating, glass.is_some() || resized);
        // the scene drew into the textures Slint already shows: Slint cannot see that. When a
        // single opaque web pane covers most of the window and nothing moved/changed structure,
        // mark only the strips around that pane plus any active overlay glass rects so Skia
        // partial rendering skips re-compositing the opaque webpage interior.
        let partial_pane = (!new_textures
            && glass.is_none()
            && !resized
            && !s.renderer.start_shown()
            && !window.get_edge_bowed()
            && window.get_page_enter() >= 1.0
            && window.get_exit_t() >= 1.0)
            .then(|| s.renderer.single_page_rect())
            .flatten()
            // the one page fills the content area: beside a start page, a built-in page or a
            // pane without a frame yet the scene shows inside the window, so all of it is marked
            .filter(|pane| (pane.rect.w - window.get_content_width()).abs() < 1.0);
        match damage {
            Damage::Card(r) => crate::partial::mark_rect(window.window(), r.x, r.y, r.w, r.h),
            Damage::Nothing => {}
            Damage::Whole => {
                if let Some(pane) = partial_pane {
                    let overlays = s.renderer.overlay_dirty_rects();
                    for r in scene_dirty_rects(w, h, pane, &overlays) {
                        crate::partial::mark_rect(window.window(), r.x, r.y, r.w, r.h);
                    }
                } else {
                    crate::partial::mark_window(window.window());
                }
            }
        }
        true
    }

    /// The panes under the glass, left to right, with their latest frames (the host's, on every
    /// frame presented and every layout): the page blur follows them when an overlay is over a
    /// pane or for the ambience, and asks for the frame that builds it.
    pub fn set_pages(&mut self, window: &AppWindow, pages: Vec<PagePane>) {
        if let Glass::Scene(s) = self {
            s.renderer.set_pages(pages);
            if s.schedule_pages(window, Instant::now(), window.get_page_fullscreen()) {
                window.window().request_redraw();
            }
        }
    }

    /// A pane's texture was written in place within `damage` (window logical px,
    /// `Host::present_panes`): marks the page blur stale only when `damage` reaches an overlay
    /// glass surface or the ambience around a pane. `None`: written whole for the glass over a
    /// page on its layer (the page layer's snapshot), stale whatever reaches it.
    pub fn pages_written(&mut self, window: &AppWindow, damage: Option<Rect>) {
        if let Glass::Scene(s) = self {
            match damage {
                Some(damage) => s.renderer.pages_written_rect(damage),
                None => s.renderer.pages_written(),
            }
            if s.schedule_pages(window, Instant::now(), window.get_page_fullscreen()) {
                window.window().request_redraw();
            }
        }
    }

    /// Whether an overlay's glass drawn now lies over `page` (window logical px); none without
    /// the scene.
    pub fn overlay_over(&self, page: Rect) -> bool {
        match self {
            Glass::Scene(s) => s.renderer.overlay_over(page),
            Glass::SlintOnly => false,
        }
    }

    /// How many times the page blur has been built (the overlay smoke's count).
    pub fn pages_built(&self) -> Option<u64> {
        match self {
            Glass::Scene(s) => Some(s.renderer.pages_built()),
            Glass::SlintOnly => None,
        }
    }

    /// The ambience's strength (the overlay smoke's control: 0 off), drawn on the next frame.
    pub fn set_ambience(&mut self, window: &AppWindow, strength: f32) {
        if let Glass::Scene(s) = self {
            s.renderer.set_ambience(strength);
            s.run.wake_if_still(window, Count::WakeAmbience);
            window.window().request_redraw();
        }
    }

    /// A window event that changes when the scene draws (`run_state::event_of`); `cause` is
    /// what `--measure` counts if it wakes the scene. Input and the window shown again also wake
    /// a settled start page, on this same frame.
    pub fn run_event(&mut self, event: RunEvent, cause: Count, window: &AppWindow) {
        if let Glass::Scene(s) = self {
            let woke = matches!(event, RunEvent::Activity | RunEvent::Shown) && s.start.wake();
            s.run.on(event, cause, window);
            if woke {
                s.run.wake_if_still(window, Count::WakeStart);
            }
        }
    }

    /// Which start tab the window shows (`None`: none) and which tabs still exist
    /// (`StartHost::sync`); the scene's living layer comes and goes with it.
    pub fn start_sync(
        &mut self,
        window: &AppWindow,
        shown: Option<TabId>,
        alive: impl Fn(TabId) -> bool,
    ) {
        let Glass::Scene(s) = self else {
            return;
        };
        let was = s.start.shown();
        s.start.sync(shown, alive);
        if shown.is_none() {
            s.renderer.start_end();
        }
        if was != shown {
            s.run.wake_if_still(window, Count::WakeStart);
            window.window().request_redraw();
        }
    }

    /// The start page's stage pointer.
    pub fn start_pointer(&mut self, window: &AppWindow, p: StagePointer) {
        if let Glass::Scene(s) = self {
            let out = s.start.pointer(p, Instant::now());
            s.apply_start(window, out);
            s.run.wake_if_still(window, Count::WakeStart);
        }
    }

    /// The start page's layer is up with its W field: the bubble is drawn (the start smoke's).
    pub fn start_ready(&self) -> bool {
        matches!(self, Glass::Scene(s) if s.renderer.has_wfield() && s.renderer.start_shown())
    }

    /// The shown start page's bubble (the start smoke's checks).
    pub fn start_living(&self) -> Option<&Living<Rng>> {
        match self {
            Glass::Scene(s) => s.start.living(),
            Glass::SlintOnly => None,
        }
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
    /// The start page's frame, before the scene renders: the bubble stepped by the frame's time
    /// (the fixed clock under `--frozen-backdrop`) at the page card's size, its effects applied,
    /// the W field handed over once it is computed, and the frame to draw.
    fn start_frame(&mut self, window: &AppWindow, since: Option<Duration>) {
        if self.start.shown().is_none() {
            return;
        }
        let frozen = crate::shot::frozen_backdrop();
        let (w, h) = (window.get_content_width(), window.get_content_height());
        let size = (f64::from(w), f64::from(h));
        let reduced = wubble_core::motion::reduced();
        // `--frozen-backdrop`: the web capture's scene time, whatever the frame count
        let out = if frozen {
            self.start.frame_frozen(size, reduced, start::FROZEN_AT)
        } else {
            self.start
                .frame(size, reduced, since.map_or(0.0, |d| d.as_secs_f64()))
        };
        self.apply_start(window, out);
        if let Some((rgba, n)) = self.start.take_wfield() {
            self.renderer.set_wfield(&rgba, n);
        }
        let card = StartCard {
            rect: Rect::new(window.get_content_x(), window.get_content_y(), w, h),
            radius: if window.get_page_fullscreen() {
                0.0
            } else {
                window.global::<Radius>().get_md()
            },
        };
        if let Some(l) = self.start.living() {
            self.renderer.start_frame(start::draw_of(l, card, frozen));
        }
    }

    /// What the start page did: its liquid's effects, the stage's cursor.
    fn apply_start(&mut self, window: &AppWindow, out: start::Out) {
        for fx in out.fx {
            self.renderer.start_effect(fx);
        }
        if let Some(grabbing) = out.grabbing {
            window.global::<StartBridge>().set_grabbing(grabbing);
        }
    }

    /// The page blur's verdict now (`PageBlur`): whether the next frame builds it, else arms the
    /// wake-up for when it is due (the ambience's rate). Nothing while a page fills the window.
    fn schedule_pages(&mut self, window: &AppWindow, now: Instant, covered: bool) -> bool {
        if covered {
            return false;
        }
        match self.renderer.page_verdict(now) {
            PageBlur::Build => true,
            PageBlur::Skip => false,
            PageBlur::After(d) => {
                if !self.page_wake.running() {
                    let w = window.as_weak();
                    self.page_wake
                        .start(slint::TimerMode::SingleShot, d, move || {
                            if let Some(w) = w.upgrade() {
                                w.window().request_redraw();
                            }
                        });
                }
                false
            }
        }
    }

    fn set_items(&mut self, window: &AppWindow, items: &[GlassItem]) {
        let keys = Keys::of(window);
        let rects: Vec<GlassRect> = items.iter().map(|i| to_rect(i, &keys)).collect();
        self.renderer.scene.set_glass(&rects);
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

/// The logical rectangles of a window `w`×`h` that may show the scene when a single opaque web
/// pane (`pane`) covers the content area: the four strips around `pane`'s inscribed opaque box
/// (sidebar, gutters, and the pane's rounded corners), plus every active overlay glass rect
/// (clamped to the window). Excludes the opaque interior of `pane` so Skia partial rendering does
/// not re-composite the whole webpage on sidebar ambience or pointer-light updates.
fn scene_dirty_rects(w: f32, h: f32, pane: PageRect, overlays: &[Rect]) -> Vec<Rect> {
    let mut out = Vec::with_capacity(4 + overlays.len());
    let r = pane.radius.max(0.0);
    let ix0 = (pane.rect.x + r).clamp(0.0, w);
    let iy0 = (pane.rect.y + r).clamp(0.0, h);
    let ix1 = (pane.rect.x + pane.rect.w - r).clamp(0.0, w);
    let iy1 = (pane.rect.y + pane.rect.h - r).clamp(0.0, h);
    if ix1 <= ix0 || iy1 <= iy0 {
        out.push(Rect::new(0.0, 0.0, w, h));
        return out;
    }
    let mut push = |x0: f32, y0: f32, x1: f32, y1: f32| {
        let (x0, y0) = (x0.clamp(0.0, w), y0.clamp(0.0, h));
        let (x1, y1) = (x1.clamp(0.0, w), y1.clamp(0.0, h));
        if x1 > x0 && y1 > y0 {
            out.push(Rect::new(x0, y0, x1 - x0, y1 - y0));
        }
    };
    // Left strip (sidebar + left rounded corners), right strip (right gutter + right corners),
    // and top/bottom strips between them.
    push(0.0, 0.0, ix0, h);
    push(ix1, 0.0, w, h);
    push(ix0, 0.0, ix1, iy0);
    push(ix0, iy1, ix1, h);
    for o in overlays {
        push(o.x, o.y, o.x + o.w, o.y + o.h);
    }
    out
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
            layer: 0,
            alpha: 1.0,
        };
        assert_eq!(
            to_rect(&item, &KEYS),
            GlassRect {
                rect: Rect::new(1.0, 2.0, 30.0, 40.0),
                radius: 12.0,
                tier: Tier::Thick,
                pressed: true,
                hover: false,
                focus: true,
                tint: [0.0; 3],
                shadow: 0.6,
                beam: false,
                layer: None,
                alpha: 1.0,
            }
        );
        // an overlay's layer and entrance fade
        let menu = GlassItem {
            key: 1_000_330,
            layer: 3,
            alpha: 0.5,
            ..item.clone()
        };
        let r = to_rect(&menu, &KEYS);
        assert_eq!((r.layer, r.alpha), (Some(OverLayer::Top), 0.5));
        assert_eq!(r.shadow, 0.6, "useGlass's");
        // a Space colour is mixed in (0..1); transparent is none
        let tinted = GlassItem {
            tint: slint::Color::from_rgb_u8(255, 0, 51),
            ..item
        };
        assert_eq!(to_rect(&tinted, &KEYS).tint, [1.0, 0.0, 0.2]);
        // the address pill carries the beam
        let address = GlassItem { key: 1, ..item };
        assert!(to_rect(&address, &KEYS).beam && !to_rect(&item, &KEYS).beam);
    }

    const KEYS: Keys = Keys {
        window: 0,
        address: 1,
        favourites: 100,
        overlays: 1_000_000,
        layers: [1, 2, 3],
    };

    #[test]
    fn each_panel_casts_the_design_s_shadow() {
        assert_eq!(KEYS.shadow(0), 1.0, "the window");
        assert_eq!(KEYS.shadow(1), 0.0, "the address bar");
        assert_eq!(KEYS.shadow(100), 0.3, "a favourite");
        assert_eq!(KEYS.shadow(107), 0.3);
        assert_eq!(KEYS.shadow(1_000_120), 0.6, "an overlay");
        assert_eq!(KEYS.shadow(3), 0.6, "useGlass's default");
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

    #[test]
    fn scene_dirty_rects_excludes_opaque_pane_interior_and_includes_overlays() {
        let pane = PageRect {
            rect: Rect::new(240.0, 8.0, 1000.0, 780.0),
            radius: 12.0,
        };
        let overlay = Rect::new(500.0, 200.0, 200.0, 100.0);
        let rects = scene_dirty_rects(1248.0, 796.0, pane, &[overlay]);
        // 4 strips around [252..1228, 20..776] + 1 overlay
        assert_eq!(rects.len(), 5);
        assert_eq!(rects[0], Rect::new(0.0, 0.0, 252.0, 796.0));
        assert_eq!(rects[1], Rect::new(1228.0, 0.0, 20.0, 796.0));
        assert_eq!(rects[2], Rect::new(252.0, 0.0, 976.0, 20.0));
        assert_eq!(rects[3], Rect::new(252.0, 776.0, 976.0, 20.0));
        assert_eq!(rects[4], overlay);
    }
}
