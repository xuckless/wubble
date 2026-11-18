//! The pages on screen: one view per pane, each with its own frame stream, size and ring of
//! textures; the split's strings stepped once per rendered frame (`crate::panes`); the shell's
//! `panes`/`seams` models rewritten whole from them, like the sidebar from the model.

use std::time::Instant;

use slint::ComponentHandle;
use wubble_core::model::Model;
use wubble_core::motion::DT_CAP;
use wubble_core::space::MAX_PANES;
use wubble_core::stretch::{Pane, PaneEvent, PaneSprings, PaneView, Px, transition};
use wubble_core::tab::TabId;
use wubble_engine::view::Layout;
use wubble_gfx::pacing::PaceEvent;
use wubble_ui::{AppWindow, SplitPane, SplitSeam};

use super::{Host, PaneFrame, Snapshot};
use crate::measure;
use crate::menu::{self, MenuEvent};
use crate::panes::PageRect;
use crate::screen::Screen;
use crate::{physical, pixels, slint_int, with_host};

/// The content area's size, logical pixels.
fn content_size(window: &AppWindow) -> (f64, f64) {
    (
        f64::from(window.get_content_width()),
        f64::from(window.get_content_height()),
    )
}

fn px(size: wubble_gfx::FrameSize) -> Px {
    Px::new(size.width, size.height)
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
        framed_tabs(&self.model, &self.screen)
    }

    /// The tabs on screen, left to right: a fullscreen page alone, else the model's.
    pub(super) fn shown_tabs(&self) -> Vec<TabId> {
        match self.screen {
            Screen::Page { tab, .. } => vec![tab],
            _ => self.model.visible_tabs(),
        }
    }

    /// Each on whole physical pixels, as the content area is (app.slint).
    pub(super) fn pane_rects(&self, window: &AppWindow) -> Vec<PageRect> {
        let (width, _) = content_size(window);
        let scale = window.window().scale_factor();
        let rects = match self.screen {
            Screen::Page { tab, .. } => vec![PageRect { tab, x: 0.0, width }],
            _ => self.panes.rects(&self.model.visible_tabs(), width),
        };
        rects.into_iter().map(|r| r.on_pixels(scale)).collect()
    }

    /// The layout of `tab`'s view: its pane's, or the whole content area for a tab not on screen.
    pub(super) fn pane_layout(&self, window: &AppWindow, tab: TabId) -> Layout {
        let (width, height) = content_size(window);
        let rect = self
            .pane_rects(window)
            .into_iter()
            .find(|r| r.tab == tab)
            .unwrap_or(PageRect { tab, x: 0.0, width });
        rect.layout(height, window.window().scale_factor())
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

    /// The rendering notifier's frame, before the shell is drawn: the split's strings, every
    /// view's size from the window as this frame lays it out, and each pane's stretch
    /// (`wubble_core::stretch`). Returns whether another frame is needed.
    pub fn panes_frame(&mut self, window: &AppWindow, now: Instant) -> bool {
        let dt = self.panes_last.replace(now).map_or(0.0, |prev| {
            now.duration_since(prev).as_secs_f64().min(DT_CAP)
        });
        let strings = self.model.shown_split().is_some() && self.step_strings(window, dt);
        self.lay_out_views(window);
        let stretching = self.step_stretch(window, dt);
        self.render_panes(window);
        let moving = strings || stretching;
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
        if self.frames.values().any(|f| f.pane.view != PaneView::Live) {
            window.window().request_redraw();
        }
    }

    /// Each view on screen gets its pane's layout, and its pane the size it now wants.
    fn lay_out_views(&mut self, window: &AppWindow) {
        let (_, height) = content_size(window);
        let scale = window.window().scale_factor();
        for rect in self.pane_rects(window) {
            let layout = rect.layout(height, scale);
            if let Some(view) = self.tab_view(rect.tab) {
                view.set_layout(layout);
            }
            if let Some(frame) = self.frames.get_mut(&rect.tab) {
                frame.pane = transition(frame.pane, PaneEvent::Layout(px(layout.frame_size())));
            }
        }
    }

    /// One frame of `dt` seconds of each pane's stretch; a settled pane goes live. Returns
    /// whether any still moves.
    fn step_stretch(&mut self, window: &AppWindow, dt: f64) -> bool {
        let (_, height) = content_size(window);
        let mut moving = false;
        for rect in self.pane_rects(window) {
            if let Some(frame) = self.frames.get_mut(&rect.tab) {
                if frame.springs.step((rect.width, height), dt) {
                    moving = true;
                } else {
                    frame.pane = transition(frame.pane, PaneEvent::Rest);
                }
            }
        }
        moving
    }

    /// Writes the panes, seams and ring, row by row where they changed (a row's own state, a
    /// seam's grab mid-drag, survives).
    fn render_panes(&mut self, window: &AppWindow) {
        let (_, height) = content_size(window);
        let focused = self.model.visible_tab();
        let rects = self.pane_rects(window);
        let fill = rects.len() == 1;
        let panes: Vec<SplitPane> = rects
            .into_iter()
            .map(|r| {
                let frame = self.frames.get(&r.tab);
                let (sx, sy) = frame.map_or((1.0, 1.0), |f| f.springs.scale(f.pane.view));
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
                    focused: Some(r.tab) == focused,
                }
            })
            .collect();
        let fullscreen = matches!(self.screen, Screen::Page { .. });
        let seams: Vec<SplitSeam> = if fullscreen {
            vec![]
        } else {
            self.panes
                .seams(height)
                .into_iter()
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
        if let Some(i) = self.slots.iter().position(|s| *s == Some(tab)) {
            return i;
        }
        let shown = self.framed_tabs();
        let i = self
            .slots
            .iter()
            .position(|s| s.is_none_or(|t| !shown.contains(&t)))
            .unwrap_or(MAX_PANES - 1);
        self.slots[i] = Some(tab);
        i
    }

    /// Takes each pane's new frame, if it has one, and the focused page's popup frame. Returns
    /// whether any pane got a frame.
    pub(super) fn present_panes(&mut self, window: &AppWindow) -> Result<bool, String> {
        let (_, height) = content_size(window);
        let scale = window.window().scale_factor();
        let focused = self.model.visible_tab();
        let mut presented = false;
        for rect in self.pane_rects(window) {
            let layout = rect.layout(height, scale);
            let Some(view) = self.tab_view(rect.tab) else {
                continue;
            };
            view.set_layout(layout);
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
            let wanted = px(layout.frame_size());
            let entry = self.frames.entry(rect.tab).or_insert_with(|| PaneFrame {
                image: image.clone(),
                clip,
                size,
                pane: Pane::new(wanted),
                springs: PaneSprings::default(),
            });
            entry.pane = transition(
                transition(entry.pane, PaneEvent::Layout(wanted)),
                PaneEvent::Frame(px(size)),
            );
            entry.image = image;
            entry.clip = clip;
            entry.size = size;
            presented = true;
        }
        if presented {
            self.render_panes(window);
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
    pub(super) fn frame_size(&self, tab: TabId) -> Option<wubble_gfx::FrameSize> {
        self.frames.get(&tab).map(|f| f.size)
    }

    // ----- the seams' gestures and the panes' close buttons, from the shell

    fn kick(&self) {
        if let Some(window) = self.window.upgrade() {
            window.window().request_redraw();
        }
    }

    pub fn split_grab(&mut self, seam: i32, x: f32, y: f32) {
        if let Ok(seam) = usize::try_from(seam) {
            self.panes.grab(seam, x.into(), y.into());
            self.kick();
        }
    }

    pub fn split_drag(&mut self, x: f32, y: f32) {
        self.panes.drag(x.into(), y.into());
        self.kick();
    }

    pub fn split_release(&mut self) {
        self.panes.release();
        self.kick();
    }

    pub fn split_even(&mut self) {
        self.panes.even();
        self.kick();
    }

    pub fn pane_close(&mut self, pane: i32) {
        if let Some(tab) = crate::shell::tab_id(pane) {
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
