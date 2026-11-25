//! The pages on screen: one view per pane, each with its own frame stream, size and ring of
//! textures; the split's strings stepped once per rendered frame (`crate::panes`); the shell's
//! `panes`/`seams` models rewritten whole from them, like the sidebar from the model.

use std::time::Instant;

use slint::ComponentHandle;
use wubble_core::bow::{BOWED, EdgeTable};
use wubble_core::devtools::PANEL_TAB;
use wubble_core::extensions::shell::POPUP_TAB;
use wubble_core::model::Model;
use wubble_core::motion::DT_CAP;
use wubble_core::split::{Bow, SPLIT, string_line};
use wubble_core::stretch::{
    PaneEffect, PaneEvent, PaneSprings, PaneView, Px, after_frame, transition,
};
use wubble_core::tab::TabId;
use wubble_core::window::WindowId;
use wubble_engine::input::Point;
use wubble_engine::view::{Layout, ScreenArea};
use wubble_gfx::mask::{MaskJob, Prev};
use wubble_gfx::pacing::PaceEvent;
use wubble_gfx::scene::{PagePane, PageRect as ScenePage, Rect};
use wubble_gfx::{Edge, FrameSize, NativeFrame};
use wubble_ui::{AppWindow, GlanceView, Palette, Radius, SplitPane, SplitSeam};

/// The peek's card covers this much of the content area each way.
const GLANCE_FRACTION: f64 = 0.86;
/// The card's header strip (title and buttons) above the peek's page, logical px.
const GLANCE_HEADER: f64 = 44.0;

/// Where the window's peek shows its page, content-area logical px (`Host::glance_rect`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct GlanceRect {
    pub tab: TabId,
    pub x: f64,
    /// The card's top (its header's).
    pub card_y: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

use super::window::{PANEL_SLOT, PEEK_SLOT, pick_slot};
use super::{Host, PaneFrame, Snapshot};
use crate::clipboard::Bridge as _;
use crate::measure;
use crate::menu;
use crate::panes::{PageRect, RADIUS, SeamView, page_width};
use crate::screen::Screen;
use crate::{physical, pixels, slint_int, with_host};

/// The part of a frame of `size` a pane laid out as `layout` shows: a frame painted for this
/// layout shows its layout's part (CEF can paint a pixel more); a stale one, painted for
/// another, is shown whole, scaled to the pane.
pub(super) fn frame_clip(size: FrameSize, layout: &Layout) -> FrameSize {
    if size == layout.frame_size() {
        size.cropped_to(layout.size)
    } else {
        size
    }
}

/// A pane's corner radius: none while its page is fullscreen.
pub(super) fn pane_radius(window: &AppWindow) -> f32 {
    if window.get_page_fullscreen() {
        0.0
    } else {
        window.global::<Radius>().get_md()
    }
}

/// Pane `r`'s card in the window, logical px: the content's origin plus its x, the content's top
/// and height (the scene's pages; the page layer's glass over a pane).
pub(super) fn pane_card(window: &AppWindow, r: &PageRect) -> Rect {
    let (_, height) = content_size(window);
    let (cx, cy) = (window.get_content_x(), window.get_content_y());
    Rect::new(cx + r.x as f32, cy, r.width as f32, height as f32)
}

/// The content area's size, logical pixels.
pub(super) fn content_size(window: &AppWindow) -> (f64, f64) {
    (
        f64::from(window.get_content_width()),
        f64::from(window.get_content_height()),
    )
}

pub(super) fn px(size: FrameSize) -> Px {
    Px::new(size.width, size.height)
}

/// The sidebar seam's edge table, as the driver published it this frame (window heights).
fn seam_edge(window: &AppWindow) -> EdgeTable {
    use slint::Model as _;
    EdgeTable {
        y0: 0.0,
        step: window.get_seam_edge_step().into(),
        offset: window.get_seam_edge().iter().collect(),
    }
}

// a table the core samples fits the shaders' edge uniform whole
const _: () = assert!(wubble_core::bow::MAX_SAMPLES <= wubble_gfx::EDGE_SAMPLES);

fn edge(table: &EdgeTable) -> Edge<'_> {
    Edge {
        y0: table.y0 as f32,
        step: table.step as f32,
        offset: &table.offset,
    }
}

/// Whether two rows show the same: their `images` (wgpu textures) compared by texture, as Slint
/// 1.19 never finds two such images equal ([`wubble_ui::same_image`]), the rest by `==`;
/// rewriting an unchanged row every frame asked for the next frame, so the shell rendered at the
/// display rate forever. Compared by reference to avoid cloning images and strings every frame.
fn same_pane(a: &SplitPane, b: &SplitPane) -> bool {
    a.id == b.id
        && a.x0 == b.x0
        && a.x1 == b.x1
        && a.clip_width == b.clip_width
        && a.clip_height == b.clip_height
        && a.stretch_x == b.stretch_x
        && a.stretch_y == b.stretch_y
        && a.fill == b.fill
        && a.focused == b.focused
        && a.closable == b.closable
        && a.margin == b.margin
        && a.prev_opacity == b.prev_opacity
        && a.prev_clip_width == b.prev_clip_width
        && a.prev_clip_height == b.prev_clip_height
        && a.status_shown == b.status_shown
        && a.status_host == b.status_host
        && a.status_rest == b.status_rest
        && a.hole == b.hole
        && wubble_ui::same_image(&a.frame, &b.frame)
        && wubble_ui::same_image(&a.edge, &b.edge)
        && wubble_ui::same_image(&a.prev, &b.prev)
}

/// [`same_pane`] for the peek's card.
fn same_glance(a: &GlanceView, b: &GlanceView) -> bool {
    a.open == b.open
        && a.id == b.id
        && a.title == b.title
        && a.clip_width == b.clip_width
        && a.clip_height == b.clip_height
        && a.x == b.x
        && a.card_y == b.card_y
        && a.y == b.y
        && a.width == b.width
        && a.height == b.height
        && wubble_ui::same_image(&a.frame, &b.frame)
}

/// The tabs whose last frame is kept: those the model shows, and a fullscreen page. A split's
/// other panes stay shown to CEF while one of them is fullscreen, so CEF will not repaint them
/// when it ends: their frames must outlive it.
fn framed_tabs(model: &Model, screen: &Screen, window: WindowId) -> Vec<TabId> {
    let mut tabs = model.window_tabs(window);
    if let Screen::Page { tab, .. } = *screen
        && !tabs.contains(&tab)
        && model.window_of(tab) == Some(window)
    {
        tabs.push(tab);
    }
    tabs
}

impl Host {
    pub(super) fn framed_tabs(&self) -> Vec<TabId> {
        let mut tabs = framed_tabs(&self.model, &self.w.screen, self.w.id);
        tabs.extend((self.panel_width() > 0.0).then_some(PANEL_TAB));
        tabs.extend(self.popup_open().then_some(POPUP_TAB));
        tabs
    }

    /// The tabs on screen, left to right: a fullscreen page alone, else the model's.
    pub(super) fn shown_tabs(&self) -> Vec<TabId> {
        match self.w.screen {
            Screen::Page { tab, .. } if self.model.window_of(tab) == Some(self.w.id) => vec![tab],
            _ => {
                let mut tabs = self.model.window_tabs(self.w.id);
                // the docked panel is a pane too
                tabs.extend((self.panel_width() > 0.0).then_some(PANEL_TAB));
                // an extension's popup is a pane too
                tabs.extend(self.popup_open().then_some(POPUP_TAB));
                tabs
            }
        }
    }

    /// Each on whole physical pixels, as the content area is (app.slint).
    pub(super) fn pane_rects(&self, window: &AppWindow) -> Vec<PageRect> {
        let (width, _) = content_size(window);
        let scale = window.window().scale_factor();
        let rects = match self.w.screen {
            Screen::Page { tab, .. } if self.model.window_of(tab) == Some(self.w.id) => {
                vec![PageRect { tab, x: 0.0, width }]
            }
            _ => self.w.panes.rects(
                &self.model.window_tabs(self.w.id),
                width,
                self.panel_width(),
            ),
        };
        rects.into_iter().map(|r| r.on_pixels(scale)).collect()
    }

    /// The window's peek and where its page sits in the content area (logical px): a card
    /// centred over the page ([`GLANCE_FRACTION`] of the content area), its header on top.
    pub(super) fn glance_rect(&self, window: &AppWindow) -> Option<GlanceRect> {
        let g = self.model.glance_of(self.w.id)?;
        let (width, height) = content_size(window);
        let (w, h) = (width * GLANCE_FRACTION, height * GLANCE_FRACTION);
        let card_y = (height - h) / 2.0;
        Some(GlanceRect {
            tab: g.tab,
            x: (width - w) / 2.0,
            card_y,
            y: card_y + GLANCE_HEADER,
            width: w,
            height: (h - GLANCE_HEADER).max(0.0),
        })
    }

    /// The open peek's card and the scrim that closes it: the content area (window logical px).
    pub(super) fn glance_hit(&self, window: &AppWindow) -> Option<wubble_core::tabdrag::Rect> {
        self.model.glance_of(self.w.id)?;
        let (x, y) = self.content_origin(window);
        let (w, h) = content_size(window);
        Some(wubble_core::tabdrag::Rect {
            x: f64::from(x),
            y: f64::from(y),
            w,
            h,
        })
    }

    /// The peek's view layout: its page rect, with where it sits in the window.
    fn glance_layout(&self, window: &AppWindow, r: &GlanceRect) -> Layout {
        let scale = window.window().scale_factor();
        let at = |v: f32| (f64::from(v) * f64::from(scale)).round() as i32;
        Layout {
            size: FrameSize::new(
                physical(r.width as f32, scale),
                physical(r.height as f32, scale),
            ),
            scale,
            area: Some(self.screen_area(
                window,
                (
                    at(window.get_content_x() + r.x as f32),
                    at(window.get_content_y() + r.y as f32),
                ),
            )),
        }
    }

    /// Where a view whose top left is `origin` in the window (physical px) sits: in the window,
    /// and (`Host::follow_desktop`) the window's place on its monitor.
    fn screen_area(&self, window: &AppWindow, origin: (i32, i32)) -> ScreenArea {
        let size = window.window().size();
        ScreenArea {
            window: FrameSize::new(size.width, size.height),
            origin,
            desktop: self.w.desktop,
        }
    }

    /// Every view on screen and its layout: the panes, then the peek.
    fn view_layouts(&self, window: &AppWindow) -> Vec<(TabId, Layout)> {
        let rects = self.pane_rects(window);
        self.view_layouts_with(window, &rects)
    }

    fn view_layouts_with(&self, window: &AppWindow, rects: &[PageRect]) -> Vec<(TabId, Layout)> {
        let mut out: Vec<(TabId, Layout)> = rects
            .iter()
            .map(|r| (r.tab, self.layout_of(window, r)))
            .collect();
        out.extend(
            self.glance_rect(window)
                .map(|g| (g.tab, self.glance_layout(window, &g))),
        );
        out.extend(
            self.popup_open()
                .then(|| (POPUP_TAB, self.popup_layout(window))),
        );
        out
    }

    /// The layout of `tab`'s view: its pane's, or the whole content area for a tab not on screen.
    pub(super) fn pane_layout(&self, window: &AppWindow, tab: TabId) -> Layout {
        if tab == POPUP_TAB {
            return self.popup_layout(window);
        }
        if let Some(g) = self.glance_rect(window).filter(|g| g.tab == tab) {
            return self.glance_layout(window, &g);
        }
        let (width, _) = content_size(window);
        let rect = self
            .pane_rects(window)
            .into_iter()
            .find(|r| r.tab == tab)
            .unwrap_or(PageRect { tab, x: 0.0, width });
        self.layout_of(window, &rect)
    }

    /// `rect`'s view layout, with where it sits in the window and the window on the desktop:
    /// what the page reads as its screen, and what a `<select>` list is placed against (it can
    /// open outside the page; `popup_view` keeps it in the window).
    fn layout_of(&self, window: &AppWindow, rect: &PageRect) -> Layout {
        let (_, height) = content_size(window);
        let scale = window.window().scale_factor();
        let (x, y) = (
            window.get_content_x() + rect.x as f32,
            window.get_content_y(),
        );
        Layout {
            area: Some(self.screen_area(
                window,
                (
                    (f64::from(x) * f64::from(scale)).round() as i32,
                    (f64::from(y) * f64::from(scale)).round() as i32,
                ),
            )),
            ..rect.layout(height, scale)
        }
    }

    /// The focused page's top left in the window, logical pixels: its menu, popup and IME
    /// candidates are placed from there.
    pub(super) fn focus_origin(&self, window: &AppWindow) -> (f32, f32) {
        match self.shown() {
            Some(t) => self.view_origin(window, t),
            None => (window.get_content_x(), window.get_content_y()),
        }
    }

    /// Where `tab`'s view's top left sits in the window (logical px): the peek's page in its
    /// card, else its pane.
    pub(super) fn view_origin(&self, window: &AppWindow, tab: TabId) -> (f32, f32) {
        let (x, y) = match self.glance_rect(window).filter(|g| g.tab == tab) {
            Some(g) => (g.x as f32, g.y as f32),
            None => (
                self.pane_rects(window)
                    .into_iter()
                    .find(|r| r.tab == tab)
                    .map_or(0.0, |r| r.x as f32),
                0.0,
            ),
        };
        (window.get_content_x() + x, window.get_content_y() + y)
    }

    /// The window point `at` (logical px) in `tab`'s view: where a drag or a pinch over it is.
    pub(super) fn page_point(&self, window: &AppWindow, tab: TabId, at: (f32, f32)) -> Point {
        let (x, y) = self.view_origin(window, tab);
        Point {
            x: at.0 - x,
            y: at.1 - y,
        }
    }

    /// One frame of `dt` seconds of the split's strings. What they settled is dispatched later,
    /// outside rendering. Returns whether they move.
    fn step_strings(&mut self, window: &AppWindow, dt: f64) -> bool {
        let (width, height) = content_size(window);
        let width = page_width(width, self.panel_width());
        let (moving, commands) = self
            .w
            .panes
            .frame(&self.model, self.w.id, dt, width, height);
        if !commands.is_empty() {
            let first = self.w.split_commands.is_empty();
            self.w.split_commands.extend(commands);
            if first {
                let _ = slint::invoke_from_event_loop(|| with_host(Host::apply_split_commands));
            }
        }
        moving
    }

    /// One frame of the DevTools panel's string; whether it moves.
    fn step_source(&mut self, window: &AppWindow, dt: f64) -> bool {
        if self.panel_width() == 0.0 && !self.devtools.seam.held {
            return false;
        }
        let scale = f64::from(window.window().scale_factor());
        let size = window.window().size();
        let (w, h) = (
            f64::from(size.width) / scale,
            f64::from(size.height) / scale,
        );
        self.devtools.seam.step(dt, w, h)
    }

    /// The rendering notifier's frame, before the shell is drawn: the split's strings, every
    /// view's size from the window as this frame lays it out, and each pane's stretch
    /// (`wubble_core::stretch`). Returns whether another frame is needed.
    pub fn panes_frame(&mut self, window: &AppWindow, now: Instant) -> bool {
        let dt = self.w.panes_last.replace(now).map_or(0.0, |prev| {
            now.duration_since(prev).as_secs_f64().min(DT_CAP)
        });
        let strings = self.model.window_split(self.w.id).is_some() && self.step_strings(window, dt);
        let source = self.step_source(window, dt);
        let rects = self.pane_rects(window);
        self.lay_out_views_with(window, &rects);
        let stretching = self.step_stretch(window, &rects, dt);
        let bowed = self.mask_panes(window, &rects);
        // what this render draws over a page on its layer, and whether it still may stay there
        self.layers_changed(window, false);
        self.render_panes_with(window, &rects);
        self.layers_drawn();
        self.measure_shell(window, &rects, bowed);
        let autoscrolling = self.autoscroll_frame(window, now);
        let moving = strings || stretching || autoscrolling || source;
        if !moving {
            self.w.panes_last = None;
        }
        moving
    }

    fn apply_split_commands(&mut self) {
        for command in std::mem::take(&mut self.w.split_commands) {
            self.dispatch(command);
        }
        // a dispatch may have settled more, already queued again
        if self.w.split_commands.is_empty() {
            self.w.panes.dispatched();
        }
    }

    /// After a model change: the strings adopt it at once (no time passes) and animate from
    /// the next rendered frame.
    pub(super) fn settle_panes(&mut self, window: &AppWindow) {
        let strings_moving = self.step_strings(window, 0.0);
        self.layout_panes(window);
        if strings_moving {
            window.window().request_redraw();
        }
    }

    /// Each view on screen gets its pane's size (unchanged layouts cost nothing; a new one only
    /// asks CEF to resize, the stretched last frame shows meanwhile), then the shell's panes.
    pub(super) fn layout_panes(&mut self, window: &AppWindow) {
        let rects = self.pane_rects(window);
        self.lay_out_views_with(window, &rects);
        self.render_panes_with(window, &rects);
        // a pane that started stretching needs the frame driver
        if self.w.frames.values().any(|f| !f.view.is_live()) {
            window.window().request_redraw();
        }
    }

    /// Each view on screen gets its pane's layout; its pane follows the size the view now wants
    /// (the engine's `ViewState` owns it).
    pub(super) fn lay_out_views_with(&mut self, window: &AppWindow, rects: &[PageRect]) {
        for (tab, layout) in self.view_layouts_with(window, rects) {
            let Some(view) = self.tab_view(tab) else {
                continue;
            };
            view.set_layout(layout);
            let wanted = view.wanted_frame();
            if let (Some(frame), Some(wanted)) = (self.w.frames.get_mut(&tab), wanted) {
                frame.view = transition(frame.view, px(wanted), PaneEvent::Layout).0;
            }
        }
    }

    /// One frame of `dt` seconds of each pane's stretch and fade; a finished fade lets the stale
    /// frame go, a settled pane goes live. A pane off screen forgets its last size, so a resize
    /// it missed is no speed when it comes back. Returns whether any still moves.
    fn step_stretch(&mut self, window: &AppWindow, rects: &[PageRect], dt: f64) -> bool {
        let (_, height) = content_size(window);
        for (tab, frame) in &mut self.w.frames {
            if !rects.iter().any(|r| r.tab == *tab) {
                frame.springs.forget();
            }
        }
        let mut moving = false;
        for rect in rects {
            let Some(frame) = self.w.frames.get_mut(&rect.tab) else {
                continue;
            };
            // Faded and Rest do not depend on the wanted size
            let shown = frame.view.shown();
            let still = frame.springs.step((rect.width, height), dt);
            if frame.view.is_fading() && frame.springs.fade() >= 1.0 {
                frame.view = transition(frame.view, shown, PaneEvent::Faded).0;
                frame.prev = None;
            }
            if still {
                moving = true;
            } else {
                frame.view = transition(frame.view, shown, PaneEvent::Rest).0;
            }
        }
        moving
    }

    /// While a string bends a pane's edge (the sidebar seam's along the first pane's left edge, a
    /// split seam's along both panes it divides), the pane's frame is redrawn with that edge cut
    /// along the string (`wubble_gfx::mask`); at rest nothing is masked. The content area stops
    /// clipping while any pane is masked, so a bulge shows. Returns whether any pane is masked.
    fn mask_panes(&mut self, window: &AppWindow, rects: &[PageRect]) -> bool {
        let (_, height) = content_size(window);
        let scale = window.window().scale_factor();
        let fullscreen = matches!(self.w.screen, Screen::Page { .. });
        // the sidebar's string, in the panes' heights (they start at the content's top)
        let seam = (!fullscreen)
            .then(|| seam_edge(window))
            .filter(|t| !t.is_straight())
            .map(|t| t.shifted(-f64::from(window.get_content_y())));
        let web = window.global::<Palette>().get_web();
        let fill = [web.red(), web.green(), web.blue()].map(|c| f32::from(c) / 255.0);
        let mut any = false;
        for (i, rect) in rects.iter().enumerate() {
            // a fullscreen page is square and alone: nothing bends it
            let (split_left, right) = if fullscreen {
                (None, None)
            } else {
                self.w.panes.edges(rect.tab, height)
            };
            let left = if i == 0 { seam.clone() } else { split_left };
            let slot = self.w.slots.iter().position(|s| *s == Some(rect.tab));
            let Some(frame) = self.w.frames.get_mut(&rect.tab) else {
                continue;
            };
            frame.masked = None;
            let (Some(slot), true) = (slot, left.is_some() || right.is_some()) else {
                continue;
            };
            let (sx, sy) = frame.springs.scale(frame.view);
            let job = MaskJob {
                pane: (rect.width as f32, height as f32),
                scale,
                stretch: ((sx - 1.0) as f32, (sy - 1.0) as f32),
                clip: frame.clip,
                radius: RADIUS as f32,
                margin: SPLIT::margin as f32,
                left: left.as_ref().map(edge),
                right: right.as_ref().map(edge),
                fill,
                prev: frame.fading().map(|(image, clip, opacity)| Prev {
                    image,
                    clip,
                    opacity,
                }),
            };
            match self.w.presenters[slot].mask(&job) {
                Ok(masked) => frame.masked = masked,
                Err(e) => eprintln!("wubble: page edge mask: {e}"),
            }
            any |= frame.masked.is_some();
        }
        window.set_edge_bowed(any);
        any
    }

    /// `--measure resize`: whether the scene and the panes were laid out for the window size of
    /// this frame (observations; never asserted).
    fn measure_shell(&self, window: &AppWindow, rects: &[PageRect], bowed: bool) {
        let size = window.window().size();
        let scene_off = self
            .w
            .glass
            .try_borrow()
            .ok()
            .and_then(|g| g.scene_size())
            .is_some_and(|s| s != (size.width, size.height));
        let (width, _) = content_size(window);
        let pane_off = rects
            .last()
            .is_some_and(|r| (r.x + r.width - width).abs() > 0.5);
        // stretched past filling the pane, not only stale
        let stretched = rects.iter().any(|r| {
            self.w.frames.get(&r.tab).is_some_and(|f| {
                let (sx, sy) = f.springs.scale(f.view);
                sx > 1.0 || sy > 1.0
            })
        });
        measure::shell(measure::Shell {
            scene_off,
            pane_off,
            stretched,
            bowed,
        });
    }

    /// Whether `tab` shows the start page: no frame and no pane fill, the scene's living layer
    /// shows there (no `SplitPane`).
    pub(super) fn start_page(&self, tab: TabId) -> bool {
        self.model.tabs.get(&tab).is_some_and(|t| {
            wubble_core::pages::InternalPage::from_url(&t.url)
                == Some(wubble_core::pages::InternalPage::Start)
        })
    }

    /// Writes the panes, seams and ring, row by row where they changed (a row's own state, a
    /// seam's grab mid-drag, survives).
    pub(super) fn render_panes(&mut self, window: &AppWindow) {
        let rects = self.pane_rects(window);
        self.render_panes_with(window, &rects);
    }

    fn render_panes_with(&mut self, window: &AppWindow, rects: &[PageRect]) {
        self.render_glance(window);
        self.render_popup(window);
        let (_, height) = content_size(window);
        let focused = self.shown();
        let fill = rects.len() == 1;
        let panel_x = rects.iter().find(|r| r.tab == PANEL_TAB).map(|r| r.x);
        let panes: Vec<SplitPane> = rects
            .iter()
            .filter(|r| !self.start_page(r.tab))
            .map(|r| {
                let frame = self.w.frames.get(&r.tab);
                let (sx, sy) = frame.map_or((1.0, 1.0), |f| f.springs.scale(f.view));
                let prev = frame.and_then(PaneFrame::fading);
                let masked = frame.and_then(|f| f.masked.clone());
                let (status_shown, status_host, status_rest) =
                    self.status_view(r.tab, super::status::status_chars(r.width));
                SplitPane {
                    id: crate::shell::pane_id(r.tab),
                    x0: r.x as f32,
                    x1: (r.x + r.width) as f32,
                    frame: frame.map(|f| f.image.clone()).unwrap_or_default(),
                    clip_width: frame.map_or(0, |f| slint_int(f.clip.width)),
                    clip_height: frame.map_or(0, |f| slint_int(f.clip.height)),
                    stretch_x: (sx - 1.0) as f32,
                    stretch_y: (sy - 1.0) as f32,
                    fill,
                    focused: if r.tab == PANEL_TAB {
                        self.key_pane == Some(PANEL_TAB)
                    } else {
                        Some(r.tab) == focused
                    },
                    // only the docked panel shows a close button (wubble's, derived): the design's
                    // split panes have none (webview.tsx); a page leaves the split from the tab
                    // menu's Take Out of Split View, Mod+| or the command bar's unsplit action
                    closable: r.tab == PANEL_TAB,
                    edge: masked.as_ref().map(|m| m.image.clone()).unwrap_or_default(),
                    margin: masked.as_ref().map_or(0.0, |m| m.margin),
                    prev_opacity: prev.map_or(0.0, |p| p.2),
                    prev_clip_width: prev.map_or(0, |p| slint_int(p.1.width)),
                    prev_clip_height: prev.map_or(0, |p| slint_int(p.1.height)),
                    prev: prev.map(|p| p.0.clone()).unwrap_or_default(),
                    status_shown,
                    status_host: status_host.into(),
                    status_rest: status_rest.into(),
                    hole: self.w.layers.holed(r.tab),
                }
            })
            .collect();
        // a bubble of a pane that left cannot report its fade: its layer goes with the pane
        let live: Vec<TabId> = rects
            .iter()
            .filter(|r| !self.start_page(r.tab))
            .map(|r| r.tab)
            .collect();
        self.panes_gone(&live);
        let fullscreen = matches!(self.w.screen, Screen::Page { .. });
        let seams: Vec<SplitSeam> = if fullscreen {
            vec![]
        } else {
            let source = &self.devtools.seam;
            // the panel's string, after the split's (`Host::split_grab`)
            let panel = panel_x.map(|x| {
                let bow = Bow {
                    bend: source.bend.x,
                    gy: source.gy - f64::from(window.get_content_y()),
                };
                let x = x - SPLIT::gap / 2.0;
                SeamView {
                    x,
                    path: if source.held || source.bend.x.abs() > BOWED {
                        string_line(x, height, RADIUS, bow)
                    } else {
                        String::new()
                    },
                    held: source.held,
                    bend: bow.bend,
                    gy: bow.gy,
                }
            });
            self.w
                .panes
                .seams(height)
                .into_iter()
                .chain(panel)
                // the pill at the string's bend and the grab height (chrome.tsx `layoutSplit`:
                // `translate(sm.x + sm.bend, sm.gy)`)
                .map(|s| SplitSeam {
                    x: s.x as f32,
                    path: s.path.into(),
                    held: s.held,
                    bend: s.bend as f32,
                    gy: s.gy as f32,
                })
                .collect()
        };
        let ring = if fullscreen {
            String::new()
        } else {
            self.w.panes.ring(focused, height)
        };
        // a built-in page draws over its pane of a split; alone, over the whole content
        let page = rects
            .iter()
            .find(|r| !fill && Some(r.tab) == self.page_tab());
        window.set_page_x(page.map_or(0.0, |r| r.x as f32));
        window.set_page_width(page.map_or(0.0, |r| r.width as f32));
        crate::bind::sync_by(window.get_panes(), panes, same_pane, |m| {
            window.set_panes(m)
        });
        crate::bind::sync(window.get_seams(), seams, |m| window.set_seams(m));
        window.set_focus_ring(ring.into());
        self.sync_scene_pages_with(window, rects);
    }

    /// The scene's view of the panes (`Glass::set_pages`): each pane's card in the window and
    /// its latest frame, the presenter's texture itself (no copy). Overlay glass over a pane
    /// frosts it; the window's glass picks up its light. Panes without a frame are left out.
    fn sync_scene_pages_with(&self, window: &AppWindow, rects: &[PageRect]) {
        let radius = pane_radius(window);
        let pages: Vec<PagePane> = rects
            .iter()
            .filter_map(|r| {
                let frame = self.w.frames.get(&r.tab)?;
                Some(PagePane {
                    rect: ScenePage {
                        rect: pane_card(window, r),
                        radius,
                    },
                    texture: frame.image.to_wgpu_30_texture()?,
                    clip: (frame.clip.width, frame.clip.height),
                })
            })
            .collect();
        if let Ok(mut glass) = self.w.glass.try_borrow_mut() {
            glass.set_pages(window, pages);
        }
    }

    /// The peek's card (`glance-card.slint`): its page's last frame where the host lays it out.
    fn render_glance(&self, window: &AppWindow) {
        let view = match self.glance_rect(window) {
            Some(r) => {
                let frame = self.w.frames.get(&r.tab);
                let tab = self.model.tabs.get(&r.tab);
                GlanceView {
                    open: true,
                    id: crate::shell::pane_id(r.tab),
                    title: tab.map(crate::shell::tab_title).unwrap_or_default().into(),
                    frame: frame.map(|f| f.image.clone()).unwrap_or_default(),
                    clip_width: frame.map_or(0, |f| slint_int(f.clip.width)),
                    clip_height: frame.map_or(0, |f| slint_int(f.clip.height)),
                    x: r.x as f32,
                    card_y: r.card_y as f32,
                    y: r.y as f32,
                    width: r.width as f32,
                    height: r.height as f32,
                }
            }
            None => GlanceView {
                open: false,
                ..window.get_glance()
            },
        };
        if !same_glance(&window.get_glance(), &view) {
            window.set_glance(view);
        }
    }

    /// The presenter slot of `tab`: its own while its frame is kept, else one no kept frame
    /// holds.
    pub(super) fn slot_for(&mut self, tab: TabId) -> usize {
        let own = if tab == PANEL_TAB {
            Some(PANEL_SLOT)
        } else if self.model.is_glance(tab) || tab == POPUP_TAB {
            Some(PEEK_SLOT)
        } else {
            None
        };
        let framed = self.framed_tabs();
        pick_slot(&mut self.w.slots, tab, own, &framed)
    }

    /// Takes each pane's new frame, if it has one, and the focused page's popup frame. Returns
    /// whether any pane got a frame: drawn by the shell, or shown on its page layer
    /// (`Host::route_frame`), which costs no render of the shell.
    pub(super) fn present_panes(&mut self, window: &AppWindow) -> Result<bool, String> {
        let keys = self.key_page();
        let mut presented = false;
        let mut shell_drew = false;
        for (tab, layout) in self.view_layouts(window) {
            let Some(view) = self.tab_view(tab) else {
                continue;
            };
            view.set_layout(layout);
            let wanted = view.wanted_frame().unwrap_or(layout.frame_size());
            let frame = view.take_frame();
            let calm = view.content_rate() == wubble_gfx::pacing::ContentRate::Calm;
            // The popup's placement comes from its events (`render_surfaces`).
            let popup = (Some(tab) == keys)
                .then(|| view.take_popup_frame())
                .flatten();
            if let Some(popup) = popup {
                let image = self
                    .w
                    .popup_presenter
                    .present(popup.frame)
                    .map_err(|e| e.to_string())?;
                window.set_popup_frame(image);
            }
            let Some(frame) = frame else {
                continue;
            };
            measure::count(measure::Count::CefFrame);
            if calm {
                measure::count(measure::Count::CefFrameCalm);
            }
            self.pace(PaceEvent::FrameArrived);
            if let Some(refresh) = self.refresh() {
                let timer = self.engine.page_painted(refresh).timer;
                self.arm_pump(timer);
            }
            self.frame_arrived(tab);
            shell_drew |= self.route_frame(window, tab, layout, wanted, frame)?;
            presented = true;
        }
        if shell_drew {
            self.render_panes(window);
            // an arrival's wobble and fade, or a stale frame's stretch, need the frame driver
            if self.w.frames.values().any(|f| !f.view.is_live()) {
                window.window().request_redraw();
            }
        }
        Ok(presented)
    }

    /// `frame` of `tab`, laid out as `layout` for a view wanting `wanted`, through the shell's
    /// presenter into the pane's entry: the page layer's `Effect::Present` (and its
    /// `PresentLatest`). The caller renders the panes.
    pub(super) fn present_frame(
        &mut self,
        window: &AppWindow,
        tab: TabId,
        layout: Layout,
        wanted: FrameSize,
        frame: NativeFrame,
    ) -> Result<(), String> {
        let focused = self.shown();
        let generation = frame.generation();
        let size = frame.size();
        let clip = frame_clip(size, &layout);
        // `--measure` follows the focused page.
        let measured =
            (Some(tab) == focused).then(|| measure::Frame::of(&frame, layout.frame_size()));
        let slot = self.slot_for(tab);
        let blit = Instant::now();
        // A pane at rest, showing this slot's last frame whole and 1:1, takes only what
        // changed, written in place: the image stays, so the changed part is marked here.
        let shown = self
            .w
            .frames
            .get(&tab)
            .filter(|f| f.view.is_live() && f.prev.is_none() && f.masked.is_none())
            .filter(|f| f.clip == clip)
            .map(|f| f.image.clone());
        let (image, damage) = match shown {
            Some(shown) => self.w.presenters[slot].present_damaged(frame, &shown),
            None => self.w.presenters[slot].present(frame).map(|i| (i, None)),
        }
        .map_err(|e| e.to_string())?;
        measure::stage(measure::Metric::Blit, blit);
        if let Some(damage) = damage {
            measure::count(measure::Count::CefFrameDamaged);
            self.mark_damage(window, tab, clip, damage);
        }
        if let Some(measured) = measured {
            let presenter = &self.w.presenters[slot];
            measure::taken(measured, || {
                presenter.probe(measure::PROBE.0, measure::PROBE.1)
            });
        }
        let wanted = px(wanted);
        let entry = self
            .w
            .frames
            .entry(tab)
            .or_insert_with(|| PaneFrame::new(image.clone(), clip, size, generation));
        let (view, effect) = after_frame(entry.view, wanted, px(size));
        entry.view = view;
        // the frame at the new size arrived: one damped wobble, and the stale frame (still
        // intact in the other ring slot or the old ring) fades out over it
        entry.prev = match effect {
            Some(PaneEffect::Arrived { from }) => {
                entry.springs.arrive(from, wanted);
                Some((
                    std::mem::replace(&mut entry.image, image.clone()),
                    entry.clip,
                ))
            }
            None => None,
        };
        entry.image = image;
        entry.clip = clip;
        entry.generation = generation;
        Ok(())
    }

    /// The part of `tab`'s pane that `damage` (in its frame's pixels, of which the pane shows
    /// `clip`) covers is redrawn in the next frame; nothing when it is empty.
    pub(super) fn mark_damage(
        &self,
        window: &AppWindow,
        tab: TabId,
        clip: FrameSize,
        damage: wubble_gfx::import::Damage,
    ) {
        if damage.width == 0 || damage.height == 0 || clip.width == 0 || clip.height == 0 {
            return;
        }
        let Some(rect) = self.pane_rects(window).into_iter().find(|r| r.tab == tab) else {
            return;
        };
        let (_, height) = content_size(window);
        let (sx, sy) = (
            rect.width / f64::from(clip.width),
            height / f64::from(clip.height),
        );
        // A pane shown 1:1 (one frame pixel to one physical pixel, on whole pixels: app.slint
        // `content`) draws a changed frame pixel into that physical pixel only. Scaled (a stale
        // frame stretched to the pane), bilinear sampling spreads each frame pixel over its
        // neighbours' footprint: one frame pixel more on each side. The renderer rounds the
        // region out to whole physical pixels.
        let scale = f64::from(window.window().scale_factor());
        let one_to_one = (rect.width * scale - f64::from(clip.width)).abs() < 0.5
            && (height * scale - f64::from(clip.height)).abs() < 0.5;
        let (mx, my) = if one_to_one { (0.0, 0.0) } else { (sx, sy) };
        let x = f64::from(window.get_content_x()) + rect.x + f64::from(damage.x) * sx - mx;
        let y = f64::from(window.get_content_y()) + f64::from(damage.y) * sy - my;
        let (w, h) = (
            f64::from(damage.width) * sx + 2.0 * mx,
            f64::from(damage.height) * sy + 2.0 * my,
        );
        crate::partial::mark_rect(window.window(), x as f32, y as f32, w as f32, h as f32);
        window.window().request_redraw();
        // the page blur (overlay glass, ambience) samples the same texture
        if let Ok(mut glass) = self.w.glass.try_borrow_mut() {
            glass.pages_written(
                window,
                Some(Rect::new(x as f32, y as f32, w as f32, h as f32)),
            );
        }
    }

    /// The window's pixels with every pane's page drawn by the shell: a page on its layer hands
    /// its latest frame over first, as the shell's own render has a hole where a layer shows.
    pub(super) fn pages_snapshot(&mut self, window: &AppWindow) -> Result<Snapshot, String> {
        let tabs: Vec<TabId> = self.pane_rects(window).iter().map(|r| r.tab).collect();
        tabs.into_iter().for_each(|tab| self.layer_pixels(tab));
        super::snapshot(window)
    }

    /// `--smoke frames` and `--smoke split`: each pane of the snapshot shows its page.
    pub(super) fn check_pixels(
        &self,
        window: &AppWindow,
        shot: &Snapshot,
    ) -> Result<pixels::Sample, String> {
        let scale = window.window().scale_factor();
        let (_, height) = content_size(window);
        let mut last = None;
        for r in self.pane_rects(window) {
            let region = pixels::Region {
                x: physical(window.get_content_x() + r.x as f32, scale),
                y: physical(window.get_content_y(), scale),
                width: physical(r.width as f32, scale),
                height: physical(height as f32, scale),
            };
            // The first (or only) pane shows the bundled page: its swatches' exact colours.
            if last.is_none() {
                pixels::check_swatches(shot, region, scale)
                    .map_err(|e| format!("pane of tab {}: {e}", r.tab.0))?;
            }
            let sample =
                pixels::check(shot, region).map_err(|e| format!("pane of tab {}: {e}", r.tab.0))?;
            last = Some(sample);
        }
        last.ok_or_else(|| "no page on screen".into())
    }

    /// The last frame's size of `tab`, if it is on screen and has painted.
    pub(super) fn frame_size(&self, tab: TabId) -> Option<FrameSize> {
        self.w.frames.get(&tab).map(|f| {
            let size = f.view.shown();
            FrameSize::new(size.w, size.h)
        })
    }

    // ----- the seams' gestures and the panes' close buttons, from the shell

    fn kick(&self) {
        if let Some(window) = self.w.window.upgrade() {
            window.window().request_redraw();
        }
    }

    /// The seam the gesture holds: the split's, or the DevTools panel's after them.
    pub fn split_grab(&mut self, seam: i32, x: f32, y: f32) {
        let Some(window) = self.w.window.upgrade() else {
            return;
        };
        if let Ok(seam) = usize::try_from(seam) {
            if seam >= self.w.panes.seam_count() && self.panel_width() > 0.0 {
                self.source_grab(&window, x, y);
            } else {
                self.w.panes.grab(seam, x.into(), y.into());
            }
            self.kick();
        }
    }

    pub fn split_drag(&mut self, x: f32, y: f32) {
        let Some(window) = self.w.window.upgrade() else {
            return;
        };
        if self.devtools.seam.held {
            self.source_drag(&window, x, y);
        } else {
            self.w.panes.drag(x.into(), y.into());
        }
        self.kick();
    }

    pub fn split_release(&mut self) {
        if self.devtools.seam.held {
            self.source_release();
        } else {
            self.w.panes.release();
        }
        self.kick();
    }

    pub fn split_even(&mut self) {
        self.w.panes.even();
        self.kick();
    }

    pub fn pane_close(&mut self, pane: i32) {
        if let Some(tab) = crate::shell::tab_id(pane) {
            if tab == PANEL_TAB {
                self.panel_closed();
                return;
            }
            self.w.panes.close(tab);
            self.kick();
        }
    }

    // ----- entry points

    /// The command bar opens to choose the page beside the focused one.
    pub(super) fn open_beside(&self) {
        let _ = self
            .w
            .window
            .upgrade_in_event_loop(|window| window.invoke_request_beside());
    }

    pub fn bar_for_beside(&mut self, window: &AppWindow) {
        self.open_bar(window, crate::bar::BarMode::Beside, String::new());
    }

    /// A tab row's right click at `x, y` (window, logical): its split entries, as the shell's
    /// menu on the focused page.
    /// A folder header's right click opens the folder's menu.
    pub fn tab_context(&mut self, id: i32, x: f32, y: f32) {
        let entries = if let Some(folder) = crate::shell::folder_of_row(id) {
            menu::folder_entries(&self.model, folder)
        } else if let Some(tab) = crate::shell::tab_id(id) {
            let facts =
                wubble_core::tabmenu::Facts {
                    muted: self.audio.get(&tab).is_some_and(|a| a.muted),
                    site_muted: self.model.tabs.get(&tab).is_some_and(|t| {
                        wubble_core::sitesound::is_muted(&self.muted_sites, &t.url)
                    }),
                };
            menu::tab_entries(&self.model, tab, facts, &self.keymap)
        } else {
            return;
        };
        self.sidebar_menu(entries, x, y);
    }

    /// A right click on the sidebar's empty glass at `x, y` (window, logical): its own menu.
    pub fn sidebar_context(&mut self, x: f32, y: f32) {
        let entries = menu::sidebar_entries(&self.model, &self.keymap);
        self.sidebar_menu(entries, x, y);
    }

    /// A right click in the command bar's field or on the address pill at `x, y` (window,
    /// logical): the address menu, its paste row named for what the clipboard holds.
    pub fn address_context(&mut self, field: menu::AddressField, x: f32, y: f32) {
        let Some(window) = self.w.window.upgrade() else {
            return;
        };
        let clipboard = self
            .clipboard
            .load(window.window(), crate::clipboard::Buffer::Clipboard)
            .unwrap_or_default();
        let bar = field == menu::AddressField::Bar;
        let facts = menu::FieldFacts {
            field,
            selected: bar
                && window.get_command_selection_start() < window.get_command_selection_end(),
            text: bar && !window.get_command_query().is_empty(),
            clipboard: &clipboard,
            full_urls: self.full_urls,
        };
        let entries = menu::address_entries(facts, &self.keymap);
        self.sidebar_menu(entries, x, y);
    }

    /// A shell button's click: its action, or the one a held Shift, Ctrl or Cmd makes of it
    /// (`perform::clicked`: a hard reload).
    pub fn shell_clicked(&mut self, action: wubble_core::action::Action) {
        let keys = self.input.keys;
        let held = keys.shift_key() || keys.control_key() || keys.super_key();
        self.perform(crate::perform::clicked(action, held));
    }

    /// A right click on a shell button at `x, y` (window, logical): the reload button's menu,
    /// with the empty-cache row while DevTools inspects the page. Other buttons have none.
    pub fn shell_action_menu(&mut self, action: wubble_core::action::Action, x: f32, y: f32) {
        use wubble_core::action::Action;
        if !matches!(action, Action::Reload | Action::Stop) {
            return;
        }
        let devtools = self.shown().is_some() && self.devtools.dock.tab() == self.shown();
        let entries = menu::reload_entries(devtools, &self.keymap);
        self.sidebar_menu(entries, x, y);
    }

    /// The zoom chip's click at `x, y` (window, logical): its popover, the percent with −, +
    /// and Reset (`menu::zoom_entries`); a step reopens it there (`MenuCommand::ZoomStep`).
    pub fn zoom_popover(&mut self, x: f32, y: f32) {
        let Some(tab) = self.shown() else {
            return;
        };
        let default = self.zoom_default;
        let Some(percent) = self.tab_view(tab).map(|v| v.zoom_percent()) else {
            return;
        };
        self.zoom_at = Some((x, y));
        let entries = menu::zoom_entries(percent, default, &self.keymap);
        self.sidebar_menu(entries, x, y);
    }

    /// The shell's menu at `x, y` (window, logical), on the focused page.
    fn sidebar_menu(&mut self, entries: Vec<menu::Entry>, x: f32, y: f32) {
        self.hover_dismiss();
        let Some(focused) = self.shown() else {
            return;
        };
        let Some(window) = self.w.window.upgrade() else {
            return;
        };
        let scale = window.window().scale_factor();
        let at = ((x * scale).round() as i32, (y * scale).round() as i32);
        self.open_shell_menu(focused, at, entries, false);
    }
}

impl PaneFrame {
    /// A pane's first frame, `image` of a frame of `size` showing `clip`, shown 1:1 (the shell's
    /// first `Present`, or the page layer's first snapshot of a page that went to its layer at
    /// once).
    pub(super) fn new(
        image: slint::Image,
        clip: FrameSize,
        size: FrameSize,
        generation: u64,
    ) -> Self {
        PaneFrame {
            image,
            clip,
            view: PaneView::Live { size: px(size) },
            springs: PaneSprings::default(),
            prev: None,
            masked: None,
            generation,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wubble_core::model::{Command, apply};

    /// A split pane fullscreen keeps the other pane's frame: CEF still shows that view and will
    /// not repaint it when fullscreen ends (it went blank).
    #[test]
    fn a_fullscreen_pane_keeps_the_other_panes_frames() {
        let m = (0..2).fold(Model::new(Instant::now()), |m, i| {
            apply(
                m,
                Command::NewTab {
                    url: format!("https://{i}.test/"),
                    container: None,
                    opener: None,
                },
            )
            .0
        });
        let (m, _) = apply(m, Command::SplitWith(TabId(1)));
        let page = Screen::Page {
            tab: TabId(1),
            restore_window: false,
        };
        assert_eq!(framed_tabs(&m, &page, m.focused()), m.visible_tabs());
        assert_eq!(m.visible_tabs().len(), 2);
    }
}
