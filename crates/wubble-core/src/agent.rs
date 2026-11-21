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
pub mod spot;

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

/// Why a session may not load an address.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VisitRefusal {
    /// Not an http or https address (`file:`, `data:`, `javascript:`, `chrome:`, ...).
    NotWeb,
    /// Not on the grant's sites.
    NotGranted,
}

impl VisitRefusal {
    pub fn note(self) -> &'static str {
        match self {
            VisitRefusal::NotWeb => "only http and https pages can be opened",
            VisitRefusal::NotGranted => "not a site this session may visit: ask with request_more",
        }
    }
}

/// Whether a scheme is the web's (http, https), as a URL parser reads it: without case.
pub fn is_web(url: &str) -> bool {
    url::Url::parse(url).is_ok_and(|u| matches!(u.scheme(), "http" | "https"))
}

/// The address a session may load for `url` (the parser's spelling: `HTTPS://A.test` is
/// `https://a.test/`), or why not.
pub fn may_visit(grant: &Grant, url: &str) -> Result<String, VisitRefusal> {
    let parsed = url::Url::parse(url.trim()).map_err(|_| VisitRefusal::NotWeb)?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(VisitRefusal::NotWeb);
    }
    let url = parsed.to_string();
    if grant.allows_site(&url) {
        Ok(url)
    } else {
        Err(VisitRefusal::NotGranted)
    }
}

/// What a running session asks for on top of its grant (`request_more`), reduced to what is
/// new ([`Grant::missing`]).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct More {
    pub sites: Vec<String>,
    pub secrets: Vec<String>,
}

impl More {
    pub fn is_empty(&self) -> bool {
        self.sites.is_empty() && self.secrets.is_empty()
    }
}

/// The answer a `request_more` gets.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MoreAnswer {
    /// The owner pressed Allow: what the grant gained (a secret name gains nothing until it is
    /// bound to a login).
    Granted(More),
    /// The owner pressed Deny (or Escape): the session runs on with its grant.
    Denied,
    /// Everything asked for is granted already: no sheet.
    NothingNew,
    /// A sheet for this session is up already.
    Busy,
}

/// What the owner lets a session use. Built by the approval sheet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Grant {
    secrets: BTreeMap<String, SecretBinding>,
    /// The site patterns as the owner left them, and parsed. Empty: any site.
    sites: Vec<(String, Pattern)>,
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
            .map(|s| {
                let p = Pattern::parse(s).ok_or_else(|| GrantError::BadSite(s.clone()))?;
                Ok((s.trim().to_owned(), p))
            })
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
        self.sites.is_empty() || self.sites.iter().any(|(_, p)| p.matches(url))
    }

    /// The site patterns granted, as the owner left them; empty: any site.
    pub fn sites(&self) -> impl Iterator<Item = &str> {
        self.sites.iter().map(|(s, _)| s.as_str())
    }

    /// What of `sites` and `secrets` this grant does not hold yet: a site not granted (none,
    /// when any site is), a secret name not granted. A bare site (`github.com`,
    /// `*.github.com`) is held when its host is allowed already, any other pattern when it is
    /// listed as written; without case, repeats once.
    pub fn missing(&self, sites: &[String], secrets: &[String]) -> More {
        let mut more = More::default();
        if !self.sites.is_empty() {
            for site in sites.iter().map(|s| s.trim()) {
                let same = |s: &str| s.eq_ignore_ascii_case(site);
                let bare = site.strip_prefix("*.").unwrap_or(site);
                let covered = !bare.contains('/')
                    && !bare.contains(':')
                    && self.allows_site(&format!("https://{bare}/"))
                    && self.allows_site(&format!("http://{bare}/"));
                if !site.is_empty()
                    && !covered
                    && !self.sites().any(same)
                    && !more.sites.iter().any(|s| same(s))
                {
                    more.sites.push(site.to_owned());
                }
            }
        }
        for name in secrets.iter().map(|s| s.trim()) {
            if !name.is_empty()
                && !self.secrets.contains_key(name)
                && !more.secrets.iter().any(|s| s == name)
            {
                more.secrets.push(name.to_owned());
            }
        }
        more
    }

    /// This grant with `sites` added (a grant of any site stays one); the secrets and the mode
    /// stay.
    pub fn widened(&self, sites: &[String]) -> Result<Grant, GrantError> {
        if self.sites.is_empty() {
            return Ok(self.clone());
        }
        let all: Vec<String> = self
            .sites()
            .map(str::to_owned)
            .chain(sites.iter().cloned())
            .collect();
        Grant::new(self.secrets.clone(), &all, self.mode)
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
    /// The owner's stop command (works while headless), or the agent's `stop_session`.
    Stopped,
    /// The agent ended the session keeping its tabs for the owner (`stop_session`
    /// `keep_tabs`).
    Released,
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
    /// Running on its grant while the owner is asked for more (`request_more`): only what is
    /// new is on the sheet, and only the owner's Allow widens the grant.
    AskingMore {
        grant: Grant,
        token: Token,
        more: More,
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
    /// The agent ends the session and leaves its tabs to the owner.
    Release,
    /// The agent asks for more sites or secret names (`request_more`).
    AskMore {
        sites: Vec<String>,
        secrets: Vec<String>,
    },
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
    /// Leave the Space and its tabs to the owner: ordinary tabs of its container, saved like
    /// any other Space; the token dies.
    Release,
    /// Ask the owner for what is new (a sheet listing only it).
    ShowMoreSheet(More),
    /// Answer the waiting `request_more`.
    AnswerMore(MoreAnswer),
}

impl State {
    pub fn is_stopped(&self) -> bool {
        matches!(self, State::Stopped(_))
    }

    /// The grant of a running session whose token is `presented`: what every call is checked by.
    /// While the owner is asked for more, the grant as it was.
    pub fn authorize(&self, presented: &str) -> Option<&Grant> {
        match self {
            State::Running {
                grant,
                token: Some(t),
            }
            | State::AskingMore {
                grant, token: t, ..
            } if t.matches(presented) => Some(grant),
            _ => None,
        }
    }

    /// The grant a running session works under (also while it asks for more).
    pub fn grant(&self) -> Option<&Grant> {
        match self {
            State::Running { grant, .. } | State::AskingMore { grant, .. } => Some(grant),
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
        (
            Running {
                grant,
                token: Some(token),
            },
            Event::AskMore { sites, secrets },
        ) => {
            let more = grant.missing(&sites, &secrets);
            if more.is_empty() {
                let state = Running {
                    grant,
                    token: Some(token),
                };
                return (state, vec![AnswerMore(MoreAnswer::NothingNew)]);
            }
            (
                AskingMore {
                    grant,
                    token,
                    more: more.clone(),
                },
                vec![ShowMoreSheet(more)],
            )
        }
        (state @ AskingMore { .. }, Event::AskMore { .. }) => {
            (state, vec![AnswerMore(MoreAnswer::Busy)])
        }
        (AskingMore { grant, token, .. }, Event::Approve(widened)) => {
            let granted = More {
                sites: widened
                    .sites()
                    .filter(|s| !grant.sites().any(|g| g == *s))
                    .map(str::to_owned)
                    .collect(),
                secrets: widened
                    .secret_names()
                    .filter(|s| !grant.secret_names().any(|g| g == *s))
                    .map(str::to_owned)
                    .collect(),
            };
            // the mode is the session's, whatever the sheet says
            let grant = Grant {
                mode: grant.mode,
                ..widened
            };
            (
                Running {
                    grant,
                    token: Some(token),
                },
                vec![
                    DismissApprovalSheet,
                    AnswerMore(MoreAnswer::Granted(granted)),
                ],
            )
        }
        (AskingMore { grant, token, .. }, Event::Deny) => (
            Running {
                grant,
                token: Some(token),
            },
            vec![DismissApprovalSheet, AnswerMore(MoreAnswer::Denied)],
        ),
        (AskingMore { .. }, Event::Stop) => (
            Stopped(StopReason::Stopped),
            vec![DismissApprovalSheet, Teardown],
        ),
        (AskingMore { .. }, Event::Release) => (
            Stopped(StopReason::Released),
            vec![DismissApprovalSheet, Release],
        ),
        (AskingMore { .. }, Event::ClientDisconnects) => (
            Stopped(StopReason::ClientGone),
            vec![DismissApprovalSheet, Teardown],
        ),
        (AskingMore { .. }, Event::BrowserQuits) => (
            Stopped(StopReason::BrowserQuit),
            vec![DismissApprovalSheet, Teardown],
        ),
        (Running { .. }, Event::Stop) => (Stopped(StopReason::Stopped), vec![Teardown]),
        (Running { token: Some(_), .. }, Event::Release) => {
            (Stopped(StopReason::Released), vec![Release])
        }
        (Running { .. }, Event::ClientDisconnects) => {
            (Stopped(StopReason::ClientGone), vec![Teardown])
        }
        (Running { .. }, Event::BrowserQuits) => (Stopped(StopReason::BrowserQuit), vec![Teardown]),
        (state, _) => (state, vec![]),
    }
}

/// What the dustbin on an agent Space's row (the Spaces popup) does, by the session that holds
/// the Space now (`None`: none does, the session ended and left its tabs).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Clear {
    /// [`Event::Stop`] to that session alone: its teardown deletes the Space, its tabs and its
    /// container.
    Stop,
    /// The host deletes the Space, its tabs and its container itself, as a teardown does.
    Delete,
}

/// The dustbin on an agent Space's row ([`crate::space::Space::agent`]; a user's Space has none).
pub fn clear(holder: Option<&State>) -> Clear {
    match holder {
        Some(State::Stopped(_)) | None => Clear::Delete,
        Some(_) => Clear::Stop,
    }
}

/// The owner's answer to a sheet for more (`State::AskingMore`): Allow widens `grant` by the
/// sites the owner left on it (`Lists::sites`); Deny keeps the grant. A site that is no pattern
/// denies.
pub fn answer_more(grant: &Grant, sites: &[String], allow: bool) -> Event {
    match grant.widened(sites) {
        Ok(widened) if allow => Event::Approve(widened),
        _ => Event::Deny,
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

    fn on(sites: &[&str]) -> State {
        let sites: Vec<String> = sites.iter().map(|s| s.to_string()).collect();
        State::Running {
            grant: Grant::new([], &sites, Mode::Headless).unwrap(),
            token: Some(Token::new("t")),
        }
    }

    fn ask(sites: &[&str], secrets: &[&str]) -> Event {
        Event::AskMore {
            sites: sites.iter().map(|s| s.to_string()).collect(),
            secrets: secrets.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn asking_for_more_shows_only_what_is_new() {
        let (s, e) = transition(
            on(&["github.com"]),
            ask(
                &[
                    "GitHub.com",
                    "gist.github.com",
                    "x.test",
                    "X.test",
                    "https://github.com/a/*",
                ],
                &["NPM", "NPM"],
            ),
        );
        // github.com covers its subdomains; a pattern with a path is listed as written
        let more = More {
            sites: vec!["x.test".into(), "https://github.com/a/*".into()],
            secrets: vec!["NPM".into()],
        };
        assert_eq!(e, vec![Effect::ShowMoreSheet(more.clone())]);
        let State::AskingMore { grant, more: m, .. } = &s else {
            panic!("{s:?}");
        };
        assert_eq!(*m, more);
        // the old grant still answers the session's calls meanwhile
        assert!(s.authorize("t").is_some());
        assert!(!grant.allows_site("https://x.test/"));
        // a second ask while the sheet is up
        let (s, e) = transition(s, ask(&["x.test"], &[]));
        assert_eq!(e, vec![Effect::AnswerMore(MoreAnswer::Busy)]);
        assert!(matches!(s, State::AskingMore { .. }));
    }

    #[test]
    fn nothing_new_needs_no_sheet() {
        for (state, sites) in [
            (on(&["a.test"]), &["a.test"][..]),
            (on(&[]), &["b.test"][..]),
        ] {
            let (s, e) = transition(state.clone(), ask(sites, &[]));
            assert_eq!(s, state);
            assert_eq!(e, vec![Effect::AnswerMore(MoreAnswer::NothingNew)]);
        }
    }

    #[test]
    fn only_allow_widens_the_grant_and_deny_keeps_the_session() {
        let (asking, _) = transition(on(&["a.test"]), ask(&["b.test"], &["NPM"]));
        let State::AskingMore { grant, .. } = &asking else {
            panic!("asking");
        };
        let grant = grant.clone();
        // Deny: running on the old grant
        let (s, e) = transition(
            asking.clone(),
            answer_more(&grant, &["b.test".into()], false),
        );
        assert_eq!(s, on(&["a.test"]));
        assert_eq!(
            e,
            vec![
                Effect::DismissApprovalSheet,
                Effect::AnswerMore(MoreAnswer::Denied)
            ]
        );
        // Allow, the owner having added one more site on the sheet
        let left = ["b.test".to_string(), "c.test".to_string()];
        let (s, e) = transition(asking.clone(), answer_more(&grant, &left, true));
        let g = s.authorize("t").unwrap();
        assert!(g.allows_site("https://b.test/") && g.allows_site("https://c.test/"));
        assert!(g.allows_site("https://a.test/") && !g.allows_site("https://d.test/"));
        assert_eq!(g.mode, Mode::Headless);
        assert_eq!(
            e,
            vec![
                Effect::DismissApprovalSheet,
                Effect::AnswerMore(MoreAnswer::Granted(More {
                    sites: left.to_vec(),
                    secrets: vec![]
                }))
            ]
        );
        // a site that is no pattern denies
        assert_eq!(answer_more(&grant, &["a b".into()], true), Event::Deny);
    }

    #[test]
    fn a_session_asking_for_more_ends_as_a_running_one() {
        let (asking, _) = transition(on(&["a.test"]), ask(&["b.test"], &[]));
        for (ev, why, last) in [
            (Event::Stop, StopReason::Stopped, Effect::Teardown),
            (Event::Release, StopReason::Released, Effect::Release),
            (
                Event::ClientDisconnects,
                StopReason::ClientGone,
                Effect::Teardown,
            ),
            (
                Event::BrowserQuits,
                StopReason::BrowserQuit,
                Effect::Teardown,
            ),
        ] {
            let (s, e) = transition(asking.clone(), ev);
            assert_eq!(s, State::Stopped(why));
            assert_eq!(e, vec![Effect::DismissApprovalSheet, last]);
        }
    }

    #[test]
    fn release_leaves_the_tabs_and_kills_the_token() {
        let (s, e) = transition(running(Some("t")), Event::Release);
        assert_eq!(s, State::Stopped(StopReason::Released));
        assert_eq!(e, vec![Effect::Release]);
        assert!(s.authorize("t").is_none());
        // before the token exists, and while awaiting approval: nothing
        for state in [running(None), awaiting(), State::Requested] {
            let (s, e) = transition(state.clone(), Event::Release);
            assert_eq!(s, state);
            assert!(e.is_empty());
        }
    }

    #[test]
    fn the_dustbin_stops_a_live_session_and_deletes_a_left_space() {
        let asking = State::AskingMore {
            grant: Grant::new([], &[], Mode::Headless).unwrap(),
            token: Token("t".into()),
            more: More::default(),
        };
        for live in [running(Some("t")), running(None), asking] {
            assert_eq!(clear(Some(&live)), Clear::Stop);
            // and the stop is the teardown that deletes the Space
            let (s, e) = transition(live, Event::Stop);
            assert!(matches!(s, State::Stopped(_)));
            assert!(e.contains(&Effect::Teardown), "{e:?}");
        }
        assert_eq!(clear(None), Clear::Delete);
        assert_eq!(
            clear(Some(&State::Stopped(StopReason::Released))),
            Clear::Delete
        );
    }

    #[test]
    fn only_web_addresses_on_the_grant_s_sites_are_visited() {
        let any = Grant::new([], &[], Mode::Headless).unwrap();
        assert_eq!(
            may_visit(&any, "HTTPS://Example.test/A"),
            Ok("https://example.test/A".into())
        );
        assert_eq!(
            may_visit(&any, " http://a.test "),
            Ok("http://a.test/".into())
        );
        for bad in [
            "file:///etc/passwd",
            "FILE:///etc/passwd",
            "data:text/html,x",
            "javascript:alert(1)",
            "chrome://version",
            "about:blank",
            "example.com",
            "",
        ] {
            assert_eq!(may_visit(&any, bad), Err(VisitRefusal::NotWeb), "{bad}");
            assert!(!is_web(bad), "{bad}");
        }
        let some = Grant::new([], &["a.test".into()], Mode::Headless).unwrap();
        assert!(may_visit(&some, "HTTP://A.TEST/x").is_ok());
        assert_eq!(
            may_visit(&some, "https://b.test/"),
            Err(VisitRefusal::NotGranted)
        );
        assert!(is_web("HTTPS://x.test"));
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
