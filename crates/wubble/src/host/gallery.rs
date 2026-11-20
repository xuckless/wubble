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
use crate::gallery_smoke::{self, Action, Event, Ground, Long, Overlay, SCENES, Scene};
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
/// Another, for a dialog shown after the first: a new one to the sheet, whose serial is the id
/// as an `i32` (every id past `i32::MAX` is the same serial).
const FAKE_2: RequestId = RequestId(i32::MAX.unsigned_abs() - 1);

/// Where the synthetic menus open, logical px in the window: beside the first tab row, and
/// under the address pill.
const TAB_MENU_AT: (f32, f32) = (120.0, 150.0);
const SITE_MENU_AT: (f32, f32) = (560.0, 96.0);
/// Under the pill's blocker badge, where its click opens the menu.
const BADGE_MENU_AT: (f32, f32) = (190.0, 92.0);

/// The address the [`Long`] scenes show: a common host with a long path.
const LONG_URL: &str =
    "https://en.wikipedia.org/wiki/List_of_longest_words_in_the_English_language";
/// How many tabs a crowded Space holds: more than the sidebar shows at the gallery's size
/// (QA MED-05 saw fifteen in a smaller window).
const CROWD: usize = 22;
/// Where the page's own menu opens, logical px in the page (over its text).
const PAGE_MENU_AT: (f32, f32) = (420.0, 120.0);

/// The sidebar's reload button: its centre x, top and bottom (logical px in the window), where
/// its tooltip hangs.
const RELOAD_TIP: (f32, f32, f32) = (243.0, 16.0, 42.0);

/// The address of a scene's plain page; `None` keeps whatever page is shown.
fn ground_url(ground: Ground) -> Option<String> {
    match ground {
        Ground::Bundled => None,
        Ground::White => Some(crate::page_url(crate::GROUND_PAGE)),
        Ground::Dark => Some(crate::page_url(crate::GROUND_PAGE) + "#dark"),
    }
}

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
                if let Some(at) = SCENES[i].click {
                    click(&window, at);
                }
                if SCENES[i].wheel > 0 {
                    shot::wheel(&window, SCENES[i].wheel as f32);
                }
                if SCENES[i].overlay == Overlay::SpacesDrag {
                    self.gallery_drag_to_spaces();
                }
            }
            Some(Action::Click(i)) => {
                self.gallery_wait(SETTLE);
                if let Some(at) = SCENES[i].then {
                    click(&window, at);
                }
            }
            Some(Action::Point(i)) => {
                self.gallery_wait(SETTLE);
                gallery_point(&window, &SCENES[i]);
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
        // Nothing of the last scene stays: a held tab, the page's surfaces, the autofill chip's
        // menu, a tooltip.
        self.cancel_tab_drag();
        self.tip_dismiss();
        if let Some(tab) = self.shown() {
            self.surfaces.remove(&tab);
        }
        // The bookmark and print sheets are the host's own state, not the tab's: closed too.
        self.bookmark_sheet_answered(false, false, "", -1);
        // A waiting notice goes; a new one replaces it by itself (hiding it in the same turn as
        // showing one races the toast's two serials).
        if !matches!(scene.overlay, Overlay::Toast | Overlay::Question) {
            self.toast_hide();
        }
        if !matches!(self.print, wubble_core::print::Sheet::Closed) {
            self.print_event(wubble_core::print::Event::Cancel);
        }
        // an agent session of the last scene ends (its sheet, its Space)
        self.agents_stop();
        if scene.page == Some(wubble_core::pages::InternalPage::AgentLog) {
            self.gallery_agent_log();
        }
        if let Some(page) = scene.page {
            self.dispatch(Command::OpenInternal(page));
        }
        // a plain page under the scene: loaded once, kept for the scenes after it on the same
        if let (Some(tab), Some(url)) = (self.shown(), ground_url(scene.ground))
            && self.model.tabs.get(&tab).is_some_and(|t| t.url != url)
        {
            self.dispatch(Command::Navigate { tab, url });
        }
        if let Overlay::Long(long) = scene.overlay {
            self.gallery_long(long);
        }
        if let Overlay::Spaces(n) = scene.overlay {
            while self.model.spaces.len() < n {
                let command = self.model.new_space();
                self.dispatch(command);
            }
        }
        self.gallery_extensions(scene.overlay);
        let shown = self.shown();
        let mut surfaces = Surfaces::default();
        let mut autofill = View::default();
        let page_url = shown
            .and_then(|t| self.model.tabs.get(&t))
            .map(|t| t.url.clone())
            .unwrap_or_default();
        let origin = "https://example.com".to_owned();
        match scene.overlay {
            Overlay::TabMenu | Overlay::Long(Long::TabMenu) => {
                if let Some(tab) = shown {
                    let facts = wubble_core::tabmenu::Facts { muted: false };
                    surfaces.menu = Menu::Shell {
                        at: at(TAB_MENU_AT),
                        entries: menu::tab_entries(&self.model, tab, facts, &self.keymap),
                    };
                }
            }
            Overlay::PageMenu => {
                let menu = wubble_engine::page::ContextMenu {
                    page_url: page_url.clone(),
                    ..Default::default()
                };
                let nav = menu::NavFlags {
                    back: true,
                    forward: false,
                    search: None,
                };
                surfaces.menu = Menu::Open {
                    id: FAKE,
                    at: at(PAGE_MENU_AT),
                    entries: menu::entries(&menu, nav),
                };
            }
            Overlay::Tooltip => {
                let (x, top, bottom) = RELOAD_TIP;
                self.shell_tip("reload", x, (top, bottom), true);
            }
            Overlay::SiteMenu | Overlay::Long(Long::SiteMenu) => {
                let hosts = [("ads.example".to_owned(), 9), ("cdn.tracker".to_owned(), 3)];
                let host = wubble_core::zoom::host_of(&page_url).unwrap_or_default();
                let mut entries = menu::blocker_entries(12, &hosts, &host, false);
                entries.extend(menu::page_theme_entries(
                    &host,
                    wubble_core::pagetheme::PageTheme::Auto,
                ));
                surfaces.menu = Menu::Shell {
                    at: at(if scene.overlay == Overlay::SiteMenu {
                        SITE_MENU_AT
                    } else {
                        BADGE_MENU_AT
                    }),
                    entries,
                };
            }
            Overlay::Find => {
                surfaces.find = FindBar::Open {
                    query: "wubble".into(),
                    matches: 7,
                    active: 2,
                    done: true,
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
            Overlay::Prompt => {
                surfaces.dialog = Dialog::Open(OpenDialog {
                    id: FAKE_2,
                    kind: JsDialogKind::Prompt,
                    origin: origin.clone(),
                    message: "Name this list".into(),
                    default_text: "Groceries".into(),
                })
            }
            Overlay::Bookmark => self.gallery_bookmark_sheet(),
            Overlay::Print => {
                if let Some(tab) = shown {
                    self.print_event(wubble_core::print::Event::Open {
                        tab,
                        printers: vec!["Office laser".into(), "Studio inkjet".into()],
                        default: None,
                    });
                }
            }
            Overlay::AgentSheet => self.gallery_agent(false),
            Overlay::AgentTabs | Overlay::AgentSpaces => self.gallery_agent(true),
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
            Overlay::Question => {
                let (text, action) = super::videos::ASK_SPONSOR;
                self.toast_with(text, action, true);
            }
            Overlay::Lists => self.gallery_lists(),
            Overlay::ExtensionsList | Overlay::ExtensionsPopover | Overlay::ExtensionPrompt => {}
            Overlay::None
            | Overlay::Long(Long::Pill | Long::Crowd)
            | Overlay::CommandBar(_)
            | Overlay::Downloads
            | Overlay::Spaces(_)
            | Overlay::SpacesDrag
            | Overlay::SettingsSearch(_)
            | Overlay::Unlock
            | Overlay::Status(_) => {}
        }
        // the link status bubble: shown for its scene, gone for the others
        if let Some(tab) = shown {
            match scene.overlay {
                Overlay::Status(url) => self.link_status(tab, url.to_owned()),
                _ if self.status.shown().is_some() => self.link_status(tab, String::new()),
                _ => {}
            }
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
        if let Overlay::Long(_) = scene.overlay {
            // the blocker's count and the reader, as a page that blocked and reads would have
            window.set_address_blocker_visible(true);
            window.set_address_blocked(14);
            window.set_reader_available(true);
        }
        gallery_point(window, scene);
        // The shell's own callbacks call back into the host: later, outside it.
        let (sidebar, overlay) = (scene.sidebar, scene.overlay);
        let _ = self.w.window.upgrade_in_event_loop(move |window| {
            window.invoke_dismiss_command_bar();
            window.set_downloads_open(overlay == Overlay::Downloads);
            window.set_spaces_open(matches!(overlay, Overlay::Spaces(_) | Overlay::AgentSpaces));
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
                Overlay::SettingsSearch(query) => window.invoke_settings_type_query(query.into()),
                _ => {}
            }
        });
    }

    /// The [`Long`] scenes' state: the active Space's first web tab shown at [`LONG_URL`]
    /// (the engine keeps its page; only the address changes), and for a crowd, background tabs
    /// until the Space holds [`CROWD`].
    fn gallery_long(&mut self, long: Long) {
        let space = self.model.active_space();
        let in_space = |model: &wubble_core::model::Model| -> Vec<wubble_core::tab::TabId> {
            model
                .tabs
                .keys()
                .copied()
                .filter(|t| model.location(*t).is_some_and(|(s, _)| s == space))
                .collect()
        };
        let Some(web) = in_space(&self.model)
            .into_iter()
            .find(|t| !self.model.is_internal(*t))
        else {
            return;
        };
        if self.shown() != Some(web) {
            self.dispatch(Command::ActivateTab(web));
        }
        if self.model.tabs.get(&web).is_some_and(|t| t.url != LONG_URL) {
            self.dispatch(Command::UrlChanged {
                tab: web,
                url: LONG_URL.into(),
            });
            self.dispatch(Command::TitleChanged {
                tab: web,
                title: "List of longest words in the English language - Wikipedia".into(),
            });
        }
        if long == Long::Crowd {
            let mut n = in_space(&self.model).len();
            while n < CROWD {
                let before = in_space(&self.model);
                self.dispatch(Command::BackgroundTab {
                    url: "about:blank".into(),
                    opener: web,
                });
                let Some(tab) = in_space(&self.model)
                    .into_iter()
                    .find(|t| !before.contains(t))
                else {
                    break;
                };
                self.dispatch(Command::TitleChanged {
                    tab,
                    title: format!("Background tab {n}"),
                });
                n += 1;
            }
        }
    }

    /// Settings' list editors with rows: two routing rules, two sites never put to sleep, two
    /// zoom levels, two remembered permissions (in memory only), three Spaces.
    fn gallery_lists(&mut self) {
        use wubble_core::settings as defs;
        use wubble_core::site::{Decision, Permission};
        for (key, text) in [
            (
                defs::ROUTING_RULES.key,
                r#"["github.com -> Work","*.bank.example -> Personal, Banking"]"#,
            ),
            (
                defs::SLEEP_NEVER_SITES.key,
                r#"["music.example","mail.example"]"#,
            ),
            (
                defs::SITE_ZOOM.key,
                r#"{"docs.example":125,"news.example":90}"#,
            ),
        ] {
            if self.settings.stored.get(key).map(String::as_str) != Some(text) {
                self.write_setting(key, text);
            }
        }
        // three Spaces, the two new ones as "+" makes them
        while self.model.spaces.len() < 3 {
            let command = self.model.new_space();
            self.dispatch(command);
        }
        self.site_permissions
            .set("https://meet.example", Permission::Camera, Decision::Allow);
        self.site_permissions.set(
            "https://ads.example",
            Permission::Notifications,
            Decision::Block,
        );
        self.render_settings();
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
/// A left click at `(x, y)`, logical px in the window.
fn click(window: &AppWindow, (x, y): (u16, u16)) {
    use slint::platform::{PointerEventButton, WindowEvent};
    let position = slint::LogicalPosition::new(f32::from(x), f32::from(y));
    let w = window.window();
    w.dispatch_event(WindowEvent::PointerMoved { position });
    w.dispatch_event(WindowEvent::PointerPressed {
        position,
        button: PointerEventButton::Left,
    });
    w.dispatch_event(WindowEvent::PointerReleased {
        position,
        button: PointerEventButton::Left,
    });
}

fn rgb(hex: &str) -> Result<wubble_core::space::Rgb, String> {
    let v =
        u32::from_str_radix(hex.trim_start_matches('#'), 16).map_err(|e| format!("{hex}: {e}"))?;
    Ok(wubble_core::space::Rgb(
        (v >> 16) as u8,
        (v >> 8) as u8,
        v as u8,
    ))
}

/// The pointer rests where the scene says (a hover state), or leaves the window.
fn gallery_point(window: &AppWindow, scene: &Scene) {
    let pointer = match scene.pointer {
        Some((x, y)) => slint::platform::WindowEvent::PointerMoved {
            position: slint::LogicalPosition::new(f32::from(x), f32::from(y)),
        },
        None => slint::platform::WindowEvent::PointerExited,
    };
    window.window().dispatch_event(pointer);
}
