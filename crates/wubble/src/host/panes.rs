//! The pages on screen: one view per pane, each with its own frame stream, size and ring of
//! textures; the split's strings stepped once per rendered frame (`crate::panes`); the shell's
//! `panes`/`seams` models rewritten whole from them, like the sidebar from the model.

use std::time::Instant;

use slint::ComponentHandle;
use wubble_core::model::Model;
use wubble_core::motion::DT_CAP;
use wubble_core::space::MAX_PANES;
use wubble_core::tab::TabId;
use wubble_engine::view::{Layout, ScreenArea};
use wubble_gfx::FrameSize;
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

    /// One frame of `dt` seconds of the split's strings, then every pane laid out and drawn.
    /// What the strings settled is dispatched later, outside rendering. Returns whether they move.
    fn step_panes(&mut self, window: &AppWindow, dt: f64) -> bool {
        let (width, height) = content_size(window);
        let (moving, commands) = self.panes.frame(&self.model, dt, width, height);
        if !commands.is_empty() {
            let first = self.split_commands.is_empty();
            self.split_commands.extend(commands);
            if first {
                let _ = slint::invoke_from_event_loop(|| with_host(Host::apply_split_commands));
            }
        }
        self.layout_panes(window);
        moving
    }

    /// The rendering notifier's frame: steps the strings while a split shows. One page alone
    /// changes only with the model (`settle_panes`).
    pub fn split_frame(&mut self, window: &AppWindow, now: Instant) -> bool {
        if self.model.shown_split().is_none() {
            self.split_last = None;
            return false;
        }
        let dt = self.split_last.replace(now).map_or(0.0, |prev| {
            now.duration_since(prev).as_secs_f64().min(DT_CAP)
        });
        let moving = self.step_panes(window, dt);
        if !moving {
            self.split_last = None;
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
        self.step_panes(window, 0.0);
        if self.model.shown_split().is_some() {
            window.window().request_redraw();
        }
    }

    /// Each view on screen gets its pane's size (unchanged layouts cost nothing; a new one only
    /// asks CEF to resize, the stretched last frame shows meanwhile), then the shell's panes.
    pub(super) fn layout_panes(&mut self, window: &AppWindow) {
        for rect in self.pane_rects(window) {
            let layout = self.layout_of(window, &rect);
            if let Some(view) = self.tab_view(rect.tab) {
                view.set_layout(layout);
            }
        }
        self.render_panes(window);
    }

    /// Writes the panes, seams and ring, when they changed (a rewritten model would restart
    /// the shell's rendering of them, and ask for another frame).
    fn render_panes(&mut self, window: &AppWindow) {
        let (_, height) = content_size(window);
        let focused = self.model.visible_tab();
        let panes: Vec<SplitPane> = self
            .pane_rects(window)
            .into_iter()
            .map(|r| {
                let frame = self.frames.get(&r.tab);
                SplitPane {
                    id: crate::shell::pane_id(r.tab),
                    x0: r.x as f32,
                    x1: (r.x + r.width) as f32,
                    frame: frame.map(|f| f.image.clone()).unwrap_or_default(),
                    clip_width: frame.map_or(0, |f| slint_int(f.clip.width)),
                    clip_height: frame.map_or(0, |f| slint_int(f.clip.height)),
                    current: frame.is_some_and(|f| f.current),
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
        if (&panes, &seams) != (&self.drawn_panes.0, &self.drawn_panes.1) {
            window.set_panes(super::surfaces::model(panes.clone()));
            window.set_seams(super::surfaces::model(seams.clone()));
            self.drawn_panes = (panes, seams);
        }
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
        let focused = self.model.visible_tab();
        let mut presented = false;
        for rect in self.pane_rects(window) {
            let layout = self.layout_of(window, &rect);
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
            let clip = size.cropped_to(layout.size);
            let current = size == layout.frame_size();
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
            self.frames.insert(
                rect.tab,
                PaneFrame {
                    image,
                    clip,
                    size,
                    current,
                },
            );
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
