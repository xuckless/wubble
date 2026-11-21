//! The host's half of password autofill: the content script's frame
//! messages in (`PageEvent::FrameMessage`), through the pure per-tab machine
//! (`wubble_core::autofill`), effects out to the vault on the store thread,
//! the page (`View::send_frame_message`) and the shell (the key chip, its
//! menu and the save prompt, written by [`Host::render_autofill`]).
//!
//! The origin a report speaks for is the frame's own as CEF names it, on a
//! web page (`autofill::may_report`): a cross-origin iframe is offered only
//! its own origin's logins, and its save prompt names its origin. A fill
//! happens only on a click on a menu row, only into the frame and fields the
//! offer named, only while the tab is still on the page origin it was picked
//! on (checked here when the vault answers) and the frame on the login's
//! origin (checked by the script against its own `location.origin`).
//! Passwords cross threads as [`Secret`]s and are never logged; none of this
//! is part of the saved session.
//!
//! `WUBBLE_AUTOFILL_TRACE=1` (and `--smoke autofill=probe`) prints each
//! report's frame origin, its forms' classes and field kinds, never a value.

use std::collections::HashMap;

use slint::{ComponentHandle, ModelRc, SharedString, VecModel};
use wubble_core::apply::PasswordSettings;
use wubble_core::autofill::{
    self, AutofillState, Candidate, Effect, Event, FieldId, FormSnapshot, MenuRow, Mode, Phase,
    Policy, SaveKind, SaveVerdict, ScriptMessage, View,
};
use wubble_core::secret::Secret;
use wubble_core::site::{Decision, Permission};
use wubble_core::tab::TabId;
use wubble_engine::page::{FrameChannel, FrameInfo};
use wubble_ui::{AppWindow, AutofillAnswer, AutofillView};
use wubble_vault::{GeneratorOptions, NewLogin, Origin, RecordId};
use zeroize::Zeroizing;

use super::{Host, now_ms};
use crate::autofill_smoke::{self, AutofillSmoke};
use crate::persist::HostVault;
use crate::{quit, with_host};

/// Each tab's autofill state (one owner), the settings, and what the shell
/// last drew.
pub(super) struct Autofill {
    tabs: HashMap<TabId, AutofillState>,
    settings: PasswordSettings,
    drawn: Option<(Option<TabId>, View)>,
    /// Print what each report holds (`WUBBLE_AUTOFILL_TRACE`; the probe does too).
    trace: bool,
}

impl Autofill {
    /// With the default settings; the stored ones arrive through `Applied::Passwords`.
    pub(super) fn new() -> Self {
        Self {
            tabs: HashMap::new(),
            settings: PasswordSettings::default(),
            drawn: None,
            trace: std::env::var_os("WUBBLE_AUTOFILL_TRACE").is_some_and(|v| v != "0"),
        }
    }

    pub(super) fn set_settings(&mut self, settings: PasswordSettings) {
        self.settings = settings;
    }
}

impl Host {
    /// A message from the content script in one of `tab`'s frames.
    pub(super) fn autofill_message(&mut self, tab: TabId, frame: FrameInfo, text: &Secret) {
        let Some(message) = autofill::parse_message(text.expose()) else {
            return;
        };
        let Some(origin) = self
            .page_url(tab)
            .and_then(|top| autofill::may_report(&top, &frame.url))
        else {
            return;
        };
        let probe = self
            .checks
            .autofill_smoke
            .as_ref()
            .is_some_and(AutofillSmoke::is_probe);
        if self.autofill.trace || probe {
            trace(&origin, frame.is_main, &message);
        }
        let FrameInfo { id, is_main, .. } = frame;
        let event = match message {
            ScriptMessage::Forms(forms) => Event::Forms {
                frame: id,
                origin,
                is_main,
                forms,
            },
            ScriptMessage::Submit(form) => Event::Submit {
                frame: id,
                origin,
                form,
                at: now_ms(),
            },
            ScriptMessage::Gone(form) => Event::Gone {
                frame: id,
                origin,
                form,
                at: now_ms(),
            },
            ScriptMessage::Settled(forms) => Event::Settled {
                frame: id,
                origin,
                is_main,
                forms,
            },
            // A new main document; an iframe's own start changes nothing.
            ScriptMessage::Start if is_main => Event::Navigated,
            ScriptMessage::Start => return,
        };
        self.autofill_event(tab, event);
    }

    /// The main frame committed `url` (call before `committed` moves on):
    /// a page of another origin ends the offer.
    pub(super) fn autofill_navigated(&mut self, tab: TabId, url: &str) {
        let before = self
            .committed
            .get(&tab)
            .and_then(|u| autofill::web_origin(u));
        if before != autofill::web_origin(url) {
            self.autofill_event(tab, Event::Navigated);
        }
    }

    /// The tab's view is gone: its state goes (and its secrets are wiped).
    pub(super) fn autofill_gone(&mut self, tab: TabId) {
        self.autofill_event(tab, Event::Closed);
    }

    fn autofill_event(&mut self, tab: TabId, event: Event) {
        let policy = self.autofill_policy(tab, event.origin().map(str::to_owned));
        let state = self.autofill.tabs.remove(&tab).unwrap_or_default();
        let (next, effects) = autofill::transition(state, event, policy);
        if next != AutofillState::default() {
            self.autofill.tabs.insert(tab, next);
        }
        for effect in effects {
            self.autofill_effect(tab, effect);
        }
        if let Some(window) = self.w.window.upgrade() {
            self.render_surfaces(&window);
        }
    }

    fn page_url(&mut self, tab: TabId) -> Option<String> {
        self.tab_view(tab).map(|v| v.nav().page.url.clone())
    }

    /// The settings, and "never save" for `origin` (a frame's), else the page's origin.
    fn autofill_policy(&mut self, tab: TabId, origin: Option<String>) -> Policy {
        let origin = origin.or_else(|| self.page_url(tab).and_then(|u| autofill::web_origin(&u)));
        let never_save = origin.is_some_and(|o| {
            self.site_permissions.get(&o, Permission::SavePasswords) == Some(Decision::Block)
        });
        // an agent session's tab: no key menu and no save prompt (its sign-ins come by
        // `fill_secret`; the key menu took the field's focus from it: TODO.md)
        let session = self.agent_grant_of(tab).is_some();
        Policy {
            autofill: self.autofill.settings.autofill && !session,
            offer_to_save: self.autofill.settings.offer_to_save && !session,
            never_save,
        }
    }

    /// Runs `f` on the vault; its event comes back to `tab` on the UI thread.
    fn vault_then(&self, tab: TabId, f: impl FnOnce(&mut HostVault) -> Event + Send + 'static) {
        self.store.vault(move |vault| {
            let event = f(vault);
            let _ = slint::invoke_from_event_loop(move || {
                with_host(|host| host.autofill_event(tab, event));
            });
        });
    }

    fn autofill_effect(&mut self, tab: TabId, effect: Effect) {
        match effect {
            Effect::QueryLogins { origin } => self.vault_then(tab, move |vault| {
                let found = Origin::parse(&origin)
                    .ok()
                    .and_then(|o| vault.find_for_origin(&o).ok())
                    .unwrap_or_default();
                Event::Logins {
                    origin,
                    candidates: found
                        .into_iter()
                        .map(|c| Candidate {
                            id: c.id.0,
                            username: c.username,
                        })
                        .collect(),
                }
            }),
            Effect::Fill {
                frame,
                origin,
                record,
                username,
                username_field,
                password_field,
            } => {
                let top = self.page_origin(tab);
                self.store.vault(move |vault| {
                    let id = RecordId(record);
                    let mut fields: Vec<(FieldId, Secret)> = username_field
                        .map(|f| (f, Secret::new(username)))
                        .into_iter()
                        .collect();
                    // A username step has no password field: nothing is revealed.
                    if let Some(field) = password_field {
                        let Ok(password) = vault.reveal(&id) else {
                            return;
                        };
                        let _ = vault.mark_used(&id, now_ms());
                        fields.push((field, Secret::new(password.as_str())));
                    }
                    let _ = slint::invoke_from_event_loop(move || {
                        with_host(|host| host.autofill_fill(tab, &top, &frame, &origin, &fields));
                    });
                });
            }
            Effect::FillGenerated {
                frame,
                origin,
                fields,
            } => {
                let Ok(password) = wubble_vault::generate(&GeneratorOptions::default()) else {
                    return;
                };
                let fields: Vec<(FieldId, Secret)> = fields
                    .into_iter()
                    .map(|f| (f, Secret::new(password.as_str())))
                    .collect();
                let top = self.page_origin(tab);
                self.autofill_fill(tab, &top, &frame, &origin, &fields);
            }
            Effect::CheckSave { origin, creds } => self.vault_then(tab, move |vault| {
                let verdict = Origin::parse(&origin).ok().and_then(|o| {
                    vault
                        .save_verdict(&o, &creds.username, creds.password.expose())
                        .ok()
                });
                Event::Verdict {
                    origin,
                    creds,
                    verdict: verdict.unwrap_or(SaveVerdict::Unavailable),
                }
            }),
            Effect::Save { origin, creds } => self.store.vault(move |vault| {
                let Ok(origin) = Origin::parse(&origin) else {
                    return;
                };
                let login = NewLogin {
                    origin,
                    form_action_origin: None,
                    username: creds.username.clone(),
                    password: Zeroizing::new(creds.password.expose().to_owned()),
                };
                if let Err(e) = vault.save(login, now_ms()) {
                    vault_failed("saved", &e);
                }
            }),
            Effect::Update { record, creds } => self.store.vault(move |vault| {
                let id = RecordId(record);
                if let Err(e) = vault.update(&id, &creds.username, creds.password.expose()) {
                    vault_failed("updated", &e);
                }
            }),
            Effect::Never { origin } => {
                self.site_permissions
                    .set(&origin, Permission::SavePasswords, Decision::Block);
                self.store.set_site_permission(
                    &origin,
                    Permission::SavePasswords,
                    Decision::Block,
                    now_ms(),
                );
            }
        }
    }

    fn page_origin(&mut self, tab: TabId) -> Option<String> {
        self.page_url(tab).and_then(|u| autofill::web_origin(&u))
    }

    /// Hands `fields` to the script in `frame` (which fills only while its
    /// document is on `origin`), if the tab's page is still on `top`, the
    /// origin it was on when the row was picked.
    fn autofill_fill(
        &mut self,
        tab: TabId,
        top: &Option<String>,
        frame: &str,
        origin: &str,
        fields: &[(FieldId, Secret)],
    ) {
        if top.is_none() || self.page_origin(tab) != *top {
            return;
        }
        let text = fill_message(origin, fields);
        if let Some(view) = self.tab_view(tab) {
            view.send_frame_message(frame, FrameChannel::Autofill, text.expose());
        }
    }

    /// Writes the visible tab's autofill into the shell when it changed (a
    /// rewritten row model would restart the rows' entrance).
    pub(super) fn render_autofill(&mut self, window: &AppWindow, tab: Option<TabId>) {
        let view = tab
            .and_then(|t| self.autofill.tabs.get(&t))
            .map(autofill::view)
            .unwrap_or_default();
        if self.autofill.drawn.as_ref() == Some(&(tab, view.clone())) {
            return;
        }
        // A prompt that closes keeps its text while it leaves.
        let mut shown = shell_view(&view);
        if !shown.prompt {
            let old = window.get_autofill();
            (shown.title, shown.detail, shown.accept) = (old.title, old.detail, old.accept);
        }
        window.set_autofill(shown);
        self.autofill.drawn = Some((tab, view));
    }

    // ----- intents from the shell

    /// The key chip or a menu row: -1 opens the menu, -2 closes it, a row
    /// index chooses it.
    pub fn autofill_menu(&mut self, index: i32) {
        let Some(tab) = self.shown() else {
            return;
        };
        let event = match index {
            -1 => Event::Open,
            -2 => Event::Close,
            i => match usize::try_from(i) {
                Ok(i) => Event::Choose(i),
                Err(_) => return,
            },
        };
        self.autofill_event(tab, event);
    }

    /// A button of the save prompt.
    pub fn autofill_answer(&mut self, answer: AutofillAnswer) {
        let Some(tab) = self.shown() else {
            return;
        };
        let answer = match answer {
            AutofillAnswer::Save => autofill::Answer::Save,
            AutofillAnswer::Never => autofill::Answer::Never,
            AutofillAnswer::NotNow => autofill::Answer::NotNow,
        };
        self.autofill_event(tab, Event::Answer(answer));
    }

    // ----- `--smoke autofill`

    /// One step of the smoke with what the visible tab shows now; `painted`: this run put a
    /// new frame on screen.
    pub(super) fn autofill_smoke_worked(&mut self, painted: bool) {
        if self.checks.autofill_smoke.is_none() {
            return;
        }
        let state = self.shown().and_then(|t| self.autofill.tabs.get(&t));
        let offer = state.and_then(|s| match &s.phase {
            Phase::Offer(o) if !o.rows().is_empty() => Some(autofill_smoke::OfferView {
                rows: o
                    .rows()
                    .into_iter()
                    .map(|r| match r {
                        MenuRow::Login { username } => username,
                        MenuRow::Generate => autofill_smoke::GENERATE.to_owned(),
                    })
                    .collect(),
                password: o.fills_password(),
            }),
            _ => None,
        });
        let prompt = state
            .and_then(|s| autofill::view(s).prompt)
            .map(|(kind, user, origin)| autofill_smoke::PromptView {
                update: matches!(kind, SaveKind::Update(_)),
                user,
                origin,
            });
        self.autofill_smoke_step(autofill_smoke::Input::Worked {
            offer,
            prompt,
            painted,
        });
    }

    pub(super) fn autofill_smoke_step(&mut self, input: autofill_smoke::Input) {
        let Some(state) = self.checks.autofill_smoke.take() else {
            return;
        };
        let (state, actions): (AutofillSmoke, _) = autofill_smoke::next(state, input);
        self.checks.autofill_smoke = Some(state);
        for action in actions {
            self.autofill_smoke_act(action);
        }
    }

    fn autofill_smoke_act(&mut self, action: autofill_smoke::Action) {
        use autofill_smoke::Action;
        use slint::platform::WindowEvent;
        match action {
            Action::Open(url) => {
                if let Some(tab) = self.shown() {
                    self.dispatch(wubble_core::model::Command::Navigate { tab, url });
                }
            }
            Action::Pick(row) => {
                // As the chip and the row would.
                self.autofill_menu(-1);
                self.autofill_menu(i32::try_from(row).unwrap_or(-2));
            }
            Action::Click(at) => {
                // Through the window, as a user's click: the page takes the shell's focus.
                let Some(window) = self.w.window.upgrade() else {
                    return;
                };
                let (x, y) = self.focus_origin(&window);
                let position = slint::LogicalPosition::new(x + at.x, y + at.y);
                let button = slint::platform::PointerEventButton::Left;
                let weak = self.w.window.clone();
                let _ = slint::invoke_from_event_loop(move || {
                    let Some(window) = weak.upgrade() else { return };
                    with_host(|h| h.input.pointer = Some(position));
                    for event in [
                        WindowEvent::PointerMoved { position },
                        WindowEvent::PointerPressed { position, button },
                        WindowEvent::PointerReleased { position, button },
                    ] {
                        window.window().dispatch_event(event);
                    }
                });
            }
            Action::Enter => {
                // The smoke window may not be the desktop's focused one; a user's is.
                if let Some(window) = self.w.window.upgrade()
                    && !self.input.window_focused
                {
                    self.input.window_focused = true;
                    self.sync_focus(&window);
                }
                let keys = crate::parity::chords("Enter")
                    .unwrap_or_default()
                    .into_iter()
                    .filter_map(crate::parity::key_inputs)
                    .flatten()
                    .collect();
                self.smoke_keys(keys);
            }
            Action::Save => self.autofill_answer(AutofillAnswer::Save),
            Action::ReadBack {
                origin,
                user,
                password,
            } => {
                let Ok(origin) = Origin::parse(&origin) else {
                    self.autofill_smoke_step(autofill_smoke::Input::ReadBack(false));
                    return;
                };
                self.store.vault(move |vault| {
                    let rows = vault.find_for_origin(&origin).unwrap_or_default();
                    let ok = rows.iter().any(|c| {
                        c.username == user
                            && vault.reveal(&c.id).is_ok_and(|p| match &password {
                                autofill_smoke::Want::Is(want) => p.as_str() == want,
                                autofill_smoke::Want::Not(old) => p.as_str() != old,
                            })
                    });
                    let _ = slint::invoke_from_event_loop(move || {
                        with_host(|host| {
                            host.autofill_smoke_step(autofill_smoke::Input::ReadBack(ok));
                        });
                    });
                });
            }
            Action::Print(line) => println!("{line}"),
            Action::Pass => {
                self.autofill_passed = true;
                quit();
            }
            Action::Fail(reason) => self.fail(reason),
        }
    }
}

/// A save the owner asked for did not happen: the log, and a toast that says so plainly (the
/// prompt already closed, as if it had). Runs on the store thread.
fn vault_failed(what: &'static str, e: &wubble_vault::VaultError) {
    eprintln!("wubble: the password was not {what}: {e}");
    let text = format!("The password was not {what}: {e}");
    let _ = slint::invoke_from_event_loop(move || {
        with_host(|host| {
            if let Some(window) = host.w.window.upgrade() {
                host.toast(&window, &text);
            }
        });
    });
}

/// One line per form of a report: the frame's origin, the form's class and
/// its fields' kinds (`*` shown, `-` hidden, `r` read-only), autocomplete,
/// name and id attributes, never a value.
fn trace(origin: &str, is_main: bool, message: &ScriptMessage) {
    let (what, forms): (&str, Vec<&FormSnapshot>) = match message {
        ScriptMessage::Forms(f) => ("forms", f.iter().collect()),
        ScriptMessage::Settled(f) => ("settled", f.iter().collect()),
        ScriptMessage::Submit(f) => ("submit", vec![f]),
        ScriptMessage::Gone(f) => ("gone", vec![f]),
        ScriptMessage::Start => ("start", vec![]),
    };
    eprintln!(
        "autofill-trace: {what} from {origin} (main {is_main}): {} form(s)",
        forms.len()
    );
    for form in forms {
        let mode = match message {
            ScriptMessage::Submit(_) | ScriptMessage::Gone(_) => Mode::Saving,
            _ => Mode::Filling,
        };
        let fields: Vec<String> = form
            .fields
            .iter()
            .map(|f| {
                let shown = match (f.visible, f.readonly) {
                    (_, true) => "r",
                    (true, false) => "*",
                    (false, false) => "-",
                };
                format!(
                    "{}:{:?}{shown}[{}] name={:?} id={:?}",
                    f.id, f.kind, f.autocomplete, f.name, f.id_attr
                )
            })
            .collect();
        eprintln!(
            "autofill-trace:   form {}: {:?} fields {}",
            form.id,
            autofill::classify(form, mode),
            fields.join(" ")
        );
    }
}

/// The script's fill message: `{"origin": .., "fields": [[id, value], ..]}`,
/// written straight into one buffer that becomes the `Secret` (sized up
/// front, so no partial copy is left behind by a reallocation).
pub(super) fn fill_message(origin: &str, fields: &[(FieldId, Secret)]) -> Secret {
    let values: Vec<(FieldId, &str)> = fields.iter().map(|(id, v)| (*id, v.expose())).collect();
    let capacity =
        32 + 6 * origin.len() + values.iter().map(|(_, v)| 16 + 6 * v.len()).sum::<usize>();
    let mut out = Vec::with_capacity(capacity);
    out.extend_from_slice(b"{\"origin\":");
    // Writing to a Vec cannot fail, and both values serialise.
    let _ = serde_json::to_writer(&mut out, origin);
    out.extend_from_slice(b",\"fields\":");
    let _ = serde_json::to_writer(&mut out, &values);
    out.push(b'}');
    Secret::new(String::from_utf8(out).unwrap_or_default())
}

/// What the shell shows for one tab's autofill.
pub(super) fn shell_view(view: &View) -> AutofillView {
    let named = |user: &str| {
        if user.is_empty() {
            "(no username)".to_owned()
        } else {
            user.to_owned()
        }
    };
    let rows: Vec<SharedString> = view
        .menu
        .iter()
        .flatten()
        .map(|row| match row {
            MenuRow::Login { username } => named(username).into(),
            MenuRow::Generate => "Suggest a strong password".into(),
        })
        .collect();
    let (title, detail, accept) = match &view.prompt {
        None => ("", String::new(), ""),
        Some((kind, user, origin)) => {
            let site = origin.split_once("://").map_or(origin.as_str(), |(_, h)| h);
            let detail = format!("{} on {site}", named(user));
            match kind {
                SaveKind::New => ("Save password?", detail, "Save"),
                SaveKind::Update(_) => ("Update password?", detail, "Update"),
            }
        }
    };
    AutofillView {
        chip: view.chip,
        menu_open: view.menu.is_some(),
        rows: ModelRc::new(VecModel::from(rows)),
        prompt: view.prompt.is_some(),
        title: title.into(),
        detail: detail.into(),
        accept: accept.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fill_message_is_the_scripts_json() {
        let text = fill_message(
            "http://127.0.0.1:8080",
            &[(3, Secret::from("alice")), (4, Secret::from("p\"w\\"))],
        );
        let v: serde_json::Value = serde_json::from_str(text.expose()).unwrap();
        assert_eq!(v["origin"], "http://127.0.0.1:8080");
        assert_eq!(
            v["fields"],
            serde_json::json!([[3, "alice"], [4, "p\"w\\"]])
        );
    }

    #[test]
    fn the_shell_view_names_rows_and_the_prompt() {
        let view = View {
            chip: true,
            menu: Some(vec![
                MenuRow::Login {
                    username: String::new(),
                },
                MenuRow::Generate,
            ]),
            prompt: None,
        };
        let shown = shell_view(&view);
        assert!(shown.chip && shown.menu_open);
        let rows: Vec<String> = slint::Model::iter(&shown.rows).map(Into::into).collect();
        assert_eq!(rows, ["(no username)", "Suggest a strong password"]);
        let prompt = View {
            prompt: Some((
                SaveKind::Update("r".into()),
                "bob".into(),
                "https://a.test".into(),
            )),
            ..View::default()
        };
        let shown = shell_view(&prompt);
        assert_eq!(
            (
                shown.prompt,
                shown.title.as_str(),
                shown.detail.as_str(),
                shown.accept.as_str()
            ),
            (true, "Update password?", "bob on a.test", "Update")
        );
    }
}
