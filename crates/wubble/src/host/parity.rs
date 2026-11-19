//! `--smoke parity`'s executor: runs what `parity::next` asks through the host's real
//! paths. Pointer input is dispatched to the Slint window (its hit testing, the web
//! FocusScope, the pane's TouchArea, then the host, as a real click goes); keys go to
//! `Host::key` as raw winit keys do, and on to Slint when the host lets them through.
//! Every input is queued on the event loop in order, outside the host's borrow.

use slint::ComponentHandle;
use slint::platform::{PointerEventButton, WindowEvent as SlintEvent};
use slint::winit_030::EventResult;
use slint::winit_030::winit::event::WindowEvent;
use slint::winit_030::winit::keyboard::{Key as WinitKey, NamedKey};
use wubble_engine::input::{KeyInput, Point};
use wubble_ui::{AppWindow, CommandBarState};

use super::Host;
use crate::clipboard::{Bridge as _, Buffer};
use crate::menu::{Command as MenuCommand, Menu};
use crate::parity::{self, Action, Button, HostCheck, HostOp, Input, Mods, Parity, Step};
use crate::with_host;

/// The smoke's state beside the pure driver's.
pub struct ParityRun {
    items: Vec<parity::Item>,
    state: Parity,
    failed: Vec<&'static str>,
    /// `Step::Restore` loads the page again: only its own load counts.
    restoring: bool,
    /// Downloads and prints started before the item now running.
    downloads_before: usize,
    /// Where a `Step::Press` holds the left button down, until its release.
    held: Option<Point>,
    /// Known gaps that failed, as expected.
    gaps: Vec<&'static str>,
}

impl Default for ParityRun {
    fn default() -> Self {
        Self {
            items: parity::selected(std::env::var("WUBBLE_PARITY").ok().as_deref()),
            state: Parity::default(),
            failed: vec![],
            restoring: false,
            downloads_before: 0,
            held: None,
            gaps: vec![],
        }
    }
}

/// `WUBBLE_PARITY_TRACE=1`: what the smoke sends and hears, on stderr.
fn trace(line: &str) {
    if std::env::var_os("WUBBLE_PARITY_TRACE").is_some() {
        eprintln!("parity trace: {line}");
    }
}

/// What Slint's own key event for `key` carries, if Slint knows the key.
fn slint_text(key: &KeyInput) -> Option<slint::SharedString> {
    use slint::platform::Key as S;
    let named = match &key.unmodified {
        WinitKey::Named(n) => *n,
        WinitKey::Character(_) => {
            return key
                .text
                .clone()
                .or_else(|| match &key.unmodified {
                    WinitKey::Character(c) => Some(c.to_string()),
                    _ => None,
                })
                .map(Into::into);
        }
        _ => return None,
    };
    let s = match named {
        NamedKey::Escape => S::Escape,
        NamedKey::Enter => S::Return,
        NamedKey::Tab => S::Tab,
        NamedKey::Backspace => S::Backspace,
        NamedKey::Delete => S::Delete,
        NamedKey::ArrowLeft => S::LeftArrow,
        NamedKey::ArrowRight => S::RightArrow,
        NamedKey::ArrowUp => S::UpArrow,
        NamedKey::ArrowDown => S::DownArrow,
        NamedKey::Home => S::Home,
        NamedKey::End => S::End,
        NamedKey::PageUp => S::PageUp,
        NamedKey::PageDown => S::PageDown,
        NamedKey::Control => S::Control,
        NamedKey::Shift => S::Shift,
        NamedKey::Alt => S::Alt,
        _ => return None,
    };
    Some(s.into())
}

impl Host {
    /// While `--smoke parity` runs it owns the pointer and the keyboard: the
    /// desktop's own (someone using the machine, the pointer resting over the
    /// window) would land between its steps. Its own input reaches Slint and
    /// `Host::key` directly, never through winit.
    pub(crate) fn parity_owns_input(&self, event: &WindowEvent) -> bool {
        self.checks.parity.is_some()
            && matches!(
                event,
                WindowEvent::CursorMoved { .. }
                    | WindowEvent::CursorEntered { .. }
                    | WindowEvent::CursorLeft { .. }
                    | WindowEvent::MouseInput { .. }
                    | WindowEvent::MouseWheel { .. }
                    | WindowEvent::KeyboardInput { .. }
                    | WindowEvent::ModifiersChanged(_)
                    | WindowEvent::Ime(_)
                    | WindowEvent::Touch(_)
                    | WindowEvent::PinchGesture { .. }
                    | WindowEvent::PanGesture { .. }
                    | WindowEvent::DoubleTapGesture { .. }
                    | WindowEvent::RotationGesture { .. }
                    | WindowEvent::TouchpadPressure { .. }
            )
    }

    fn parity_run(&mut self) -> Option<&mut ParityRun> {
        self.checks.parity.as_mut()
    }

    fn parity_url(&self) -> Option<String> {
        let tab = self.model.visible_tab()?;
        self.model.tabs.get(&tab).map(|t| t.url.clone())
    }

    pub(super) fn parity_web(&mut self, event: &wubble_engine::nav::WebEvent) {
        use wubble_engine::nav::WebEvent;
        match event {
            WebEvent::LoadingChanged { loading: false, .. } => {
                let home = crate::page_url(crate::PARITY_PAGE);
                let at_home = self
                    .parity_url()
                    .is_some_and(|u| u.split('#').next() == Some(&home));
                let Some(run) = self.parity_run() else { return };
                if run.restoring && !at_home {
                    return;
                }
                run.restoring = false;
                self.parity_step(Input::Loaded)
            }
            WebEvent::CursorChanged(cursor) => trace(&format!("cursor {cursor:?}")),
            WebEvent::TitleChanged(title) => {
                trace(&format!("title {title}"));
                self.parity_step(Input::Title(title))
            }
            _ => {}
        }
    }

    /// The visible tab's page menu just opened: the smoke may choose from it.
    pub(super) fn parity_menu(&mut self, tab: wubble_core::tab::TabId) {
        if self.checks.parity.is_none() || self.model.visible_tab() != Some(tab) {
            return;
        }
        let entries: Vec<(String, bool)> = match self.surfaces.get(&tab).map(|s| &s.menu) {
            // Separators stay in (unlabelled, disabled): a choice is an index into all rows.
            Some(Menu::Open { entries, .. }) => entries
                .iter()
                .map(|e| {
                    (
                        e.label.clone(),
                        e.enabled && e.command != MenuCommand::Separator,
                    )
                })
                .collect(),
            _ => return,
        };
        self.parity_step(Input::Menu(&entries));
    }

    pub(super) fn parity_step(&mut self, input: Input) {
        let Some(run) = self.parity_run() else {
            return;
        };
        let (state, actions) = parity::next_in(&run.items, std::mem::take(&mut run.state), input);
        if let Some(run) = self.parity_run() {
            run.state = state;
        }
        for action in actions {
            self.parity_act(action);
        }
    }

    fn parity_act(&mut self, action: Action) {
        let Some(window) = self.w.window.upgrade() else {
            return;
        };
        match action {
            Action::Script(script) => {
                trace(&format!("script {script}"));
                if let Some(view) = self.view() {
                    view.run_script(&script);
                }
            }
            Action::Step(step) => self.parity_input(&window, step),
            Action::Choose(i) => self.menu_chosen(i),
            Action::DismissMenu => {
                self.menu_dismissed();
            }
            Action::Check(check) => {
                let result = self.parity_check(&window, check);
                self.parity_step(Input::Checked(result));
            }
            Action::Report { name, result, gap } => {
                let Some(run) = self.parity_run() else { return };
                match (&result, gap) {
                    (Ok(()), false) => println!("parity: {name} ok"),
                    (Err(why), false) => {
                        println!("parity: {name} FAIL: {why}");
                        run.failed.push(name);
                    }
                    (Err(why), true) => {
                        println!("parity: {name} known gap: {why}");
                        run.gaps.push(name);
                    }
                    (Ok(()), true) => {
                        println!("parity: {name} FAIL: passes, but the table marks it a known gap");
                        run.failed.push(name);
                    }
                }
            }
            Action::Finish => {
                let Some(run) = self.checks.parity.take() else {
                    return;
                };
                if run.failed.is_empty() {
                    self.parity_passed = Some(run.gaps);
                    crate::quit();
                } else {
                    self.fail(format!(
                        "{} items: {}",
                        run.failed.len(),
                        run.failed.join(", ")
                    ));
                }
            }
        }
    }

    /// The window-logical position of `point` in the visible page.
    fn parity_at(&self, window: &AppWindow, point: Point) -> slint::LogicalPosition {
        let (x, y) = self.focus_origin(window);
        slint::LogicalPosition::new(x + point.x, y + point.y)
    }

    /// Queues pointer events for Slint, with `mods` held as the raw keyboard reports them.
    fn parity_pointer(&self, mods: Mods, events: Vec<SlintEvent>) {
        let weak = self.w.window.clone();
        let _ = slint::invoke_from_event_loop(move || {
            let Some(window) = weak.upgrade() else { return };
            with_host(|h| h.input.keys = mods.state());
            for event in events {
                // Where winit would have put the pointer: the host replays it there when
                // the page's cursor changes.
                if let SlintEvent::PointerMoved { position } = event {
                    with_host(|h| h.input.pointer = Some(position));
                }
                window.window().dispatch_event(event);
            }
            with_host(|h| h.input.keys = Default::default());
        });
    }

    fn parity_keys(&self, keys: Vec<KeyInput>) {
        let weak = self.w.window.clone();
        let _ = slint::invoke_from_event_loop(move || {
            let Some(window) = weak.upgrade() else { return };
            for key in keys {
                let mut result = EventResult::PreventDefault;
                with_host(|h| {
                    h.input.keys = key.modifiers.keys;
                    result = h.key(&window, &key);
                });
                // What winit hands on to Slint after the host's hook.
                if result == EventResult::Propagate
                    && let Some(text) = slint_text(&key)
                {
                    window.window().dispatch_event(if key.pressed {
                        SlintEvent::KeyPressed { text }
                    } else {
                        SlintEvent::KeyReleased { text }
                    });
                }
            }
        });
    }

    fn parity_input(&mut self, window: &AppWindow, step: Step) {
        // The smoke window may not be the desktop's focused one; a user's is.
        if !self.input.window_focused {
            self.input.window_focused = true;
            self.sync_focus(window);
        }
        // A left button a `Press` holds until its `Release` (or a left click).
        let held = match step {
            Step::Press(p) => Some(Some(p)),
            Step::Release(_) | Step::Click(Button::Left, ..) => Some(None),
            _ => None,
        };
        if let (Some(held), Some(run)) = (held, self.parity_run()) {
            run.held = held;
        }
        let button = |b| match b {
            Button::Left => PointerEventButton::Left,
            Button::Middle => PointerEventButton::Middle,
            Button::Right => PointerEventButton::Right,
        };
        let moved = |position| SlintEvent::PointerMoved { position };
        let press = |position, b| SlintEvent::PointerPressed {
            position,
            button: button(b),
        };
        let release = |position, b| SlintEvent::PointerReleased {
            position,
            button: button(b),
        };
        let at = |p| self.parity_at(window, p);
        match step {
            Step::Click(b, mods, p) => {
                let p = at(p);
                self.parity_pointer(mods, vec![moved(p), press(p, b), release(p, b)]);
            }
            Step::Clicks(n, p) => {
                let p = at(p);
                let mut events = vec![moved(p)];
                for _ in 0..n {
                    events.extend([press(p, Button::Left), release(p, Button::Left)]);
                }
                self.parity_pointer(parity::NONE, events);
            }
            Step::Press(p) => {
                let p = at(p);
                self.parity_pointer(parity::NONE, vec![moved(p), press(p, Button::Left)]);
            }
            Step::MoveTo(p) => self.parity_pointer(parity::NONE, vec![moved(at(p))]),
            Step::Release(p) => {
                let p = at(p);
                self.parity_pointer(parity::NONE, vec![moved(p), release(p, Button::Left)]);
            }
            Step::Keys(spec) => {
                let keys = parity::chords(spec)
                    .unwrap_or_default()
                    .into_iter()
                    .filter_map(parity::key_inputs)
                    .flatten()
                    .collect();
                self.parity_keys(keys);
            }
            Step::Type(text) => {
                let keys = parity::typed(text)
                    .unwrap_or_default()
                    .into_iter()
                    .filter_map(parity::key_inputs)
                    .flatten()
                    .collect();
                self.parity_keys(keys);
            }
            Step::Wheel(mods, p, dx, dy) => {
                let p = at(p);
                self.parity_pointer(
                    mods,
                    vec![
                        moved(p),
                        SlintEvent::PointerScrolled {
                            position: p,
                            delta_x: dx,
                            delta_y: dy,
                        },
                    ],
                );
            }
            Step::Host(op) => self.parity_host_op(window, op),
            Step::Restore => {
                let started = self.downloads.started();
                if let Some(run) = self.checks.parity.as_mut() {
                    run.downloads_before = started;
                }
                self.parity_host_op(window, HostOp::OnlyFirstTab);
                self.parity_host_op(window, HostOp::ZoomReset);
                let home = crate::page_url(crate::PARITY_PAGE);
                let tab = self.model.visible_tab();
                let away = self
                    .parity_url()
                    .is_some_and(|u| u.split('#').next() != Some(home.as_str()));
                match (tab, away) {
                    (Some(tab), true) => {
                        if let Some(run) = self.parity_run() {
                            run.restoring = true;
                        }
                        self.dispatch(wubble_core::model::Command::Navigate { tab, url: home })
                    }
                    _ => self.parity_step(Input::Loaded),
                }
            }
            Step::Js(_)
            | Step::Choose(_)
            | Step::MenuHas(_)
            | Step::Settle(_)
            | Step::Until(_)
            | Step::Retry(..) => {}
        }
    }

    fn parity_host_op(&mut self, window: &AppWindow, op: HostOp) {
        match op {
            HostOp::CloseBar => {
                if window.get_command_bar_state() != CommandBarState::Closed {
                    self.parity_input(window, Step::Keys("Escape"));
                }
            }
            HostOp::CloseFind => self.find_event(crate::surface::FindEvent::Close),
            HostOp::SetClipboard(text) => {
                self.clipboard
                    .store(window.window(), Buffer::Clipboard, text);
            }
            // As a file from another app arrives: the pointer, then the file.
            HostOp::HoverFile(p) | HostOp::DropFile(p) => {
                use slint::winit_030::winit::event::WindowEvent as W;
                let path = std::env::temp_dir().join("parity-drop.txt");
                if let Err(e) = std::fs::write(&path, "dropped") {
                    eprintln!("smoke: parity: {e}");
                }
                let position = self.parity_at(window, p);
                let scale = f64::from(window.window().scale_factor());
                self.window_event(&W::CursorMoved {
                    device_id: slint::winit_030::winit::event::DeviceId::dummy(),
                    position: slint::winit_030::winit::dpi::LogicalPosition::new(
                        f64::from(position.x),
                        f64::from(position.y),
                    )
                    .to_physical(scale),
                });
                self.window_event(&if matches!(op, HostOp::HoverFile(_)) {
                    W::HoveredFile(path)
                } else {
                    W::DroppedFile(path)
                });
            }
            HostOp::OnlyFirstTab => {
                let order = self.model.order(self.model.active_space());
                if let Some(&first) = order.first() {
                    self.dispatch(wubble_core::model::Command::ActivateTab(first));
                    for &tab in &order[1..] {
                        self.dispatch(wubble_core::model::Command::CloseTab(tab));
                    }
                }
            }
            HostOp::Neutral => {
                let surface = self.model.visible_tab().and_then(|t| self.surfaces.get(&t));
                let menu_open = surface.is_some_and(|s| matches!(s.menu, Menu::Open { .. }));
                let find_open =
                    surface.is_some_and(|s| matches!(s.find, crate::surface::FindBar::Open { .. }));
                if menu_open {
                    self.menu_dismissed();
                }
                if find_open {
                    self.parity_host_op(window, HostOp::CloseFind);
                }
                self.parity_host_op(window, HostOp::CloseBar);
                if let Some(p) = self.parity_run().and_then(|r| r.held) {
                    self.parity_input(window, Step::Release(p));
                }
                for p in [parity::at::NEUTRAL_A, parity::at::NEUTRAL] {
                    self.parity_input(window, Step::Click(Button::Left, parity::NONE, p));
                }
            }
            HostOp::ZoomReset => {
                if let Some(view) = self.view()
                    && view.zoom_percent() != 100
                {
                    view.set_zoom_percent(100);
                }
            }
        }
    }

    /// The answer to `check` now.
    fn parity_check(&mut self, window: &AppWindow, check: HostCheck) -> Result<(), String> {
        let is = |ok: bool, what: String| if ok { Ok(()) } else { Err(what) };
        let order = self.model.order(self.model.active_space());
        match check {
            // What the compositor offers the window now: the system's clipboard, read
            // the way a paste reads it. (`wl-paste` and `wl-copy` would take keyboard
            // focus to do it, and the compositor takes selections only from the
            // focused client.)
            HostCheck::Clipboard(want) => {
                let got = self.clipboard.load(window.window(), Buffer::Clipboard);
                is(
                    got.as_deref() == Some(want),
                    format!("clipboard holds {got:?}"),
                )
            }
            HostCheck::Primary(want) => {
                let got = self.clipboard.load(window.window(), Buffer::Primary);
                is(
                    got.as_deref() == Some(want),
                    format!("primary selection holds {got:?}"),
                )
            }
            HostCheck::ClipboardImage => Err("the clipboard bridge carries text only".into()),
            HostCheck::BarOpen(want) => {
                let open = window.get_command_bar_state() == CommandBarState::Open;
                is(open == want, format!("command bar open: {open}"))
            }
            HostCheck::FindOpen(want) => {
                let open = self
                    .model
                    .visible_tab()
                    .and_then(|t| self.surfaces.get(&t))
                    .is_some_and(|s| s.find != crate::surface::FindBar::default());
                is(open == want, format!("find bar open: {open}"))
            }
            HostCheck::Zoomed(above) => {
                let percent = self.view().map_or(0, |v| v.zoom_percent());
                is(
                    (percent > 100) == above && (above || percent == 100),
                    format!("zoom {percent}%"),
                )
            }
            HostCheck::Tabs(n) => is(order.len() == n, format!("{} tabs", order.len())),
            HostCheck::Visible(i) => {
                let at = self
                    .model
                    .visible_tab()
                    .and_then(|t| order.iter().position(|&o| o == t));
                is(at == Some(i), format!("visible tab at {at:?}"))
            }
            HostCheck::Downloads(n) => {
                let before = self
                    .checks
                    .parity
                    .as_ref()
                    .map_or(0, |r| r.downloads_before);
                let have = self.downloads.started() - before;
                is(have >= n, format!("{have} downloads"))
            }
            HostCheck::Cursor(want) => {
                let got = window.get_web_cursor();
                is(got == want, format!("cursor {got}"))
            }
            HostCheck::Autoscroll(want) => {
                use wubble_core::autoscroll::Autoscroll as A;
                let got = match self.autoscroll_state() {
                    A::Off => None,
                    A::Still { .. } => Some(None),
                    A::Moving { direction, .. } => Some(Some(direction)),
                };
                is(got == want, format!("autoscroll {got:?}"))
            }
            HostCheck::AutoscrollDown => {
                use wubble_core::autoscroll::{Autoscroll as A, Direction as D};
                let state = self.autoscroll_state();
                let down = matches!(
                    state,
                    A::Moving {
                        direction: D::S | D::SE | D::SW,
                        ..
                    }
                );
                is(down, format!("autoscroll {state:?}"))
            }
            HostCheck::AutoscrollStretched => {
                let stretch = window.get_autoscroll_stretch();
                is(stretch > 1.05, format!("autoscroll stretch {stretch}"))
            }
            HostCheck::Drag(want) => {
                let dragging = !self.drag_idle();
                is(dragging == want, format!("page drag running: {dragging}"))
            }
            HostCheck::Tooltip(_) => Err("no tooltip support".into()),
            HostCheck::Status(_) => Err("no status bubble".into()),
            HostCheck::External(_) => Err("no external protocol handling".into()),
        }
    }
}
