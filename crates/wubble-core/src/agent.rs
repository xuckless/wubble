//! Agent sessions (docs/research/mcp-server.md section 0): the lifecycle of one MCP client's
//! session as one enum and one pure [`transition`], the [`Grant`] the owner approves, the
//! per-session [`Token`], and the activity log ([`log`]).
//!
//! Core holds no secret value: a grant names variables and points each at a vault login
//! ([`SecretBinding`]); the host resolves the value at the moment of filling and only when
//! [`Grant::secret_for`] says the page's origin is the login's. Generating a token, showing the
//! sheet, opening and tearing down the session's Space are [`Effect`]s the host executes.

pub mod keys;
pub mod log;
pub mod refs;

use std::collections::BTreeMap;
use std::fmt;

use crate::autofill::may_fill;
use crate::boost::Pattern;

/// The host's name for a session; also what every logged call is tagged with.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SessionId(pub String);

/// Whether the owner watches the session's window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Windowed,
    Headless,
}

/// A vault login, by the vault's own id.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoginRef(pub String);

/// What a named secret points at: the login and the origin that login belongs to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SecretBinding {
    pub login: LoginRef,
    /// `scheme://host[:port]`, as the vault stores it.
    pub origin: String,
}

/// Why a grant could not be built.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GrantError {
    /// A site entry that is not a `boost::Pattern`.
    BadSite(String),
    /// A variable name that is empty or not `[A-Za-z_][A-Za-z0-9_]*`.
    BadName(String),
}

/// What the owner lets a session use. Built by the approval sheet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Grant {
    secrets: BTreeMap<String, SecretBinding>,
    /// Empty: any site.
    sites: Vec<Pattern>,
    pub mode: Mode,
}

fn valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

impl Grant {
    pub fn new(
        secrets: impl IntoIterator<Item = (String, SecretBinding)>,
        sites: &[String],
        mode: Mode,
    ) -> Result<Grant, GrantError> {
        let secrets: BTreeMap<_, _> = secrets.into_iter().collect();
        if let Some(bad) = secrets.keys().find(|n| !valid_name(n)) {
            return Err(GrantError::BadName(bad.clone()));
        }
        let sites = sites
            .iter()
            .map(|s| Pattern::parse(s).ok_or_else(|| GrantError::BadSite(s.clone())))
            .collect::<Result<_, _>>()?;
        Ok(Grant {
            secrets,
            sites,
            mode,
        })
    }

    /// The variable names the agent may mention; the only thing about secrets it ever sees.
    pub fn secret_names(&self) -> impl Iterator<Item = &str> {
        self.secrets.keys().map(String::as_str)
    }

    /// Whether the session may visit `url`: any page when the owner gave no sites.
    pub fn allows_site(&self, url: &str) -> bool {
        self.sites.is_empty() || self.sites.iter().any(|p| p.matches(url))
    }

    /// The login to fill variable `name` from into a frame of `frame_url` under `top_url`:
    /// `None` for an ungranted name, or when the vault's rule (`autofill::may_fill`) says the
    /// page is not the login's origin.
    pub fn secret_for(&self, name: &str, top_url: &str, frame_url: &str) -> Option<&LoginRef> {
        let b = self.secrets.get(name)?;
        may_fill(top_url, frame_url, &b.origin).then_some(&b.login)
    }
}

/// What the approval sheet lets the owner change before Allow: the secret names the client
/// asked for (removed only: none can be bound to a login yet) and the sites (empty: any site).
/// The host keeps one per waiting session; Allow builds the grant from it ([`answer`]).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Lists {
    pub secrets: Vec<String>,
    pub sites: Vec<String>,
}

/// One change on the sheet's list editors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListEdit<'a> {
    RemoveSecret(&'a str),
    RemoveSite(&'a str),
    /// A site added at the end, or put in the place of `replacing` (an edit in place).
    PutSite {
        replacing: Option<&'a str>,
        site: &'a str,
    },
}

/// Why a site was refused; the editor shows the note under its draft.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListRefusal {
    NotASite,
    Listed,
}

impl ListRefusal {
    pub fn note(self) -> &'static str {
        match self {
            ListRefusal::NotASite => {
                "Not a site: example.com, *.example.com or https://example.com/docs/*"
            }
            ListRefusal::Listed => "Already in the list",
        }
    }
}

impl Lists {
    /// The client's request, as the sheet first shows it.
    pub fn asked(secrets: &[String], sites: &[String]) -> Lists {
        Lists {
            secrets: secrets.to_vec(),
            sites: sites.to_vec(),
        }
    }

    /// Applies `edit`; a site must be a [`Pattern`] and not already listed (trimmed). Removing
    /// what is not listed changes nothing.
    pub fn edit(&mut self, edit: ListEdit) -> Result<(), ListRefusal> {
        match edit {
            ListEdit::RemoveSecret(name) => self.secrets.retain(|s| s != name),
            ListEdit::RemoveSite(site) => self.sites.retain(|s| s != site),
            ListEdit::PutSite { replacing, site } => {
                let site = site.trim();
                if Pattern::parse(site).is_none() {
                    return Err(ListRefusal::NotASite);
                }
                if self
                    .sites
                    .iter()
                    .any(|s| s == site && Some(s.as_str()) != replacing)
                {
                    return Err(ListRefusal::Listed);
                }
                match replacing.and_then(|r| self.sites.iter().position(|s| s == r)) {
                    Some(i) => site.clone_into(&mut self.sites[i]),
                    None => self.sites.push(site.to_owned()),
                }
            }
        }
        Ok(())
    }
}

/// A per-session bearer secret. Made by the host (`Effect::MintToken`), never printed, compared
/// without early exit.
#[derive(Clone, PartialEq, Eq)]
pub struct Token(String);

impl Token {
    pub fn new(value: impl Into<String>) -> Token {
        Token(value.into())
    }

    pub fn expose(&self) -> &str {
        &self.0
    }

    /// Whether `presented` is this token; the whole length is compared either way.
    pub fn matches(&self, presented: &str) -> bool {
        let (a, b) = (self.0.as_bytes(), presented.as_bytes());
        let diff = a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y));
        diff == 0 && a.len() == b.len()
    }
}

impl fmt::Debug for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Token(..)")
    }
}

/// What the client asked for, shown on the approval sheet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    /// The calling program, as the host found it (peer credentials), never the client's claim alone.
    pub client: String,
    /// Variable names the client says it wants.
    pub wants_secrets: Vec<String>,
    pub wants_mode: Mode,
}

/// The settings a request is judged against, read by the host when the client asks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Env {
    /// `agent.mcp_enabled`.
    pub mcp_enabled: bool,
    /// `settings::remote_port` of `devtools.remote_port`.
    pub remote_port: Option<u16>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// `agent.mcp_enabled` is off: nothing listens, so this is only a backstop.
    Disabled,
    /// Remote debugging is on: any local program already drives every tab.
    RemoteDebugging,
}

/// Checked first, whatever else is asked.
pub fn admit(env: Env) -> Result<(), Refusal> {
    if !env.mcp_enabled {
        Err(Refusal::Disabled)
    } else if env.remote_port.is_some() {
        Err(Refusal::RemoteDebugging)
    } else {
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StopReason {
    Refused(Refusal),
    Denied,
    /// The owner's stop command (works while headless).
    Stopped,
    ClientGone,
    BrowserQuit,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum State {
    /// Connected, not yet asked for anything.
    Requested,
    AwaitingApproval(Request),
    Running {
        grant: Grant,
        /// `None` until the host answered `Effect::MintToken`.
        token: Option<Token>,
    },
    Stopped(StopReason),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    ClientAsks(Request, Env),
    Approve(Grant),
    /// The host's answer to `Effect::MintToken`.
    TokenMinted(Token),
    Deny,
    Stop,
    ClientDisconnects,
    BrowserQuits,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effect {
    ShowApprovalSheet(Request),
    DismissApprovalSheet,
    /// Make a random [`Token`] and send `Event::TokenMinted`.
    MintToken,
    /// Open the session's Space and container, windowed or headless.
    OpenSpace(Mode),
    /// Close that Space's tabs, drop the token and the socket.
    Teardown,
}

impl State {
    pub fn is_stopped(&self) -> bool {
        matches!(self, State::Stopped(_))
    }

    /// The grant of a running session whose token is `presented`: what every call is checked by.
    pub fn authorize(&self, presented: &str) -> Option<&Grant> {
        match self {
            State::Running {
                grant,
                token: Some(t),
            } if t.matches(presented) => Some(grant),
            _ => None,
        }
    }
}

/// The one lifecycle. An event that means nothing in the state leaves it as it is; a stopped
/// session ignores everything.
pub fn transition(state: State, event: Event) -> (State, Vec<Effect>) {
    use Effect::*;
    use State::*;
    match (state, event) {
        (Requested, Event::ClientAsks(request, env)) => match admit(env) {
            Err(r) => (Stopped(StopReason::Refused(r)), vec![]),
            Ok(()) => (
                AwaitingApproval(request.clone()),
                vec![ShowApprovalSheet(request)],
            ),
        },
        (AwaitingApproval(_), Event::Approve(grant)) => {
            let mode = grant.mode;
            (
                Running { grant, token: None },
                vec![DismissApprovalSheet, OpenSpace(mode), MintToken],
            )
        }
        (AwaitingApproval(_), Event::Deny) => (
            Stopped(StopReason::Denied),
            vec![DismissApprovalSheet, Teardown],
        ),
        (Running { grant, token: None }, Event::TokenMinted(t)) => (
            Running {
                grant,
                token: Some(t),
            },
            vec![],
        ),
        (AwaitingApproval(_), Event::Stop) => (
            Stopped(StopReason::Stopped),
            vec![DismissApprovalSheet, Teardown],
        ),
        (AwaitingApproval(_), Event::ClientDisconnects) => (
            Stopped(StopReason::ClientGone),
            vec![DismissApprovalSheet, Teardown],
        ),
        (AwaitingApproval(_), Event::BrowserQuits) => (
            Stopped(StopReason::BrowserQuit),
            vec![DismissApprovalSheet, Teardown],
        ),
        (Requested, Event::Stop) => (Stopped(StopReason::Stopped), vec![]),
        (Requested, Event::ClientDisconnects) => (Stopped(StopReason::ClientGone), vec![]),
        (Requested, Event::BrowserQuits) => (Stopped(StopReason::BrowserQuit), vec![]),
        (Running { .. }, Event::Stop) => (Stopped(StopReason::Stopped), vec![Teardown]),
        (Running { .. }, Event::ClientDisconnects) => {
            (Stopped(StopReason::ClientGone), vec![Teardown])
        }
        (Running { .. }, Event::BrowserQuits) => (Stopped(StopReason::BrowserQuit), vec![Teardown]),
        (state, _) => (state, vec![]),
    }
}

/// The owner's answer to the sheet as the session's event: Allow grants the sites asked for in
/// the mode the sheet's choice says. No secret is bound to a vault login here (the sheet has no
/// login picker yet), so a grant holds none; a grant that cannot be built denies.
pub fn answer(sites: &[String], allow: bool, headless: bool) -> Event {
    if !allow {
        return Event::Deny;
    }
    let mode = if headless {
        Mode::Headless
    } else {
        Mode::Windowed
    };
    match Grant::new([], sites, mode) {
        Ok(grant) => Event::Approve(grant),
        Err(_) => Event::Deny,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sheet_lists_remove_add_and_edit_sites() {
        let mut l = Lists::asked(&["A".into(), "B".into()], &["example.com".into()]);
        assert_eq!(l.edit(ListEdit::RemoveSecret("A")), Ok(()));
        assert_eq!(l.secrets, ["B"]);
        let put = |replacing, site| ListEdit::PutSite { replacing, site };
        assert_eq!(l.edit(put(None, " github.com ")), Ok(()));
        assert_eq!(l.sites, ["example.com", "github.com"]);
        assert_eq!(l.edit(put(None, "github.com")), Err(ListRefusal::Listed));
        assert_eq!(l.edit(put(None, "not a site")), Err(ListRefusal::NotASite));
        assert_eq!(l.edit(put(Some("example.com"), "*.example.org")), Ok(()));
        assert_eq!(l.sites, ["*.example.org", "github.com"]);
        // an edit that keeps its own text is no duplicate
        assert_eq!(l.edit(put(Some("github.com"), "github.com")), Ok(()));
        assert_eq!(l.edit(ListEdit::RemoveSite("github.com")), Ok(()));
        assert_eq!(l.edit(ListEdit::RemoveSite("gone.com")), Ok(()));
        assert_eq!(l.sites, ["*.example.org"]);
    }

    #[test]
    fn allow_grants_the_edited_sites_and_an_empty_list_any_site() {
        let mut l = Lists::asked(&[], &["example.com".into()]);
        let Event::Approve(g) = answer(&l.sites, true, false) else {
            panic!("not approved");
        };
        assert!(!g.allows_site("https://github.com/"));
        assert_eq!(g.mode, Mode::Windowed);
        l.edit(ListEdit::RemoveSite("example.com")).unwrap();
        let Event::Approve(g) = answer(&l.sites, true, true) else {
            panic!("not approved");
        };
        assert!(g.allows_site("https://github.com/"));
        assert_eq!(g.mode, Mode::Headless);
        assert_eq!(g.secret_names().count(), 0);
    }

    const ON: Env = Env {
        mcp_enabled: true,
        remote_port: None,
    };

    fn request() -> Request {
        Request {
            client: "claude".into(),
            wants_secrets: vec!["GITHUB_PASSWORD".into()],
            wants_mode: Mode::Headless,
        }
    }

    fn grant(mode: Mode) -> Grant {
        Grant::new(
            [(
                "GITHUB_PASSWORD".to_string(),
                SecretBinding {
                    login: LoginRef("l1".into()),
                    origin: "https://github.com".into(),
                },
            )],
            &[],
            mode,
        )
        .unwrap()
    }

    fn running(token: Option<&str>) -> State {
        State::Running {
            grant: grant(Mode::Windowed),
            token: token.map(Token::new),
        }
    }

    fn awaiting() -> State {
        State::AwaitingApproval(request())
    }

    #[test]
    fn a_client_asking_shows_the_sheet() {
        let (s, e) = transition(State::Requested, Event::ClientAsks(request(), ON));
        assert_eq!(s, awaiting());
        assert_eq!(e, vec![Effect::ShowApprovalSheet(request())]);
    }

    #[test]
    fn the_refusal_comes_before_anything_else() {
        let off = Env {
            mcp_enabled: false,
            ..ON
        };
        let port = Env {
            remote_port: Some(9222),
            ..ON
        };
        // Both: the disabled setting is named first.
        let both = Env {
            mcp_enabled: false,
            remote_port: Some(9222),
        };
        for (env, why) in [
            (off, Refusal::Disabled),
            (port, Refusal::RemoteDebugging),
            (both, Refusal::Disabled),
        ] {
            let (s, e) = transition(State::Requested, Event::ClientAsks(request(), env));
            assert_eq!(s, State::Stopped(StopReason::Refused(why)));
            assert!(e.is_empty());
        }
        assert_eq!(admit(ON), Ok(()));
    }

    #[test]
    fn approval_opens_the_space_and_asks_for_a_token() {
        let (s, e) = transition(awaiting(), Event::Approve(grant(Mode::Headless)));
        assert_eq!(
            s,
            State::Running {
                grant: grant(Mode::Headless),
                token: None
            }
        );
        assert_eq!(
            e,
            vec![
                Effect::DismissApprovalSheet,
                Effect::OpenSpace(Mode::Headless),
                Effect::MintToken
            ]
        );
        let (s, e) = transition(s, Event::TokenMinted(Token::new("t")));
        assert!(e.is_empty());
        assert!(matches!(s, State::Running { token: Some(_), .. }));
    }

    #[test]
    fn a_second_token_does_not_replace_the_first() {
        let (s, e) = transition(running(Some("a")), Event::TokenMinted(Token::new("b")));
        assert_eq!(s, running(Some("a")));
        assert!(e.is_empty());
    }

    #[test]
    fn deny_stop_disconnect_and_quit_while_awaiting() {
        for (ev, why) in [
            (Event::Deny, StopReason::Denied),
            (Event::Stop, StopReason::Stopped),
            (Event::ClientDisconnects, StopReason::ClientGone),
            (Event::BrowserQuits, StopReason::BrowserQuit),
        ] {
            let (s, e) = transition(awaiting(), ev);
            assert_eq!(s, State::Stopped(why));
            assert_eq!(e, vec![Effect::DismissApprovalSheet, Effect::Teardown]);
        }
    }

    #[test]
    fn stop_disconnect_and_quit_while_running_tear_down() {
        for (ev, why) in [
            (Event::Stop, StopReason::Stopped),
            (Event::ClientDisconnects, StopReason::ClientGone),
            (Event::BrowserQuits, StopReason::BrowserQuit),
        ] {
            for token in [None, Some("t")] {
                let (s, e) = transition(running(token), ev.clone());
                assert_eq!(s, State::Stopped(why));
                assert_eq!(e, vec![Effect::Teardown]);
            }
        }
    }

    #[test]
    fn before_asking_a_session_only_ends() {
        for (ev, why) in [
            (Event::Stop, StopReason::Stopped),
            (Event::ClientDisconnects, StopReason::ClientGone),
            (Event::BrowserQuits, StopReason::BrowserQuit),
        ] {
            let (s, e) = transition(State::Requested, ev);
            assert_eq!(s, State::Stopped(why));
            assert!(e.is_empty());
        }
    }

    #[test]
    fn events_that_mean_nothing_change_nothing() {
        let nothing: [(State, Event); 6] = [
            (State::Requested, Event::Approve(grant(Mode::Windowed))),
            (State::Requested, Event::Deny),
            (awaiting(), Event::ClientAsks(request(), ON)),
            (awaiting(), Event::TokenMinted(Token::new("t"))),
            (running(None), Event::Approve(grant(Mode::Headless))),
            (running(Some("t")), Event::Deny),
        ];
        for (state, ev) in nothing {
            let (s, e) = transition(state.clone(), ev);
            assert_eq!(s, state);
            assert!(e.is_empty());
        }
    }

    #[test]
    fn a_stopped_session_stays_stopped() {
        let stopped = State::Stopped(StopReason::Denied);
        for ev in [
            Event::ClientAsks(request(), ON),
            Event::Approve(grant(Mode::Windowed)),
            Event::TokenMinted(Token::new("t")),
            Event::Deny,
            Event::Stop,
            Event::ClientDisconnects,
            Event::BrowserQuits,
        ] {
            let (s, e) = transition(stopped.clone(), ev);
            assert_eq!(s, stopped);
            assert!(e.is_empty());
        }
    }

    #[test]
    fn the_token_authorizes_only_a_running_session() {
        assert!(running(Some("tok")).authorize("tok").is_some());
        assert!(running(Some("tok")).authorize("tok2").is_none());
        assert!(running(Some("tok")).authorize("to").is_none());
        assert!(running(Some("tok")).authorize("").is_none());
        assert!(running(None).authorize("").is_none());
        assert!(awaiting().authorize("tok").is_none());
        assert!(
            State::Stopped(StopReason::Stopped)
                .authorize("tok")
                .is_none()
        );
        assert_eq!(format!("{:?}", Token::new("tok")), "Token(..)");
    }

    #[test]
    fn a_secret_fills_only_on_its_logins_origin() {
        let g = grant(Mode::Windowed);
        let gh = "https://github.com/login";
        assert_eq!(
            g.secret_for("GITHUB_PASSWORD", gh, gh),
            Some(&LoginRef("l1".into()))
        );
        // Not granted.
        assert_eq!(g.secret_for("OTHER", gh, gh), None);
        // Another origin, a look-alike, http, another port.
        for url in [
            "https://evil.test/",
            "https://github.com.evil.test/",
            "http://github.com/",
            "https://github.com:8443/",
        ] {
            assert_eq!(g.secret_for("GITHUB_PASSWORD", url, url), None, "{url}");
        }
        // A cross-origin frame under the right top page.
        assert_eq!(
            g.secret_for("GITHUB_PASSWORD", gh, "https://ads.test/frame"),
            None
        );
        assert_eq!(g.secret_names().collect::<Vec<_>>(), ["GITHUB_PASSWORD"]);
    }

    #[test]
    fn the_site_list_limits_pages_when_given() {
        let any = grant(Mode::Windowed);
        assert!(any.allows_site("https://anything.test/"));
        let some = Grant::new([], &["github.com".to_string()], Mode::Headless).unwrap();
        assert!(some.allows_site("https://gist.github.com/x"));
        assert!(!some.allows_site("https://evil.test/github.com"));
    }

    #[test]
    fn the_sheet_s_answer_is_an_event() {
        let sites = ["github.com".to_string()];
        assert_eq!(answer(&sites, false, true), Event::Deny);
        let Event::Approve(g) = answer(&sites, true, true) else {
            panic!("allow approves");
        };
        assert_eq!(g.mode, Mode::Headless);
        assert!(g.allows_site("https://github.com/x"));
        assert!(!g.allows_site("https://evil.test/"));
        assert_eq!(g.secret_names().count(), 0);
        let Event::Approve(g) = answer(&[], true, false) else {
            panic!("allow approves");
        };
        assert_eq!(g.mode, Mode::Windowed);
        // a site that is no pattern cannot be granted
        assert_eq!(answer(&["a b".to_string()], true, false), Event::Deny);
    }

    #[test]
    fn a_bad_grant_is_refused() {
        let b = SecretBinding {
            login: LoginRef("l".into()),
            origin: "https://a.test".into(),
        };
        assert_eq!(
            Grant::new([("1X".to_string(), b.clone())], &[], Mode::Headless),
            Err(GrantError::BadName("1X".into()))
        );
        assert_eq!(
            Grant::new([(String::new(), b)], &[], Mode::Headless),
            Err(GrantError::BadName(String::new()))
        );
        assert_eq!(
            Grant::new([], &["a b".to_string()], Mode::Headless),
            Err(GrantError::BadSite("a b".into()))
        );
    }
}
