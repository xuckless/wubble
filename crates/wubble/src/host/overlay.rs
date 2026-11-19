//! `--smoke overlay` (`overlay_smoke.rs`): puts each step on the shell from the host's own state
//! (the tab's menu surface, the shell's own callbacks for the command bar and the sidebar),
//! waits for it to settle, snapshots the window and measures the glass for the pure check.

use std::time::Duration;

use slint::ComponentHandle;
use wubble_core::seam::SidebarMode;
use wubble_gfx::scene::AMBIENCE_STRENGTH;
use wubble_ui::{AppWindow, GlassKeys, GlassLayer};

use super::{Host, snapshot};
use crate::menu::{self, Menu};
use crate::overlay_smoke::{self, Notes, Panel, STEPS, Seen, Setup};
use crate::pixels::{self, Region};
use crate::{physical, quit, shot, with_host};

/// How long a step settles (fades, the scene's frame, the page's next frames) before it is
/// measured. A wait in a hand-run smoke, never in a test.
const SETTLE: Duration = Duration::from_millis(1200);

/// Page frames shown before the first step: the page has loaded and painted.
const READY_FRAMES: u64 = 30;

/// Where the run is: not started, or at a step, with what it noted.
#[derive(Clone, Copy, Debug, Default)]
pub struct Run {
    step: Option<usize>,
    notes: Notes,
}

impl Host {
    /// Each CEF work run: starts the walk once the page has painted.
    pub(super) fn overlay_ready(&mut self) {
        let Some(run) = self.checks.overlay_smoke else {
            return;
        };
        if run.step.is_some() || self.stats.accelerated < READY_FRAMES {
            return;
        }
        self.overlay_show(0);
    }

    fn overlay_show(&mut self, i: usize) {
        let Some(window) = self.w.window.upgrade() else {
            return;
        };
        if let Some(run) = &mut self.checks.overlay_smoke {
            run.step = Some(i);
        }
        let ambience = |host: &mut Host, on: bool| {
            let strength = if on { AMBIENCE_STRENGTH } else { 0.0 };
            host.w.glass.borrow_mut().set_ambience(&window, strength);
        };
        match STEPS[i].setup {
            Setup::Ambience(on) => ambience(self, on),
            Setup::Menu => {
                if let Some(tab) = self.shown() {
                    let scale = window.window().scale_factor();
                    let (x, y) = (
                        window.get_content_x() + 40.0,
                        window.get_content_y() + window.get_content_height() * 0.34,
                    );
                    let facts = wubble_core::tabmenu::Facts { muted: false };
                    let mut surfaces = self.surfaces.remove(&tab).unwrap_or_default();
                    surfaces.menu = Menu::Shell {
                        at: ((x * scale).round() as i32, (y * scale).round() as i32),
                        entries: menu::tab_entries(&self.model, tab, facts, &self.keymap),
                    };
                    self.surfaces.insert(tab, surfaces);
                    self.render_surfaces(&window);
                }
            }
            Setup::Bar | Setup::Peek => {
                self.overlay_close_menu(&window);
            }
            Setup::Quiet => {
                self.overlay_close_menu(&window);
                ambience(self, false);
            }
            Setup::Wait => {}
        }
        // The shell's own callbacks call back into the host: later, outside it.
        let setup = STEPS[i].setup;
        let _ = self.w.window.upgrade_in_event_loop(move |window| {
            match setup {
                Setup::Bar => window.invoke_request_new_tab(),
                Setup::Peek => {
                    window.invoke_dismiss_command_bar();
                    window.invoke_sidebar_mode_requested(SidebarMode::Hidden.into());
                    window.set_peek(true);
                    // the pointer rests on the peek, so it stays out
                    window
                        .window()
                        .dispatch_event(slint::platform::WindowEvent::PointerMoved {
                            position: slint::LogicalPosition::new(4.0, 400.0),
                        });
                }
                Setup::Quiet => {
                    window.invoke_dismiss_command_bar();
                    window.set_peek(false);
                    window.invoke_sidebar_mode_requested(SidebarMode::Extended.into());
                    window
                        .window()
                        .dispatch_event(slint::platform::WindowEvent::PointerExited);
                }
                _ => {}
            }
            window.window().request_redraw();
        });
        self.overlay_timer
            .start(slint::TimerMode::SingleShot, SETTLE, || {
                with_host(Host::overlay_settled)
            });
    }

    fn overlay_close_menu(&mut self, window: &AppWindow) {
        if let Some(tab) = self.shown()
            && let Some(surfaces) = self.surfaces.get_mut(&tab)
        {
            surfaces.menu = Menu::Closed;
            self.render_surfaces(window);
        }
    }

    /// The step settled: measure, check, shoot it if asked, then the next step or the end.
    fn overlay_settled(&mut self) {
        let Some(Run {
            step: Some(i),
            notes,
        }) = self.checks.overlay_smoke
        else {
            return;
        };
        let Some(window) = self.w.window.upgrade() else {
            return;
        };
        let step = &STEPS[i];
        // shot first, so a failed step leaves its picture
        if shot::has_dir() {
            match shot::take_scene(&window, i, step.name) {
                Ok(path) => println!("smoke: shot {}", path.display()),
                Err(e) => return self.fail(format!("overlay shot {}: {e}", step.name)),
            }
        }
        let checked = self.overlay_seen(&window, step).and_then(|seen| {
            println!("smoke: overlay {} {seen:?}", step.name);
            overlay_smoke::check(step, &seen, notes)
        });
        let notes = match checked {
            Ok(notes) => notes,
            Err(e) => return self.fail(format!("overlay {}: {e}", step.name)),
        };
        if let Some(run) = &mut self.checks.overlay_smoke {
            run.notes = notes;
        }
        if i + 1 < STEPS.len() {
            self.overlay_show(i + 1);
        } else {
            // the ambience back as it runs
            self.w
                .glass
                .borrow_mut()
                .set_ambience(&window, AMBIENCE_STRENGTH);
            self.overlay_passed = true;
            quit();
        }
    }

    /// What the snapshot shows: the gutter's glass, the checked panel's glass, the build count.
    fn overlay_seen(&self, window: &AppWindow, step: &overlay_smoke::Step) -> Result<Seen, String> {
        let shot = snapshot(window)?;
        let scale = window.window().scale_factor();
        let region = |x: f32, y: f32, w: f32, h: f32| Region {
            x: physical(x, scale),
            y: physical(y, scale),
            width: physical(w, scale).max(1),
            height: physical(h, scale).max(1),
        };
        let (cx, cy, ch) = (
            window.get_content_x(),
            window.get_content_y(),
            window.get_content_height(),
        );
        // the window's glass between the sidebar and the page, beside the page's red part
        let gutter = pixels::mean_rgb(&shot, region(cx - 6.0, cy + ch * 0.55, 4.0, ch * 0.3))?;
        let mut layer_image = false;
        let panel = match step.check {
            overlay_smoke::Check::Hue(panel, _) => {
                // the scene's image of the panel's layer, as app.slint shows it
                let image = match panel {
                    Panel::Menu => window.get_over_top(),
                    Panel::Bar => window.get_over_card(),
                    Panel::Peek => window.get_over_peek(),
                };
                layer_image = window.global::<GlassLayer>().get_over_on() && image.size().width > 0;
                let keys = window.global::<GlassKeys>();
                let key = match panel {
                    Panel::Menu => keys.get_menu(),
                    Panel::Bar => keys.get_command_bar(),
                    Panel::Peek => keys.get_peek(),
                };
                let item = self.w.panels.borrow().get(key).cloned();
                // inside the glass, clear of its rim; the menu and the peek over the page's
                // red part only (well below its green top), wherever the window's size put them
                item.map(|g| {
                    let mut top = g.y + g.height * 0.25;
                    let bottom = g.y + g.height * 0.75;
                    if panel != Panel::Bar {
                        top = top.max(cy + ch * 0.45).min(bottom - 1.0);
                    }
                    pixels::mean_rgb(
                        &shot,
                        region(g.x + g.width * 0.2, top, g.width * 0.6, bottom - top),
                    )
                })
                .transpose()?
            }
            _ => None,
        };
        Ok(Seen {
            gutter,
            panel,
            layer_image,
            builds: self.w.glass.borrow().pages_built().unwrap_or(0),
        })
    }
}
