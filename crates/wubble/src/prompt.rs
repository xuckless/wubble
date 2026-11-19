//! The page's requests that wait on the user, drawn by the shell: the
//! permission prompt ([`PermissionPrompt`]) and the HTTP sign-in sheet
//! ([`Auth`]), each one enum with a pure transition that answers every
//! request exactly once (unless the engine answered it itself).
//!
//! Remembered decisions answer a permission request before any prompt opens
//! (`wubble_core::site`); nothing is granted by default. Wording follows
//! Chromium `components/permissions` ("<site> wants to: Use your camera") and
//! `chrome/browser/ui/login/login_prompt` ("Sign in", "<host> requires a
//! username and password"), BSD-3-Clause; strings only.

use std::collections::VecDeque;

use wubble_core::external;
use wubble_core::site::{self, Decision, Permission, SitePermissions, Verdict};
use wubble_engine::page::{PermissionKind, RequestId};

// ------------------------------------------------------------ permissions

/// The prompt's kind for an engine kind; `None` for what is never prompted
/// for (protected media, desktop capture, unknown bits).
pub fn permission_of(kind: PermissionKind) -> Option<Permission> {
    Some(match kind {
        PermissionKind::CameraStream => Permission::Camera,
        PermissionKind::MicStream => Permission::Microphone,
        PermissionKind::Geolocation => Permission::Geolocation,
        PermissionKind::Notifications => Permission::Notifications,
        PermissionKind::Clipboard => Permission::Clipboard,
        PermissionKind::MidiSysex => Permission::MidiSysex,
        PermissionKind::PointerLock => Permission::PointerLock,
        PermissionKind::KeyboardLock => Permission::KeyboardLock,
        PermissionKind::IdleDetection => Permission::IdleDetection,
        PermissionKind::LocalFonts => Permission::LocalFonts,
        PermissionKind::StorageAccess => Permission::StorageAccess,
        PermissionKind::ProtectedMediaIdentifier
        | PermissionKind::DesktopAudioCapture
        | PermissionKind::DesktopVideoCapture
        | PermissionKind::Other(_) => return None,
        // non_exhaustive: a kind added later is never prompted for.
        _ => return None,
    })
}

/// One permission request as the page made it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PermissionRequest {
    pub id: RequestId,
    /// The requesting origin as the engine reported it.
    pub origin: String,
    /// The engine's kinds, answered back unchanged.
    pub kinds: Vec<PermissionKind>,
    /// A link to open in another app (`wubble_core::external`) instead of engine kinds: the
    /// host's own question, never answered to the engine; `origin` is the page's URL.
    pub external: Option<String>,
}

impl PermissionRequest {
    /// The key decisions are remembered under; `None` for opaque origins. An external link's
    /// is per site and scheme.
    pub fn site(&self) -> Option<String> {
        match &self.external {
            Some(url) => external::key_for(url, &self.origin),
            None => site::origin_of(&self.origin),
        }
    }

    /// The kinds as the prompt knows them; `None` if any is never prompted
    /// for.
    pub fn asked(&self) -> Option<Vec<Permission>> {
        if self.external.is_some() {
            return Some(vec![Permission::ExternalLinks]);
        }
        let mut asked: Vec<Permission> = self
            .kinds
            .iter()
            .map(|&k| permission_of(k))
            .collect::<Option<_>>()?;
        asked.sort();
        asked.dedup();
        Some(asked)
    }

    /// What it gets before anyone is asked.
    pub fn verdict(&self, table: &SitePermissions) -> Verdict {
        match self.asked() {
            None => Verdict::Deny,
            Some(asked) => table.decide(self.site().as_deref(), &asked),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum PermissionPrompt {
    #[default]
    None,
    /// `shown` is asked; requests made meanwhile wait in `queued`, in order
    /// (Chromium's `PermissionRequestManager` queues them too).
    Open {
        shown: PermissionRequest,
        queued: VecDeque<PermissionRequest>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PermissionEvent {
    Requested(PermissionRequest),
    /// Allow or Block; `remember`: for this site from now on. Escape is
    /// Block, not remembered.
    Answer {
        allow: bool,
        remember: bool,
    },
    /// CEF gave up on these requests; welding answered them.
    Dismissed(Vec<RequestId>),
    /// The view closed; welding answered every held request.
    ViewGone,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PermissionAnswer {
    pub id: RequestId,
    /// The kinds granted: all of the request's, or none (deny).
    pub granted: Vec<PermissionKind>,
    /// A decision to remember: (site, kinds, decision).
    pub remember: Option<(String, Vec<Permission>, Decision)>,
    /// An external link's answer, for the host (the engine is not answered): the URL and
    /// whether to open it.
    pub external: Option<(String, bool)>,
}

impl PermissionAnswer {
    fn new(request: &PermissionRequest, grant: bool) -> Self {
        Self {
            id: request.id,
            granted: if grant { request.kinds.clone() } else { vec![] },
            remember: None,
            external: request.external.clone().map(|url| (url, grant)),
        }
    }
}

/// Every answer this step gives (a request with a remembered verdict is
/// answered at once; one made while a prompt is open waits its turn).
pub fn permission(
    state: PermissionPrompt,
    event: PermissionEvent,
    table: &SitePermissions,
) -> (PermissionPrompt, Vec<PermissionAnswer>) {
    use PermissionEvent::*;
    use PermissionPrompt::Open;
    match (state, event) {
        (state, Requested(new)) => match (new.verdict(table), state) {
            (Verdict::Grant, state) => (state, vec![PermissionAnswer::new(&new, true)]),
            (Verdict::Deny, state) => (state, vec![PermissionAnswer::new(&new, false)]),
            (Verdict::Ask, PermissionPrompt::None) => next_prompt(VecDeque::from([new]), table),
            (Verdict::Ask, Open { shown, mut queued }) => {
                queued.push_back(new);
                (Open { shown, queued }, vec![])
            }
        },
        (Open { shown, queued }, Answer { allow, remember }) => {
            let mut answer = PermissionAnswer::new(&shown, allow);
            let mut table = table.clone();
            if remember && let (Some(site), Some(asked)) = (shown.site(), shown.asked()) {
                let decision = if allow {
                    Decision::Allow
                } else {
                    Decision::Block
                };
                for &kind in &asked {
                    table.set(&site, kind, decision);
                }
                answer.remember = Some((site, asked, decision));
            }
            // What was just remembered answers the waiting ones it covers.
            let (next, more) = next_prompt(queued, &table);
            (next, [vec![answer], more].concat())
        }
        (Open { shown, mut queued }, Dismissed(ids)) => {
            queued.retain(|r| !ids.contains(&r.id));
            if ids.contains(&shown.id) {
                next_prompt(queued, table)
            } else {
                (Open { shown, queued }, vec![])
            }
        }
        (_, ViewGone) | (PermissionPrompt::None, Answer { .. } | Dismissed(_)) => {
            (PermissionPrompt::None, vec![])
        }
    }
}

/// The first of `queued` that still needs asking, shown; those before it
/// that a remembered decision now covers are answered.
fn next_prompt(
    mut queued: VecDeque<PermissionRequest>,
    table: &SitePermissions,
) -> (PermissionPrompt, Vec<PermissionAnswer>) {
    let mut answers = vec![];
    while let Some(r) = queued.pop_front() {
        match r.verdict(table) {
            Verdict::Grant => answers.push(PermissionAnswer::new(&r, true)),
            Verdict::Deny => answers.push(PermissionAnswer::new(&r, false)),
            Verdict::Ask => return (PermissionPrompt::Open { shown: r, queued }, answers),
        }
    }
    (PermissionPrompt::None, answers)
}

/// What the prompt says and its controls are called.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PromptText {
    pub title: String,
    pub lines: Vec<String>,
    /// The "remember" toggle.
    pub remember: String,
    pub allow: &'static str,
    pub block: &'static str,
}

/// The longest link an external-link prompt shows whole.
const LINK_CHARS: usize = 80;

/// What the prompt says: the site, and one line per kind; for a link to another app,
/// Chromium's "Open <app>?" with the scheme for the app (wubble does not look the app up), the
/// link, and "Always for <site> and <scheme> links".
pub fn permission_text(r: &PermissionRequest) -> PromptText {
    let site = wubble_core::url::address_parts(&r.origin).0;
    let named = !(site.is_empty() || site.ends_with(':'));
    if let Some(url) = &r.external {
        let scheme = external::scheme(url).unwrap_or_default();
        return PromptText {
            title: format!("Open {scheme} link?"),
            lines: vec![
                format!(
                    "{} wants to open this link in another app:",
                    if named { site } else { "This page" }
                ),
                wubble_core::url::elide_middle(url, LINK_CHARS).into_owned(),
            ],
            remember: if named {
                format!("Always for {site} and {scheme} links")
            } else {
                format!("Always for {scheme} links on this page")
            },
            allow: "Open",
            block: "Cancel",
        };
    }
    let lines = r
        .asked()
        .unwrap_or_default()
        .into_iter()
        .map(|p| p.ask_text().to_string())
        .collect();
    PromptText {
        title: format!("{} wants to", if named { site } else { "This page" }),
        lines,
        remember: "Remember for this site".into(),
        allow: "Allow",
        block: "Block",
    }
}

// ------------------------------------------------------------------- auth

/// A typed password: never printed, wiped on drop.
pub use wubble_core::secret::Secret;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthRequest {
    pub id: RequestId,
    pub host: String,
    pub port: u16,
    pub realm: String,
    pub proxy: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Auth {
    #[default]
    None,
    Open(AuthRequest),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AuthEvent {
    Requested(AuthRequest),
    Submit { user: String, password: Secret },
    Cancel,
    Dismissed(Vec<RequestId>),
    ViewGone,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthAnswer {
    pub id: RequestId,
    /// `None` cancels.
    pub credentials: Option<(String, Secret)>,
}

pub fn auth(state: Auth, event: AuthEvent) -> (Auth, Option<AuthAnswer>) {
    use AuthEvent::*;
    let cancel = |r: &AuthRequest| AuthAnswer {
        id: r.id,
        credentials: None,
    };
    match (state, event) {
        (Auth::Open(old), Requested(new)) => (Auth::Open(new), Some(cancel(&old))),
        (Auth::None, Requested(new)) => (Auth::Open(new), None),
        (Auth::Open(r), Submit { user, password }) => (
            Auth::None,
            Some(AuthAnswer {
                id: r.id,
                credentials: Some((user, password)),
            }),
        ),
        (Auth::Open(r), Cancel) => (Auth::None, Some(cancel(&r))),
        (Auth::Open(r), Dismissed(ids)) if !ids.contains(&r.id) => (Auth::Open(r), None),
        (_, Dismissed(_) | ViewGone) | (Auth::None, Submit { .. } | Cancel) => (Auth::None, None),
    }
}

/// The sheet's title and message.
pub fn auth_text(r: &AuthRequest) -> (String, String) {
    let who = if r.proxy {
        format!("The proxy {}:{}", r.host, r.port)
    } else {
        r.host.clone()
    };
    let mut message = format!("{who} requires a username and password.");
    if !r.realm.is_empty() {
        message.push_str(&format!(" It says: \u{201c}{}\u{201d}", r.realm));
    }
    (format!("Sign in to {}", r.host), message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use PermissionKind::*;

    fn request(id: u32, kinds: &[PermissionKind]) -> PermissionRequest {
        PermissionRequest {
            id: RequestId(id),
            origin: "https://a.test/".into(),
            kinds: kinds.to_vec(),
            external: None,
        }
    }

    fn ask(id: u32, kinds: &[PermissionKind]) -> PermissionEvent {
        PermissionEvent::Requested(request(id, kinds))
    }

    fn granted(id: u32, kinds: &[PermissionKind]) -> PermissionAnswer {
        PermissionAnswer {
            id: RequestId(id),
            granted: kinds.to_vec(),
            remember: None,
            external: None,
        }
    }

    #[test]
    fn every_engine_kind_maps_or_is_never_prompted() {
        let prompted = [
            CameraStream,
            MicStream,
            Geolocation,
            Notifications,
            Clipboard,
            MidiSysex,
            PointerLock,
            KeyboardLock,
            IdleDetection,
            LocalFonts,
            StorageAccess,
        ];
        let mapped: Vec<Permission> = prompted.iter().filter_map(|&k| permission_of(k)).collect();
        // Every kind but the save-password and popup decisions, which no prompt asks for, and
        // external links, which the host asks about itself.
        let asked = Permission::ALL
            .iter()
            .filter(|&&p| {
                !matches!(
                    p,
                    Permission::SavePasswords | Permission::Popups | Permission::ExternalLinks
                )
            })
            .count();
        assert_eq!(mapped.len(), asked);
        for never in [
            ProtectedMediaIdentifier,
            DesktopAudioCapture,
            DesktopVideoCapture,
            Other(1 << 30),
        ] {
            assert_eq!(permission_of(never), None);
        }
    }

    #[test]
    fn a_request_opens_and_answers_once() {
        let t = SitePermissions::default();
        let (s, a) = permission(PermissionPrompt::None, ask(1, &[CameraStream]), &t);
        assert!(a.is_empty() && matches!(s, PermissionPrompt::Open { .. }));
        let answer = PermissionEvent::Answer {
            allow: true,
            remember: false,
        };
        let (s, a) = permission(s, answer.clone(), &t);
        assert_eq!(
            (s.clone(), a),
            (PermissionPrompt::None, vec![granted(1, &[CameraStream])])
        );
        assert_eq!(permission(s, answer, &t).1, vec![]);
    }

    #[test]
    fn block_and_escape_deny() {
        let t = SitePermissions::default();
        let (s, _) = permission(PermissionPrompt::None, ask(1, &[Geolocation]), &t);
        let (_, a) = permission(
            s,
            PermissionEvent::Answer {
                allow: false,
                remember: false,
            },
            &t,
        );
        assert_eq!(a, vec![granted(1, &[])]);
    }

    #[test]
    fn remembering_names_the_site_and_kinds() {
        let t = SitePermissions::default();
        let (s, _) = permission(
            PermissionPrompt::None,
            ask(1, &[MicStream, CameraStream]),
            &t,
        );
        let (_, a) = permission(
            s,
            PermissionEvent::Answer {
                allow: false,
                remember: true,
            },
            &t,
        );
        assert_eq!(
            a[0].remember,
            Some((
                "https://a.test".into(),
                vec![Permission::Camera, Permission::Microphone],
                Decision::Block
            ))
        );
        // An opaque origin is never remembered.
        let opaque = PermissionRequest {
            origin: "data:text/html,x".into(),
            ..request(2, &[Geolocation])
        };
        let (s, _) = permission(
            PermissionPrompt::None,
            PermissionEvent::Requested(opaque),
            &t,
        );
        let (_, a) = permission(
            s,
            PermissionEvent::Answer {
                allow: true,
                remember: true,
            },
            &t,
        );
        assert_eq!(a, vec![granted(2, &[Geolocation])]);
    }

    #[test]
    fn remembered_decisions_answer_without_a_prompt() {
        let mut t = SitePermissions::default();
        t.set("https://a.test", Permission::Camera, Decision::Allow);
        t.set("https://a.test", Permission::Geolocation, Decision::Block);
        let (s, a) = permission(PermissionPrompt::None, ask(1, &[CameraStream]), &t);
        assert_eq!(
            (s, a),
            (PermissionPrompt::None, vec![granted(1, &[CameraStream])])
        );
        let (s, a) = permission(PermissionPrompt::None, ask(2, &[Geolocation]), &t);
        assert_eq!((s, a), (PermissionPrompt::None, vec![granted(2, &[])]));
        // Partly decided: asked.
        let (s, a) = permission(
            PermissionPrompt::None,
            ask(3, &[CameraStream, MicStream]),
            &t,
        );
        assert!(a.is_empty() && matches!(s, PermissionPrompt::Open { .. }));
        // A remembered answer leaves an open prompt alone.
        let (s2, a) = permission(s.clone(), ask(4, &[CameraStream]), &t);
        assert_eq!((s2, a), (s, vec![granted(4, &[CameraStream])]));
    }

    #[test]
    fn never_prompted_kinds_are_denied_at_once() {
        let t = SitePermissions::default();
        for kinds in [
            vec![DesktopVideoCapture],
            vec![CameraStream, ProtectedMediaIdentifier],
            vec![Other(7)],
            vec![],
        ] {
            let (s, a) = permission(PermissionPrompt::None, ask(1, &kinds), &t);
            assert_eq!(
                (s, a),
                (PermissionPrompt::None, vec![granted(1, &[])]),
                "{kinds:?}"
            );
        }
    }

    fn shown(s: &PermissionPrompt) -> Option<u32> {
        match s {
            PermissionPrompt::Open { shown, .. } => Some(shown.id.0),
            PermissionPrompt::None => None,
        }
    }

    #[test]
    fn a_newer_request_waits_for_the_shown_one() {
        let t = SitePermissions::default();
        let allow = PermissionEvent::Answer {
            allow: true,
            remember: false,
        };
        let (s, _) = permission(PermissionPrompt::None, ask(1, &[Notifications]), &t);
        let (s, a) = permission(s, ask(2, &[Geolocation]), &t);
        assert_eq!((shown(&s), a), (Some(1), vec![]));
        let (s, a) = permission(s, allow.clone(), &t);
        assert_eq!(
            (shown(&s), a),
            (Some(2), vec![granted(1, &[Notifications])])
        );
        let (s, a) = permission(s, allow, &t);
        assert_eq!((shown(&s), a), (None, vec![granted(2, &[Geolocation])]));
    }

    #[test]
    fn a_remembered_answer_covers_the_waiting_ones() {
        let t = SitePermissions::default();
        let (s, _) = permission(PermissionPrompt::None, ask(1, &[CameraStream]), &t);
        let (s, _) = permission(s, ask(2, &[CameraStream]), &t);
        let (s, _) = permission(s, ask(3, &[Geolocation]), &t);
        let remember = PermissionEvent::Answer {
            allow: false,
            remember: true,
        };
        let (s, a) = permission(s, remember, &t);
        let ids: Vec<u32> = a.iter().map(|a| a.id.0).collect();
        assert_eq!(ids, [1, 2]);
        assert!(a.iter().all(|a| a.granted.is_empty()));
        assert_eq!(shown(&s), Some(3));
    }

    #[test]
    fn dismissing_the_shown_one_shows_the_next() {
        let t = SitePermissions::default();
        let (s, _) = permission(PermissionPrompt::None, ask(1, &[CameraStream]), &t);
        let (s, _) = permission(s, ask(2, &[Geolocation]), &t);
        let (s, _) = permission(s, ask(3, &[MicStream]), &t);
        let gone = PermissionEvent::Dismissed(vec![RequestId(1), RequestId(2)]);
        let (s, a) = permission(s, gone, &t);
        assert_eq!((shown(&s), a), (Some(3), vec![]));
    }

    #[test]
    fn the_engine_answering_closes_without_an_answer() {
        let t = SitePermissions::default();
        let (open, _) = permission(PermissionPrompt::None, ask(1, &[CameraStream]), &t);
        for e in [
            PermissionEvent::ViewGone,
            PermissionEvent::Dismissed(vec![RequestId(1)]),
        ] {
            assert_eq!(
                permission(open.clone(), e, &t),
                (PermissionPrompt::None, vec![])
            );
        }
        let (s, a) = permission(
            open.clone(),
            PermissionEvent::Dismissed(vec![RequestId(9)]),
            &t,
        );
        assert_eq!((s, a), (open, vec![]));
    }

    #[test]
    fn prompt_text() {
        let t = permission_text(&request(1, &[MicStream, CameraStream]));
        assert_eq!(t.title, "a.test wants to");
        assert_eq!(t.lines, ["Use your camera", "Use your microphone"]);
        assert_eq!((t.allow, t.block), ("Allow", "Block"));
    }

    fn mailto(id: u32) -> PermissionRequest {
        PermissionRequest {
            external: Some("mailto:x@y.test".into()),
            ..request(id, &[])
        }
    }

    #[test]
    fn an_external_link_is_asked_about_and_remembered_per_site_and_scheme() {
        let t = permission_text(&mailto(1));
        assert_eq!(t.title, "Open mailto link?");
        assert_eq!(
            t.lines,
            [
                "a.test wants to open this link in another app:",
                "mailto:x@y.test"
            ]
        );
        assert_eq!(t.remember, "Always for a.test and mailto links");
        assert_eq!((t.allow, t.block), ("Open", "Cancel"));

        let table = SitePermissions::default();
        let (s, a) = permission(
            PermissionPrompt::None,
            PermissionEvent::Requested(mailto(1)),
            &table,
        );
        assert!(a.is_empty() && matches!(s, PermissionPrompt::Open { .. }));
        let (s, a) = permission(
            s,
            PermissionEvent::Answer {
                allow: true,
                remember: true,
            },
            &table,
        );
        assert_eq!(s, PermissionPrompt::None);
        assert_eq!(
            a,
            vec![PermissionAnswer {
                id: RequestId(1),
                granted: vec![],
                remember: Some((
                    "mailto:https://a.test".into(),
                    vec![Permission::ExternalLinks],
                    Decision::Allow
                )),
                external: Some(("mailto:x@y.test".into(), true)),
            }]
        );
        // Remembered: the next one opens without a prompt.
        let mut table = table;
        table.set(
            "mailto:https://a.test",
            Permission::ExternalLinks,
            Decision::Allow,
        );
        let (s, a) = permission(
            PermissionPrompt::None,
            PermissionEvent::Requested(mailto(2)),
            &table,
        );
        assert_eq!(s, PermissionPrompt::None);
        assert_eq!(a[0].external, Some(("mailto:x@y.test".into(), true)));
    }

    fn challenge(id: u32) -> AuthEvent {
        AuthEvent::Requested(AuthRequest {
            id: RequestId(id),
            host: "a.test".into(),
            port: 443,
            realm: "Staff".into(),
            proxy: false,
        })
    }

    #[test]
    fn auth_submits_both_fields_or_cancels() {
        let (s, a) = auth(Auth::None, challenge(1));
        assert_eq!(a, None);
        let (s, a) = auth(
            s,
            AuthEvent::Submit {
                user: "u".into(),
                password: Secret::from("p"),
            },
        );
        assert_eq!(s, Auth::None);
        assert_eq!(
            a,
            Some(AuthAnswer {
                id: RequestId(1),
                credentials: Some(("u".into(), Secret::from("p")))
            })
        );
        let (s, _) = auth(Auth::None, challenge(2));
        let (_, a) = auth(s, AuthEvent::Cancel);
        assert_eq!(a.unwrap().credentials, None);
        // A newer challenge cancels the older.
        let (s, _) = auth(Auth::None, challenge(3));
        let (_, a) = auth(s, challenge(4));
        assert_eq!(
            a,
            Some(AuthAnswer {
                id: RequestId(3),
                credentials: None
            })
        );
    }

    #[test]
    fn a_password_is_never_printed() {
        let e = AuthEvent::Submit {
            user: "u".into(),
            password: Secret::from("hunter2"),
        };
        assert!(!format!("{e:?}").contains("hunter2"));
    }

    #[test]
    fn auth_text_names_the_host_and_realm() {
        let AuthEvent::Requested(r) = challenge(1) else {
            unreachable!()
        };
        let (title, message) = auth_text(&r);
        assert_eq!(title, "Sign in to a.test");
        assert_eq!(
            message,
            "a.test requires a username and password. It says: \u{201c}Staff\u{201d}"
        );
        let proxy = AuthRequest {
            proxy: true,
            realm: String::new(),
            ..r
        };
        assert_eq!(
            auth_text(&proxy).1,
            "The proxy a.test:443 requires a username and password."
        );
    }

    /// Every ordering of events answers each id at most once, and never
    /// one the engine answered.
    #[test]
    fn auth_sequences_answer_each_id_at_most_once() {
        use AuthEvent::*;
        let events = [
            challenge(1),
            Submit {
                user: "u".into(),
                password: Secret::default(),
            },
            Cancel,
            Dismissed(vec![RequestId(1), RequestId(2)]),
            ViewGone,
            challenge(2),
            challenge(3),
        ];
        let n = events.len();
        'sequence: for code in 0..n.pow(4) {
            let mut s = Auth::None;
            let (mut answered, mut engine, mut requested) = (vec![], vec![], vec![]);
            let mut c = code;
            for _ in 0..4 {
                let e = events[c % n].clone();
                c /= n;
                if let Requested(r) = &e {
                    if requested.contains(&r.id) {
                        continue 'sequence;
                    }
                    requested.push(r.id);
                }
                if let Auth::Open(r) = &s
                    && match &e {
                        ViewGone => true,
                        Dismissed(ids) => ids.contains(&r.id),
                        _ => false,
                    }
                {
                    engine.push(r.id);
                }
                let (next, a) = auth(s, e);
                s = next;
                if let Some(a) = a {
                    assert!(!answered.contains(&a.id), "{code}: twice");
                    assert!(!engine.contains(&a.id), "{code}: after the engine");
                    answered.push(a.id);
                }
            }
        }
    }
}
