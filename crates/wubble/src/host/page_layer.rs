//! The page on its own OS layer (`wubble_core::page_layer`, macOS; `--page-layer=fullscreen`: a
//! page in fullscreen and the picture-in-picture window, over the shell; `--page-layer=window`:
//! every pane, under the shell, which draws a hole where the pane's layer shows: its
//! `SplitPane.hole`, from [`PageLayers::holed`]; glass over it blurs a snapshot of the layer, which
//! `Effect::Snapshot` copies into the pane's presenter texture). Each pane's route has one
//! owner, its window's [`PageLayers`]; its effects have one executor, [`Host::run_layer`], fed
//! from the frame path: a frame taken (`present_panes`), each render before it draws
//! (`panes_frame`: what is over the page now), the same render just before it is presented
//! (the vendored renderer's hook, `Host::presenting`: what it drew), the surfaces' exits
//! (`mounted-changed`) and a snapshot (its pixels). What keeps a page on the shell's path is read
//! from its owners on every call ([`Host::layer_needs`]), never kept.
//!
//! With the mode off (`--page-layer=off`, Settings' row off, any platform but macOS, every smoke
//! and measurement that does not ask) no layer is made, so no pane has an entry and every frame
//! goes to `present_frame` as before. The route lines and the per-second summary print only with
//! `WUBBLE_TRACE` (`crate::trace`); failures always do.

use std::collections::BTreeMap;
use std::time::Instant;

use slint::ComponentHandle;
use wubble_core::overlay::{self, Coverage};
use wubble_core::page_layer::{
    self, Effect, Event, HideAt, Hold, Mode, Need, Needs, Order, Owners, Route,
};
use wubble_core::stretch::after_frame;
use wubble_core::tab::TabId;
use wubble_core::window::{WindowId, WindowKind};
use wubble_engine::view::Layout;
use wubble_gfx::layer::{PageLayer, Unsupported};
use wubble_gfx::{FrameSize, NativeFrame};
use wubble_ui::AppWindow;

use super::panes::{content_size, frame_clip, pane_radius, px};
use super::window::WindowHost;
use super::{Host, PaneFrame};
use crate::measure::{self, Count};
use crate::screen::Screen;

/// A window's page layers.
#[derive(Default)]
pub(crate) struct PageLayers {
    /// Whether this window can have them: unknown until a pane could first use one; false once
    /// making one failed (another platform, no view), never tried again.
    capable: Option<bool>,
    /// The panes with a layer, made at their first frame that could use one.
    panes: BTreeMap<TabId, LayerPane>,
    /// Layers that went while the shell's last frame still has their pane's hole (a tab left, a
    /// layer failed): still shown under the shell until the window's next present, which draws
    /// no hole there, then hidden with it and dropped (`Host::presenting`).
    retired: Vec<Box<dyn PageLayer>>,
    /// What the shell's layer was last told (`presentsWithTransaction`): what the next render
    /// acquires its drawable with.
    transacting: bool,
    render: Render,
}

/// Where the window's renders stand, for its layers' changes: one lands with the shell's frame
/// only inside a render whose drawable was acquired with the transaction
/// (`PageLayer::join_transaction`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Render {
    /// Between renders: a change commits on its own, at once.
    #[default]
    Between,
    /// A render's start (`panes_frame`, from `render_began` to `layers_drawn`): its `Changed`
    /// places the layers under their holes. `transacted`: its drawable was acquired with the
    /// transaction (read as it begins: nothing tells the shell's layer between the acquire and
    /// the render's start).
    Starting { transacted: bool },
    /// The rest of the render, to its present (`Host::presenting`).
    Drawing { transacted: bool },
}

impl PageLayers {
    /// No pane of the window has a layer, and none is retired.
    pub(super) fn is_empty(&self) -> bool {
        self.panes.is_empty() && self.retired.is_empty()
    }

    /// The shell's next render presents with the Core Animation transaction exactly while a
    /// retired layer waits or a pane's layer changes with it (`page_layer::transacts`); told to
    /// every layer (they share the shell's), a retired one too.
    fn transact(&mut self) {
        let on = !self.retired.is_empty()
            || self
                .panes
                .values()
                .any(|p| page_layer::transacts(p.route, p.hide_due, p.moved));
        self.transacting = on;
        self.layers().for_each(|layer| layer.present_in_transaction(on));
    }

    /// Every layer of the window, a retired one too.
    fn layers(&mut self) -> impl Iterator<Item = &mut Box<dyn PageLayer>> {
        self.panes
            .values_mut()
            .map(|p| &mut p.layer)
            .chain(self.retired.iter_mut())
    }

    /// The window's render now running is in `render`: its layers' changes join its transaction
    /// exactly while its drawable was acquired with one.
    fn set_render(&mut self, render: Render) {
        self.render = render;
        let join = self.transacted();
        self.layers().for_each(|layer| layer.join_transaction(join));
    }

    /// A render of the window begins (`panes_frame`): it acquired its drawable with the value
    /// last told.
    pub(super) fn render_began(&mut self) {
        let transacted = self.transacting;
        self.set_render(Render::Starting { transacted });
    }

    /// The render now running presents its drawable with the transaction: what changes now
    /// lands with its frame.
    fn transacted(&self) -> bool {
        matches!(
            self.render,
            Render::Starting { transacted: true } | Render::Drawing { transacted: true }
        )
    }

    /// The shell draws `tab`'s pane as a hole (`Route::holed`): its layer under the shell shows
    /// the page.
    pub(super) fn holed(&self, tab: TabId) -> bool {
        let mode = crate::page_layer();
        self.panes.get(&tab).is_some_and(|p| p.route.holed(mode))
    }
}

/// Pages on a layer whose newest frame welding dropped because every OS buffer was held (a burst
/// of frames, docs/research/page-layer.md "Dropped frames"), and the timer that asks them for it
/// again a refresh later, once the compositor has let a buffer go.
#[derive(Default)]
pub(crate) struct LostFrames {
    tabs: std::collections::BTreeSet<TabId>,
    timer: slint::Timer,
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
    /// A hide waits for the window's next transacted present (`HideAt::Due`); a new frame shown
    /// cancels it.
    hide_due: bool,
    /// The pane's place changed at the last render's start (or a frame shown placed the layer
    /// anew): the next render presents with the transaction (`page_layer::transacts`), so a move
    /// that goes on lands with the shell's frames.
    moved: bool,
    /// Where the pane was at the last render's start (`Host::pane_rect`).
    rect: Option<page_layer::Rect>,
}

/// A frame taken for a pane, with the layout it was taken for and the size its view wants.
struct Arrival {
    layout: Layout,
    wanted: FrameSize,
    frame: NativeFrame,
}

impl Host {
    /// `tab`'s new frame through its route: the shell's presenter, or its layer. Whether the
    /// shell's panes must be written again (the caller renders them): it was given the frame, or
    /// a hole opened or closed. A pane gets its layer at its first frame where the mode lets
    /// it use one (`page_layer::layer_unused`: the window's whole content in `fullscreen`, any
    /// pane of its own in `window`).
    pub(super) fn route_frame(
        &mut self,
        window: &AppWindow,
        tab: TabId,
        layout: Layout,
        wanted: FrameSize,
        frame: NativeFrame,
    ) -> Result<bool, String> {
        let mode = crate::page_layer();
        // with the mode off nothing here runs, and the map below stays empty; a pane without a
        // layer is asked only where it is
        if mode != Mode::Off
            && self.w.layers.capable != Some(false)
            && !self.w.layers.panes.contains_key(&tab)
            && !page_layer::layer_unused(
                Route::Unsupported,
                Needs::from_place(
                    matches!(self.whole_window(tab), (true, _) | (_, true)),
                    self.placed(window, tab),
                ),
                mode,
            )
        {
            match make_layer(window, mode.row().order == Order::Below) {
                Ok(layer) => {
                    self.w.layers.capable = Some(true);
                    let pane = LayerPane {
                        route: Route::Unsupported,
                        layer,
                        latest: None,
                        drawn: None,
                        hide_due: false,
                        moved: false,
                        rect: None,
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
    /// with a pixel reader's. Whether the shell's panes must be written again.
    pub(super) fn layers_changed(&mut self, window: &AppWindow, pixels: bool) -> bool {
        let tabs: Vec<TabId> = self.w.layers.panes.keys().copied().collect();
        let mut drew = false;
        for tab in tabs {
            match self.run_layer(window, tab, Event::Changed, None, pixels) {
                Ok(d) => drew |= d,
                Err(e) => eprintln!("wubble: page layer: {e}"),
            }
        }
        // a page where it may not use one keeps no layer (it is made again where it may)
        let mode = crate::page_layer();
        self.layers_gone(window, |host, tab| {
            host.w.layers.panes.get(&tab).is_some_and(|p| {
                let needs = host.layer_needs(window, tab, None, false, false);
                page_layer::layer_unused(p.route, needs, mode)
            })
        });
        drew
    }

    /// The render beginning draws each pane's frame its entry holds now; its start is over.
    pub(super) fn layers_drawn(&mut self) {
        let frames = &self.w.frames;
        for (tab, pane) in &mut self.w.layers.panes {
            pane.drawn = frames.get(tab).map(|f| f.generation);
        }
        let transacted = self.w.layers.transacted();
        self.w.layers.set_render(Render::Drawing { transacted });
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
            // a layer whose hide waits for a transacted present waits retired
            let layers = &mut self.w.layers;
            if let Some(pane) = layers.panes.remove(&tab).filter(|p| p.hide_due) {
                layers.retired.push(pane.layer);
                layers.transact();
                window.window().request_redraw();
            }
        }
    }

    /// The renderer is about to present a frame of `window` (the vendored renderer's hook):
    /// each of its panes with a layer hears what that render drew of it, and the layers retired
    /// before it (it draws no hole of theirs) go with it.
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
        // a render presented with the transaction (acquired so) takes the due hides and the retired
        // layers in its commit; one that was not leaves them for the next, asked for here; then
        // the shell's layer told what the next render does, and the hidden retired ones dropped
        let layers = &mut self.w.layers;
        let mut hidden = Vec::new();
        if layers.transacted() {
            hidden = std::mem::take(&mut layers.retired);
            hidden.iter_mut().for_each(|layer| layer.hide());
            for pane in layers.panes.values_mut().filter(|p| p.hide_due) {
                pane.hide_due = false;
                pane.layer.hide();
            }
        }
        let waiting = !layers.retired.is_empty() || layers.panes.values().any(|p| p.hide_due);
        layers.transact();
        let on = layers.transacting;
        hidden
            .iter_mut()
            .for_each(|layer| layer.present_in_transaction(on));
        // dropped still joined (removed in this present's commit); every change after this one
        // commits on its own until the next render begins
        drop(hidden);
        layers.set_render(Render::Between);
        if waiting {
            window.request_redraw();
        }
        self.enter(here);
    }

    /// After CEF's work: the pages on a layer in any window that lost their newest frame to a busy
    /// pool paint again a refresh from now (`repaint_lost`): else the layer shows the frame before
    /// until the page paints by itself, which a still page never does.
    pub(super) fn layer_frames_lost(&mut self) {
        let tabs: Vec<TabId> = std::iter::once(&self.w)
            .chain(self.parked.values())
            .flat_map(|w| w.layers.panes.keys().copied())
            .collect();
        for tab in tabs {
            if self.tab_view(tab).is_some_and(|v| v.take_os_buffers_lost()) {
                self.lost_frames.tabs.insert(tab);
            }
        }
        if self.lost_frames.tabs.is_empty() || self.lost_frames.timer.running() {
            return;
        }
        let delay = self
            .refresh()
            .unwrap_or(std::time::Duration::from_millis(16));
        self.lost_frames
            .timer
            .start(slint::TimerMode::SingleShot, delay, || {
                crate::with_host(Host::repaint_lost)
            });
    }

    /// Each page that lost its newest frame paints it again (`repaint_page`). One dropped again
    /// is reported again.
    fn repaint_lost(&mut self) {
        for tab in std::mem::take(&mut self.lost_frames.tabs) {
            if let Some(id) = self.model.window_of(tab)
                && self.enter(id)
            {
                self.repaint_page(tab);
            }
        }
        self.enter_focused();
    }

    /// `tab`'s page, in this window, paints its frame again: CEF's `invalidate`; with the shell's
    /// begin frames, the window's driver woken for it.
    fn repaint_page(&mut self, tab: TabId) {
        if let Some(view) = self.tab_view(tab) {
            view.repaint();
        }
        self.pace(wubble_gfx::pacing::PaceEvent::Wake);
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

    /// Whether `tab` is one of this window's panes with a page of its own to show
    /// (`page_layer::Owners::placed`): not the peek's page, an extension's popup or a built-in
    /// page (the shell draws it; a split's pane of one included).
    fn placed(&self, window: &AppWindow, tab: TabId) -> bool {
        self.pane_rects(window).iter().any(|r| r.tab == tab) && !self.model.is_internal(tab)
    }

    /// What keeps `tab`'s page on the shell's path now, read from each owner: the window's
    /// screen and kind, the surfaces over the pane, the pane's stretch (after `arrival`, for a
    /// frame), the shots and pixel checks (and `pixels`, a caller's), and `moved_alone`: the
    /// render starting moved the pane under its hole with a drawable presented on its own.
    fn layer_needs(
        &self,
        window: &AppWindow,
        tab: TabId,
        arrival: Option<&Arrival>,
        pixels: bool,
        moved_alone: bool,
    ) -> Needs {
        let (page_fullscreen, pip_window) = self.whole_window(tab);
        let view_live = self.w.frames.get(&tab).is_some_and(|f| match arrival {
            Some(a) => after_frame(f.view, px(a.wanted), px(a.frame.size()))
                .0
                .is_live(),
            None => f.view.is_live(),
        });
        let (stack, facts) = self.page_covers(window, tab);
        Needs::from_owners(&Owners {
            page_fullscreen,
            pip_window,
            placed: self.placed(window, tab),
            coverage: overlay::covers(&stack, &facts, tab),
            view_live,
            pixels: self.pixel_readers(pixels).iter().any(|&(_, on)| on),
            moved_alone,
        })
    }

    /// Who wants a page's pixels in the shell's textures now (`Need::Pixels`): a caller's
    /// (`pixels`, a snapshot), a shot, the pixel checks.
    fn pixel_readers(&self, pixels: bool) -> [(&'static str, bool); 3] {
        [
            ("snapshot", pixels),
            ("shot", crate::shot::pending()),
            ("pixel check", self.checks.check_pixels),
        ]
    }

    /// The one executor: `event` for `tab`'s pane through `page_layer::transition` with the
    /// needs as they hold now in the mode, each effect carried out in order; a failure goes back
    /// to the table once (it always ends on the shell's path). Whether the shell's panes must be
    /// written again: it was given a frame, or the pane's hole opened or closed.
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
        // a render's start (`panes_frame`): a layer under the shell follows its pane into it,
        // and whether the pane moved since the last render's start is read once here
        let start = matches!(event, Event::Changed)
            && matches!(self.w.layers.render, Render::Starting { .. });
        let moved = start && {
            let rect = self.pane_rect(window, tab);
            self.w
                .layers
                .panes
                .get_mut(&tab)
                .is_some_and(|p| std::mem::replace(&mut p.rect, rect) != rect)
        };
        // its layer cannot move with the hole in a render presented on its own: the shell draws
        // the page for it (`Need::Moved`), over the layer left where the last frame's hole is
        let moved_alone = moved && !self.w.layers.transacted() && self.w.layers.holed(tab);
        let mode = crate::page_layer();
        // the clock the machine's deadlines are kept in, read once here
        let now = self.surface_now();
        let mut drew = false;
        // the first failure, returned once every effect ran (a `Hide` after it must still run)
        let mut error = None;
        let mut next = Some(event);
        while let Some(event) = next.take() {
            let Some(before) = self.w.layers.panes.get(&tab).map(|p| p.route) else {
                break;
            };
            let needs = self.layer_needs(window, tab, arrival.as_ref(), pixels, moved_alone);
            let hold = needs.hold(mode);
            let (route, effects) = page_layer::transition(before, hold, now, event);
            if let Some(pane) = self.w.layers.panes.get_mut(&tab) {
                pane.route = route;
            }
            // the one place a route is replaced: counted; with `WUBBLE_TRACE`, each change, why,
            // and who holds the page
            let replaced = before.name() != route.name();
            if replaced {
                measure::count(Count::LayerRoute);
            }
            if replaced && crate::tracing().is_some() {
                let (stack, facts) = self.page_covers(window, tab);
                let owner = |need: Need| -> String {
                    match need {
                        Need::Covered | Need::Sampled | Need::Frosted => {
                            overlay::coverers(&stack, &facts, tab)
                                .filter(|&(_, c)| c != Coverage::None)
                                .map(|(kind, c)| format!("{kind:?} {c:?}"))
                                .collect::<Vec<_>>()
                                .join(", ")
                        }
                        Need::Resizing => self
                            .w
                            .frames
                            .get(&tab)
                            .map(|f| format!("stretch {:?}", f.view))
                            .unwrap_or_default(),
                        Need::Pixels => self
                            .pixel_readers(pixels)
                            .iter()
                            .filter(|&&(_, on)| on)
                            .map(|&(name, _)| name)
                            .collect::<Vec<_>>()
                            .join(", "),
                        Need::Windowed | Need::Unplaced | Need::Moved => String::new(),
                    }
                };
                let held: Vec<String> = Need::ALL
                    .into_iter()
                    .filter(|&n| needs.has(n))
                    .map(|n| match mode.row().holds[n as usize] {
                        Hold::Free => format!("{n:?}"),
                        hold => format!("{n:?} {hold:?} [{}]", owner(n)),
                    })
                    .collect();
                // a brief hold kept past its deadline reads as a long one (`Route::hold`)
                let expired = before.hold(hold, now) != hold;
                crate::trace(&format!(
                    "page layer: tab {}: {} -> {} on {event:?}; needs: {}{}",
                    tab.0,
                    before.name(),
                    route.name(),
                    if held.is_empty() {
                        "none".to_string()
                    } else {
                        held.join(", ")
                    },
                    if expired {
                        " (brief hold expired: Long)"
                    } else {
                        ""
                    },
                ));
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
                        let placed = self.place_layer(window, tab, size, &a.layout);
                        let Some(pane) = self.w.layers.panes.get_mut(&tab) else {
                            continue;
                        };
                        // placed anew: the render that opens or keeps its hole there transacts
                        pane.moved |= placed;
                        match pane.layer.show(&a.frame) {
                            // the stretch follows a frame on the layer as it does the shell's
                            // (a refused one reaches the shell, which advances it then)
                            Ok(()) => {
                                pane.hide_due = false;
                                measure::count(Count::LayerShow);
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
                        let transacted = presenting && self.w.layers.transacted();
                        if let Some(pane) = self.w.layers.panes.get_mut(&tab) {
                            match page_layer::hide_at(before, mode, transacted) {
                                // the shell's last frame still has the hole over it: it stays
                                // under the shell until a transacted present (`retired`)
                                HideAt::Retire => {
                                    let none: Box<dyn PageLayer> = Box::new(Unsupported);
                                    let layer = std::mem::replace(&mut pane.layer, none);
                                    self.w.layers.retired.push(layer);
                                }
                                HideAt::Due => pane.hide_due = true,
                                HideAt::Now => pane.layer.hide(),
                            }
                            pane.latest = None;
                        }
                    }
                    Effect::Present => {
                        if let Some(a) = arrival.take() {
                            match self.present_frame(window, tab, a.layout, a.wanted, a.frame) {
                                Ok(()) => {
                                    measure::count(Count::LayerPresent);
                                    drew = true;
                                }
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
                                Ok(()) => {
                                    measure::count(Count::LayerPresent);
                                    drew = true;
                                }
                                Err(e) => {
                                    failed = true;
                                    error.get_or_insert(e);
                                }
                            }
                        }
                    }
                    Effect::Snapshot => {
                        // the layer's latest frame into the pane's presenter texture, which only
                        // the glass reads (the pane is a hole): no Slint damage, the page blur
                        // stale, so the scene's next frame redraws the glass's rectangles only
                        let slot = self.slot_for(tab);
                        let layout = self.pane_layout(window, tab);
                        let latest = self
                            .w
                            .layers
                            .panes
                            .get(&tab)
                            .and_then(|p| p.latest.as_ref());
                        let Some(frame) = latest else {
                            continue;
                        };
                        let (size, generation) = (frame.size(), frame.generation());
                        match self.w.presenters[slot].refresh(frame) {
                            Ok(image) => {
                                measure::count(Count::LayerSnapshot);
                                let clip = frame_clip(size, &layout);
                                let entry = self.w.frames.entry(tab).or_insert_with(|| {
                                    PaneFrame::new(image.clone(), clip, size, generation)
                                });
                                // a new ring (the page resized on its layer): the panes again
                                drew |= entry.image != image || entry.clip != clip;
                                entry.image = image;
                                entry.clip = clip;
                                entry.generation = generation;
                                if let Ok(mut glass) = self.w.glass.try_borrow_mut() {
                                    glass.pages_written(window, None);
                                }
                            }
                            // the glass blurs the snapshot before it; the layer is unharmed
                            Err(e) => eprintln!("wubble: page layer snapshot: {e}"),
                        }
                    }
                    Effect::Redraw => {
                        measure::count(Count::LayerRedraw);
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
                    Effect::Repaint => self.repaint_page(tab),
                }
            }
            // the hole opened or closed: the shell's panes say so
            drew |= before.holed(mode) != route.holed(mode);
            if failed && route != Route::Unsupported {
                next = Some(Event::Failed);
            }
        }
        // the pane may have moved without a frame (no resize holds it): a layer still under its
        // hole follows at the render's start, in that render's transaction (a move in a render
        // presented alone left the hole above), and the renders after it present with the
        // transaction while the pane keeps moving
        if start {
            let latest = self
                .w
                .layers
                .panes
                .get(&tab)
                .and_then(|p| p.latest.as_ref())
                .map(|f| f.size());
            if let Some(size) = latest.filter(|_| self.w.layers.holed(tab)) {
                let layout = self.pane_layout(window, tab);
                self.place_layer(window, tab, size, &layout);
            }
            if let Some(pane) = self.w.layers.panes.get_mut(&tab) {
                pane.moved = moved;
            }
        }
        if !presenting {
            self.w.layers.transact();
        }
        // with `WUBBLE_TRACE`, a second's summary, from this frame path (never a timer of its
        // own: nothing happening, nothing printed): its counts (`measure`), the frames welding
        // dropped for a busy pool, each pane's route, the window's focus and where the pointer is
        let second = crate::tracing().and_then(|_| measure::layer_second(Instant::now()));
        if let Some((took, counts)) = second {
            let routes: Vec<(TabId, Route)> = std::iter::once(&self.w)
                .chain(self.parked.values())
                .flat_map(|w| w.layers.panes.iter().map(|(&t, p)| (t, p.route)))
                .collect();
            let busy: u64 = routes
                .iter()
                .map(|&(t, _)| self.tab_view(t).map_or(0, |v| v.take_os_buffers_busy()))
                .sum();
            let counted: Vec<String> = Count::ALL
                .into_iter()
                .filter(|&c| counts.counted(c) > 0)
                .map(|c| format!("{} {}", c.name(), counts.counted(c)))
                .collect();
            let routes: Vec<String> = routes
                .iter()
                .map(|(t, r)| format!("tab {} {}", t.0, r.name()))
                .collect();
            let focused = self.input.window_focused && self.model.focused() == self.w.id;
            let pointer = match self.input.pointer {
                None => "outside the window",
                Some(_) if self.w.cursor.over_page => "over a page",
                Some(_) => "over the shell",
            };
            crate::trace(&format!(
                "page layer: {:.2}s: {}; pool_busy {busy}; {}; window {}, pointer {pointer}",
                took.as_secs_f64(),
                counted.join(", "),
                routes.join(", "),
                if focused { "focused" } else { "unfocused" },
            ));
        }
        error.map_or(Ok(drew), Err)
    }

    /// `tab`'s layer where its pane is (the rect its `SplitPane` is drawn at: the content's
    /// origin plus the pane's x, the content's top and height), showing the part of a frame of
    /// `size` that a pane laid out as `layout` shows, with the pane's corners (`pane_radius`: the
    /// generated `Radius.md` the kit's `SplitView` draws with; none in page fullscreen). Nothing
    /// for a tab without a pane.
    fn place_layer(
        &mut self,
        window: &AppWindow,
        tab: TabId,
        size: FrameSize,
        layout: &Layout,
    ) -> bool {
        let clip = frame_clip(size, layout);
        let rect = self.pane_rect(window, tab);
        let scale = f64::from(window.window().scale_factor());
        let radius = f64::from(pane_radius(window));
        let (Some(pane_rect), Some(pane)) = (rect, self.w.layers.panes.get_mut(&tab)) else {
            return false;
        };
        let (root_h, flipped) = pane.layer.root();
        let buffer = (size.width, size.height);
        let shown = (clip.width, clip.height);
        let p = page_layer::place(pane_rect, root_h, flipped, scale, buffer, shown);
        let (f, c) = (p.frame, p.contents);
        pane.layer
            .place([f.x, f.y, f.w, f.h], [c.x, c.y, c.w, c.h], p.scale, radius)
    }

    /// Where `tab`'s pane is in the window (points, origin top left): the rect its `SplitPane`
    /// is drawn at, the content's origin plus the pane's x, the content's top and height. None
    /// for a tab without a pane.
    fn pane_rect(&self, window: &AppWindow, tab: TabId) -> Option<page_layer::Rect> {
        let (_, height) = content_size(window);
        let (cx, cy) = (window.get_content_x(), window.get_content_y());
        let r = self.pane_rects(window).into_iter().find(|r| r.tab == tab)?;
        Some(page_layer::Rect {
            x: f64::from(cx) + r.x,
            y: f64::from(cy),
            w: r.width,
            h: height,
        })
    }
}

/// A layer over the shell's in `window`'s view, or under it (`below`).
#[cfg(target_os = "macos")]
fn make_layer(window: &AppWindow, below: bool) -> Result<Box<dyn PageLayer>, String> {
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
    // drops its panes' layers before the window goes (`Host::close_window`); a retired one may
    // outlive the view, and touches only the layers it retains, never the view.
    let layer =
        unsafe { wubble_gfx::layer::CaLayer::new(view, below) }.map_err(|e| e.to_string())?;
    Ok(Box::new(layer))
}

/// No page layer here: the shell draws every frame.
#[cfg(not(target_os = "macos"))]
fn make_layer(_window: &AppWindow, _below: bool) -> Result<Box<dyn PageLayer>, String> {
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
            Effect::Snapshot => "Effect::Snapshot =>",
            Effect::Repaint => "Effect::Repaint =>",
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
            Effect::Snapshot,
            Effect::Repaint,
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
