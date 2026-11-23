//! The page on its own OS layer (`wubble_core::page_layer`; stage 1, `--page-layer=fullscreen`:
//! a page in fullscreen and the picture-in-picture window, macOS). Each pane's route has one
//! owner, its window's [`PageLayers`]; its effects have one executor, [`Host::run_layer`], fed
//! from the frame path: a frame taken (`present_panes`), each render before it draws
//! (`panes_frame`: what is over the page now), the same render just before it is presented
//! (the vendored renderer's hook, `Host::presenting`: what it drew), the surfaces' exits
//! (`mounted-changed`) and a snapshot (its pixels). What keeps a page on the shell's path is read
//! from its owners on every call ([`Host::layer_needs`]), never kept.
//!
//! With the flag off (the default) no layer is made, so no pane has an entry and every frame
//! goes to `present_frame` as before.

use std::collections::BTreeMap;

use slint::ComponentHandle;
use wubble_core::page_layer::{self, Effect, Event, Mode, Needs, Owners, Route};
use wubble_core::stretch::after_frame;
use wubble_core::tab::TabId;
use wubble_core::window::{WindowId, WindowKind};
use wubble_engine::view::Layout;
use wubble_gfx::layer::PageLayer;
use wubble_gfx::{FrameSize, NativeFrame};
use wubble_ui::AppWindow;

use super::Host;
use super::panes::{content_size, frame_clip, pane_radius, px};
use super::window::WindowHost;
use crate::screen::Screen;

/// A window's page layers.
#[derive(Default)]
pub(crate) struct PageLayers {
    /// Whether this window can have them: unknown until a pane could first use one; false once
    /// making one failed (another platform, no view), never tried again.
    capable: Option<bool>,
    /// The panes with a layer, made at their first frame that could use one.
    panes: BTreeMap<TabId, LayerPane>,
}

impl PageLayers {
    /// No pane of the window has a layer.
    pub(super) fn is_empty(&self) -> bool {
        self.panes.is_empty()
    }

    /// A layer of the window is visible: the shell presents with its transaction.
    fn on_layer(&self) -> bool {
        self.panes.values().any(|p| p.route.on_layer())
    }
}

struct LayerPane {
    route: Route,
    layer: Box<dyn PageLayer>,
    /// The frame on the layer (moved here by `Show`: a frame cannot be copied), for
    /// `PresentLatest`.
    latest: Option<NativeFrame>,
    /// The generation of the page's frame the coming render draws: written as the render
    /// begins (`layers_drawn`), taken as it is presented; `None` for a render that began
    /// without it. The payload of `Event::Presenting`, nobody's state.
    drawn: Option<u64>,
}

/// A frame taken for a pane, with the layout it was taken for and the size its view wants.
struct Arrival {
    layout: Layout,
    wanted: FrameSize,
    frame: NativeFrame,
}

impl Host {
    /// `tab`'s new frame through its route: the shell's presenter, or its layer. Whether the
    /// shell drew it (the caller then renders the panes). A pane gets its layer at its first
    /// frame as the window's whole content while the mode allows one.
    pub(super) fn route_frame(
        &mut self,
        window: &AppWindow,
        tab: TabId,
        layout: Layout,
        wanted: FrameSize,
        frame: NativeFrame,
    ) -> Result<bool, String> {
        // with the mode off nothing here runs, and the map below stays empty
        if crate::page_layer() != Mode::Off
            && self.w.layers.capable != Some(false)
            && !self.w.layers.panes.contains_key(&tab)
            && matches!(self.whole_window(tab), (true, _) | (_, true))
        {
            match make_layer(window) {
                Ok(layer) => {
                    self.w.layers.capable = Some(true);
                    let pane = LayerPane {
                        route: Route::Unsupported,
                        layer,
                        latest: None,
                        drawn: None,
                    };
                    self.w.layers.panes.insert(tab, pane);
                    self.run_layer(window, tab, Event::Capable(true), None, false)?;
                }
                Err(e) => {
                    eprintln!("wubble: page layer: {e}");
                    self.w.layers.capable = Some(false);
                }
            }
        }
        if !self.w.layers.panes.contains_key(&tab) {
            self.present_frame(window, tab, layout, wanted, frame)?;
            return Ok(true);
        }
        let event = Event::Frame {
            os: frame.has_os_buffer(),
            painted: frame.generation(),
        };
        let arrival = Arrival {
            layout,
            wanted,
            frame,
        };
        self.run_layer(window, tab, event, Some(arrival), false)
    }

    /// The needs of every pane with a layer were derived again (`Event::Changed`), `pixels`:
    /// with a pixel reader's. Whether the shell was given a frame.
    pub(super) fn layers_changed(&mut self, window: &AppWindow, pixels: bool) -> bool {
        let tabs: Vec<TabId> = self.w.layers.panes.keys().copied().collect();
        let mut drew = false;
        for tab in tabs {
            match self.run_layer(window, tab, Event::Changed, None, pixels) {
                Ok(d) => drew |= d,
                Err(e) => eprintln!("wubble: page layer: {e}"),
            }
        }
        // a page that left fullscreen keeps no layer (it is made again at the next one)
        self.layers_gone(window, |host, tab| {
            host.w.layers.panes.get(&tab).is_some_and(|p| {
                page_layer::layer_unused(p.route, host.layer_needs(window, tab, None, false))
            })
        });
        drew
    }

    /// The render beginning draws each pane's frame its entry holds now.
    pub(super) fn layers_drawn(&mut self) {
        let frames = &self.w.frames;
        for (tab, pane) in &mut self.w.layers.panes {
            pane.drawn = frames.get(tab).map(|f| f.generation);
        }
    }

    /// A surface of this window began or ended its exit (`mounted-changed`, which Slint fires
    /// inside a render: queued to here): the overlays' exits end (`layers_mounted`); then only
    /// a window with a layer cares.
    pub(crate) fn surfaces_mounted(&mut self) {
        let Some(window) = self.w.window.upgrade() else {
            return;
        };
        self.layers_mounted();
        if self.w.layers.is_empty() {
            return;
        }
        if self.layers_changed(&window, false) {
            self.render_panes(&window);
        }
        window.window().request_redraw();
    }

    /// A snapshot is about to read `tab`'s frame from its presenter: a page on its layer hands
    /// its latest frame over first.
    pub(super) fn layer_pixels(&mut self, tab: TabId) {
        let Some(id) = self.window_where(|w| w.layers.panes.contains_key(&tab)) else {
            return;
        };
        let here = self.w.id;
        self.enter(id);
        if let Some(window) = self.w.window.upgrade() {
            match self.run_layer(&window, tab, Event::Changed, None, true) {
                Ok(true) => {
                    self.render_panes(&window);
                    window.window().request_redraw();
                }
                Ok(false) => {}
                Err(e) => eprintln!("wubble: page layer: {e}"),
            }
        }
        self.enter(here);
    }

    /// The panes of this window whose tab `gone` names left it (or the window closes): their
    /// layer goes at once (`Capable(false)`), and their entry with it.
    pub(super) fn layers_gone(&mut self, window: &AppWindow, gone: impl Fn(&Self, TabId) -> bool) {
        let tabs: Vec<TabId> = self
            .w
            .layers
            .panes
            .keys()
            .copied()
            .filter(|&t| gone(self, t))
            .collect();
        for tab in tabs {
            if let Err(e) = self.run_layer(window, tab, Event::Capable(false), None, false) {
                eprintln!("wubble: page layer: {e}");
            }
            self.w.layers.panes.remove(&tab);
        }
    }

    /// The renderer is about to present a frame of `window` (the vendored renderer's hook):
    /// each of its panes with a layer hears what that render drew of it.
    pub(crate) fn presenting(&mut self, window: &slint::Window) {
        let id =
            self.window_where(|w| std::ptr::eq(w.handle.window(), window) && !w.layers.is_empty());
        let Some(id) = id else {
            return;
        };
        let here = self.w.id;
        self.enter(id);
        if let Some(app) = self.w.window.upgrade() {
            let drawn: Vec<(TabId, Option<u64>)> = self
                .w
                .layers
                .panes
                .iter_mut()
                .map(|(&tab, pane)| (tab, pane.drawn.take()))
                .collect();
            for (tab, drawn) in drawn {
                let event = Event::Presenting { drawn };
                if let Err(e) = self.run_layer(&app, tab, event, None, false) {
                    eprintln!("wubble: page layer: {e}");
                }
            }
        }
        self.enter(here);
    }

    /// The window (the current one or a parked one) `holds` picks.
    pub(super) fn window_where(&self, holds: impl Fn(&WindowHost) -> bool) -> Option<WindowId> {
        std::iter::once(&self.w)
            .chain(self.parked.values())
            .find(|w| holds(w))
            .map(|w| w.id)
    }

    /// Whether `tab` is this window's whole content: its page fullscreen, and whether this is
    /// the picture-in-picture window.
    fn whole_window(&self, tab: TabId) -> (bool, bool) {
        (
            matches!(self.w.screen, Screen::Page { tab: t, .. } if t == tab),
            self.w.kind == WindowKind::Pip,
        )
    }

    /// What keeps `tab`'s page on the shell's path now, read from each owner: the window's
    /// screen and kind, the surfaces over the pane, the pane's stretch (after `arrival`, for a
    /// frame), the shots and pixel checks (and `pixels`, a caller's).
    fn layer_needs(
        &self,
        window: &AppWindow,
        tab: TabId,
        arrival: Option<&Arrival>,
        pixels: bool,
    ) -> Needs {
        let (page_fullscreen, pip_window) = self.whole_window(tab);
        let view_live = self.w.frames.get(&tab).is_some_and(|f| match arrival {
            Some(a) => after_frame(f.view, px(a.wanted), px(a.frame.size()))
                .0
                .is_live(),
            None => f.view.is_live(),
        });
        Needs::from_owners(&Owners {
            page_fullscreen,
            pip_window,
            coverage: self.page_covered(window, tab),
            view_live,
            pixels: pixels || crate::shot::pending() || self.checks.check_pixels,
        })
    }

    /// The one executor: `event` for `tab`'s pane through `page_layer::transition` with the
    /// needs as they hold now, each effect carried out in order; a failure goes back to the
    /// table once (it always ends on the shell's path). Whether the shell was given a frame.
    fn run_layer(
        &mut self,
        window: &AppWindow,
        tab: TabId,
        event: Event,
        mut arrival: Option<Arrival>,
        pixels: bool,
    ) -> Result<bool, String> {
        // a hide in a render's present hook joins that render's drawable, presented after the
        // hook returns: the shell stops presenting with the transaction at the next run that is
        // not one (the next render's start, before it acquires its drawable)
        let presenting = matches!(event, Event::Presenting { .. });
        let mut drew = false;
        // the first failure, returned once every effect ran (a `Hide` after it must still run)
        let mut error = None;
        let mut next = Some(event);
        while let Some(event) = next.take() {
            let Some(route) = self.w.layers.panes.get(&tab).map(|p| p.route) else {
                break;
            };
            let needs = self.layer_needs(window, tab, arrival.as_ref(), pixels);
            let (route, effects) = page_layer::transition(route, needs, event);
            if let Some(pane) = self.w.layers.panes.get_mut(&tab) {
                pane.route = route;
            }
            let mut failed = false;
            for effect in effects {
                match effect {
                    Effect::Buffers(on) => {
                        if let Some(Err(e)) = self.tab_view(tab).map(|v| v.set_os_buffers(on)) {
                            eprintln!("wubble: page layer buffers: {e}");
                            failed = true;
                        }
                    }
                    Effect::Show => {
                        let Some(a) = arrival.take() else {
                            continue;
                        };
                        let size = a.frame.size();
                        let clip = frame_clip(size, &a.layout);
                        let (_, height) = content_size(window);
                        let (cx, cy) = (window.get_content_x(), window.get_content_y());
                        let rect = self.pane_rects(window).into_iter().find(|r| r.tab == tab);
                        let scale = f64::from(window.window().scale_factor());
                        let radius = f64::from(pane_radius(window));
                        let Some(pane) = self.w.layers.panes.get_mut(&tab) else {
                            continue;
                        };
                        if let Some(r) = rect {
                            let (root_h, flipped) = pane.layer.root();
                            let pane_rect = page_layer::Rect {
                                x: f64::from(cx) + r.x,
                                y: f64::from(cy),
                                w: r.width,
                                h: height,
                            };
                            let buffer = (size.width, size.height);
                            let shown = (clip.width, clip.height);
                            let p =
                                page_layer::place(pane_rect, root_h, flipped, scale, buffer, shown);
                            let (f, c) = (p.frame, p.contents);
                            pane.layer.place(
                                [f.x, f.y, f.w, f.h],
                                [c.x, c.y, c.w, c.h],
                                p.scale,
                                radius,
                            );
                        }
                        match pane.layer.show(&a.frame) {
                            // the stretch follows a frame on the layer as it does the shell's
                            // (a refused one reaches the shell, which advances it then)
                            Ok(()) => {
                                if let Some(f) = self.w.frames.get_mut(&tab) {
                                    f.view = after_frame(f.view, px(a.wanted), px(size)).0;
                                }
                            }
                            Err(e) => {
                                eprintln!("wubble: page layer: {e}");
                                failed = true;
                            }
                        }
                        // kept even when refused: the failure hands it to the shell
                        pane.latest = Some(a.frame);
                    }
                    Effect::Hide => {
                        if let Some(pane) = self.w.layers.panes.get_mut(&tab) {
                            pane.layer.hide();
                            pane.latest = None;
                        }
                    }
                    Effect::Present => {
                        if let Some(a) = arrival.take() {
                            match self.present_frame(window, tab, a.layout, a.wanted, a.frame) {
                                Ok(()) => drew = true,
                                Err(e) => {
                                    failed = true;
                                    error.get_or_insert(e);
                                }
                            }
                        }
                    }
                    Effect::PresentLatest => {
                        let latest = self
                            .w
                            .layers
                            .panes
                            .get_mut(&tab)
                            .and_then(|p| p.latest.take());
                        if let Some(frame) = latest {
                            let layout = self.pane_layout(window, tab);
                            let wanted = self
                                .tab_view(tab)
                                .and_then(|v| v.wanted_frame())
                                .unwrap_or(layout.frame_size());
                            match self.present_frame(window, tab, layout, wanted, frame) {
                                Ok(()) => drew = true,
                                Err(e) => {
                                    failed = true;
                                    error.get_or_insert(e);
                                }
                            }
                        }
                    }
                    Effect::Redraw => {
                        if let Some(clip) = self.w.frames.get(&tab).map(|f| f.clip) {
                            let whole = wubble_gfx::import::Damage {
                                x: 0,
                                y: 0,
                                width: clip.width,
                                height: clip.height,
                            };
                            self.mark_damage(window, tab, clip, whole);
                        }
                        window.window().request_redraw();
                    }
                }
            }
            if failed && route != Route::Unsupported {
                next = Some(Event::Failed);
            }
        }
        if !presenting {
            let on = self.w.layers.on_layer();
            for pane in self.w.layers.panes.values_mut() {
                pane.layer.present_in_transaction(on);
            }
        }
        error.map_or(Ok(drew), Err)
    }
}

/// A layer over the shell's in `window`'s view.
#[cfg(target_os = "macos")]
fn make_layer(window: &AppWindow) -> Result<Box<dyn PageLayer>, String> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use slint::winit_030::WinitWindowAccessor;
    let view = window
        .window()
        .with_winit_window(|w| match w.window_handle().ok()?.as_raw() {
            RawWindowHandle::AppKit(h) => Some(h.ns_view),
            _ => None,
        })
        .flatten()
        .ok_or("the window has no AppKit view")?;
    // SAFETY: winit's NSView for this window, alive while the window is; the window's host
    // drops its layers before the window goes (`Host::close_window`).
    let layer = unsafe { wubble_gfx::layer::CaLayer::new(view) }.map_err(|e| e.to_string())?;
    Ok(Box::new(layer))
}

/// No page layer here: the shell draws every frame.
#[cfg(not(target_os = "macos"))]
fn make_layer(_window: &AppWindow) -> Result<Box<dyn PageLayer>, String> {
    Err("no page layer on this platform".into())
}

#[cfg(test)]
mod tests {
    use wubble_core::page_layer::Effect;

    /// Each effect's arm, as the executor matches it: a new effect does not compile until it
    /// is listed here.
    fn arm(effect: Effect) -> &'static str {
        match effect {
            Effect::Buffers(_) => "Effect::Buffers(",
            Effect::Show => "Effect::Show =>",
            Effect::Hide => "Effect::Hide =>",
            Effect::Present => "Effect::Present =>",
            Effect::PresentLatest => "Effect::PresentLatest =>",
            Effect::Redraw => "Effect::Redraw =>",
        }
    }

    /// Every effect has exactly one executor in the host's frame path.
    #[test]
    fn page_layer_effects_have_one_executor() {
        let here = include_str!("page_layer.rs");
        let here = &here[..here
            .find("#[cfg(test)]")
            .expect("the tests follow the code")];
        let sources = [here, include_str!("panes.rs")];
        let effects = [
            Effect::Buffers(true),
            Effect::Show,
            Effect::Hide,
            Effect::Present,
            Effect::PresentLatest,
            Effect::Redraw,
        ];
        for effect in effects {
            let arm = arm(effect);
            let n: usize = sources
                .iter()
                .map(|s| {
                    s.lines()
                        .filter(|l| l.contains(arm) && l.contains("=>"))
                        .count()
                })
                .sum();
            assert_eq!(n, 1, "{arm}");
        }
    }
}
