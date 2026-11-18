//! Password autofill: what the content script saw, which fields of a form
//! are the username and the passwords, and each tab's lifecycle from a form
//! on the page to a fill or a save prompt. Pure: the host
//! (`crates/wubble/src/autofill.rs`) runs the effects against the vault and
//! the page.
//!
//! The content script (`crates/wubble-helper/src/autofill.js`) only reports
//! snapshots and fills; every decision is made here, once.
//!
//! Classification is a port of Chromium's form parser,
//! `components/password_manager/core/browser/form_parsing/form_data_parser.cc`
//! (`ParseAutocomplete`, `IsNotPasswordField`, `IsProbablyNotUsername`,
//! `ParseUsingAutocomplete`, `GetRelevantPasswords`,
//! `SignificantFields::LocateSpecificPasswords`,
//! `FindUsernameFieldBaseHeuristics`, `ParseUsingBaseHeuristics`),
//! BSD-3-Clause, simplified: no server predictions, no single-username
//! forms. See `THIRD_PARTY_NOTICES.md`.
//!
//! Secrets: values travel as [`Secret`] (zeroized on drop, never printed).
//! Nothing here is part of the saved session.

use serde_json::Value;

use crate::secret::Secret;

/// A field's id, unique in its document (the script assigns it).
pub type FieldId = u32;

/// At most this many forms, and fields per form, are read from one message.
pub const MAX_FORMS: usize = 32;
pub const MAX_FIELDS: usize = 128;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FieldKind {
    Text,
    Email,
    Tel,
    Password,
    Other,
}

impl FieldKind {
    fn parse(s: &str) -> Self {
        match s {
            "text" | "search" => Self::Text,
            "email" => Self::Email,
            "tel" => Self::Tel,
            "password" => Self::Password,
            _ => Self::Other,
        }
    }

    /// Could hold a username.
    fn text_like(self) -> bool {
        matches!(self, Self::Text | Self::Email | Self::Tel)
    }
}

/// One `<input>` as the script saw it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldSnapshot {
    pub id: FieldId,
    pub kind: FieldKind,
    pub name: String,
    /// The element's `id` attribute.
    pub id_attr: String,
    pub autocomplete: String,
    /// Rendered and not hidden.
    pub visible: bool,
    /// `readonly` or `disabled`.
    pub readonly: bool,
    /// Sent only in `submit` and `gone` snapshots.
    pub value: Secret,
}

/// A `<form>`, or the page's password fields outside any form (id 0).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FormSnapshot {
    pub id: u32,
    pub fields: Vec<FieldSnapshot>,
}

/// What the content script reports.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ScriptMessage {
    /// The page's forms with a password field (on load and on change).
    Forms(Vec<FormSnapshot>),
    /// A form was submitted (form submit, Enter, a click on its submit control), with values.
    Submit(FormSnapshot),
    /// The last submitted form's password field left the page (a single-page login).
    Gone(FormSnapshot),
    /// The document finished loading; its forms then.
    Settled(Vec<FormSnapshot>),
}

/// The script's JSON, or `None` for anything else.
pub fn parse_message(text: &str) -> Option<ScriptMessage> {
    let v: Value = serde_json::from_str(text).ok()?;
    let forms = |v: &Value| -> Option<Vec<FormSnapshot>> {
        Some(
            v.get("forms")?
                .as_array()?
                .iter()
                .take(MAX_FORMS)
                .filter_map(parse_form)
                .collect(),
        )
    };
    match v.get("t")?.as_str()? {
        "forms" => Some(ScriptMessage::Forms(forms(&v)?)),
        "settled" => Some(ScriptMessage::Settled(forms(&v)?)),
        "submit" => Some(ScriptMessage::Submit(parse_form(v.get("form")?)?)),
        "gone" => Some(ScriptMessage::Gone(parse_form(v.get("form")?)?)),
        _ => None,
    }
}

fn parse_form(v: &Value) -> Option<FormSnapshot> {
    Some(FormSnapshot {
        id: u32::try_from(v.get("id")?.as_u64()?).ok()?,
        fields: v
            .get("fields")?
            .as_array()?
            .iter()
            .take(MAX_FIELDS)
            .filter_map(parse_field)
            .collect(),
    })
}

fn parse_field(v: &Value) -> Option<FieldSnapshot> {
    let text = |key: &str| v.get(key).and_then(Value::as_str).unwrap_or("").to_owned();
    let flag = |key: &str| v.get(key).and_then(Value::as_bool).unwrap_or(false);
    Some(FieldSnapshot {
        id: u32::try_from(v.get("id")?.as_u64()?).ok()?,
        kind: FieldKind::parse(v.get("type")?.as_str()?),
        name: text("name"),
        id_attr: text("idAttr"),
        autocomplete: text("autocomplete"),
        visible: flag("visible"),
        readonly: flag("readonly"),
        value: Secret::new(text("value")),
    })
}

// ------------------------------------------------------------ classification

/// Chromium's `FormDataParser::Mode`: filling looks at the page as it is,
/// saving at what the user submitted (empty fields do not count).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Filling,
    Saving,
}

/// What a form is for, and which of its fields matter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FormClass {
    NotAForm,
    Login {
        username: Option<FieldId>,
        password: FieldId,
    },
    SignUp {
        username: Option<FieldId>,
        new_password: FieldId,
        confirm: Option<FieldId>,
    },
    Change {
        username: Option<FieldId>,
        current: FieldId,
        new_password: FieldId,
        confirm: Option<FieldId>,
    },
}

/// The autocomplete token that decides (Chromium `ParseAutocomplete`): the
/// last one, ignoring `webauthn`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Hint {
    None,
    Username,
    CurrentPassword,
    NewPassword,
    OneTimeCode,
    CreditCard,
}

fn hint(autocomplete: &str) -> Hint {
    let last = autocomplete
        .split_ascii_whitespace()
        .map(str::to_ascii_lowercase)
        .rfind(|t| t != "webauthn");
    match last.as_deref() {
        Some("username") => Hint::Username,
        Some("current-password") => Hint::CurrentPassword,
        Some("new-password") => Hint::NewPassword,
        Some("one-time-code") => Hint::OneTimeCode,
        Some(t) if t.starts_with("cc-") => Hint::CreditCard,
        _ => Hint::None,
    }
}

/// Names that mark a password-typed field as something else: one-time
/// codes, card security codes, social security numbers (Chromium
/// `IsNotPasswordField` plus its OTP and CVC name rules).
const NOT_PASSWORD_NAMES: [&str; 9] = [
    "otp",
    "onetime",
    "one-time",
    "one_time",
    "cvc",
    "cvv",
    "csc",
    "ssn",
    "verification",
];

fn is_not_password(f: &FieldSnapshot) -> bool {
    if matches!(
        hint(&f.autocomplete),
        Hint::OneTimeCode | Hint::CreditCard | Hint::Username
    ) {
        return true;
    }
    let name = f.name.to_ascii_lowercase();
    let id = f.id_attr.to_ascii_lowercase();
    NOT_PASSWORD_NAMES
        .iter()
        .any(|n| name.contains(n) || id.contains(n))
}

/// Chromium `IsProbablyNotUsername`.
fn probably_not_username(value: &str) -> bool {
    value.is_empty() || (value.chars().count() < 3 && value.chars().all(|c| c.is_ascii_digit()))
}

/// A password field the parser may use in `mode`.
fn usable_password(f: &FieldSnapshot, mode: Mode) -> bool {
    f.kind == FieldKind::Password
        && !f.readonly
        && !is_not_password(f)
        && (mode == Mode::Filling || !f.value.is_empty())
}

/// Chromium `GetRelevantPasswords`: the usable password fields, the visible
/// ones if there are any. Filling never uses a hidden field.
fn relevant_passwords(form: &FormSnapshot, mode: Mode) -> Vec<&FieldSnapshot> {
    let usable: Vec<&FieldSnapshot> = form
        .fields
        .iter()
        .filter(|f| usable_password(f, mode))
        .collect();
    let visible: Vec<&FieldSnapshot> = usable.iter().copied().filter(|f| f.visible).collect();
    if !visible.is_empty() || mode == Mode::Filling {
        visible
    } else {
        usable
    }
}

/// (current, new, confirmation) from the relevant passwords, by their
/// count and which values repeat (Chromium `LocateSpecificPasswords`).
type Located<'a> = (
    Option<&'a FieldSnapshot>,
    Option<&'a FieldSnapshot>,
    Option<&'a FieldSnapshot>,
);

fn locate<'a>(passwords: &[&'a FieldSnapshot]) -> Located<'a> {
    let v = |i: usize| passwords[i].value.expose();
    match passwords.len() {
        0 => (None, None, None),
        1 => (Some(passwords[0]), None, None),
        2 => {
            if !v(0).is_empty() && v(0) == v(1) {
                // Two identical: a new password and its confirmation.
                (None, Some(passwords[0]), Some(passwords[1]))
            } else {
                (Some(passwords[0]), Some(passwords[1]), None)
            }
        }
        // More than three: only the first three count.
        _ => {
            if !v(0).is_empty() && v(0) == v(1) && v(0) == v(2) {
                // All the same: old and new may be equal; any one will do.
                (Some(passwords[0]), None, None)
            } else if v(1) == v(2) {
                // The repeated one is the new password, second.
                (Some(passwords[0]), Some(passwords[1]), Some(passwords[2]))
            } else if v(0) == v(1) {
                // New first: trust the repetition over the order.
                (None, Some(passwords[0]), Some(passwords[1]))
            } else {
                // No idea: save the first.
                (Some(passwords[0]), None, None)
            }
        }
    }
}

/// Chromium `FindUsernameFieldBaseHeuristics`: the nearest text field before
/// the first password, editable and visible; when saving, with a value that
/// looks like a username (a hidden one as the last resort).
fn username_before(form: &FormSnapshot, first_password: FieldId, mode: Mode) -> Option<FieldId> {
    let end = form.fields.iter().position(|f| f.id == first_password)?;
    let before = &form.fields[..end];
    let candidate = |f: &&FieldSnapshot| {
        f.kind.text_like()
            && !matches!(hint(&f.autocomplete), Hint::OneTimeCode | Hint::CreditCard)
            && (mode == Mode::Filling || !probably_not_username(f.value.expose()))
    };
    let editable = before
        .iter()
        .rev()
        .filter(candidate)
        .find(|f| f.visible && !f.readonly);
    match (editable, mode) {
        (Some(f), _) => Some(f.id),
        (None, Mode::Saving) => before.iter().rev().find(candidate).map(|f| f.id),
        (None, Mode::Filling) => None,
    }
}

fn class_of(username: Option<FieldId>, (current, new, confirm): Located) -> FormClass {
    match (current, new) {
        (Some(c), Some(n)) => FormClass::Change {
            username,
            current: c.id,
            new_password: n.id,
            confirm: confirm.map(|f| f.id),
        },
        (Some(c), None) => FormClass::Login {
            username,
            password: c.id,
        },
        (None, Some(n)) => FormClass::SignUp {
            username,
            new_password: n.id,
            confirm: confirm.map(|f| f.id),
        },
        (None, None) => FormClass::NotAForm,
    }
}

/// What `form` is for (Chromium `ParseUsingAutocomplete`, then
/// `ParseUsingBaseHeuristics` for what the page's autocomplete left open).
pub fn classify(form: &FormSnapshot, mode: Mode) -> FormClass {
    // Autocomplete first: the page said which field is which.
    let mut username = None;
    let (mut current, mut new, mut confirm) = (None, None, None);
    for f in &form.fields {
        match hint(&f.autocomplete) {
            Hint::Username if f.kind.text_like() && username.is_none() => {
                let ok = match mode {
                    Mode::Filling => f.visible && !f.readonly,
                    Mode::Saving => !probably_not_username(f.value.expose()),
                };
                if ok {
                    username = Some(f.id);
                }
            }
            Hint::CurrentPassword if usable_password(f, mode) && current.is_none() => {
                current = Some(f);
            }
            Hint::NewPassword if usable_password(f, mode) => {
                if new.is_none() {
                    new = Some(f);
                } else if confirm.is_none() {
                    confirm = Some(f);
                }
            }
            _ => {}
        }
    }
    // Filling never touches a hidden field.
    let shown = |f: &&FieldSnapshot| mode == Mode::Saving || f.visible;
    let (current, new, confirm) = (
        current.filter(shown),
        new.filter(shown),
        confirm.filter(shown),
    );
    let located = if current.is_some() || new.is_some() {
        (current, new, confirm)
    } else {
        locate(&relevant_passwords(form, mode))
    };
    let Some(first) = [located.0, located.1]
        .into_iter()
        .flatten()
        .map(|f| f.id)
        .min_by_key(|id| form.fields.iter().position(|f| f.id == *id))
    else {
        return FormClass::NotAForm;
    };
    let username = username.or_else(|| username_before(form, first, mode));
    class_of(username, located)
}

/// The fields a picked login fills: (username, password).
pub fn fill_target(class: FormClass) -> Option<(Option<FieldId>, FieldId)> {
    match class {
        FormClass::Login { username, password } => Some((username, password)),
        FormClass::Change {
            username, current, ..
        } => Some((username, current)),
        FormClass::SignUp { .. } | FormClass::NotAForm => None,
    }
}

/// The fields a generated password fills: the new one and its confirmation.
pub fn generate_target(class: FormClass) -> Vec<FieldId> {
    match class {
        FormClass::SignUp {
            new_password,
            confirm,
            ..
        }
        | FormClass::Change {
            new_password,
            confirm,
            ..
        } => std::iter::once(new_password).chain(confirm).collect(),
        FormClass::Login { .. } | FormClass::NotAForm => vec![],
    }
}

/// A username and the password to keep for it.
#[derive(Clone, PartialEq, Eq)]
pub struct Creds {
    pub username: String,
    pub password: Secret,
}

impl std::fmt::Debug for Creds {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Creds(..)")
    }
}

/// What the user submitted in `form`: the username and the password that
/// is now theirs (the new one on a sign-up or change form).
pub fn submitted(form: &FormSnapshot) -> Option<Creds> {
    let value = |id: FieldId| form.fields.iter().find(|f| f.id == id).map(|f| &f.value);
    let (username, password) = match classify(form, Mode::Saving) {
        FormClass::NotAForm => return None,
        FormClass::Login { username, password } => (username, password),
        FormClass::SignUp {
            username,
            new_password,
            ..
        }
        | FormClass::Change {
            username,
            new_password,
            ..
        } => (username, new_password),
    };
    Some(Creds {
        username: username
            .and_then(value)
            .map(|s| s.expose().to_owned())
            .unwrap_or_default(),
        password: value(password)?.clone(),
    })
}

/// The form to offer on: the first a saved login can fill, else the first
/// sign-up form.
fn offered_form(forms: &[FormSnapshot]) -> Option<(u32, FormClass)> {
    let classes: Vec<(u32, FormClass)> = forms
        .iter()
        .map(|f| (f.id, classify(f, Mode::Filling)))
        .collect();
    classes
        .iter()
        .find(|(_, c)| fill_target(*c).is_some())
        .or_else(|| {
            classes
                .iter()
                .find(|(_, c)| !generate_target(*c).is_empty())
        })
        .copied()
}

/// A sign-in form is still on the page (a failed login shows it again).
fn has_login_form(forms: &[FormSnapshot]) -> bool {
    forms
        .iter()
        .any(|f| fill_target(classify(f, Mode::Filling)).is_some())
}

// ------------------------------------------------------------ origin rules

/// `url`'s origin as `scheme://host[:port]` (default port left out), for
/// http and https only: the form the vault stores (`wubble_vault::Origin`).
pub fn web_origin(url: &str) -> Option<String> {
    let url = url::Url::parse(url).ok()?;
    matches!(url.scheme(), "http" | "https").then(|| url.origin().ascii_serialization())
}

/// Whether a frame's report may be acted on, and the origin it speaks for:
/// the frame's origin must be the top-level page's (no cross-origin iframe,
/// no opaque origin, http or https only).
pub fn may_report(top_url: &str, frame_url: &str) -> Option<String> {
    let top = web_origin(top_url)?;
    (web_origin(frame_url)? == top).then_some(top)
}

/// Whether a login saved for `login_origin` may be filled into a frame:
/// the report rule, and the exact origin of the login (so an https login
/// never fills on http, and a subdomain never matches).
pub fn may_fill(top_url: &str, frame_url: &str, login_origin: &str) -> bool {
    may_report(top_url, frame_url).is_some_and(|o| Some(o) == web_origin(login_origin))
}

// ------------------------------------------------------------ per-tab state

/// A saved login offered for a form (no password).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    /// The vault's record id.
    pub id: String,
    pub username: String,
}

/// A form the key chip offers to fill.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Offer {
    pub frame: String,
    pub origin: String,
    pub form: u32,
    pub class: FormClass,
    /// The vault's logins for the origin; empty until it answered.
    pub candidates: Vec<Candidate>,
}

/// A row of the key menu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MenuRow {
    Login { username: String },
    Generate,
}

impl Offer {
    pub fn rows(&self) -> Vec<MenuRow> {
        let logins = fill_target(self.class).map_or(&[][..], |_| &self.candidates[..]);
        let mut rows: Vec<MenuRow> = logins
            .iter()
            .map(|c| MenuRow::Login {
                username: c.username.clone(),
            })
            .collect();
        if !generate_target(self.class).is_empty() {
            rows.push(MenuRow::Generate);
        }
        rows
    }
}

/// Submitted credentials and where they came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pending {
    pub frame: String,
    pub origin: String,
    pub creds: Creds,
}

/// What the vault already holds for submitted credentials.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SaveVerdict {
    New,
    Same,
    /// That username is saved, with another password: this record.
    Differs(String),
    /// The vault cannot be used.
    Unavailable,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SaveKind {
    New,
    Update(String),
}

/// One tab's autofill lifecycle.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum AutofillState {
    #[default]
    Idle,
    /// A form the key chip offers to fill or generate for.
    Offer(Offer),
    /// The user opened the key menu.
    Menu(Offer),
    /// A form was submitted; waiting to see whether the sign-in worked.
    Submitted(Pending),
    /// It worked; the vault is asked whether the login is new.
    Checking(Pending),
    /// The save (or update) prompt shows.
    Prompt { kind: SaveKind, pending: Pending },
}

/// The prompt's buttons.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Answer {
    Save,
    /// Never save for this site.
    Never,
    NotNow,
}

/// The settings and the site's "never" decision, as they stand.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Policy {
    /// `passwords.autofill`.
    pub autofill: bool,
    /// `passwords.offer_to_save`.
    pub offer_to_save: bool,
    /// The user chose "Never" for the page's origin.
    pub never_save: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// The script in `frame` reported its forms; the host checked the frame
    /// (`may_report`), `origin` is its origin.
    Forms {
        frame: String,
        origin: String,
        forms: Vec<FormSnapshot>,
    },
    /// The vault's logins for `origin`.
    Logins {
        origin: String,
        candidates: Vec<Candidate>,
    },
    /// The key chip was clicked.
    Open,
    /// The menu closed without a choice.
    Close,
    /// A menu row was clicked (the user gesture every fill needs).
    Choose(usize),
    Submit {
        frame: String,
        origin: String,
        form: FormSnapshot,
    },
    Gone {
        frame: String,
        origin: String,
        form: FormSnapshot,
    },
    Settled {
        frame: String,
        origin: String,
        is_main: bool,
        forms: Vec<FormSnapshot>,
    },
    /// The main frame committed a page of another origin.
    Navigated,
    Verdict(SaveVerdict),
    Answer(Answer),
    /// The tab or its view is gone.
    Closed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effect {
    /// Ask the vault for `origin`'s logins (answered with `Event::Logins`).
    QueryLogins {
        origin: String,
    },
    /// Reveal `record`'s password and fill it, with `username`, into `frame`.
    Fill {
        frame: String,
        origin: String,
        record: String,
        username: String,
        username_field: Option<FieldId>,
        password_field: FieldId,
    },
    /// Generate a password and fill it into `fields` of `frame`.
    FillGenerated {
        frame: String,
        origin: String,
        fields: Vec<FieldId>,
    },
    /// Ask the vault what it holds for these (answered with `Event::Verdict`).
    CheckSave {
        origin: String,
        creds: Creds,
    },
    Save {
        origin: String,
        creds: Creds,
    },
    Update {
        record: String,
        creds: Creds,
    },
    /// Remember "never save" for the origin.
    Never {
        origin: String,
    },
}

type Step = (AutofillState, Vec<Effect>);

/// The one transition of a tab's autofill state.
pub fn transition(state: AutofillState, event: Event, policy: Policy) -> Step {
    use AutofillState as S;
    match (state, event) {
        (_, Event::Closed) => (S::Idle, vec![]),
        (
            state @ (S::Idle | S::Offer(_) | S::Menu(_)),
            Event::Forms {
                frame,
                origin,
                forms,
            },
        ) => offer(state, frame, origin, &forms, false, policy),
        (
            state @ (S::Idle | S::Offer(_) | S::Menu(_)),
            Event::Settled {
                frame,
                origin,
                is_main,
                forms,
            },
        ) => offer(state, frame, origin, &forms, is_main, policy),
        (S::Offer(mut o), Event::Logins { origin, candidates }) if o.origin == origin => {
            o.candidates = candidates;
            (S::Offer(o), vec![])
        }
        (S::Menu(mut o), Event::Logins { origin, candidates }) if o.origin == origin => {
            o.candidates = candidates;
            (S::Menu(o), vec![])
        }
        (S::Offer(o), Event::Open) if !o.rows().is_empty() => (S::Menu(o), vec![]),
        (S::Menu(o), Event::Close) => (S::Offer(o), vec![]),
        (S::Menu(o), Event::Choose(i)) => choose(o, i),
        (
            state,
            Event::Submit {
                frame,
                origin,
                form,
            },
        ) => match pending(frame, origin, &form, policy) {
            Some(p) => (S::Submitted(p), vec![]),
            None => (state, vec![]),
        },
        (
            state,
            Event::Gone {
                frame,
                origin,
                form,
            },
        ) => match pending(frame, origin, &form, policy) {
            Some(p) => check(p),
            None => (state, vec![]),
        },
        (
            S::Submitted(p),
            Event::Settled {
                frame,
                origin,
                is_main,
                forms,
            },
        ) if is_main || frame == p.frame => {
            if origin == p.origin && has_login_form(&forms) {
                // The sign-in form is back: the login failed. Offer it again.
                offer(S::Idle, frame, origin, &forms, is_main, policy)
            } else {
                check(p)
            }
        }
        (S::Offer(_) | S::Menu(_), Event::Navigated) => (S::Idle, vec![]),
        (S::Checking(p), Event::Verdict(verdict)) => match verdict {
            SaveVerdict::New => (
                S::Prompt {
                    kind: SaveKind::New,
                    pending: p,
                },
                vec![],
            ),
            SaveVerdict::Differs(record) => (
                S::Prompt {
                    kind: SaveKind::Update(record),
                    pending: p,
                },
                vec![],
            ),
            SaveVerdict::Same | SaveVerdict::Unavailable => (S::Idle, vec![]),
        },
        (S::Prompt { kind, pending }, Event::Answer(answer)) => {
            let effects = match (answer, kind) {
                (Answer::Save, SaveKind::New) => vec![Effect::Save {
                    origin: pending.origin,
                    creds: pending.creds,
                }],
                (Answer::Save, SaveKind::Update(record)) => vec![Effect::Update {
                    record,
                    creds: pending.creds,
                }],
                (Answer::Never, _) => vec![Effect::Never {
                    origin: pending.origin,
                }],
                (Answer::NotNow, _) => vec![],
            };
            (S::Idle, effects)
        }
        (state, _) => (state, vec![]),
    }
}

/// A report of `forms` in `frame` while nothing is submitted. A main
/// document that settled without a form ends any offer: CEF may give the
/// main frame a new id when a navigation changes process.
fn offer(
    state: AutofillState,
    frame: String,
    origin: String,
    forms: &[FormSnapshot],
    main_settled: bool,
    policy: Policy,
) -> Step {
    use AutofillState as S;
    let current = match &state {
        S::Offer(o) | S::Menu(o) => Some(o),
        _ => None,
    };
    let Some((form, class)) = offered_form(forms).filter(|_| policy.autofill) else {
        // This frame has no form (any more): its offer goes.
        return match current {
            Some(o) if o.frame == frame || main_settled || !policy.autofill => (S::Idle, vec![]),
            _ => (state, vec![]),
        };
    };
    // The same form again (a DOM change): keep the logins and an open menu.
    if let Some(o) = current
        && o.frame == frame
        && o.origin == origin
        && o.form == form
    {
        let o = Offer { class, ..o.clone() };
        return match state {
            S::Menu(_) => (S::Menu(o), vec![]),
            _ => (S::Offer(o), vec![]),
        };
    }
    let query = fill_target(class).map(|_| Effect::QueryLogins {
        origin: origin.clone(),
    });
    let o = Offer {
        frame,
        origin,
        form,
        class,
        candidates: vec![],
    };
    (S::Offer(o), query.into_iter().collect())
}

fn choose(o: Offer, index: usize) -> Step {
    let row = o.rows().get(index).cloned();
    let effect = match row {
        Some(MenuRow::Login { .. }) => fill_target(o.class).and_then(|(user, pass)| {
            let c = o.candidates.get(index)?;
            Some(Effect::Fill {
                frame: o.frame.clone(),
                origin: o.origin.clone(),
                record: c.id.clone(),
                username: c.username.clone(),
                username_field: user,
                password_field: pass,
            })
        }),
        Some(MenuRow::Generate) => Some(Effect::FillGenerated {
            frame: o.frame.clone(),
            origin: o.origin.clone(),
            fields: generate_target(o.class),
        }),
        None => None,
    };
    (AutofillState::Offer(o), effect.into_iter().collect())
}

/// Submitted credentials worth keeping, if saving is on for the site.
fn pending(frame: String, origin: String, form: &FormSnapshot, policy: Policy) -> Option<Pending> {
    if !policy.offer_to_save || policy.never_save {
        return None;
    }
    Some(Pending {
        frame,
        origin,
        creds: submitted(form)?,
    })
}

fn check(p: Pending) -> Step {
    let effect = Effect::CheckSave {
        origin: p.origin.clone(),
        creds: p.creds.clone(),
    };
    (AutofillState::Checking(p), vec![effect])
}

/// What the shell shows for a tab.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct View {
    /// The key chip by the address pill.
    pub chip: bool,
    /// The key menu's rows, while it is open.
    pub menu: Option<Vec<MenuRow>>,
    /// The save prompt: its kind, the username and the origin.
    pub prompt: Option<(SaveKind, String, String)>,
}

pub fn view(state: &AutofillState) -> View {
    match state {
        AutofillState::Offer(o) => View {
            chip: !o.rows().is_empty(),
            ..View::default()
        },
        AutofillState::Menu(o) => View {
            chip: true,
            menu: Some(o.rows()),
            prompt: None,
        },
        AutofillState::Prompt { kind, pending } => View {
            prompt: Some((
                kind.clone(),
                pending.creds.username.clone(),
                pending.origin.clone(),
            )),
            ..View::default()
        },
        AutofillState::Idle | AutofillState::Submitted(_) | AutofillState::Checking(_) => {
            View::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ----- fixtures, the shape `autofill.js` sends

    fn field(id: FieldId, kind: &str) -> FieldSnapshot {
        FieldSnapshot {
            id,
            kind: FieldKind::parse(kind),
            name: String::new(),
            id_attr: String::new(),
            autocomplete: String::new(),
            visible: true,
            readonly: false,
            value: Secret::default(),
        }
    }

    fn text(id: FieldId) -> FieldSnapshot {
        field(id, "text")
    }

    fn pw(id: FieldId) -> FieldSnapshot {
        field(id, "password")
    }

    fn with_value(mut f: FieldSnapshot, v: &str) -> FieldSnapshot {
        f.value = Secret::from(v);
        f
    }

    fn ac(mut f: FieldSnapshot, a: &str) -> FieldSnapshot {
        f.autocomplete = a.into();
        f
    }

    fn named(mut f: FieldSnapshot, n: &str) -> FieldSnapshot {
        f.name = n.into();
        f
    }

    fn hidden(mut f: FieldSnapshot) -> FieldSnapshot {
        f.visible = false;
        f
    }

    fn readonly(mut f: FieldSnapshot) -> FieldSnapshot {
        f.readonly = true;
        f
    }

    fn form(fields: Vec<FieldSnapshot>) -> FormSnapshot {
        FormSnapshot { id: 1, fields }
    }

    fn login(username: Option<FieldId>, password: FieldId) -> FormClass {
        FormClass::Login { username, password }
    }

    // ----- parsing

    #[test]
    fn the_scripts_messages_parse() {
        let forms = r#"{"t":"forms","forms":[{"id":1,"fields":[
            {"id":1,"type":"email","name":"e","idAttr":"mail","autocomplete":"username","visible":true,"readonly":false,"value":""},
            {"id":2,"type":"password","visible":true}]}]}"#;
        let Some(ScriptMessage::Forms(f)) = parse_message(forms) else {
            panic!("forms did not parse");
        };
        assert_eq!(f[0].fields[0].kind, FieldKind::Email);
        assert_eq!(f[0].fields[0].id_attr, "mail");
        assert_eq!(f[0].fields[1].kind, FieldKind::Password);
        let submit = r#"{"t":"submit","form":{"id":3,"fields":[{"id":9,"type":"password","visible":true,"value":"pw"}]}}"#;
        let Some(ScriptMessage::Submit(s)) = parse_message(submit) else {
            panic!("submit did not parse");
        };
        assert_eq!((s.id, s.fields[0].value.expose()), (3, "pw"));
        assert!(matches!(
            parse_message(r#"{"t":"settled","forms":[]}"#),
            Some(ScriptMessage::Settled(f)) if f.is_empty()
        ));
        assert!(matches!(
            parse_message(r#"{"t":"gone","form":{"id":0,"fields":[]}}"#),
            Some(ScriptMessage::Gone(_))
        ));
        for bad in ["", "{}", r#"{"t":"other"}"#, r#"{"t":"submit"}"#, "[1]"] {
            assert_eq!(parse_message(bad), None, "{bad}");
        }
    }

    #[test]
    fn oversized_messages_are_truncated() {
        let fields: Vec<String> = (0..300)
            .map(|i| format!(r#"{{"id":{i},"type":"text"}}"#))
            .collect();
        let msg = format!(
            r#"{{"t":"forms","forms":[{{"id":1,"fields":[{}]}}]}}"#,
            fields.join(",")
        );
        let Some(ScriptMessage::Forms(f)) = parse_message(&msg) else {
            panic!();
        };
        assert_eq!(f[0].fields.len(), MAX_FIELDS);
    }

    // ----- classification

    #[test]
    fn classification_table() {
        use Mode::Filling;
        let cases: Vec<(&str, FormSnapshot, Mode, FormClass)> = vec![
            (
                "text + password",
                form(vec![text(1), pw(2)]),
                Filling,
                login(Some(1), 2),
            ),
            (
                "email + password",
                form(vec![field(1, "email"), pw(2)]),
                Filling,
                login(Some(1), 2),
            ),
            ("password only", form(vec![pw(1)]), Filling, login(None, 1)),
            (
                "username after the password is not the username",
                form(vec![pw(1), text(2)]),
                Filling,
                login(None, 1),
            ),
            (
                "nearest text field before the password",
                form(vec![text(1), text(2), pw(3)]),
                Filling,
                login(Some(2), 3),
            ),
            (
                "hidden and readonly text skipped",
                form(vec![text(1), hidden(text(2)), readonly(text(3)), pw(4)]),
                Filling,
                login(Some(1), 4),
            ),
            (
                "autocomplete username wins over position",
                form(vec![ac(text(1), "username"), text(2), pw(3)]),
                Filling,
                login(Some(1), 3),
            ),
            (
                "autocomplete current-password",
                form(vec![text(1), pw(2), ac(pw(3), "current-password")]),
                Filling,
                login(Some(1), 3),
            ),
            (
                "autocomplete new-password twice: sign-up",
                form(vec![
                    text(1),
                    ac(pw(2), "new-password"),
                    ac(pw(3), "new-password"),
                ]),
                Filling,
                FormClass::SignUp {
                    username: Some(1),
                    new_password: 2,
                    confirm: Some(3),
                },
            ),
            (
                "autocomplete current + new: change",
                form(vec![
                    ac(pw(1), "current-password"),
                    ac(pw(2), "new-password"),
                ]),
                Filling,
                FormClass::Change {
                    username: None,
                    current: 1,
                    new_password: 2,
                    confirm: None,
                },
            ),
            (
                "last token wins, webauthn ignored",
                form(vec![ac(text(1), "section-x username webauthn"), pw(2)]),
                Filling,
                login(Some(1), 2),
            ),
            (
                "one-time-code and cc fields are not passwords",
                form(vec![
                    text(1),
                    ac(pw(2), "one-time-code"),
                    ac(pw(3), "cc-csc"),
                ]),
                Filling,
                FormClass::NotAForm,
            ),
            (
                "otp, cvc, ssn names are not passwords",
                form(vec![
                    named(pw(1), "otp_code"),
                    named(pw(2), "card-cvc"),
                    named(pw(3), "SSN"),
                ]),
                Filling,
                FormClass::NotAForm,
            ),
            (
                "a password next to an otp field",
                form(vec![text(1), pw(2), named(pw(3), "totp")]),
                Filling,
                login(Some(1), 2),
            ),
            (
                "readonly password ignored",
                form(vec![text(1), readonly(pw(2))]),
                Filling,
                FormClass::NotAForm,
            ),
            (
                "hidden password never filled",
                form(vec![text(1), hidden(pw(2)), pw(3)]),
                Filling,
                login(Some(1), 3),
            ),
            (
                "only a hidden password: nothing to fill",
                form(vec![text(1), hidden(pw(2))]),
                Filling,
                FormClass::NotAForm,
            ),
            (
                "no password field",
                form(vec![text(1), text(2)]),
                Filling,
                FormClass::NotAForm,
            ),
        ];
        for (what, f, mode, want) in cases {
            assert_eq!(classify(&f, mode), want, "{what}");
        }
    }

    #[test]
    fn password_counts_follow_chromium() {
        let p = |id, v| with_value(pw(id), v);
        let cases: Vec<(&str, Vec<FieldSnapshot>, FormClass)> = vec![
            (
                "two identical: sign-up",
                vec![text(1), p(2, "a"), p(3, "a")],
                FormClass::SignUp {
                    username: Some(1),
                    new_password: 2,
                    confirm: Some(3),
                },
            ),
            (
                "two different: change",
                vec![p(1, "old"), p(2, "new")],
                FormClass::Change {
                    username: None,
                    current: 1,
                    new_password: 2,
                    confirm: None,
                },
            ),
            (
                "two empty: change",
                vec![pw(1), pw(2)],
                FormClass::Change {
                    username: None,
                    current: 1,
                    new_password: 2,
                    confirm: None,
                },
            ),
            (
                "three, new repeated second",
                vec![p(1, "old"), p(2, "n"), p(3, "n")],
                FormClass::Change {
                    username: None,
                    current: 1,
                    new_password: 2,
                    confirm: Some(3),
                },
            ),
            (
                "three, new repeated first",
                vec![p(1, "n"), p(2, "n"), p(3, "hint")],
                FormClass::SignUp {
                    username: None,
                    new_password: 1,
                    confirm: Some(2),
                },
            ),
            (
                "three identical",
                vec![p(1, "x"), p(2, "x"), p(3, "x")],
                login(None, 1),
            ),
            (
                "three different",
                vec![p(1, "a"), p(2, "b"), p(3, "c")],
                login(None, 1),
            ),
            (
                "four: the first three count",
                vec![p(1, "o"), p(2, "n"), p(3, "n"), p(4, "z")],
                FormClass::Change {
                    username: None,
                    current: 1,
                    new_password: 2,
                    confirm: Some(3),
                },
            ),
        ];
        for (what, fields, want) in cases {
            assert_eq!(classify(&form(fields), Mode::Filling), want, "{what}");
        }
    }

    #[test]
    fn saving_drops_empty_values_and_unlikely_usernames() {
        let f = form(vec![
            with_value(text(1), "alice"),
            with_value(text(2), "12"),
            pw(3),
            with_value(pw(4), "secret"),
        ]);
        // The empty password is not the password; "12" is no username.
        assert_eq!(classify(&f, Mode::Saving), login(Some(1), 4));
        let only_short = form(vec![with_value(text(1), "12"), with_value(pw(2), "s")]);
        assert_eq!(classify(&only_short, Mode::Saving), login(None, 2));
        // A hidden username with a value counts when saving (a two-step form's echo).
        let hidden_user = form(vec![
            hidden(with_value(text(1), "bob")),
            with_value(pw(2), "s"),
        ]);
        assert_eq!(classify(&hidden_user, Mode::Saving), login(Some(1), 2));
        assert_eq!(classify(&hidden_user, Mode::Filling), login(None, 2));
        assert_eq!(
            classify(&form(vec![text(1), pw(2)]), Mode::Saving),
            FormClass::NotAForm
        );
    }

    #[test]
    fn submitted_takes_the_password_that_is_now_theirs() {
        let signin = form(vec![with_value(text(1), "alice"), with_value(pw(2), "pw")]);
        let c = submitted(&signin).unwrap();
        assert_eq!((c.username.as_str(), c.password.expose()), ("alice", "pw"));
        let change = form(vec![
            with_value(text(1), "alice"),
            with_value(pw(2), "old"),
            with_value(pw(3), "new"),
            with_value(pw(4), "new"),
        ]);
        assert_eq!(submitted(&change).unwrap().password.expose(), "new");
        let signup = form(vec![
            with_value(field(1, "email"), "a@b.test"),
            with_value(pw(2), "fresh"),
            with_value(pw(3), "fresh"),
        ]);
        let c = submitted(&signup).unwrap();
        assert_eq!(
            (c.username.as_str(), c.password.expose()),
            ("a@b.test", "fresh")
        );
        assert_eq!(submitted(&form(vec![text(1), pw(2)])), None);
    }

    // ----- origin rules

    #[test]
    fn origin_rules() {
        let top = "https://a.test/login";
        let cases = [
            (
                "same origin",
                top,
                "https://a.test/x",
                "https://a.test",
                true,
            ),
            (
                "default port spelled out",
                top,
                "https://a.test:443/",
                "https://a.test",
                true,
            ),
            (
                "subdomain frame",
                top,
                "https://www.a.test/",
                "https://a.test",
                false,
            ),
            (
                "subdomain login",
                top,
                "https://a.test/",
                "https://www.a.test",
                false,
            ),
            (
                "other port",
                top,
                "https://a.test:8443/",
                "https://a.test",
                false,
            ),
            (
                "https login on http",
                "http://a.test/",
                "http://a.test/",
                "https://a.test",
                false,
            ),
            (
                "http login on https",
                top,
                "https://a.test/",
                "http://a.test",
                false,
            ),
            (
                "cross-origin iframe",
                top,
                "https://evil.test/",
                "https://evil.test",
                false,
            ),
            (
                "about:blank frame",
                top,
                "about:blank",
                "https://a.test",
                false,
            ),
            (
                "data: frame",
                top,
                "data:text/html,x",
                "https://a.test",
                false,
            ),
            (
                "file: top",
                "file:///tmp/x",
                "file:///tmp/x",
                "https://a.test",
                false,
            ),
            (
                "login for another site",
                top,
                "https://a.test/",
                "https://b.test",
                false,
            ),
        ];
        for (what, top, frame, login, want) in cases {
            assert_eq!(may_fill(top, frame, login), want, "{what}");
        }
        assert_eq!(
            may_report("https://a.test/", "https://a.test/f").as_deref(),
            Some("https://a.test")
        );
        assert_eq!(
            may_report("http://127.0.0.1:8080/", "http://127.0.0.1:8080/x").as_deref(),
            Some("http://127.0.0.1:8080")
        );
        assert_eq!(may_report("https://a.test/", "https://b.test/"), None);
        assert_eq!(may_report("about:blank", "about:blank"), None);
    }

    // ----- transitions

    const ON: Policy = Policy {
        autofill: true,
        offer_to_save: true,
        never_save: false,
    };
    const ORIGIN: &str = "https://a.test";

    fn login_page() -> Vec<FormSnapshot> {
        vec![form(vec![text(1), pw(2)])]
    }

    fn forms(frame: &str, forms: Vec<FormSnapshot>) -> Event {
        Event::Forms {
            frame: frame.into(),
            origin: ORIGIN.into(),
            forms,
        }
    }

    fn settled(frame: &str, is_main: bool, forms: Vec<FormSnapshot>) -> Event {
        Event::Settled {
            frame: frame.into(),
            origin: ORIGIN.into(),
            is_main,
            forms,
        }
    }

    fn filled_login(user: &str, pass: &str) -> FormSnapshot {
        form(vec![with_value(text(1), user), with_value(pw(2), pass)])
    }

    fn submit(user: &str, pass: &str) -> Event {
        Event::Submit {
            frame: "main".into(),
            origin: ORIGIN.into(),
            form: filled_login(user, pass),
        }
    }

    fn alice() -> Candidate {
        Candidate {
            id: "r1".into(),
            username: "alice".into(),
        }
    }

    fn run(events: Vec<Event>, policy: Policy) -> (AutofillState, Vec<Effect>) {
        events
            .into_iter()
            .fold((AutofillState::Idle, vec![]), |(s, mut all), e| {
                let (s, effects) = transition(s, e, policy);
                all.extend(effects);
                (s, all)
            })
    }

    fn logins() -> Event {
        Event::Logins {
            origin: ORIGIN.into(),
            candidates: vec![alice()],
        }
    }

    #[test]
    fn a_fill_needs_the_menu_and_a_pick() {
        let (s, effects) = run(vec![forms("main", login_page())], ON);
        assert_eq!(
            effects,
            [Effect::QueryLogins {
                origin: ORIGIN.into()
            }]
        );
        assert!(matches!(s, AutofillState::Offer(_)));
        assert!(!view(&s).chip, "no chip before the vault answered");
        let (s, _) = transition(s, logins(), ON);
        assert!(view(&s).chip);
        // Choosing without the menu open fills nothing.
        let (s, effects) = transition(s, Event::Choose(0), ON);
        assert!(effects.is_empty());
        let (s, _) = transition(s, Event::Open, ON);
        assert_eq!(
            view(&s).menu,
            Some(vec![MenuRow::Login {
                username: "alice".into()
            }])
        );
        let (s, effects) = transition(s, Event::Choose(0), ON);
        assert_eq!(
            effects,
            [Effect::Fill {
                frame: "main".into(),
                origin: ORIGIN.into(),
                record: "r1".into(),
                username: "alice".into(),
                username_field: Some(1),
                password_field: 2,
            }]
        );
        assert!(matches!(s, AutofillState::Offer(_)), "the menu closes");
    }

    #[test]
    fn the_menu_closes_and_a_row_out_of_range_fills_nothing() {
        let (s, _) = run(vec![forms("main", login_page()), logins(), Event::Open], ON);
        let (s2, _) = transition(s.clone(), Event::Close, ON);
        assert!(matches!(s2, AutofillState::Offer(_)));
        let (_, effects) = transition(s, Event::Choose(5), ON);
        assert!(effects.is_empty());
    }

    #[test]
    fn a_sign_up_form_offers_the_generator() {
        let signup = form(vec![
            text(1),
            ac(pw(2), "new-password"),
            ac(pw(3), "new-password"),
        ]);
        let (s, effects) = run(vec![forms("main", vec![signup]), Event::Open], ON);
        assert!(effects.is_empty(), "nothing to look up for a sign-up form");
        assert_eq!(view(&s).menu, Some(vec![MenuRow::Generate]));
        let (_, effects) = transition(s, Event::Choose(0), ON);
        assert_eq!(
            effects,
            [Effect::FillGenerated {
                frame: "main".into(),
                origin: ORIGIN.into(),
                fields: vec![2, 3],
            }]
        );
    }

    #[test]
    fn a_repeated_report_keeps_the_logins_and_does_not_query_again() {
        let (s, _) = run(vec![forms("main", login_page()), logins(), Event::Open], ON);
        let (s, effects) = transition(s, forms("main", login_page()), ON);
        assert!(effects.is_empty());
        assert!(matches!(&s, AutofillState::Menu(o) if o.candidates == [alice()]));
    }

    #[test]
    fn the_form_leaving_its_frame_ends_the_offer_but_another_frame_does_not() {
        let (s, _) = run(vec![forms("main", login_page()), logins()], ON);
        let (s, _) = transition(s, forms("sub", vec![]), ON);
        assert!(matches!(s, AutofillState::Offer(_)));
        let (s, _) = transition(s, forms("main", vec![]), ON);
        assert_eq!(s, AutofillState::Idle);
    }

    #[test]
    fn a_main_document_without_a_form_ends_the_offer_whatever_its_frame_id() {
        let (s, _) = run(vec![forms("main", login_page()), logins()], ON);
        let (s2, _) = transition(s.clone(), settled("sub", false, vec![]), ON);
        assert!(matches!(s2, AutofillState::Offer(_)));
        let (s, _) = transition(s, settled("main-after-swap", true, vec![]), ON);
        assert_eq!(s, AutofillState::Idle);
    }

    #[test]
    fn a_main_frame_navigation_clears_the_offer() {
        for extra in [vec![], vec![Event::Open]] {
            let mut events = vec![forms("main", login_page()), logins()];
            events.extend(extra);
            events.push(Event::Navigated);
            assert_eq!(run(events, ON).0, AutofillState::Idle);
        }
    }

    #[test]
    fn a_login_answered_for_another_origin_is_dropped() {
        let (s, _) = run(vec![forms("main", login_page())], ON);
        let (s, _) = transition(
            s,
            Event::Logins {
                origin: "https://b.test".into(),
                candidates: vec![alice()],
            },
            ON,
        );
        assert!(!view(&s).chip);
    }

    #[test]
    fn a_successful_sign_in_asks_to_save() {
        let (s, effects) = run(
            vec![
                forms("main", login_page()),
                submit("alice", "pw"),
                Event::Navigated,
                settled("main2", true, vec![]),
            ],
            ON,
        );
        let creds = Creds {
            username: "alice".into(),
            password: Secret::from("pw"),
        };
        assert!(matches!(s, AutofillState::Checking(_)));
        assert_eq!(
            effects.last(),
            Some(&Effect::CheckSave {
                origin: ORIGIN.into(),
                creds: creds.clone(),
            })
        );
        let (s, _) = transition(s, Event::Verdict(SaveVerdict::New), ON);
        assert_eq!(
            view(&s).prompt,
            Some((SaveKind::New, "alice".into(), ORIGIN.into()))
        );
        let (s, effects) = transition(s, Event::Answer(Answer::Save), ON);
        assert_eq!(s, AutofillState::Idle);
        assert_eq!(
            effects,
            [Effect::Save {
                origin: ORIGIN.into(),
                creds,
            }]
        );
    }

    #[test]
    fn verdicts_choose_the_prompt() {
        let checking = run(
            vec![submit("alice", "pw"), settled("main", true, vec![])],
            ON,
        )
        .0;
        let (s, _) = transition(
            checking.clone(),
            Event::Verdict(SaveVerdict::Differs("r1".into())),
            ON,
        );
        assert!(matches!(
            view(&s).prompt,
            Some((SaveKind::Update(ref r), _, _)) if r == "r1"
        ));
        let (_, effects) = transition(s, Event::Answer(Answer::Save), ON);
        assert!(matches!(&effects[..], [Effect::Update { record, .. }] if record == "r1"));
        for nothing in [SaveVerdict::Same, SaveVerdict::Unavailable] {
            let (s, _) = transition(checking.clone(), Event::Verdict(nothing), ON);
            assert_eq!(s, AutofillState::Idle);
        }
    }

    #[test]
    fn never_and_not_now() {
        let prompt = run(
            vec![
                submit("alice", "pw"),
                settled("main", true, vec![]),
                Event::Verdict(SaveVerdict::New),
            ],
            ON,
        )
        .0;
        let (s, effects) = transition(prompt.clone(), Event::Answer(Answer::Never), ON);
        assert_eq!(s, AutofillState::Idle);
        assert_eq!(
            effects,
            [Effect::Never {
                origin: ORIGIN.into()
            }]
        );
        let (s, effects) = transition(prompt, Event::Answer(Answer::NotNow), ON);
        assert_eq!((s, effects), (AutofillState::Idle, vec![]));
    }

    #[test]
    fn the_sign_in_form_coming_back_means_it_failed() {
        let (s, effects) = run(
            vec![
                submit("alice", "wrong"),
                settled("main", true, login_page()),
            ],
            ON,
        );
        assert!(matches!(s, AutofillState::Offer(_)), "offered again");
        assert!(
            !effects
                .iter()
                .any(|e| matches!(e, Effect::CheckSave { .. }))
        );
    }

    #[test]
    fn another_frame_settling_does_not_decide() {
        let (s, effects) = run(
            vec![submit("alice", "pw"), settled("ad-frame", false, vec![])],
            ON,
        );
        assert!(matches!(s, AutofillState::Submitted(_)));
        assert!(effects.is_empty());
    }

    #[test]
    fn a_single_page_sign_in_is_seen_when_the_form_goes() {
        let gone = Event::Gone {
            frame: "main".into(),
            origin: ORIGIN.into(),
            form: filled_login("alice", "pw"),
        };
        let (s, effects) = run(vec![submit("alice", "pw"), gone.clone()], ON);
        assert!(matches!(s, AutofillState::Checking(_)));
        assert!(matches!(&effects[..], [Effect::CheckSave { .. }]));
        // Without a submit seen first, too.
        let (s, _) = run(vec![gone], ON);
        assert!(matches!(s, AutofillState::Checking(_)));
    }

    #[test]
    fn settings_off_ask_nothing() {
        let off = Policy {
            autofill: false,
            offer_to_save: false,
            never_save: false,
        };
        let (s, effects) = run(
            vec![
                forms("main", login_page()),
                Event::Open,
                submit("alice", "pw"),
                settled("main", true, vec![]),
            ],
            off,
        );
        assert_eq!((s, effects), (AutofillState::Idle, vec![]));
        let never = Policy {
            never_save: true,
            ..ON
        };
        let (s, effects) = run(
            vec![submit("alice", "pw"), settled("m", true, vec![])],
            never,
        );
        assert_eq!((s, effects), (AutofillState::Idle, vec![]));
    }

    #[test]
    fn closing_the_tab_drops_everything() {
        let prompt = run(
            vec![
                submit("alice", "pw"),
                settled("main", true, vec![]),
                Event::Verdict(SaveVerdict::New),
            ],
            ON,
        )
        .0;
        assert_eq!(
            transition(prompt, Event::Closed, ON),
            (AutofillState::Idle, vec![])
        );
    }

    #[test]
    fn a_new_submit_replaces_an_unanswered_prompt() {
        let (s, _) = run(
            vec![
                submit("alice", "pw"),
                settled("main", true, vec![]),
                Event::Verdict(SaveVerdict::New),
                submit("bob", "pw2"),
            ],
            ON,
        );
        assert!(matches!(&s, AutofillState::Submitted(p) if p.creds.username == "bob"));
    }

    #[test]
    fn no_secret_in_debug_output() {
        let (s, effects) = run(
            vec![
                submit("alice", "hunter2-secret"),
                settled("main", true, vec![]),
            ],
            ON,
        );
        let shown = format!("{s:?} {effects:?} {:?}", submit("alice", "hunter2-secret"));
        assert!(!shown.contains("hunter2-secret"), "{shown}");
    }
}
