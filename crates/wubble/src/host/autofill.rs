//! The host's half of password autofill: the content script's frame
//! messages in (`PageEvent::FrameMessage`), through the pure per-tab machine
//! (`wubble_core::autofill`), effects out to the vault on the store thread,
//! the page (`View::send_frame_message`) and the shell (the key chip, its
//! menu and the save prompt, written by [`Host::render_autofill`]).
//!
//! The origin a report speaks for is the frame's as CEF names it, checked
//! against the tab's page (`autofill::may_report`): a cross-origin iframe is
//! never offered a login and never saved. A fill happens only on a click on
//! a menu row, only into the frame and fields the offer named, and only while
//! the page is still on the origin the login was saved for (checked again
//! here when the vault answers, and by the script against its own
//! `location.origin`). Passwords cross threads as [`Secret`]s and are never
//! logged; none of this is part of the saved session.

use std::collections::HashMap;

use slint::{ModelRc, SharedString, VecModel};
use wubble_core::apply::PasswordSettings;
use wubble_core::autofill::{
    self, AutofillState, Candidate, Effect, Event, FieldId, MenuRow, Policy, SaveKind, SaveVerdict,
    ScriptMessage, View,
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
}

impl Autofill {
    /// With the default settings; the stored ones arrive through `Applied::Passwords`.
    pub(super) fn new() -> Self {
        Self {
            tabs: HashMap::new(),
            settings: PasswordSettings::default(),
            drawn: None,
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
        let FrameInfo { id, is_main, .. } = frame;
        let event = match message {
            ScriptMessage::Forms(forms) => Event::Forms {
                frame: id,
                origin,
                forms,
            },
            ScriptMessage::Submit(form) => Event::Submit {
                frame: id,
                origin,
                form,
            },
            ScriptMessage::Gone(form) => Event::Gone {
                frame: id,
                origin,
                form,
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
        let policy = self.autofill_policy(tab);
        let state = self.autofill.tabs.remove(&tab).unwrap_or_default();
        let (next, effects) = autofill::transition(state, event, policy);
        if next != AutofillState::Idle {
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

    fn autofill_policy(&mut self, tab: TabId) -> Policy {
        let origin = self.page_url(tab).and_then(|u| autofill::web_origin(&u));
        let never_save = origin.is_some_and(|o| {
            self.site_permissions.get(&o, Permission::SavePasswords) == Some(Decision::Block)
        });
        Policy {
            autofill: self.autofill.settings.autofill,
            offer_to_save: self.autofill.settings.offer_to_save,
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
            } => self.store.vault(move |vault| {
                let id = RecordId(record);
                let Ok(password) = vault.reveal(&id) else {
                    return;
                };
                let _ = vault.mark_used(&id, now_ms());
                let fields: Vec<(FieldId, Secret)> = username_field
                    .map(|f| (f, Secret::new(username)))
                    .into_iter()
                    .chain([(password_field, Secret::new(password.as_str()))])
                    .collect();
                let _ = slint::invoke_from_event_loop(move || {
                    with_host(|host| host.autofill_fill(tab, &frame, &origin, &fields));
                });
            }),
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
                self.autofill_fill(tab, &frame, &origin, &fields);
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
                    eprintln!("wubble: the password was not saved: {e}");
                }
            }),
            Effect::Update { record, creds } => self.store.vault(move |vault| {
                let id = RecordId(record);
                if let Err(e) = vault.update(&id, &creds.username, creds.password.expose()) {
                    eprintln!("wubble: the password was not updated: {e}");
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

    /// Hands `fields` to the script in `frame`, if the tab is still on `origin`.
    fn autofill_fill(
        &mut self,
        tab: TabId,
        frame: &str,
        origin: &str,
        fields: &[(FieldId, Secret)],
    ) {
        let here = self.page_url(tab).and_then(|u| autofill::web_origin(&u));
        if here.as_deref() != Some(origin) {
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
        window.set_autofill(shell_view(&view));
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

    /// One step of the smoke with what the visible tab shows now.
    pub(super) fn autofill_smoke_worked(&mut self) {
        if self.checks.autofill_smoke.is_none() {
            return;
        }
        let view = self
            .shown()
            .and_then(|t| self.autofill.tabs.get(&t))
            .map(autofill::view)
            .unwrap_or_default();
        let chip = view.chip
            && matches!(
                self.shown().and_then(|t| self.autofill.tabs.get(&t)),
                Some(AutofillState::Offer(o)) if o.candidates.len() == 1
                    && o.candidates[0].username == autofill_smoke::SEEDED_USER
            );
        let save_prompt = matches!(
            view.prompt,
            Some((SaveKind::New, ref user, _)) if user == autofill_smoke::NEW_USER
        );
        self.autofill_smoke_step(autofill_smoke::Input::Worked { chip, save_prompt });
    }

    pub(super) fn autofill_smoke_step(&mut self, input: autofill_smoke::Input) {
        let Some(state) = self.checks.autofill_smoke.take() else {
            return;
        };
        let (state, actions): (AutofillSmoke, _) = autofill_smoke::next(state, input);
        self.checks.autofill_smoke = Some(state);
        for action in actions {
            match action {
                autofill_smoke::Action::Pick => {
                    // As the chip and the row would.
                    self.autofill_menu(-1);
                    self.autofill_menu(0);
                }
                autofill_smoke::Action::Submit => {
                    let pane = self.shown().map_or(-1, crate::shell::pane_id);
                    let at = autofill_smoke::SUBMIT_AT;
                    let left = slint::platform::PointerEventButton::Left;
                    self.pointer_moved(pane, at);
                    self.pointer_button(pane, left, true, at);
                    self.pointer_button(pane, left, false, at);
                }
                autofill_smoke::Action::Save => self.autofill_answer(AutofillAnswer::Save),
                autofill_smoke::Action::ReadBack => {
                    let Some(origin) = self
                        .shown()
                        .and_then(|t| self.page_url(t))
                        .and_then(|u| Origin::parse(&u).ok())
                    else {
                        self.fail("the smoke page has no origin".into());
                        continue;
                    };
                    self.store.vault(move |vault| {
                        let rows = vault.find_for_origin(&origin).unwrap_or_default();
                        let has = |user: &str, password: &str| {
                            rows.iter().any(|c| {
                                c.username == user
                                    && vault.reveal(&c.id).is_ok_and(|p| p.as_str() == password)
                            })
                        };
                        let ok = has(autofill_smoke::NEW_USER, autofill_smoke::NEW_PASSWORD)
                            && has(autofill_smoke::SEEDED_USER, autofill_smoke::SEEDED_PASSWORD);
                        let _ = slint::invoke_from_event_loop(move || {
                            with_host(|host| {
                                host.autofill_smoke_step(autofill_smoke::Input::ReadBack(ok));
                            });
                        });
                    });
                }
                autofill_smoke::Action::Pass => {
                    self.autofill_passed = true;
                    quit();
                }
                autofill_smoke::Action::Fail(reason) => self.fail(reason),
            }
        }
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
fn shell_view(view: &View) -> AutofillView {
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
    let (prompt, title, detail) = match &view.prompt {
        None => (0, "", String::new()),
        Some((kind, user, origin)) => {
            let site = origin.split_once("://").map_or(origin.as_str(), |(_, h)| h);
            let detail = format!("{} on {site}", named(user));
            match kind {
                SaveKind::New => (1, "Save password?", detail),
                SaveKind::Update(_) => (2, "Update password?", detail),
            }
        }
    };
    AutofillView {
        chip: view.chip,
        menu_open: view.menu.is_some(),
        rows: ModelRc::new(VecModel::from(rows)),
        prompt,
        title: title.into(),
        detail: detail.into(),
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
            (shown.prompt, shown.title.as_str(), shown.detail.as_str()),
            (2, "Update password?", "bob on a.test")
        );
    }
}
