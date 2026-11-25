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

/// One request the prompt asks about: the page's, as it made it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PermissionRequest {
    /// The requesting origin as the engine reported it; for a link, the page's URL.
    pub origin: String,
    pub ask: Ask,
}

/// What is asked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ask {
    /// The engine's request: answered back to it under `id` with `kinds` unchanged.
    Engine {
        id: RequestId,
        kinds: Vec<PermissionKind>,
    },
    /// A link to open in another app (`wubble_core::external`): the host's own question,
    /// never answered to the engine.
    ExternalLink { url: String },
}

impl PermissionRequest {
    /// The key decisions are remembered under; `None` for opaque origins. An external link's
    /// is per site and scheme.
    pub fn site(&self) -> Option<String> {
        match &self.ask {
            Ask::ExternalLink { url } => external::key_for(url, &self.origin),
            Ask::Engine { .. } => site::origin_of(&self.origin),
        }
    }

    /// The kinds as the prompt knows them; `None` if any is never prompted
    /// for.
    pub fn asked(&self) -> Option<Vec<Permission>> {
        let kinds = match &self.ask {
            Ask::ExternalLink { .. } => return Some(vec![Permission::ExternalLinks]),
            Ask::Engine { kinds, .. } => kinds,
        };
        let mut asked: Vec<Permission> = kinds
            .iter()
            .map(|&k| permission_of(k))
            .collect::<Option<_>>()?;
        asked.sort();
        asked.dedup();
        Some(asked)
    }

    /// Tells this request apart from the one shown before it, so a new prompt starts with
    /// its toggle off: the engine's id, or the link's hash.
    pub fn serial(&self) -> i32 {
        use std::hash::{DefaultHasher, Hash, Hasher};
        match &self.ask {
            Ask::Engine { id, .. } => i32::try_from(id.0).unwrap_or(i32::MAX),
            Ask::ExternalLink { url } => {
                let mut h = DefaultHasher::new();
                (url, &self.origin).hash(&mut h);
                h.finish() as i32
            }
        }
    }

    fn engine_id(&self) -> Option<RequestId> {
        match self.ask {
            Ask::Engine { id, .. } => Some(id),
            Ask::ExternalLink { .. } => None,
        }
    }

    /// What it gets before anyone is asked.
    pub fn verdict(&self, table: &SitePermissions) -> Verdict {
        match self.asked() {
            None => Verdict::Deny,
            Some(asked) => table.decide(self.site().as_deref(), &asked),
        }
    }
}

/// The open prompt's content (`overlay::Kind::Permission` says whether it is open): `shown` is
/// asked; requests made meanwhile wait in `queued`, in order (Chromium's
/// `PermissionRequestManager` queues them too).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PermissionPrompt {
    pub shown: PermissionRequest,
    pub queued: VecDeque<PermissionRequest>,
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
    /// The page committed a new document: links the old one asked to open are dropped
    /// unanswered (the engine dismisses its own requests itself).
    PageChanged,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PermissionAnswer {
    pub reply: Reply,
    /// A decision to remember: (site, kinds, decision).
    pub remember: Option<(String, Vec<Permission>, Decision)>,
}

/// Who an answer goes to and what it says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reply {
    /// To the engine: the kinds granted, all of the request's or none (deny).
    Engine {
        id: RequestId,
        granted: Vec<PermissionKind>,
    },
    /// To the host: open the link in its app, or not.
    External { url: String, open: bool },
}

impl PermissionAnswer {
    fn new(request: &PermissionRequest, grant: bool) -> Self {
        let reply = match &request.ask {
            Ask::Engine { id, kinds } => Reply::Engine {
                id: *id,
                granted: if grant { kinds.clone() } else { vec![] },
            },
            Ask::ExternalLink { url } => Reply::External {
                url: url.clone(),
                open: grant,
            },
        };
        Self {
            reply,
            remember: None,
        }
    }
}

/// Every answer this step gives (a request with a remembered verdict is
/// answered at once; one made while a prompt is open waits its turn). `None`: no live prompt
/// (the view gone is the overlays' store's: no event comes here).
pub fn permission(
    state: Option<PermissionPrompt>,
    event: PermissionEvent,
    table: &SitePermissions,
) -> (Option<PermissionPrompt>, Vec<PermissionAnswer>) {
    use PermissionEvent::*;
    let open = |shown, queued| Some(PermissionPrompt { shown, queued });
    match (state, event) {
        // A link already waiting is not asked about twice.
        (Some(PermissionPrompt { shown, queued }), Requested(new))
            if matches!(new.ask, Ask::ExternalLink { .. })
                && (shown == new || queued.contains(&new)) =>
        {
            (open(shown, queued), vec![])
        }
        (state, Requested(new)) => match (new.verdict(table), state) {
            (Verdict::Grant, state) => (state, vec![PermissionAnswer::new(&new, true)]),
            (Verdict::Deny, state) => (state, vec![PermissionAnswer::new(&new, false)]),
            (Verdict::Ask, None) => next_prompt(VecDeque::from([new]), table),
            (Verdict::Ask, Some(PermissionPrompt { shown, mut queued })) => {
                queued.push_back(new);
                (open(shown, queued), vec![])
            }
        },
        (Some(PermissionPrompt { shown, queued }), Answer { allow, remember }) => {
            let mut answer = PermissionAnswer::new(&shown, allow);
            let mut table = table.clone();
            // Chromium's "always" applies to Open only: a link refused once is asked again.
            let remember = remember && (allow || matches!(shown.ask, Ask::Engine { .. }));
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
        (Some(PermissionPrompt { shown, queued }), Dismissed(ids)) => {
            let gone = |r: &PermissionRequest| r.engine_id().is_some_and(|id| ids.contains(&id));
            drop_requests(shown, queued, gone, table)
        }
        (Some(PermissionPrompt { shown, queued }), PageChanged) => {
            let link = |r: &PermissionRequest| matches!(r.ask, Ask::ExternalLink { .. });
            drop_requests(shown, queued, link, table)
        }
        (None, Answer { .. } | Dismissed(_) | PageChanged) => (None, vec![]),
    }
}

/// The prompt without the requests `gone` picks, unanswered; the next one is shown if the
/// shown one went.
fn drop_requests(
    shown: PermissionRequest,
    mut queued: VecDeque<PermissionRequest>,
    gone: impl Fn(&PermissionRequest) -> bool,
    table: &SitePermissions,
) -> (Option<PermissionPrompt>, Vec<PermissionAnswer>) {
    queued.retain(|r| !gone(r));
    if gone(&shown) {
        next_prompt(queued, table)
    } else {
        (Some(PermissionPrompt { shown, queued }), vec![])
    }
}

/// The first of `queued` that still needs asking, shown; those before it
/// that a remembered decision now covers are answered.
fn next_prompt(
    mut queued: VecDeque<PermissionRequest>,
    table: &SitePermissions,
) -> (Option<PermissionPrompt>, Vec<PermissionAnswer>) {
    let mut answers = vec![];
    while let Some(r) = queued.pop_front() {
        match r.verdict(table) {
            Verdict::Grant => answers.push(PermissionAnswer::new(&r, true)),
            Verdict::Deny => answers.push(PermissionAnswer::new(&r, false)),
            Verdict::Ask => return (Some(PermissionPrompt { shown: r, queued }), answers),
        }
    }
    (None, answers)
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
/// link, and "Always for <site> and <scheme> links". No toggle (`remember` empty) where
/// nothing could be remembered: an opaque origin.
pub fn permission_text(r: &PermissionRequest) -> PromptText {
    let site = wubble_core::url::address_parts(&r.origin).0;
    let named = !(site.is_empty() || site.ends_with(':'));
    let rememberable = r.site().is_some();
    if let Ask::ExternalLink { url } = &r.ask {
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
            remember: if rememberable {
                format!("Always for {site} and {scheme} links")
            } else {
                String::new()
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
        remember: if rememberable {
            "Remember for this site".into()
        } else {
            String::new()
        },
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AuthEvent {
    Requested(AuthRequest),
    Submit { user: String, password: Secret },
    Cancel,
    Dismissed(Vec<RequestId>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthAnswer {
    pub id: RequestId,
    /// `None` cancels.
    pub credentials: Option<(String, Secret)>,
}

/// The sign-in sheet's request (`None`: none waits) after one event, and its answer.
pub fn auth(
    state: Option<AuthRequest>,
    event: AuthEvent,
) -> (Option<AuthRequest>, Option<AuthAnswer>) {
    use AuthEvent::*;
    let cancel = |r: &AuthRequest| AuthAnswer {
        id: r.id,
        credentials: None,
    };
    match (state, event) {
        (Some(old), Requested(new)) => (Some(new), Some(cancel(&old))),
        (None, Requested(new)) => (Some(new), None),
        (Some(r), Submit { user, password }) => (
            None,
            Some(AuthAnswer {
                id: r.id,
                credentials: Some((user, password)),
            }),
        ),
        (Some(r), Cancel) => (None, Some(cancel(&r))),
        (Some(r), Dismissed(ids)) if !ids.contains(&r.id) => (Some(r), None),
        (_, Dismissed(_)) | (None, Submit { .. } | Cancel) => (None, None),
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
            origin: "https://a.test/".into(),
            ask: Ask::Engine {
                id: RequestId(id),
                kinds: kinds.to_vec(),
            },
        }
    }

    fn ask(id: u32, kinds: &[PermissionKind]) -> PermissionEvent {
        PermissionEvent::Requested(request(id, kinds))
    }

    fn granted(id: u32, kinds: &[PermissionKind]) -> PermissionAnswer {
        PermissionAnswer {
            reply: Reply::Engine {
                id: RequestId(id),
                granted: kinds.to_vec(),
            },
            remember: None,
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
        let (s, a) = permission(None, ask(1, &[CameraStream]), &t);
        assert!(a.is_empty() && s.is_some());
        let answer = PermissionEvent::Answer {
            allow: true,
            remember: false,
        };
        let (s, a) = permission(s, answer.clone(), &t);
        assert_eq!((s.clone(), a), (None, vec![granted(1, &[CameraStream])]));
        assert_eq!(permission(s, answer, &t).1, vec![]);
    }

    #[test]
    fn block_and_escape_deny() {
        let t = SitePermissions::default();
        let (s, _) = permission(None, ask(1, &[Geolocation]), &t);
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
        let (s, _) = permission(None, ask(1, &[MicStream, CameraStream]), &t);
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
        let (s, _) = permission(None, PermissionEvent::Requested(opaque), &t);
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
        let (s, a) = permission(None, ask(1, &[CameraStream]), &t);
        assert_eq!((s, a), (None, vec![granted(1, &[CameraStream])]));
        let (s, a) = permission(None, ask(2, &[Geolocation]), &t);
        assert_eq!((s, a), (None, vec![granted(2, &[])]));
        // Partly decided: asked.
        let (s, a) = permission(None, ask(3, &[CameraStream, MicStream]), &t);
        assert!(a.is_empty() && s.is_some());
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
            let (s, a) = permission(None, ask(1, &kinds), &t);
            assert_eq!((s, a), (None, vec![granted(1, &[])]), "{kinds:?}");
        }
    }

    fn shown(s: &Option<PermissionPrompt>) -> Option<u32> {
        s.as_ref().and_then(|p| p.shown.engine_id()).map(|id| id.0)
    }

    #[test]
    fn a_newer_request_waits_for_the_shown_one() {
        let t = SitePermissions::default();
        let allow = PermissionEvent::Answer {
            allow: true,
            remember: false,
        };
        let (s, _) = permission(None, ask(1, &[Notifications]), &t);
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
        let (s, _) = permission(None, ask(1, &[CameraStream]), &t);
        let (s, _) = permission(s, ask(2, &[CameraStream]), &t);
        let (s, _) = permission(s, ask(3, &[Geolocation]), &t);
        let remember = PermissionEvent::Answer {
            allow: false,
            remember: true,
        };
        let (s, a) = permission(s, remember, &t);
        let replies: Vec<&Reply> = a.iter().map(|a| &a.reply).collect();
        assert_eq!(replies, [&granted(1, &[]).reply, &granted(2, &[]).reply]);
        assert_eq!(shown(&s), Some(3));
    }

    #[test]
    fn dismissing_the_shown_one_shows_the_next() {
        let t = SitePermissions::default();
        let (s, _) = permission(None, ask(1, &[CameraStream]), &t);
        let (s, _) = permission(s, ask(2, &[Geolocation]), &t);
        let (s, _) = permission(s, ask(3, &[MicStream]), &t);
        let gone = PermissionEvent::Dismissed(vec![RequestId(1), RequestId(2)]);
        let (s, a) = permission(s, gone, &t);
        assert_eq!((shown(&s), a), (Some(3), vec![]));
    }

    #[test]
    fn the_engine_answering_closes_without_an_answer() {
        let t = SitePermissions::default();
        let (open, _) = permission(None, ask(1, &[CameraStream]), &t);
        let gone = PermissionEvent::Dismissed(vec![RequestId(1)]);
        assert_eq!(permission(open.clone(), gone, &t), (None, vec![]));
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

    fn mailto() -> PermissionRequest {
        PermissionRequest {
            origin: "https://a.test/".into(),
            ask: Ask::ExternalLink {
                url: "mailto:x@y.test".into(),
            },
        }
    }

    fn opened(open: bool) -> Reply {
        Reply::External {
            url: "mailto:x@y.test".into(),
            open,
        }
    }

    #[test]
    fn an_external_link_is_asked_about_and_remembered_per_site_and_scheme() {
        let t = permission_text(&mailto());
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
        let (s, a) = permission(None, PermissionEvent::Requested(mailto()), &table);
        assert!(a.is_empty() && s.is_some());
        let (s, a) = permission(
            s,
            PermissionEvent::Answer {
                allow: true,
                remember: true,
            },
            &table,
        );
        assert_eq!(s, None);
        assert_eq!(
            a,
            vec![PermissionAnswer {
                reply: opened(true),
                remember: Some((
                    "mailto:https://a.test".into(),
                    vec![Permission::ExternalLinks],
                    Decision::Allow
                )),
            }]
        );
        // Remembered: the next one opens without a prompt.
        let mut table = table;
        table.set(
            "mailto:https://a.test",
            Permission::ExternalLinks,
            Decision::Allow,
        );
        let (s, a) = permission(None, PermissionEvent::Requested(mailto()), &table);
        assert_eq!(s, None);
        assert_eq!(a[0].reply, opened(true));
    }

    #[test]
    fn a_cancelled_link_is_never_remembered() {
        let table = SitePermissions::default();
        let (s, _) = permission(None, PermissionEvent::Requested(mailto()), &table);
        let cancel = PermissionEvent::Answer {
            allow: false,
            remember: true,
        };
        let (_, a) = permission(s, cancel, &table);
        assert_eq!(
            a,
            [PermissionAnswer {
                reply: opened(false),
                remember: None
            }]
        );
    }

    #[test]
    fn a_link_on_an_opaque_page_has_no_toggle() {
        let file = PermissionRequest {
            origin: "file:///x.html".into(),
            ..mailto()
        };
        assert_eq!(permission_text(&file).remember, "");
        let data = PermissionRequest {
            origin: "data:text/html,x".into(),
            ..request(1, &[Geolocation])
        };
        assert_eq!(permission_text(&data).remember, "");
    }

    #[test]
    fn a_new_page_drops_links_and_a_repeated_link_is_asked_once() {
        let t = SitePermissions::default();
        let (s, _) = permission(None, ask(1, &[CameraStream]), &t);
        let (s, _) = permission(s, PermissionEvent::Requested(mailto()), &t);
        let (s, a) = permission(s, PermissionEvent::Requested(mailto()), &t);
        assert!(a.is_empty());
        let Some(PermissionPrompt { queued, .. }) = &s else {
            panic!("open")
        };
        assert_eq!(queued.len(), 1, "the second click is not queued again");
        let (s, a) = permission(s, PermissionEvent::PageChanged, &t);
        assert!(a.is_empty());
        assert_eq!(
            s,
            permission(None, ask(1, &[CameraStream]), &t).0,
            "the engine's request stays, the link goes"
        );
        let (s, _) = permission(None, PermissionEvent::Requested(mailto()), &t);
        assert_eq!(
            permission(s, PermissionEvent::PageChanged, &t),
            (None, vec![]),
            "a shown link goes too"
        );
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
        let (s, a) = auth(None, challenge(1));
        assert_eq!(a, None);
        let (s, a) = auth(
            s,
            AuthEvent::Submit {
                user: "u".into(),
                password: Secret::from("p"),
            },
        );
        assert_eq!(s, None);
        assert_eq!(
            a,
            Some(AuthAnswer {
                id: RequestId(1),
                credentials: Some(("u".into(), Secret::from("p")))
            })
        );
        let (s, _) = auth(None, challenge(2));
        let (_, a) = auth(s, AuthEvent::Cancel);
        assert_eq!(a.unwrap().credentials, None);
        // A newer challenge cancels the older.
        let (s, _) = auth(None, challenge(3));
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
            challenge(2),
            challenge(3),
        ];
        let n = events.len();
        'sequence: for code in 0..n.pow(4) {
            let mut s: Option<AuthRequest> = None;
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
                if let Some(r) = &s
                    && let Dismissed(ids) = &e
                    && ids.contains(&r.id)
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
