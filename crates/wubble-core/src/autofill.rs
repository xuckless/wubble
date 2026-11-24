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
//! `FindUsernameFieldBaseHeuristics`, `ParseUsingBaseHeuristics`, the
//! single-username rule at the end of `Parse`), with its field-name patterns
//! (`components/password_manager/core/common/password_manager_constants.h`
//! `kOneTimePwdRe`, `kSocialSecurityRe`; `components/autofill/core/common/
//! autofill_regex_constants.h` `kCardCvcRe`) and its username-first flow
//! (`password_manager_util.cc` `IsLikelyOtp`,
//! `CanFieldBeConsideredAsSingleUsername`, `CanValueBeConsideredAsSingleUsername`;
//! `possible_username_data.cc` with `kSingleUsernameTimeToLive`), BSD-3-Clause,
//! simplified: no server predictions. See `THIRD_PARTY_NOTICES.md`.
//!
//! A form the script reports holds a password field (or had one: a field
//! shown as text by a show-password toggle still counts, Firefox's
//! `hasBeenTypePassword`), or passed the script's username-only test (Firefox
//! `LoginManagerChild.getUsernameFieldFromUsernameOnlyForm`). A reported form
//! without any password field is therefore a username-only form here.
//!
//! Secrets: values travel as [`Secret`] (zeroized on drop, never printed).
//! Nothing here is part of the saved session.

use std::sync::LazyLock;

use regex::Regex;
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
    /// The script started in a new document (sent before any page script).
    Start,
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
        "start" => Some(ScriptMessage::Start),
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
    /// The first step of a two-step sign-in: a username and no password yet
    /// (Chromium's `is_single_username`, "username-first" forms).
    SingleUsername {
        username: FieldId,
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

/// `webauthn` alone is Firefox's field name "webauthn", which its
/// `isUsernameFieldType` and `isInferredUsernameField` take for a username
/// (Amazon's first sign-in step has only that).
fn hint(autocomplete: &str) -> Hint {
    let tokens: Vec<String> = autocomplete
        .split_ascii_whitespace()
        .map(str::to_ascii_lowercase)
        .collect();
    let last = tokens.iter().rfind(|t| *t != "webauthn");
    match last.map(String::as_str) {
        None if !tokens.is_empty() => Hint::Username,
        Some("username") => Hint::Username,
        Some("current-password") => Hint::CurrentPassword,
        Some("new-password") => Hint::NewPassword,
        Some("one-time-code") => Hint::OneTimeCode,
        Some(t) if t.starts_with("cc-") => Hint::CreditCard,
        _ => Hint::None,
    }
}

/// Chromium's patterns, matched case-insensitively like its `MatchesRegex`:
/// `password_manager_constants.h` `kOneTimePwdRe` and `kSocialSecurityRe`,
/// `autofill_regex_constants.h` `kCardCvcRe`.
static ONE_TIME_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        r"(?i)one.?time|",
        r"(?:\b|_)(?:otp|otc|totp|sms|2fa|mfa)(?:\b|_)|",
        r"(?:otp|otc|totp|sms|2fa|mfa).?(?:code|token|input|val|pin|login|verif|pass|pwd|psw|auth|field)|",
        r"(?:verif(?:y|ication)?|email|phone|text|login|input|txt|user).?(?:otp|otc|totp|sms|2fa|mfa)|",
        r"sms.?otp|mfa.?otp|",
        r"verif(?:y|ication)?.?code|(?:\b|_)vcode|",
        r"(?:second|two|2).?factor|",
        r"wfls-token|email_code",
    ))
    .expect("kOneTimePwdRe compiles")
});
static SSN_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)ssn|social.?security.?(num(ber)?|#)*").expect("kSocialSecurityRe compiles")
});
static CVC_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        r"(?i)verification|card.?identification|security.?code|card.?code",
        r"|security.?value|security.?number|card.?pin|c-v-v",
        r"|código de segurança|código de seguridad|karten.?prüfn",
        r"|(?:cvn|cvv|cvc|csc|cvd|ccv)|\bcid\b|cccid",
    ))
    .expect("kCardCvcRe compiles")
});

fn name_or_id_matches(f: &FieldSnapshot, re: &Regex) -> bool {
    re.is_match(&f.name) || re.is_match(&f.id_attr)
}

/// Chromium `IsNotPasswordField` (with `IsCreditCardField`): a card code, a
/// one-time code or a social security number, by autocomplete or name.
fn is_not_password_field(f: &FieldSnapshot) -> bool {
    matches!(hint(&f.autocomplete), Hint::OneTimeCode | Hint::CreditCard)
        || name_or_id_matches(f, &CVC_RE)
        || name_or_id_matches(f, &SSN_RE)
        || name_or_id_matches(f, &ONE_TIME_RE)
}

/// Chromium `password_manager::util::IsLikelyOtp`.
fn is_likely_otp(f: &FieldSnapshot) -> bool {
    f.autocomplete.contains("one-time-code") || name_or_id_matches(f, &ONE_TIME_RE)
}

/// Chromium `IsProbablyNotUsername`.
fn probably_not_username(value: &str) -> bool {
    value.is_empty() || (value.chars().count() < 3 && value.chars().all(|c| c.is_ascii_digit()))
}

/// Chromium `CanFieldBeConsideredAsSingleUsername`: a meaningful name or id
/// (two letters at least, `IsMeaningfulNameOrId`) that does not say "search".
/// The label is not reported, so only the name and the id are checked.
fn may_be_single_username(f: &FieldSnapshot) -> bool {
    let meaningful = |s: &str| s.chars().filter(|c| c.is_alphabetic()).nth(1).is_some();
    let search = |s: &str| s.to_lowercase().contains("search");
    (meaningful(&f.name) || meaningful(&f.id_attr)) && !search(&f.name) && !search(&f.id_attr)
}

/// Chromium `CanValueBeConsideredAsSingleUsername`: 2 to 100 characters.
fn single_username_value(value: &str) -> bool {
    (2..=100).contains(&value.encode_utf16().count())
}

/// A password field the parser may use in `mode`. A password-typed field
/// marked `autocomplete=username` is a username (wubble's rule, not Chromium's).
fn usable_password(f: &FieldSnapshot, mode: Mode) -> bool {
    f.kind == FieldKind::Password
        && !f.readonly
        && hint(&f.autocomplete) != Hint::Username
        && !is_not_password_field(f)
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
            && !is_not_password_field(f)
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
        // No password to use. Chromium: a username found by its autocomplete
        // attribute makes a single-username form. wubble: a form the script
        // reported without any password field passed its username-only test.
        return username
            .or_else(|| username_only(form, mode))
            .map_or(FormClass::NotAForm, |username| FormClass::SingleUsername {
                username,
            });
    };
    let username = username.or_else(|| username_before(form, first, mode));
    class_of(username, located)
}

/// The username of a form the script reported with no password field (its
/// username-only test, Firefox `getUsernameFieldFromUsernameOnlyForm`, passed):
/// the one text field that may be a single username (Chromium
/// `CanFieldBeConsideredAsSingleUsername`, not `IsLikelyOtp`), editable and
/// shown when filling, with a value that may be a username when saving
/// (`CanValueBeConsideredAsSingleUsername`).
fn username_only(form: &FormSnapshot, mode: Mode) -> Option<FieldId> {
    if form.fields.iter().any(|f| f.kind == FieldKind::Password) {
        return None;
    }
    let mut candidates = form.fields.iter().filter(|f| {
        f.kind.text_like()
            && may_be_single_username(f)
            && !is_likely_otp(f)
            && match mode {
                Mode::Filling => f.visible && !f.readonly,
                Mode::Saving => single_username_value(f.value.expose()),
            }
    });
    let one = candidates.next()?;
    candidates.next().is_none().then_some(one.id)
}

/// The fields a picked login fills: (username, password).
pub fn fill_target(class: FormClass) -> Option<(Option<FieldId>, Option<FieldId>)> {
    match class {
        FormClass::Login { username, password } => Some((username, Some(password))),
        FormClass::Change {
            username, current, ..
        } => Some((username, Some(current))),
        FormClass::SingleUsername { username } => Some((Some(username), None)),
        FormClass::SignUp { .. } | FormClass::NotAForm => None,
    }
}

/// A form a password is typed into to sign in (not a sign-up form, not a
/// username step).
fn signs_in(class: FormClass) -> bool {
    matches!(class, FormClass::Login { .. } | FormClass::Change { .. })
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
        FormClass::Login { .. } | FormClass::SingleUsername { .. } | FormClass::NotAForm => vec![],
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
/// is now theirs (the new one on a sign-up or change form), and whether the
/// form was a sign-in (not a sign-up or change) form.
pub fn submitted(form: &FormSnapshot) -> Option<(Creds, bool)> {
    let value = |id: FieldId| form.fields.iter().find(|f| f.id == id).map(|f| &f.value);
    let class = classify(form, Mode::Saving);
    let (username, password) = match class {
        FormClass::NotAForm | FormClass::SingleUsername { .. } => return None,
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
    let creds = Creds {
        username: username
            .and_then(value)
            .map(|s| s.expose().to_owned())
            .unwrap_or_default(),
        password: value(password)?.clone(),
    };
    Some((creds, matches!(class, FormClass::Login { .. })))
}

/// The username a username-first step submitted (`form` is a single-username
/// form with a value that may be a username).
pub fn submitted_username(form: &FormSnapshot) -> Option<String> {
    let FormClass::SingleUsername { username } = classify(form, Mode::Saving) else {
        return None;
    };
    let field = form.fields.iter().find(|f| f.id == username)?;
    Some(field.value.expose().to_owned())
}

/// The form to offer on: the first sign-in form, else the first username
/// step, else the first sign-up form.
fn offered_form(forms: &[FormSnapshot]) -> Option<(u32, FormClass)> {
    let rank = |c: FormClass| match c {
        FormClass::Login { .. } | FormClass::Change { .. } => Some(0),
        FormClass::SingleUsername { .. } => Some(1),
        FormClass::SignUp { .. } => Some(2),
        FormClass::NotAForm => None,
    };
    forms
        .iter()
        .map(|f| (f.id, classify(f, Mode::Filling)))
        .filter_map(|(id, c)| Some((rank(c)?, id, c)))
        .min_by_key(|(rank, ..)| *rank)
        .map(|(_, id, c)| (id, c))
}

/// A sign-in form is still on the page (a failed login shows it again).
fn has_login_form(forms: &[FormSnapshot]) -> bool {
    forms.iter().any(|f| signs_in(classify(f, Mode::Filling)))
}

// ------------------------------------------------------------ origin rules

/// `url`'s origin as `scheme://host[:port]` (default port left out), for
/// http and https only: the form the vault stores (`wubble_vault::Origin`).
pub fn web_origin(url: &str) -> Option<String> {
    let url = url::Url::parse(url).ok()?;
    matches!(url.scheme(), "http" | "https").then(|| url.origin().ascii_serialization())
}

/// Whether a frame's report may be acted on, and the origin it speaks for:
/// the frame's own (a cross-origin iframe speaks for its origin, never for
/// the page's), on a web page (http or https, no opaque origin).
pub fn may_report(top_url: &str, frame_url: &str) -> Option<String> {
    web_origin(top_url)?;
    web_origin(frame_url)
}

/// Whether a login saved for `login_origin` may be filled into a frame:
/// the report rule, and the frame's origin is the login's exact origin (so
/// an https login never fills on http, a subdomain never matches, and a
/// cross-origin iframe gets only its own origin's logins).
pub fn may_fill(top_url: &str, frame_url: &str, login_origin: &str) -> bool {
    may_report(top_url, frame_url).is_some_and(|o| Some(o) == web_origin(login_origin))
}

// ------------------------------------------------------------ per-tab state

/// How long a username submitted on a username-first step is kept for the
/// password step (Chromium `kSingleUsernameTimeToLive`, 5 minutes), in ms.
pub const SINGLE_USERNAME_TTL_MS: i64 = 5 * 60 * 1000;

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
    /// The frame's origin: the only origin whose logins are offered here.
    pub origin: String,
    /// The offer is the main frame's (it wins over an iframe's).
    pub main: bool,
    pub form: u32,
    pub class: FormClass,
    /// The vault's logins for the origin: `None` while asked and not answered.
    pub candidates: Option<Vec<Candidate>>,
}

/// A row of the key menu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MenuRow {
    Login { username: String },
    Generate,
}

impl Offer {
    pub fn rows(&self) -> Vec<MenuRow> {
        let logins = fill_target(self.class)
            .map_or(&[][..], |_| self.candidates.as_deref().unwrap_or_default());
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

    /// A picked login fills a password here (not only a username).
    pub fn fills_password(&self) -> bool {
        fill_target(self.class).is_some_and(|(_, password)| password.is_some())
    }
}

/// Submitted credentials and where they came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pending {
    pub frame: String,
    pub origin: String,
    pub creds: Creds,
    /// From a sign-in form: only then does the form coming back mean the
    /// submit failed (a sign-up often lands on the site's sign-in page).
    pub sign_in: bool,
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

/// Where one tab's autofill is, from a form on the page to a save prompt.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Phase {
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

/// The username a username-first step submitted (Chromium's
/// `PossibleUsernameData`): a password-only form submitted on the same
/// origin within [`SINGLE_USERNAME_TTL_MS`] is saved with it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FirstStep {
    pub origin: String,
    pub username: String,
    /// When it was submitted (ms, the host's clock).
    pub at: i64,
}

impl FirstStep {
    fn fresh(&self, now: i64) -> bool {
        now.saturating_sub(self.at) <= SINGLE_USERNAME_TTL_MS
    }
}

/// One tab's autofill state: the phase, and the username a first step left
/// for the next (it outlives the navigation to the password page).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AutofillState {
    pub phase: Phase,
    pub first_step: Option<FirstStep>,
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
    /// The user chose "Never" for the event's origin (the page's when the event has none).
    pub never_save: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// The script in `frame` reported its forms; the host checked the frame
    /// (`may_report`), `origin` is its origin.
    Forms {
        frame: String,
        origin: String,
        is_main: bool,
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
    /// A form was submitted at `at` (ms, the host's clock).
    Submit {
        frame: String,
        origin: String,
        form: FormSnapshot,
        at: i64,
    },
    Gone {
        frame: String,
        origin: String,
        form: FormSnapshot,
        at: i64,
    },
    Settled {
        frame: String,
        origin: String,
        is_main: bool,
        forms: Vec<FormSnapshot>,
    },
    /// The main frame began a new document (or committed a page of another
    /// origin): what the old one offered is gone, whatever frame offered it.
    Navigated,
    /// The vault's answer to `CheckSave` for these credentials.
    Verdict {
        origin: String,
        creds: Creds,
        verdict: SaveVerdict,
    },
    Answer(Answer),
    /// The tab or its view is gone.
    Closed,
}

impl Event {
    /// The origin a frame's event speaks for (whose "never save" applies).
    pub fn origin(&self) -> Option<&str> {
        match self {
            Self::Forms { origin, .. }
            | Self::Submit { origin, .. }
            | Self::Gone { origin, .. }
            | Self::Settled { origin, .. } => Some(origin),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effect {
    /// Ask the vault for `origin`'s logins (answered with `Event::Logins`).
    QueryLogins {
        origin: String,
    },
    /// Fill `username` into `frame`, and `record`'s password when the form
    /// has a password field (a username step has none).
    Fill {
        frame: String,
        origin: String,
        record: String,
        username: String,
        username_field: Option<FieldId>,
        password_field: Option<FieldId>,
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
type PhaseStep = (Phase, Vec<Effect>);

/// The one transition of a tab's autofill state.
pub fn transition(state: AutofillState, event: Event, policy: Policy) -> Step {
    if matches!(event, Event::Closed) {
        return (AutofillState::default(), vec![]);
    }
    let AutofillState { phase, first_step } = state;
    // A username step's submit is remembered for the password step; an old one goes.
    let first_step = match &event {
        Event::Submit {
            origin, form, at, ..
        }
        | Event::Gone {
            origin, form, at, ..
        } => match submitted_username(form) {
            Some(username) => Some(FirstStep {
                origin: origin.clone(),
                username,
                at: *at,
            }),
            None => first_step.filter(|f| f.fresh(*at)),
        },
        _ => first_step,
    };
    let (phase, effects) = phase_transition(phase, event, policy, first_step.as_ref());
    (AutofillState { phase, first_step }, effects)
}

fn phase_transition(
    phase: Phase,
    event: Event,
    policy: Policy,
    first_step: Option<&FirstStep>,
) -> PhaseStep {
    use Phase as S;
    match (phase, event) {
        (
            phase @ (S::Idle | S::Offer(_) | S::Menu(_)),
            Event::Forms {
                frame,
                origin,
                is_main,
                forms,
            }
            | Event::Settled {
                frame,
                origin,
                is_main,
                forms,
            },
        ) => offer(phase, frame, origin, is_main, &forms, policy),
        (S::Offer(mut o), Event::Logins { origin, candidates }) if o.origin == origin => {
            o.candidates = Some(candidates);
            (S::Offer(o), vec![])
        }
        (S::Menu(mut o), Event::Logins { origin, candidates }) if o.origin == origin => {
            o.candidates = Some(candidates);
            (S::Menu(o), vec![])
        }
        (S::Offer(o), Event::Open) if !o.rows().is_empty() => (S::Menu(o), vec![]),
        (S::Menu(o), Event::Close) => (S::Offer(o), vec![]),
        (S::Menu(o), Event::Choose(i)) => choose(o, i),
        (
            phase,
            Event::Submit {
                frame,
                origin,
                form,
                at,
            },
        ) => match pending(frame, origin, &form, policy, first_step, at) {
            Some(p) => (S::Submitted(p), vec![]),
            None => (phase, vec![]),
        },
        (
            phase,
            Event::Gone {
                frame,
                origin,
                form,
                at,
            },
        ) => match pending(frame, origin, &form, policy, first_step, at) {
            Some(p) => check(p),
            None => (phase, vec![]),
        },
        (
            phase @ (S::Submitted(_) | S::Checking(_) | S::Prompt { .. }),
            Event::Settled {
                frame,
                origin,
                is_main,
                forms,
            },
        ) => settled_after_submit(phase, frame, origin, is_main, &forms, policy),
        (S::Offer(_) | S::Menu(_), Event::Navigated) => (S::Idle, vec![]),
        (
            S::Checking(p),
            Event::Verdict {
                origin,
                creds,
                verdict,
            },
        ) if origin == p.origin && creds == p.creds => match verdict {
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
        (phase, _) => (phase, vec![]),
    }
}

/// A report of `forms` in `frame` while nothing is submitted. A frame
/// without a form ends only its own offer: an offer in a same-origin iframe
/// outlives the main document settling (its `load` waits for the iframe's).
/// A new main document (CEF may give it a new frame id) ends any offer
/// through `Event::Navigated`. Another frame's offer with rows, or still
/// waiting for its logins, stands, unless this report is the main frame's
/// and that one is an iframe's (an answer fills only an offer of its
/// own origin).
fn offer(
    phase: Phase,
    frame: String,
    origin: String,
    is_main: bool,
    forms: &[FormSnapshot],
    policy: Policy,
) -> PhaseStep {
    use Phase as S;
    let current = match &phase {
        S::Offer(o) | S::Menu(o) => Some(o),
        _ => None,
    };
    let Some((form, class)) = offered_form(forms).filter(|_| policy.autofill) else {
        // This frame has no form (any more): its offer goes.
        return match current {
            Some(o) if o.frame == frame || !policy.autofill => (S::Idle, vec![]),
            _ => (phase, vec![]),
        };
    };
    if let Some(o) = current {
        // The same form again (a DOM change): keep the logins and an open menu.
        if o.frame == frame && o.origin == origin && o.form == form {
            let o = Offer { class, ..o.clone() };
            return match phase {
                S::Menu(_) => (S::Menu(o), vec![]),
                _ => (S::Offer(o), vec![]),
            };
        }
        let holds = o.candidates.is_none() || !o.rows().is_empty();
        if o.frame != frame && holds && !(is_main && !o.main) {
            return (phase, vec![]);
        }
    }
    let query = fill_target(class).map(|_| Effect::QueryLogins {
        origin: origin.clone(),
    });
    let o = Offer {
        frame,
        origin,
        main: is_main,
        form,
        class,
        candidates: query.is_none().then(Vec::new),
    };
    (S::Offer(o), query.into_iter().collect())
}

fn choose(o: Offer, index: usize) -> PhaseStep {
    let row = o.rows().get(index).cloned();
    let effect = match row {
        Some(MenuRow::Login { .. }) => fill_target(o.class).and_then(|(user, pass)| {
            let c = o.candidates.as_deref()?.get(index)?;
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
    (Phase::Offer(o), effect.into_iter().collect())
}

/// Submitted credentials worth keeping, if saving is on for the site. A form
/// without a username takes the one a first step submitted on the origin,
/// while it is fresh (Chromium's username-first flow).
fn pending(
    frame: String,
    origin: String,
    form: &FormSnapshot,
    policy: Policy,
    first_step: Option<&FirstStep>,
    at: i64,
) -> Option<Pending> {
    if !policy.offer_to_save || policy.never_save {
        return None;
    }
    let (mut creds, sign_in) = submitted(form)?;
    if creds.username.is_empty()
        && let Some(f) = first_step.filter(|f| f.origin == origin && f.fresh(at))
    {
        creds.username = f.username.clone();
    }
    Some(Pending {
        frame,
        origin,
        creds,
        sign_in,
    })
}

/// The main document (or the submitting frame) settled after a submit. A
/// sign-in form of the same origin back on the page means the sign-in
/// failed, also after the form had already gone (a page that hid the form
/// while it waited for the server): the credentials are dropped and the
/// form offered again. Otherwise a submit counts as done; a check or a
/// prompt already under way stays.
fn settled_after_submit(
    phase: Phase,
    frame: String,
    origin: String,
    is_main: bool,
    forms: &[FormSnapshot],
    policy: Policy,
) -> PhaseStep {
    use Phase as S;
    let (S::Submitted(p) | S::Checking(p) | S::Prompt { pending: p, .. }) = &phase else {
        return (phase, vec![]);
    };
    if !is_main && frame != p.frame {
        return (phase, vec![]);
    }
    if p.sign_in && origin == p.origin && has_login_form(forms) {
        return offer(S::Idle, frame, origin, is_main, forms, policy);
    }
    match phase {
        S::Submitted(p) => check(p),
        phase => (phase, vec![]),
    }
}

fn check(p: Pending) -> PhaseStep {
    let effect = Effect::CheckSave {
        origin: p.origin.clone(),
        creds: p.creds.clone(),
    };
    (Phase::Checking(p), vec![effect])
}

/// What the shell shows for a tab.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct View {
    /// The key chip by the address pill.
    pub chip: bool,
    /// The key menu's rows, while it is open.
    pub menu: Option<Vec<MenuRow>>,
    /// The origin the menu's logins belong to, when it is a frame's, not the page's own: the
    /// menu names it.
    pub menu_origin: Option<String>,
    /// The save prompt: its kind, the username and the origin.
    pub prompt: Option<(SaveKind, String, String)>,
}

pub fn view(state: &AutofillState) -> View {
    match &state.phase {
        Phase::Offer(o) => View {
            chip: !o.rows().is_empty(),
            ..View::default()
        },
        Phase::Menu(o) => View {
            chip: true,
            menu: Some(o.rows()),
            menu_origin: (!o.main).then(|| o.origin.clone()),
            prompt: None,
        },
        Phase::Prompt { kind, pending } => View {
            prompt: Some((
                kind.clone(),
                pending.creds.username.clone(),
                pending.origin.clone(),
            )),
            ..View::default()
        },
        Phase::Idle | Phase::Submitted(_) | Phase::Checking(_) => View::default(),
    }
}

/// The store's layers of a tab's autofill, in the order of [`open_parts`].
pub const PARTS: [crate::overlay::Kind; 3] = [
    crate::overlay::Kind::AutofillChip,
    crate::overlay::Kind::AutofillMenu,
    crate::overlay::Kind::AutofillPrompt,
];

/// Which of [`PARTS`] the shell shows for `view`: the one source of the chip's, the menu's and
/// the prompt's openness.
pub fn open_parts(view: &View) -> [bool; 3] {
    [view.chip, view.menu.is_some(), view.prompt.is_some()]
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
        assert_eq!(
            parse_message(r#"{"t":"start"}"#),
            Some(ScriptMessage::Start)
        );
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
        let (c, sign_in) = submitted(&signin).unwrap();
        assert_eq!((c.username.as_str(), c.password.expose()), ("alice", "pw"));
        assert!(sign_in);
        let change = form(vec![
            with_value(text(1), "alice"),
            with_value(pw(2), "old"),
            with_value(pw(3), "new"),
            with_value(pw(4), "new"),
        ]);
        let (c, sign_in) = submitted(&change).unwrap();
        assert_eq!((c.password.expose(), sign_in), ("new", false));
        let signup = form(vec![
            with_value(field(1, "email"), "a@b.test"),
            with_value(pw(2), "fresh"),
            with_value(pw(3), "fresh"),
        ]);
        let (c, sign_in) = submitted(&signup).unwrap();
        assert_eq!(
            (c.username.as_str(), c.password.expose(), sign_in),
            ("a@b.test", "fresh", false)
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
                "cross-origin iframe, its own origin's login",
                top,
                "https://evil.test/",
                "https://evil.test",
                true,
            ),
            (
                "cross-origin iframe, the page's login",
                top,
                "https://evil.test/",
                "https://a.test",
                false,
            ),
            (
                "cross-origin iframe on http, its https login",
                top,
                "http://b.test/",
                "https://b.test",
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
        // A frame speaks for its own origin, never for the page's.
        assert_eq!(
            may_report("https://a.test/", "https://b.test/").as_deref(),
            Some("https://b.test")
        );
        assert_eq!(may_report("https://a.test/", "about:blank"), None);
        assert_eq!(may_report("file:///x", "https://b.test/"), None);
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
            is_main: frame == "main",
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
            at: 0,
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
            .fold((AutofillState::default(), vec![]), |(s, mut all), e| {
                let (s, effects) = transition(s, e, policy);
                all.extend(effects);
                (s, all)
            })
    }

    fn creds(user: &str, pass: &str) -> Creds {
        Creds {
            username: user.into(),
            password: Secret::from(pass),
        }
    }

    fn verdict(user: &str, pass: &str, verdict: SaveVerdict) -> Event {
        Event::Verdict {
            origin: ORIGIN.into(),
            creds: creds(user, pass),
            verdict,
        }
    }

    fn gone(user: &str, pass: &str) -> Event {
        Event::Gone {
            frame: "main".into(),
            origin: ORIGIN.into(),
            form: filled_login(user, pass),
            at: 0,
        }
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
        assert!(matches!(s.phase, Phase::Offer(_)));
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
                password_field: Some(2),
            }]
        );
        assert!(matches!(s.phase, Phase::Offer(_)), "the menu closes");
    }

    #[test]
    fn the_menu_closes_and_a_row_out_of_range_fills_nothing() {
        let (s, _) = run(vec![forms("main", login_page()), logins(), Event::Open], ON);
        let (s2, _) = transition(s.clone(), Event::Close, ON);
        assert!(matches!(s2.phase, Phase::Offer(_)));
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
        assert!(matches!(&s.phase, Phase::Menu(o) if o.candidates == Some(vec![alice()])));
    }

    #[test]
    fn the_form_leaving_its_frame_ends_the_offer_but_another_frame_does_not() {
        let (s, _) = run(vec![forms("main", login_page()), logins()], ON);
        let (s, _) = transition(s, forms("sub", vec![]), ON);
        assert!(matches!(s.phase, Phase::Offer(_)));
        let (s, _) = transition(s, forms("main", vec![]), ON);
        assert_eq!(s.phase, Phase::Idle);
    }

    #[test]
    fn a_frames_menu_names_the_origin_its_logins_belong_to() {
        let offer = |main: bool| Offer {
            frame: "f".into(),
            origin: "https://login.idp.test".into(),
            main,
            form: 0,
            class: FormClass::Login {
                username: Some(1),
                password: 2,
            },
            candidates: Some(vec![]),
        };
        let state = |o| AutofillState {
            phase: Phase::Menu(o),
            first_step: None,
        };
        assert_eq!(view(&state(offer(true))).menu_origin, None);
        assert_eq!(
            view(&state(offer(false))).menu_origin.as_deref(),
            Some("https://login.idp.test")
        );
    }

    #[test]
    fn an_offer_in_a_same_origin_iframe_outlives_the_main_document_settling() {
        // The iframe loads first; the main frame's `load` (no form of its own) comes after.
        let (s, _) = run(
            vec![
                settled("frame", false, login_page()),
                logins(),
                settled("main", true, vec![]),
            ],
            ON,
        );
        assert!(matches!(&s.phase, Phase::Offer(o) if o.frame == "frame"));
        assert!(view(&s).chip);
        let (s, _) = transition(s, Event::Open, ON);
        let (_, effects) = transition(s, Event::Choose(0), ON);
        assert!(matches!(&effects[..], [Effect::Fill { frame, .. }] if frame == "frame"));
    }

    #[test]
    fn a_new_main_document_ends_the_offer_whatever_its_frame_id() {
        for offered in ["main", "frame"] {
            let (s, _) = run(vec![forms(offered, login_page()), logins()], ON);
            // The main document settling with no form ends only the main frame's own offer.
            let (s2, _) = transition(s.clone(), settled("main", true, vec![]), ON);
            assert_eq!(matches!(s2.phase, Phase::Idle), offered == "main");
            // A new main document (its script's `start`, perhaps under a new frame id).
            let (s, _) = transition(s, Event::Navigated, ON);
            assert_eq!(s.phase, Phase::Idle, "{offered}");
        }
    }

    /// The store closes none of the autofill layers on a navigation (their rows' `on_nav` is
    /// off): the machine ends the offer, and the host closes the parts by the view's change.
    #[test]
    fn autofill_on_nav_matches_the_machine() {
        assert!(PARTS.iter().all(|k| !k.row().on_nav));
        for extra in [vec![], vec![Event::Open]] {
            let mut events = vec![forms("main", login_page()), logins()];
            events.extend(extra);
            let (shown, _) = run(events, ON);
            assert!(open_parts(&view(&shown)).contains(&true));
            let (gone, _) = transition(shown, Event::Navigated, ON);
            assert_eq!(open_parts(&view(&gone)), [false; 3]);
        }
    }

    #[test]
    fn a_main_frame_navigation_clears_the_offer() {
        for extra in [vec![], vec![Event::Open]] {
            let mut events = vec![forms("main", login_page()), logins()];
            events.extend(extra);
            events.push(Event::Navigated);
            assert_eq!(run(events, ON).0.phase, Phase::Idle);
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
        let creds = creds("alice", "pw");
        assert!(matches!(s.phase, Phase::Checking(_)));
        assert_eq!(
            effects.last(),
            Some(&Effect::CheckSave {
                origin: ORIGIN.into(),
                creds: creds.clone(),
            })
        );
        let (s, _) = transition(s, verdict("alice", "pw", SaveVerdict::New), ON);
        assert_eq!(
            view(&s).prompt,
            Some((SaveKind::New, "alice".into(), ORIGIN.into()))
        );
        let (s, effects) = transition(s, Event::Answer(Answer::Save), ON);
        assert_eq!(s.phase, Phase::Idle);
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
            verdict("alice", "pw", SaveVerdict::Differs("r1".into())),
            ON,
        );
        assert!(matches!(
            view(&s).prompt,
            Some((SaveKind::Update(ref r), _, _)) if r == "r1"
        ));
        let (_, effects) = transition(s, Event::Answer(Answer::Save), ON);
        assert!(matches!(&effects[..], [Effect::Update { record, .. }] if record == "r1"));
        for nothing in [SaveVerdict::Same, SaveVerdict::Unavailable] {
            let (s, _) = transition(checking.clone(), verdict("alice", "pw", nothing), ON);
            assert_eq!(s.phase, Phase::Idle);
        }
    }

    #[test]
    fn never_and_not_now() {
        let prompt = run(
            vec![
                submit("alice", "pw"),
                settled("main", true, vec![]),
                verdict("alice", "pw", SaveVerdict::New),
            ],
            ON,
        )
        .0;
        let (s, effects) = transition(prompt.clone(), Event::Answer(Answer::Never), ON);
        assert_eq!(s.phase, Phase::Idle);
        assert_eq!(
            effects,
            [Effect::Never {
                origin: ORIGIN.into()
            }]
        );
        let (s, effects) = transition(prompt, Event::Answer(Answer::NotNow), ON);
        assert_eq!((s.phase, effects), (Phase::Idle, vec![]));
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
        assert!(matches!(s.phase, Phase::Offer(_)), "offered again");
        assert!(
            !effects
                .iter()
                .any(|e| matches!(e, Effect::CheckSave { .. }))
        );
    }

    #[test]
    fn a_sign_up_landing_on_the_sign_in_page_still_asks_to_save() {
        let signup = form(vec![
            with_value(text(1), "alice"),
            with_value(pw(2), "generated"),
            with_value(pw(3), "generated"),
        ]);
        let (s, effects) = run(
            vec![
                Event::Submit {
                    frame: "main".into(),
                    origin: ORIGIN.into(),
                    form: signup,
                    at: 0,
                },
                // "Account created, please sign in."
                settled("main", true, login_page()),
            ],
            ON,
        );
        assert!(matches!(s.phase, Phase::Checking(_)));
        assert_eq!(
            effects,
            [Effect::CheckSave {
                origin: ORIGIN.into(),
                creds: creds("alice", "generated"),
            }]
        );
    }

    #[test]
    fn a_failed_sign_in_after_the_form_hid_drops_the_check_and_the_prompt() {
        // The page hid the form while it waited; the server then sent the form back.
        let checking = run(vec![submit("alice", "wrong"), gone("alice", "wrong")], ON).0;
        assert!(matches!(checking.phase, Phase::Checking(_)));
        let prompt = transition(
            checking.clone(),
            verdict("alice", "wrong", SaveVerdict::Differs("r1".into())),
            ON,
        )
        .0;
        for state in [checking, prompt] {
            let (s, effects) = transition(state, settled("main", true, login_page()), ON);
            assert!(matches!(s.phase, Phase::Offer(_)), "offered again");
            assert_eq!(view(&s).prompt, None);
            assert!(!effects.iter().any(|e| matches!(e, Effect::Update { .. })));
        }
        // A page without the form keeps the prompt.
        let prompt = run(
            vec![
                submit("alice", "pw"),
                gone("alice", "pw"),
                verdict("alice", "pw", SaveVerdict::New),
                settled("main", true, vec![]),
            ],
            ON,
        )
        .0;
        assert!(view(&prompt).prompt.is_some());
    }

    #[test]
    fn a_verdict_for_other_credentials_is_dropped() {
        let (s, _) = run(
            vec![
                submit("alice", "pw"),
                gone("alice", "pw"),
                submit("bob", "pw2"),
                gone("bob", "pw2"),
            ],
            ON,
        );
        // Alice's late verdict must not become Bob's update of Alice's record.
        let (s, _) = transition(
            s,
            verdict("alice", "pw", SaveVerdict::Differs("alice-record".into())),
            ON,
        );
        assert!(matches!(&s.phase, Phase::Checking(p) if p.creds.username == "bob"));
        let (s, _) = transition(s, verdict("bob", "pw2", SaveVerdict::New), ON);
        assert_eq!(
            view(&s).prompt,
            Some((SaveKind::New, "bob".into(), ORIGIN.into()))
        );
        let other_origin = Event::Verdict {
            origin: "https://b.test".into(),
            creds: creds("bob", "pw2"),
            verdict: SaveVerdict::New,
        };
        let checking = run(vec![submit("bob", "pw2"), gone("bob", "pw2")], ON).0;
        assert!(matches!(
            transition(checking, other_origin, ON).0.phase,
            Phase::Checking(_)
        ));
    }

    #[test]
    fn another_frame_settling_does_not_decide() {
        let (s, effects) = run(
            vec![submit("alice", "pw"), settled("ad-frame", false, vec![])],
            ON,
        );
        assert!(matches!(s.phase, Phase::Submitted(_)));
        assert!(effects.is_empty());
    }

    #[test]
    fn a_single_page_sign_in_is_seen_when_the_form_goes() {
        let gone = gone("alice", "pw");
        let (s, effects) = run(vec![submit("alice", "pw"), gone.clone()], ON);
        assert!(matches!(s.phase, Phase::Checking(_)));
        assert!(matches!(&effects[..], [Effect::CheckSave { .. }]));
        // Without a submit seen first, too.
        let (s, _) = run(vec![gone], ON);
        assert!(matches!(s.phase, Phase::Checking(_)));
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
        assert_eq!((s.phase, effects), (Phase::Idle, vec![]));
        let never = Policy {
            never_save: true,
            ..ON
        };
        let (s, effects) = run(
            vec![submit("alice", "pw"), settled("m", true, vec![])],
            never,
        );
        assert_eq!((s.phase, effects), (Phase::Idle, vec![]));
    }

    #[test]
    fn closing_the_tab_drops_everything() {
        let prompt = run(
            vec![
                submit("alice", "pw"),
                settled("main", true, vec![]),
                verdict("alice", "pw", SaveVerdict::New),
            ],
            ON,
        )
        .0;
        assert_eq!(
            transition(prompt, Event::Closed, ON),
            (AutofillState::default(), vec![])
        );
    }

    #[test]
    fn a_new_submit_replaces_an_unanswered_prompt() {
        let (s, _) = run(
            vec![
                submit("alice", "pw"),
                settled("main", true, vec![]),
                verdict("alice", "pw", SaveVerdict::New),
                submit("bob", "pw2"),
            ],
            ON,
        );
        assert!(matches!(&s.phase, Phase::Submitted(p) if p.creds.username == "bob"));
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

    // ----- username-first, one-time codes, frames

    fn id_attr(mut f: FieldSnapshot, id: &str) -> FieldSnapshot {
        f.id_attr = id.into();
        f
    }

    #[test]
    fn single_username_forms() {
        use Mode::{Filling, Saving};
        let single = |username| FormClass::SingleUsername { username };
        let cases: Vec<(&str, FormSnapshot, Mode, FormClass)> = vec![
            (
                "Chromium: autocomplete username, no password",
                form(vec![ac(field(1, "email"), "username webauthn")]),
                Filling,
                single(1),
            ),
            (
                "Chromium: autocomplete username next to a hidden password (Google, Microsoft)",
                form(vec![
                    ac(field(1, "email"), "username"),
                    hidden(named(pw(2), "hiddenPassword")),
                ]),
                Filling,
                single(1),
            ),
            (
                "the script's username-only form: its one named text field",
                form(vec![named(field(1, "email"), "email")]),
                Filling,
                single(1),
            ),
            (
                "a username-only form with a hidden extra field",
                form(vec![named(text(1), "login"), hidden(named(text(2), "x"))]),
                Filling,
                single(1),
            ),
            (
                "two text fields: no single username",
                form(vec![named(text(1), "first"), named(text(2), "last")]),
                Filling,
                FormClass::NotAForm,
            ),
            (
                "a search field is no username (CanFieldBeConsideredAsSingleUsername)",
                form(vec![named(text(1), "q_search")]),
                Filling,
                FormClass::NotAForm,
            ),
            (
                "a meaningless name is no username",
                form(vec![named(text(1), "q")]),
                Filling,
                FormClass::NotAForm,
            ),
            (
                "a one-time code is no username (IsLikelyOtp)",
                form(vec![ac(named(text(1), "code"), "one-time-code")]),
                Filling,
                FormClass::NotAForm,
            ),
            (
                "an sms code by name is no username",
                form(vec![id_attr(text(1), "sms_code")]),
                Filling,
                FormClass::NotAForm,
            ),
            (
                "saving: a one-letter value is no username",
                form(vec![with_value(named(text(1), "login"), "a")]),
                Saving,
                FormClass::NotAForm,
            ),
            (
                "saving: the typed username",
                form(vec![with_value(named(text(1), "login"), "alice")]),
                Saving,
                single(1),
            ),
            (
                "Firefox: autocomplete webauthn alone is a username (Amazon)",
                form(vec![
                    ac(named(text(1), "email"), "webauthn"),
                    hidden(named(pw(2), "password")),
                ]),
                Filling,
                single(1),
            ),
            (
                "a visible password wins: a sign-in form",
                form(vec![ac(text(1), "username"), pw(2)]),
                Filling,
                login(Some(1), 2),
            ),
        ];
        for (what, f, mode, want) in cases {
            assert_eq!(classify(&f, mode), want, "{what}");
        }
    }

    #[test]
    fn chromium_one_time_code_names_are_not_passwords() {
        for name in [
            "otp",
            "one-time-code",
            "oneTimePassword",
            "sms_code",
            "totp",
            "2fa_token",
            "mfa-code",
            "verification_code",
            "vcode",
            "second_factor",
            "email_code",
            "card_cvv",
            "security-code",
            "ssn",
        ] {
            let f = form(vec![text(1), named(pw(2), name)]);
            assert_eq!(classify(&f, Mode::Filling), FormClass::NotAForm, "{name}");
        }
        for name in ["password", "pass", "pwd", "passwd", "Passwd", "user_pass"] {
            let f = form(vec![text(1), named(pw(2), name)]);
            assert_eq!(classify(&f, Mode::Filling), login(Some(1), 2), "{name}");
        }
        // A code field before the password is not the username either.
        let f = form(vec![text(1), named(text(2), "otp"), pw(3)]);
        assert_eq!(classify(&f, Mode::Filling), login(Some(1), 3));
    }

    #[test]
    fn a_username_step_fills_only_the_username() {
        let step = form(vec![ac(field(1, "email"), "username")]);
        assert_eq!(
            fill_target(classify(&step, Mode::Filling)),
            Some((Some(1), None))
        );
        let (s, _) = run(vec![forms("main", vec![step]), logins(), Event::Open], ON);
        let (_, effects) = transition(s, Event::Choose(0), ON);
        assert!(matches!(
            &effects[..],
            [Effect::Fill {
                username_field: Some(1),
                password_field: None,
                ..
            }]
        ));
    }

    #[test]
    fn a_sign_up_form_never_offers_a_saved_login() {
        let signup = form(vec![
            ac(text(1), "username"),
            ac(pw(2), "new-password"),
            ac(pw(3), "new-password"),
        ]);
        let (s, effects) = run(vec![forms("main", vec![signup]), logins()], ON);
        assert!(effects.is_empty());
        assert_eq!(
            match &s.phase {
                Phase::Offer(o) => o.rows(),
                _ => vec![],
            },
            [MenuRow::Generate]
        );
        // A change form fills the current password only, never a new one.
        let change = form(vec![
            ac(text(1), "username"),
            ac(pw(2), "current-password"),
            ac(pw(3), "new-password"),
            ac(pw(4), "new-password"),
        ]);
        assert_eq!(
            fill_target(classify(&change, Mode::Filling)),
            Some((Some(1), Some(2)))
        );
    }

    fn step_submit(user: &str, at: i64) -> Event {
        Event::Submit {
            frame: "main".into(),
            origin: ORIGIN.into(),
            form: form(vec![with_value(ac(field(1, "email"), "username"), user)]),
            at,
        }
    }

    fn password_submit(pass: &str, origin: &str, at: i64) -> Event {
        Event::Submit {
            frame: "main".into(),
            origin: origin.into(),
            form: FormSnapshot {
                id: 2,
                fields: vec![with_value(ac(pw(5), "current-password"), pass)],
            },
            at,
        }
    }

    fn pending_user(s: &AutofillState) -> Option<&str> {
        match &s.phase {
            Phase::Submitted(p) => Some(&p.creds.username),
            _ => None,
        }
    }

    #[test]
    fn a_two_step_sign_in_saves_the_first_steps_username() {
        let (s, effects) = run(vec![step_submit("alice@a.test", 1_000)], ON);
        assert!(effects.is_empty());
        assert_eq!(s.phase, Phase::Idle, "a username alone is not saved");
        assert_eq!(
            s.first_step.as_ref().map(|f| f.username.as_str()),
            Some("alice@a.test")
        );
        // The password page loads (a navigation keeps it) and its form is submitted.
        let (s, _) = transition(s, Event::Navigated, ON);
        let (s, _) = transition(s, password_submit("pw", ORIGIN, 61_000), ON);
        assert_eq!(pending_user(&s), Some("alice@a.test"));
        let (_, effects) = transition(s, settled("next", true, vec![]), ON);
        assert_eq!(
            effects,
            [Effect::CheckSave {
                origin: ORIGIN.into(),
                creds: creds("alice@a.test", "pw"),
            }]
        );
        // A step in the same document, its password seen gone.
        let (s, _) = run(
            vec![
                step_submit("bob", 0),
                Event::Gone {
                    frame: "main".into(),
                    origin: ORIGIN.into(),
                    form: form(vec![
                        hidden(with_value(field(1, "email"), "")),
                        with_value(pw(2), "pw2"),
                    ]),
                    at: 10,
                },
            ],
            ON,
        );
        assert!(matches!(&s.phase, Phase::Checking(p) if p.creds.username == "bob"));
    }

    #[test]
    fn the_first_steps_username_is_bounded_by_origin_and_time() {
        let late = SINGLE_USERNAME_TTL_MS + 1;
        let (s, _) = run(
            vec![step_submit("alice", 0), password_submit("pw", ORIGIN, late)],
            ON,
        );
        assert_eq!(pending_user(&s), Some(""), "stale after five minutes");
        assert_eq!(s.first_step, None, "and dropped");
        let (s, _) = run(
            vec![
                step_submit("alice", 0),
                password_submit("pw", "https://b.test", 5),
            ],
            ON,
        );
        assert_eq!(pending_user(&s), Some(""), "another origin's step");
        // A form with its own username keeps it.
        let (s, _) = run(vec![step_submit("alice", 0), submit("carol", "pw")], ON);
        assert_eq!(pending_user(&s), Some("carol"));
        // Closing the tab forgets it.
        let (s, _) = run(vec![step_submit("alice", 0), Event::Closed], ON);
        assert_eq!(s, AutofillState::default());
    }

    fn frame_forms(frame: &str, origin: &str, is_main: bool) -> Event {
        Event::Forms {
            frame: frame.into(),
            origin: origin.into(),
            is_main,
            forms: login_page(),
        }
    }

    #[test]
    fn a_cross_origin_frame_is_offered_only_its_own_origins_logins() {
        let other = "https://login.b.test";
        let (s, effects) = run(vec![frame_forms("f", other, false)], ON);
        assert_eq!(
            effects,
            [Effect::QueryLogins {
                origin: other.into()
            }]
        );
        // The page's logins answered for the frame's offer are dropped.
        let (s, _) = transition(s, logins(), ON);
        assert!(!view(&s).chip);
        let (s, _) = transition(
            s,
            Event::Logins {
                origin: other.into(),
                candidates: vec![alice()],
            },
            ON,
        );
        let (s, _) = transition(s, Event::Open, ON);
        let (_, effects) = transition(s, Event::Choose(0), ON);
        assert!(
            matches!(&effects[..], [Effect::Fill { frame, origin, .. }] if frame == "f" && origin == other)
        );
    }

    #[test]
    fn the_main_frames_form_wins_over_an_iframes() {
        // An iframe's offer with rows stands against another iframe, not against the main frame.
        let (s, _) = run(vec![frame_forms("f", ORIGIN, false), logins()], ON);
        let (s2, effects) = transition(s.clone(), frame_forms("g", ORIGIN, false), ON);
        assert!(effects.is_empty());
        assert!(matches!(&s2.phase, Phase::Offer(o) if o.frame == "f"));
        let (s, effects) = transition(s, frame_forms("main", ORIGIN, true), ON);
        assert_eq!(effects.len(), 1);
        assert!(matches!(&s.phase, Phase::Offer(o) if o.frame == "main" && o.main));
        // The main frame's offer stands against an iframe's report.
        let (s, _) = transition(s, logins(), ON);
        let (s, _) = transition(s, frame_forms("f", ORIGIN, false), ON);
        assert!(matches!(&s.phase, Phase::Offer(o) if o.frame == "main"));
    }

    #[test]
    fn an_offer_waiting_for_its_logins_stands_against_another_frames_report() {
        // The basic smoke page: a main frame of ORIGIN (login alice) and an
        // iframe of B (login nina); "g" is a second iframe of B.
        const B: &str = "https://login.b.test";
        let origin_of = |frame: &str| if frame == "main" { ORIGIN } else { B };
        let user_of = |origin: &str| if origin == ORIGIN { "alice" } else { "nina" };
        let answer = |origin: &str, users: &[&str]| Event::Logins {
            origin: origin.into(),
            candidates: users
                .iter()
                .map(|u| Candidate {
                    id: format!("{origin} {u}"),
                    username: (*u).into(),
                })
                .collect(),
        };
        let main = || frame_forms("main", ORIGIN, true);
        let iframe = |frame: &str| frame_forms(frame, B, false);
        // Name, events in order, the offer's frame and its logins at the end.
        let rows: Vec<(&str, Vec<Event>, &str, Vec<&str>)> = vec![
            (
                "the stalled order: main reports and settles, the iframe reports, then the answers",
                vec![
                    main(),
                    settled("main", true, login_page()),
                    iframe("f"),
                    answer(B, &["nina"]),
                    answer(ORIGIN, &["alice"]),
                ],
                "main",
                vec!["alice"],
            ),
            (
                "the stalled order, the main's answer first",
                vec![
                    main(),
                    settled("main", true, login_page()),
                    iframe("f"),
                    answer(ORIGIN, &["alice"]),
                    answer(B, &["nina"]),
                ],
                "main",
                vec!["alice"],
            ),
            (
                "the iframe reports while the main's logins are fetched",
                vec![main(), iframe("f"), answer(ORIGIN, &["alice"])],
                "main",
                vec!["alice"],
            ),
            (
                "a main answered with no logins gives way to the iframe",
                vec![
                    main(),
                    answer(ORIGIN, &[]),
                    iframe("f"),
                    answer(B, &["nina"]),
                ],
                "f",
                vec!["nina"],
            ),
            (
                "the main replaces an iframe still waiting, whose answer is dropped",
                vec![
                    iframe("f"),
                    main(),
                    answer(B, &["nina"]),
                    answer(ORIGIN, &["alice"]),
                ],
                "main",
                vec!["alice"],
            ),
            (
                "an iframe still waiting stands against another iframe",
                vec![iframe("f"), iframe("g"), answer(B, &["nina"])],
                "f",
                vec!["nina"],
            ),
        ];
        for (name, events, frame, users) in rows {
            let s = events.into_iter().fold(AutofillState::default(), |s, e| {
                let (s, _) = transition(s, e, ON);
                // At every step: the offer is its frame's origin's, and so are its logins.
                if let Phase::Offer(o) | Phase::Menu(o) = &s.phase {
                    assert_eq!(o.origin, origin_of(&o.frame), "{name}");
                    for row in o.rows() {
                        assert_eq!(
                            row,
                            MenuRow::Login {
                                username: user_of(&o.origin).into()
                            },
                            "{name}"
                        );
                    }
                }
                s
            });
            let Phase::Offer(o) = &s.phase else {
                panic!("{name}: {:?}", s.phase)
            };
            let logins: Vec<MenuRow> = users
                .iter()
                .map(|u| MenuRow::Login {
                    username: (*u).into(),
                })
                .collect();
            assert_eq!((o.frame.as_str(), o.rows()), (frame, logins), "{name}");
        }
    }

    #[test]
    fn a_frames_save_names_its_origin() {
        let other = "https://login.b.test";
        let (s, _) = run(
            vec![
                Event::Submit {
                    frame: "f".into(),
                    origin: other.into(),
                    form: filled_login("alice", "pw"),
                    at: 0,
                },
                Event::Settled {
                    frame: "main".into(),
                    origin: other.into(),
                    is_main: true,
                    forms: vec![],
                },
                Event::Verdict {
                    origin: other.into(),
                    creds: creds("alice", "pw"),
                    verdict: SaveVerdict::New,
                },
            ],
            ON,
        );
        assert_eq!(
            view(&s).prompt,
            Some((SaveKind::New, "alice".into(), other.into()))
        );
    }

    #[test]
    fn events_name_their_origin() {
        assert_eq!(
            frame_forms("f", "https://b.test", false).origin(),
            Some("https://b.test")
        );
        assert_eq!(Event::Open.origin(), None);
    }
}
