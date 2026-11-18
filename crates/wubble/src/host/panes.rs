//! The pages on screen: one view per pane, each with its own frame stream, size and ring of
//! textures; the split's strings stepped once per rendered frame (`crate::panes`); the shell's
//! `panes`/`seams` models rewritten whole from them, like the sidebar from the model.

use std::time::Instant;

use slint::ComponentHandle;
use wubble_core::bow::{BOWED, EdgeTable};
use wubble_core::devtools::PANEL_TAB;
use wubble_core::model::Model;
use wubble_core::motion::DT_CAP;
use wubble_core::space::MAX_PANES;
use wubble_core::split::{Bow, SPLIT, string_line};
use wubble_core::stretch::{PaneEffect, PaneEvent, PaneSprings, PaneView, Px, transition};
use wubble_core::tab::TabId;
use wubble_engine::view::{Layout, ScreenArea};
use wubble_gfx::mask::{MaskJob, Prev};
use wubble_gfx::pacing::PaceEvent;
use wubble_gfx::{Edge, FrameSize};
use wubble_ui::{AppWindow, Palette, SplitPane, SplitSeam};

use super::{Host, PaneFrame, Snapshot};
use crate::measure;
use crate::menu::{self, MenuEvent};
use crate::panes::{PageRect, RADIUS, page_width};
use crate::screen::Screen;
use crate::{physical, pixels, slint_int, with_host};

/// The content area's size, logical pixels.
fn content_size(window: &AppWindow) -> (f64, f64) {
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

/// Writes `rows` into the shell's model in place when it is one this host made (rows kept, only
/// changed ones set), else hands `set` a new one.
fn sync_rows<T: Clone + PartialEq + 'static>(
    model: slint::ModelRc<T>,
    rows: Vec<T>,
    set: impl FnOnce(slint::ModelRc<T>),
) {
    match slint::Model::as_any(&model).downcast_ref::<slint::VecModel<T>>() {
        Some(model) => wubble_ui::sync(model, rows),
        None => set(super::surfaces::model(rows)),
    }
}

/// The tabs whose last frame is kept: those the model shows, and a fullscreen page. A split's
/// other panes stay shown to CEF while one of them is fullscreen, so CEF will not repaint them
/// when it ends: their frames must outlive it.
fn framed_tabs(model: &Model, screen: &Screen) -> Vec<TabId> {
    let mut tabs = model.visible_tabs();
    if let Screen::Page { tab, .. } = *screen
        && !tabs.contains(&tab)
    {
        tabs.push(tab);
    }
    tabs
}

impl Host {
    pub(super) fn framed_tabs(&self) -> Vec<TabId> {
        let mut tabs = framed_tabs(&self.model, &self.screen);
        tabs.extend((self.devtools.width() > 0.0).then_some(PANEL_TAB));
        tabs
    }

    /// The tabs on screen, left to right: a fullscreen page alone, else the model's.
    pub(super) fn shown_tabs(&self) -> Vec<TabId> {
        match self.screen {
            Screen::Page { tab, .. } => vec![tab],
            _ => {
                let mut tabs = self.model.visible_tabs();
                // the docked panel is a pane too
                tabs.extend((self.devtools.width() > 0.0).then_some(PANEL_TAB));
                tabs
            }
        }
    }

    /// Each on whole physical pixels, as the content area is (app.slint).
    pub(super) fn pane_rects(&self, window: &AppWindow) -> Vec<PageRect> {
        let (width, _) = content_size(window);
        let scale = window.window().scale_factor();
        let rects = match self.screen {
            Screen::Page { tab, .. } => vec![PageRect { tab, x: 0.0, width }],
            _ => self
                .panes
                .rects(&self.model.visible_tabs(), width, self.devtools.width()),
        };
        rects.into_iter().map(|r| r.on_pixels(scale)).collect()
    }

    /// The layout of `tab`'s view: its pane's, or the whole content area for a tab not on screen.
    pub(super) fn pane_layout(&self, window: &AppWindow, tab: TabId) -> Layout {
        let (width, _) = content_size(window);
        let rect = self
            .pane_rects(window)
            .into_iter()
            .find(|r| r.tab == tab)
            .unwrap_or(PageRect { tab, x: 0.0, width });
        self.layout_of(window, &rect)
    }

    /// `rect`'s view layout, with where it sits in the window: CEF treats the window as the
    /// screen, so a `<select>` list can open outside the page.
    fn layout_of(&self, window: &AppWindow, rect: &PageRect) -> Layout {
        let (_, height) = content_size(window);
        let scale = window.window().scale_factor();
        let (x, y) = (
            window.get_content_x() + rect.x as f32,
            window.get_content_y(),
        );
        let size = window.window().size();
        Layout {
            area: Some(ScreenArea {
                window: FrameSize::new(size.width, size.height),
                origin: (
                    (f64::from(x) * f64::from(scale)).round() as i32,
                    (f64::from(y) * f64::from(scale)).round() as i32,
                ),
            }),
            ..rect.layout(height, scale)
        }
    }

    /// The focused page's top left in the window, logical pixels: its menu, popup and IME
    /// candidates are placed from there.
    pub(super) fn focus_origin(&self, window: &AppWindow) -> (f32, f32) {
        let x = self
            .model
            .visible_tab()
            .and_then(|t| self.pane_rects(window).into_iter().find(|r| r.tab == t))
            .map_or(0.0, |r| r.x as f32);
        (window.get_content_x() + x, window.get_content_y())
    }

    /// One frame of `dt` seconds of the split's strings. What they settled is dispatched later,
    /// outside rendering. Returns whether they move.
    fn step_strings(&mut self, window: &AppWindow, dt: f64) -> bool {
        let (width, height) = content_size(window);
        let width = page_width(width, self.devtools.width());
        let (moving, commands) = self.panes.frame(&self.model, dt, width, height);
        if !commands.is_empty() {
            let first = self.split_commands.is_empty();
            self.split_commands.extend(commands);
            if first {
                let _ = slint::invoke_from_event_loop(|| with_host(Host::apply_split_commands));
            }
        }
        moving
    }

    /// One frame of the DevTools panel's string; whether it moves.
    fn step_source(&mut self, window: &AppWindow, dt: f64) -> bool {
        if !self.devtools.seam.open && !self.devtools.seam.held {
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
        let dt = self.panes_last.replace(now).map_or(0.0, |prev| {
            now.duration_since(prev).as_secs_f64().min(DT_CAP)
        });
        let strings = self.model.shown_split().is_some() && self.step_strings(window, dt);
        let source = self.step_source(window, dt);
        self.lay_out_views(window);
        let stretching = self.step_stretch(window, dt);
        let bowed = self.mask_panes(window);
        self.render_panes(window);
        self.measure_shell(window, bowed);
        let moving = strings || stretching || source;
        if !moving {
            self.panes_last = None;
        }
        moving
    }

    fn apply_split_commands(&mut self) {
        for command in std::mem::take(&mut self.split_commands) {
            self.dispatch(command);
        }
        // a dispatch may have settled more, already queued again
        if self.split_commands.is_empty() {
            self.panes.dispatched();
        }
    }

    /// After a model change: the strings adopt it at once (no time passes) and animate from
    /// the next rendered frame.
    pub(super) fn settle_panes(&mut self, window: &AppWindow) {
        self.step_strings(window, 0.0);
        self.layout_panes(window);
        if self.model.shown_split().is_some() {
            window.window().request_redraw();
        }
    }

    /// Each view on screen gets its pane's size (unchanged layouts cost nothing; a new one only
    /// asks CEF to resize, the stretched last frame shows meanwhile), then the shell's panes.
    pub(super) fn layout_panes(&mut self, window: &AppWindow) {
        self.lay_out_views(window);
        self.render_panes(window);
        // a pane that started stretching needs the frame driver
        if self.frames.values().any(|f| !f.view.is_live()) {
            window.window().request_redraw();
        }
    }

    /// Each view on screen gets its pane's layout; its pane follows the size the view now wants
    /// (the engine's `ViewState` owns it).
    fn lay_out_views(&mut self, window: &AppWindow) {
        for rect in self.pane_rects(window) {
            let layout = self.layout_of(window, &rect);
            let Some(view) = self.tab_view(rect.tab) else {
                continue;
            };
            view.set_layout(layout);
            let wanted = view.wanted_frame();
            if let (Some(frame), Some(wanted)) = (self.frames.get_mut(&rect.tab), wanted) {
                frame.view = transition(frame.view, px(wanted), PaneEvent::Layout).0;
            }
        }
    }

    /// One frame of `dt` seconds of each pane's stretch and fade; a finished fade lets the stale
    /// frame go, a settled pane goes live. A pane off screen forgets its last size, so a resize
    /// it missed is no speed when it comes back. Returns whether any still moves.
    fn step_stretch(&mut self, window: &AppWindow, dt: f64) -> bool {
        let (_, height) = content_size(window);
        let rects = self.pane_rects(window);
        for (tab, frame) in &mut self.frames {
            if !rects.iter().any(|r| r.tab == *tab) {
                frame.springs.forget();
            }
        }
        let mut moving = false;
        for rect in rects {
            let Some(frame) = self.frames.get_mut(&rect.tab) else {
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
    fn mask_panes(&mut self, window: &AppWindow) -> bool {
        let (_, height) = content_size(window);
        let scale = window.window().scale_factor();
        let fullscreen = matches!(self.screen, Screen::Page { .. });
        // the sidebar's string, in the panes' heights (they start at the content's top)
        let seam = (!fullscreen)
            .then(|| seam_edge(window))
            .filter(|t| !t.is_straight())
            .map(|t| t.shifted(-f64::from(window.get_content_y())));
        let web = window.global::<Palette>().get_web();
        let fill = [web.red(), web.green(), web.blue()].map(|c| f32::from(c) / 255.0);
        let mut any = false;
        for (i, rect) in self.pane_rects(window).into_iter().enumerate() {
            // a fullscreen page is square and alone: nothing bends it
            let (split_left, right) = if fullscreen {
                (None, None)
            } else {
                self.panes.edges(rect.tab, height)
            };
            let left = if i == 0 { seam.clone() } else { split_left };
            let slot = self.slots.iter().position(|s| *s == Some(rect.tab));
            let Some(frame) = self.frames.get_mut(&rect.tab) else {
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
            match self.presenters[slot].mask(&job) {
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
    fn measure_shell(&self, window: &AppWindow, bowed: bool) {
        let size = window.window().size();
        let scene_off = self
            .glass
            .try_borrow()
            .ok()
            .and_then(|g| g.scene_size())
            .is_some_and(|s| s != (size.width, size.height));
        let (width, _) = content_size(window);
        let rects = self.pane_rects(window);
        let pane_off = rects
            .last()
            .is_some_and(|r| (r.x + r.width - width).abs() > 0.5);
        // stretched past filling the pane, not only stale
        let stretched = rects.iter().any(|r| {
            self.frames.get(&r.tab).is_some_and(|f| {
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

    /// Writes the panes, seams and ring, row by row where they changed (a row's own state, a
    /// seam's grab mid-drag, survives).
    fn render_panes(&mut self, window: &AppWindow) {
        let (_, height) = content_size(window);
        let focused = self.model.visible_tab();
        let rects = self.pane_rects(window);
        let fill = rects.len() == 1;
        let split = self.panes.seam_count() > 0;
        let panel_x = rects.iter().find(|r| r.tab == PANEL_TAB).map(|r| r.x);
        let panes: Vec<SplitPane> = rects
            .into_iter()
            .map(|r| {
                let frame = self.frames.get(&r.tab);
                let (sx, sy) = frame.map_or((1.0, 1.0), |f| f.springs.scale(f.view));
                let prev = frame.and_then(PaneFrame::fading);
                let masked = frame.and_then(|f| f.masked.clone());
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
                        self.devtools.focused
                    } else {
                        Some(r.tab) == focused
                    },
                    // the panes of a split and the docked panel close; a lone page does not
                    closable: split || r.tab == PANEL_TAB,
                    edge: masked.as_ref().map(|m| m.image.clone()).unwrap_or_default(),
                    margin: masked.as_ref().map_or(0.0, |m| m.margin),
                    prev_opacity: prev.map_or(0.0, |p| p.2),
                    prev_clip_width: prev.map_or(0, |p| slint_int(p.1.width)),
                    prev_clip_height: prev.map_or(0, |p| slint_int(p.1.height)),
                    prev: prev.map(|p| p.0.clone()).unwrap_or_default(),
                }
            })
            .collect();
        let fullscreen = matches!(self.screen, Screen::Page { .. });
        let seams: Vec<SplitSeam> = if fullscreen {
            vec![]
        } else {
            let source = &self.devtools.seam;
            // the panel's string, after the split's (`Host::split_grab`)
            let panel = panel_x.map(|x| {
                let held = source.held || source.bend.x.abs() > BOWED;
                let bow = Bow {
                    bend: source.bend.x,
                    gy: source.gy - f64::from(window.get_content_y()),
                };
                let x = x - SPLIT::gap / 2.0;
                (
                    x,
                    if held {
                        string_line(x, height, RADIUS, bow)
                    } else {
                        String::new()
                    },
                )
            });
            self.panes
                .seams(height)
                .into_iter()
                .chain(panel)
                .map(|(x, path)| SplitSeam {
                    x: x as f32,
                    path: path.into(),
                })
                .collect()
        };
        let ring = if fullscreen {
            String::new()
        } else {
            self.panes.ring(focused, height)
        };
        sync_rows(window.get_panes(), panes, |m| window.set_panes(m));
        sync_rows(window.get_seams(), seams, |m| window.set_seams(m));
        window.set_focus_ring(ring.into());
    }

    /// The presenter slot of `tab`: its own while its frame is kept, else one no kept frame
    /// holds.
    fn slot_for(&mut self, tab: TabId) -> usize {
        // the panel's presenter is the one after the pages'
        if tab == PANEL_TAB {
            self.slots[MAX_PANES] = Some(tab);
            return MAX_PANES;
        }
        if let Some(i) = self.slots.iter().position(|s| *s == Some(tab)) {
            return i;
        }
        let shown = self.framed_tabs();
        let i = self.slots[..MAX_PANES]
            .iter()
            .position(|s| s.is_none_or(|t| !shown.contains(&t)))
            .unwrap_or(MAX_PANES - 1);
        self.slots[i] = Some(tab);
        i
    }

    /// Takes each pane's new frame, if it has one, and the focused page's popup frame. Returns
    /// whether any pane got a frame.
    pub(super) fn present_panes(&mut self, window: &AppWindow) -> Result<bool, String> {
        let focused = self.model.visible_tab();
        let mut presented = false;
        for rect in self.pane_rects(window) {
            let layout = self.layout_of(window, &rect);
            let Some(view) = self.tab_view(rect.tab) else {
                continue;
            };
            view.set_layout(layout);
            let wanted = view.wanted_frame().unwrap_or(layout.frame_size());
            let frame = view.take_frame();
            // The popup's placement comes from its events (`render_surfaces`).
            let popup = (Some(rect.tab) == focused)
                .then(|| view.take_popup_frame())
                .flatten();
            if let Some(popup) = popup {
                let image = self
                    .popup_presenter
                    .present(popup.frame)
                    .map_err(|e| e.to_string())?;
                window.set_popup_frame(image);
            }
            let Some(frame) = frame else {
                continue;
            };
            self.pace(PaceEvent::FrameArrived);
            let size = frame.size();
            // A frame painted for this layout shows its layout's part (CEF can paint a pixel
            // more); a stale one, painted for another, is shown whole, scaled to the pane.
            let clip = if size == layout.frame_size() {
                size.cropped_to(layout.size)
            } else {
                size
            };
            // `--measure` follows the focused page.
            let measured = (Some(rect.tab) == focused)
                .then(|| measure::Frame::of(&frame, layout.frame_size()));
            let slot = self.slot_for(rect.tab);
            let image = self.presenters[slot]
                .present(frame)
                .map_err(|e| e.to_string())?;
            if let Some(measured) = measured {
                let presenter = &self.presenters[slot];
                measure::taken(measured, || {
                    presenter.probe(measure::PROBE.0, measure::PROBE.1)
                });
            }
            let wanted = px(wanted);
            self.frame_arrived(rect.tab);
            let entry = self.frames.entry(rect.tab).or_insert_with(|| PaneFrame {
                image: image.clone(),
                clip,
                view: PaneView::Live { size: px(size) },
                springs: PaneSprings::default(),
                prev: None,
                masked: None,
            });
            entry.view = transition(entry.view, wanted, PaneEvent::Layout).0;
            let (view, effect) = transition(entry.view, wanted, PaneEvent::Frame(px(size)));
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
            presented = true;
        }
        if presented {
            self.render_panes(window);
            // an arrival's wobble and fade, or a stale frame's stretch, need the frame driver
            if self.frames.values().any(|f| !f.view.is_live()) {
                window.window().request_redraw();
            }
        }
        Ok(presented)
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
                pixels::check_swatches(shot, region)
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
        self.frames.get(&tab).map(|f| {
            let size = f.view.shown();
            FrameSize::new(size.w, size.h)
        })
    }

    // ----- the seams' gestures and the panes' close buttons, from the shell

    fn kick(&self) {
        if let Some(window) = self.window.upgrade() {
            window.window().request_redraw();
        }
    }

    /// The seam the gesture holds: the split's, or the DevTools panel's after them.
    pub fn split_grab(&mut self, seam: i32, x: f32, y: f32) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        if let Ok(seam) = usize::try_from(seam) {
            if seam >= self.panes.seam_count() && self.devtools.width() > 0.0 {
                self.source_grab(&window, x, y);
            } else {
                self.panes.grab(seam, x.into(), y.into());
            }
            self.kick();
        }
    }

    pub fn split_drag(&mut self, x: f32, y: f32) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        if self.devtools.seam.held {
            self.source_drag(&window, x, y);
        } else {
            self.panes.drag(x.into(), y.into());
        }
        self.kick();
    }

    pub fn split_release(&mut self) {
        if self.devtools.seam.held {
            self.source_release();
        } else {
            self.panes.release();
        }
        self.kick();
    }

    pub fn split_even(&mut self) {
        self.panes.even();
        self.kick();
    }

    pub fn pane_close(&mut self, pane: i32) {
        if let Some(tab) = crate::shell::tab_id(pane) {
            if tab == PANEL_TAB {
                self.panel_closed();
                return;
            }
            self.panes.close(tab);
            self.kick();
        }
    }

    // ----- entry points

    /// The command bar opens to choose the page beside the focused one.
    pub(super) fn open_beside(&self) {
        let _ = self
            .window
            .upgrade_in_event_loop(|window| window.invoke_request_beside());
    }

    pub fn bar_for_beside(&mut self, window: &AppWindow) {
        self.open_bar(window, crate::bar::BarMode::Beside, String::new());
    }

    /// A tab row's right click at `x, y` (window, logical): its split entries, as the shell's
    /// menu on the focused page.
    pub fn tab_context(&mut self, id: i32, x: f32, y: f32) {
        let (Some(tab), Some(focused)) = (crate::shell::tab_id(id), self.model.visible_tab())
        else {
            return;
        };
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let scale = window.window().scale_factor();
        let at = ((x * scale).round() as i32, (y * scale).round() as i32);
        let entries = menu::tab_entries(&self.model, tab);
        self.apply_menu(focused, MenuEvent::ShellRequested { at, entries });
        self.render_surfaces(&window);
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
        assert_eq!(framed_tabs(&m, &page), m.visible_tabs());
        assert_eq!(m.visible_tabs().len(), 2);
    }
}
