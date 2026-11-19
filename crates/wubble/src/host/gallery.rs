//! `--smoke gallery` (`gallery_smoke.rs`): puts each scene into the host's own state and writes
//! the window as a PNG once it settled. Surfaces are the tab's [`Surfaces`] (what a page's
//! requests would have put there, with request ids no engine knows: the gallery never answers
//! them, it only quits), the autofill chip's [`autofill::View`], or the shell's own callbacks
//! (command bar, downloads popover, sidebar mode), invoked later, outside the host, as a click
//! sends them.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use slint::ComponentHandle;
use wubble_core::autofill::{MenuRow, SaveKind, View};
use wubble_core::model::Command;
use wubble_engine::page::{JsDialogKind, PermissionKind, RequestId};
use wubble_ui::{AppWindow, AutofillView};

use super::Host;
use super::autofill::shell_view;
use crate::gallery_smoke::{self, Action, Event, Overlay, SCENES, Scene};
use crate::menu::{self, Menu};
use crate::page_error::PageError;
use crate::prompt::{Ask, PermissionPrompt, PermissionRequest};
use crate::surface::{Dialog, FindBar, OpenDialog, Surfaces};
use crate::{quit, shot, with_host};

/// How long a scene settles (springs, fades, the command bar's results) before it is shot.
/// A wait in a hand-run smoke, never in a test.
const SETTLE: Duration = Duration::from_millis(1200);

/// How long the parity profile's pages get to load (title, favicon) before the first scene.
const SEED_SETTLE: Duration = Duration::from_millis(4000);

/// A request id no engine view holds: its surface is never answered.
const FAKE: RequestId = RequestId(u32::MAX);

/// Where the synthetic menus open, logical px in the window: beside the first tab row, and
/// under the address pill.
const TAB_MENU_AT: (f32, f32) = (120.0, 150.0);
const SITE_MENU_AT: (f32, f32) = (560.0, 96.0);

impl Host {
    /// One step of the walk: `Ready` each CEF work run once the frame rate is known, `Settled`
    /// from the settle timer, `Shot` after a write.
    pub(super) fn gallery_step(&mut self, event: Event) {
        let Some(stage) = self.checks.gallery_smoke.take() else {
            return;
        };
        let Some(window) = self.w.window.upgrade() else {
            self.checks.gallery_smoke = Some(stage);
            return;
        };
        let (stage, action) = gallery_smoke::next(stage, event);
        self.checks.gallery_smoke = Some(stage);
        match action {
            Some(Action::Seed) => {
                let seeded = shot::parity().map(|path| self.gallery_seed(&path));
                if let Some(Err(e)) = seeded {
                    return self.fail(format!("gallery parity: {e}"));
                }
                self.gallery_wait(if seeded.is_some() {
                    SEED_SETTLE
                } else {
                    Duration::ZERO
                });
            }
            Some(Action::Show(i)) => {
                // A burst starts from the show: its first frame is the next run of the loop.
                self.gallery_wait(if SCENES[i].frames > 0 {
                    Duration::ZERO
                } else {
                    SETTLE
                });
                self.gallery_show(&window, &SCENES[i]);
                self.gallery_burst_start = Instant::now();
            }
            Some(Action::Scroll(i)) => {
                self.gallery_wait(SETTLE);
                shot::wheel(&window, SCENES[i].wheel as f32);
            }
            Some(Action::ShootFrame(i, k)) => {
                let ms = self.gallery_burst_start.elapsed().as_secs_f64() * 1000.0;
                match shot::take_frame(&window, i, SCENES[i].name, k, ms) {
                    Ok(path) if k == 0 => println!("smoke: burst {}", path.display()),
                    Ok(_) => {}
                    Err(e) => return self.fail(format!("gallery frame {}: {e}", SCENES[i].name)),
                }
                // Back to the loop between frames, so the animations advance.
                self.gallery_timer
                    .start(slint::TimerMode::SingleShot, Duration::ZERO, || {
                        with_host(|host| host.gallery_step(Event::Shot))
                    });
            }
            Some(Action::Shoot(i)) => {
                match shot::take_scene(&window, i, SCENES[i].name) {
                    Ok(path) => println!("smoke: shot {}", path.display()),
                    Err(e) => return self.fail(format!("gallery shot {}: {e}", SCENES[i].name)),
                }
                self.gallery_step(Event::Shot);
            }
            Some(Action::Finish) => {
                self.gallery_passed = true;
                quit();
            }
            None => {}
        }
    }

    /// The scene now on screen is not yet settled; the timer says when it is.
    fn gallery_wait(&mut self, settle: Duration) {
        self.gallery_timer
            .start(slint::TimerMode::SingleShot, settle, || {
                with_host(|host| {
                    host.gallery_step(Event::Settled);
                })
            });
    }

    fn gallery_show(&mut self, window: &AppWindow, scene: &Scene) {
        let scale = window.window().scale_factor();
        let at = |(x, y): (f32, f32)| ((x * scale).round() as i32, (y * scale).round() as i32);
        // Nothing of the last scene stays: the page's surfaces, the autofill chip's menu.
        if let Some(tab) = self.shown() {
            self.surfaces.remove(&tab);
        }
        if let Some(page) = scene.page {
            self.dispatch(Command::OpenInternal(page));
        }
        let shown = self.shown();
        let mut surfaces = Surfaces::default();
        let mut autofill = View::default();
        let page_url = shown
            .and_then(|t| self.model.tabs.get(&t))
            .map(|t| t.url.clone())
            .unwrap_or_default();
        let origin = "https://example.com".to_owned();
        match scene.overlay {
            Overlay::TabMenu => {
                if let Some(tab) = shown {
                    let facts = wubble_core::tabmenu::Facts { muted: false };
                    surfaces.menu = Menu::Shell {
                        at: at(TAB_MENU_AT),
                        entries: menu::tab_entries(&self.model, tab, facts, &self.keymap),
                    };
                }
            }
            Overlay::SiteMenu => {
                let hosts = [("ads.example".to_owned(), 9), ("cdn.tracker".to_owned(), 3)];
                let host = wubble_core::zoom::host_of(&page_url).unwrap_or_default();
                let mut entries = menu::blocker_entries(12, &hosts, &host, false);
                entries.extend(menu::page_theme_entries(
                    &host,
                    wubble_core::pagetheme::PageTheme::Auto,
                ));
                surfaces.menu = Menu::Shell {
                    at: at(SITE_MENU_AT),
                    entries,
                };
            }
            Overlay::Find => {
                surfaces.find = FindBar::Open {
                    query: "wubble".into(),
                    matches: 7,
                    active: 2,
                }
            }
            Overlay::Permission => {
                surfaces.permission = PermissionPrompt::Open {
                    shown: PermissionRequest {
                        origin: origin.clone(),
                        ask: Ask::Engine {
                            id: FAKE,
                            kinds: vec![PermissionKind::CameraStream, PermissionKind::MicStream],
                        },
                    },
                    queued: VecDeque::new(),
                }
            }
            Overlay::Dialog => {
                surfaces.dialog = Dialog::Open(OpenDialog {
                    id: FAKE,
                    kind: JsDialogKind::Confirm,
                    origin: origin.clone(),
                    message: "Discard the changes to this page?".into(),
                    default_text: String::new(),
                })
            }
            Overlay::LoadError => {
                surfaces.error = PageError::Load {
                    url: "https://example.invalid/".into(),
                    code: -105,
                }
            }
            Overlay::CertError => {
                surfaces.error = PageError::Cert {
                    url: "https://expired.example/".into(),
                    code: -201,
                    request: FAKE,
                }
            }
            Overlay::AutofillMenu => {
                autofill = View {
                    chip: true,
                    menu: Some(vec![
                        MenuRow::Login {
                            username: "alice".into(),
                        },
                        MenuRow::Generate,
                    ]),
                    prompt: None,
                }
            }
            Overlay::AutofillSave => {
                autofill = View {
                    chip: true,
                    menu: None,
                    prompt: Some((SaveKind::New, "alice".into(), origin)),
                }
            }
            // Shown for longer than a scene settles, gone before the next one is shot.
            Overlay::Toast => {
                self.toast(
                    window,
                    wubble_core::shot::saved_text(Some(true), Some(true)),
                );
            }
            Overlay::None | Overlay::CommandBar(_) | Overlay::Downloads | Overlay::Unlock => {}
        }
        if let Some(tab) = shown {
            self.surfaces.insert(tab, surfaces);
        }
        self.render_surfaces(window);
        window.set_autofill(if autofill == View::default() {
            AutofillView::default()
        } else {
            shell_view(&autofill)
        });
        // The pointer rests where the scene says (a hover state), or leaves the window.
        let pointer = match scene.pointer {
            Some((x, y)) => slint::platform::WindowEvent::PointerMoved {
                position: slint::LogicalPosition::new(f32::from(x), f32::from(y)),
            },
            None => slint::platform::WindowEvent::PointerExited,
        };
        window.window().dispatch_event(pointer);
        // The shell's own callbacks call back into the host: later, outside it.
        let (sidebar, overlay) = (scene.sidebar, scene.overlay);
        let _ = self.w.window.upgrade_in_event_loop(move |window| {
            window.invoke_dismiss_command_bar();
            window.set_downloads_open(overlay == Overlay::Downloads);
            window.invoke_sidebar_mode_requested(sidebar.into());
            match overlay {
                Overlay::CommandBar(query) => {
                    window.invoke_request_new_tab();
                    if !query.is_empty() {
                        window.set_command_query(query.into());
                        window.invoke_command_query_changed(query.into());
                    }
                }
                Overlay::Unlock => window.invoke_passwords_unlock(),
                _ => {}
            }
        });
    }

    /// `--parity`: the design's Spaces (name, tint, icon), their favourites and tabs, from the
    /// manifest `tools/fidelity/parity.mjs` writes; the tabs the run started with are closed,
    /// and the first Space shows its first tab.
    fn gallery_seed(&mut self, path: &std::path::Path) -> Result<(), String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let manifest: serde_json::Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
        let spaces = manifest["spaces"].as_array().ok_or("no spaces")?;
        let started: Vec<_> = self.model.tabs.keys().copied().collect();
        let mut first = None;
        for (i, space) in spaces.iter().enumerate() {
            let name = space["name"].as_str().unwrap_or("Space").to_owned();
            let tint = rgb(space["tint"].as_str().unwrap_or("#7CC8FF"))?;
            let id = if i == 0 {
                let id = self.model.active_space();
                self.dispatch(Command::RenameSpace { space: id, name });
                self.dispatch(Command::SetSpaceTint { space: id, tint });
                id
            } else {
                self.dispatch(Command::NewSpace {
                    name,
                    tint,
                    container: wubble_core::space::ContainerId::DEFAULT,
                });
                let id = self.model.spaces.last().ok_or("no space")?.id;
                self.dispatch(Command::SwitchSpace(id));
                id
            };
            let icon = match space["icon"].as_str() {
                Some("work") => wubble_core::space::SpaceIcon::Work,
                Some("study") => wubble_core::space::SpaceIcon::Study,
                _ => wubble_core::space::SpaceIcon::Personal,
            };
            self.dispatch(Command::SetSpaceIcon { space: id, icon });
            let urls = |key: &str| -> Vec<String> {
                space[key]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|u| u.as_str().map(str::to_owned))
                    .collect()
            };
            for (pin, url) in urls("pinned")
                .into_iter()
                .map(|u| (true, u))
                .chain(urls("tabs").into_iter().map(|u| (false, u)))
            {
                self.dispatch(Command::NewTab {
                    url,
                    container: None,
                    opener: None,
                });
                let tab = self.shown().ok_or("the new tab is not shown")?;
                if pin {
                    self.dispatch(Command::PinTab(tab));
                } else if i == 0 && first.is_none() {
                    first = Some(tab);
                }
            }
        }
        for tab in started {
            self.dispatch(Command::CloseTab(tab));
        }
        if let Some(tab) = first {
            let space = self.model.spaces.first().ok_or("no space")?.id;
            self.dispatch(Command::SwitchSpace(space));
            self.dispatch(Command::ActivateTab(tab));
        }
        Ok(())
    }
}

/// `#RRGGBB` to [`wubble_core::space::Rgb`].
fn rgb(hex: &str) -> Result<wubble_core::space::Rgb, String> {
    let v =
        u32::from_str_radix(hex.trim_start_matches('#'), 16).map_err(|e| format!("{hex}: {e}"))?;
    Ok(wubble_core::space::Rgb(
        (v >> 16) as u8,
        (v >> 8) as u8,
        v as u8,
    ))
}
