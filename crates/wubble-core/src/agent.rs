//! Agent sessions (docs/research/mcp-server.md section 0): the lifecycle of one MCP client's
//! session as one enum and one pure [`transition`], the [`Grant`] the owner approves, the
//! per-session [`Token`], and the activity log ([`log`]).
//!
//! Core holds no secret value: a grant names variables and points each at a vault login
//! ([`SecretBinding`]); the host resolves the value at the moment of filling and only when
//! [`Grant::secret_for`] says the page's origin is the login's. Generating a token, showing the
//! sheet, opening and tearing down the session's Space are [`Effect`]s the host executes.

pub mod forms;
pub mod harness;
pub mod hosting;
pub mod jsonc;
pub mod keys;
pub mod log;
pub mod manifest;
pub mod refs;
pub mod spot;
pub mod upload;

use std::collections::BTreeMap;
use std::fmt;

use crate::autofill::may_fill;
use crate::boost::Pattern;
use crate::space::{ContainerId, SpaceId};

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

/// Which part of a login a secret name types.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    Username,
    Password,
}

/// The part the sheet suggests for a secret name (the owner picks; docs/research/mcp-server.md
/// 0.3): one ending in `USER`, `USERNAME`, `LOGIN` or `EMAIL` (any case) the login's username,
/// any other its password. Only a suggestion: `GITHUB_ID` may well be a username.
pub fn suggested_part(name: &str) -> Part {
    let upper = name.to_ascii_uppercase();
    if ["USER", "USERNAME", "LOGIN", "EMAIL"]
        .iter()
        .any(|end| upper.ends_with(end))
    {
        Part::Username
    } else {
        Part::Password
    }
}

/// What a named secret points at: the login, the origin that login belongs to, and which of its
/// parts the name types.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SecretBinding {
    pub login: LoginRef,
    /// `scheme://host[:port]`, as the vault stores it.
    pub origin: String,
    pub part: Part,
}

/// One granted site whose sign-ins the owner shared with a session ("Use my sign-ins for these
/// sites"): the site as written, and the Space (container) its cookies were copied from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SharedSite {
    pub site: String,
    pattern: Pattern,
    pub from: ContainerId,
    /// The Space's name, for the sheet and the activity log.
    pub space: String,
}

/// Where a session's filled secrets could be read back by a page's script: the tabs a secret
/// was typed into (for the rest of the session, wherever they go) and the origins it was typed
/// on (any of the session's tabs while it shows one). `evaluate` is refused there; the
/// session owns it, and nothing (`request_more` neither) lifts it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Tainted {
    tabs: std::collections::BTreeSet<crate::tab::TabId>,
    origins: std::collections::BTreeSet<String>,
}

impl Tainted {
    /// A secret was typed into `tab`, on the page at `url`.
    pub fn filled(&mut self, tab: crate::tab::TabId, url: &str) {
        self.tabs.insert(tab);
        if let Some(origin) = crate::autofill::web_origin(url) {
            self.origins.insert(origin);
        }
    }

    /// Whether a script may not run in `tab`, now at `url`.
    pub fn refuses_scripts(&self, tab: crate::tab::TabId, url: &str) -> bool {
        self.tabs.contains(&tab)
            || crate::autofill::web_origin(url).is_some_and(|o| self.origins.contains(&o))
    }
}

/// The cookies a copy for `sites` takes ([`shares_cookie`]): the sites as patterns.
pub fn cookie_scope(sites: &[String]) -> Vec<Pattern> {
    sites.iter().filter_map(|s| Pattern::parse(s)).collect()
}

/// The one matcher of which cookies "Use my sign-ins" copies: those a page of one of `scope`'s
/// sites would be sent (`boost::Pattern::sends_cookie`; `domain` as CEF names it, a leading dot
/// for a domain cookie).
pub fn shares_cookie(scope: &[Pattern], domain: &str, path: &str, secure: bool) -> bool {
    scope.iter().any(|p| p.sends_cookie(domain, path, secure))
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

/// Whether a session's tab may go to `url` by any top-level navigation (the agent's own, a
/// link, a script, a form, each hop of a redirect, history): a blank page, or what
/// [`may_visit`] allows. The grant's sites are where the tab may be, not what its pages may
/// fetch: sub-resources are never asked.
pub fn may_go(grant: &Grant, url: &str) -> Result<(), VisitRefusal> {
    if is_blank(url) {
        return Ok(());
    }
    may_visit(grant, url).map(|_| ())
}

/// Whether `url` is the blank page, with or without a fragment (Chromium commits
/// `about:blank#blocked` for a navigation it blocked itself).
pub fn is_blank(url: &str) -> bool {
    url.split_once('#').map_or(url, |(page, _)| page) == "about:blank"
}

/// The agent's error for a navigation [`may_go`] refused: the site named, and the way on.
pub fn refused_note(url: &str) -> String {
    match url::Url::parse(url) {
        Ok(u) if matches!(u.scheme(), "http" | "https") => format!(
            "the page tried to go to {}, which is not on this session's sites: the navigation \
             was refused and the tab stayed where it was (ask for the site with request_more)",
            u.host_str().unwrap_or_default()
        ),
        _ => format!(
            "the page tried to go to {url}, which is not a web address: the navigation was \
             refused and the tab stayed where it was"
        ),
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
    /// The owner allowed, sharing sign-ins, but they could not be copied: nothing was granted,
    /// the session runs on with what it had.
    NotCopied,
}

/// What the owner lets a session use. Built by the approval sheet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Grant {
    secrets: BTreeMap<String, SecretBinding>,
    /// The site patterns as the owner left them, and parsed. Empty: any site.
    sites: Vec<(String, Pattern)>,
    pub mode: Mode,
    /// The granted sites whose sign-ins the owner shared (copied in), each with its source.
    shared: Vec<SharedSite>,
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
            shared: Vec::new(),
        })
    }

    /// This grant with the owner's sign-ins for `sites` shared, taken from container `from`
    /// (Space `space`): only sites this grant lists (never with a grant of any site, which
    /// lists none) and that name a site ([`Pattern::names_a_site`]: never a public suffix),
    /// added to those shared before, each keeping the Space it first came from.
    pub fn with_signins(mut self, from: ContainerId, space: &str, sites: &[String]) -> Grant {
        for site in sites.iter().map(|s| s.trim()) {
            if let Some((s, p)) = self.sites.iter().find(|(s, _)| s == site)
                && p.names_a_site()
                && !self.shared.iter().any(|t| t.site == *s)
            {
                self.shared.push(SharedSite {
                    site: s.clone(),
                    pattern: p.clone(),
                    from,
                    space: space.to_owned(),
                });
            }
        }
        self
    }

    /// The sites whose sign-ins the owner shared, as written.
    pub fn signin_sites(&self) -> impl Iterator<Item = &str> {
        self.shared.iter().map(|s| s.site.as_str())
    }

    /// The shared sites and where each came from.
    pub fn shared(&self) -> &[SharedSite] {
        &self.shared
    }

    /// Whether the page at `url` is on a site whose sign-ins the owner shared.
    pub fn signs_in(&self, url: &str) -> bool {
        self.shared.iter().any(|s| s.pattern.matches(url))
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
                    && self.allows_site(&format!("https://{bare}/__wubble_probe__"))
                    && self.allows_site(&format!("http://{bare}/__wubble_probe__"))
                    && self.allows_site(&format!("https://sub.{bare}/__wubble_probe__"))
                    && self.allows_site(&format!("http://sub.{bare}/__wubble_probe__"));
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

    /// This grant with `sites` and `secrets` added (a grant of any site stays one); the mode
    /// and what was shared stay.
    pub fn widened(
        &self,
        sites: &[String],
        secrets: impl IntoIterator<Item = (String, SecretBinding)>,
    ) -> Result<Grant, GrantError> {
        let all: Vec<String> = if self.sites.is_empty() {
            vec![]
        } else {
            self.sites()
                .map(str::to_owned)
                .chain(sites.iter().cloned())
                .collect()
        };
        let secrets = self.secrets.clone().into_iter().chain(secrets);
        Ok(Grant {
            shared: self.shared.clone(),
            ..Grant::new(secrets, &all, self.mode)?
        })
    }

    /// The login (and part) to fill variable `name` from into a frame of `frame_url` under
    /// `top_url`: `None` for an ungranted name, or when the vault's rule (`autofill::may_fill`)
    /// says the page is not the login's origin.
    pub fn secret_for(&self, name: &str, top_url: &str, frame_url: &str) -> Option<&SecretBinding> {
        let b = self.secrets.get(name)?;
        may_fill(top_url, frame_url, &b.origin).then_some(b)
    }

    /// The binding of granted name `name`, wherever the page is (the sheet's and the log's).
    pub fn binding(&self, name: &str) -> Option<&SecretBinding> {
        self.secrets.get(name)
    }
}

/// What the approval sheet lets the owner change before Allow: the secret names the client
/// asked for (removed, or bound to a saved login: an unbound name is not granted), the sites
/// (empty: any site), and whether the owner's sign-ins for those sites are shared, from which
/// Space. The host keeps one per waiting session; Allow builds the grant from it ([`answer`]).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Lists {
    pub secrets: Vec<String>,
    pub sites: Vec<String>,
    /// The logins the owner picked, by name.
    pub bound: BTreeMap<String, SecretBinding>,
    /// "Use my sign-ins for these sites": off until the owner turns it on.
    pub share: bool,
    /// The Space to take them from; `None`: the one the owner is in.
    pub from: Option<SpaceId>,
}

/// One change on the sheet's list editors and choices.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListEdit<'a> {
    /// A name stands for this login (`None`: for none).
    Bind {
        name: &'a str,
        login: Option<&'a SecretBinding>,
    },
    /// "Use my sign-ins for these sites", on or off.
    Share(bool),
    /// The Space the sign-ins come from.
    ShareFrom(SpaceId),
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
            ..Lists::default()
        }
    }

    /// Whether the sheet offers "Use my sign-ins": only for a list of sites (never any site).
    pub fn may_share(&self) -> bool {
        !self.sites.is_empty()
    }

    /// The listed sites whose sign-ins are never shared (a public suffix or a single label:
    /// `com`, `*.co.uk`, `github.io`); they may still be visited.
    pub fn unshareable(&self) -> Vec<&str> {
        self.sites
            .iter()
            .filter(|s| Pattern::parse(s).is_some_and(|p| !p.names_a_site()))
            .map(String::as_str)
            .collect()
    }

    /// The bindings Allow grants: those of the names still listed.
    fn granted_secrets(&self) -> impl Iterator<Item = (String, SecretBinding)> + '_ {
        self.bound
            .iter()
            .filter(|(name, _)| self.secrets.contains(name))
            .map(|(n, b)| (n.clone(), b.clone()))
    }

    /// Applies `edit`; a site must be a [`Pattern`] and not already listed (trimmed). Removing
    /// what is not listed changes nothing.
    pub fn edit(&mut self, edit: ListEdit) -> Result<(), ListRefusal> {
        match edit {
            ListEdit::Bind { name, login } => {
                if self.secrets.iter().any(|s| s == name) {
                    match login {
                        Some(b) => {
                            self.bound.insert(name.to_owned(), b.clone());
                        }
                        None => {
                            self.bound.remove(name);
                        }
                    }
                }
            }
            ListEdit::Share(on) => self.share = on,
            ListEdit::ShareFrom(space) => self.from = Some(space),
            ListEdit::RemoveSecret(name) => {
                self.secrets.retain(|s| s != name);
                self.bound.remove(name);
            }
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

/// The debugging port that refuses every session: the one this run's engine opened (a start
/// switch, so turning the setting off changes nothing until the restart: any program can still
/// drive every tab), else the one the setting asks for at the next start.
pub fn refusing_port(opened: Option<u16>, stored: Option<u16>) -> Option<u16> {
    opened.or(stored)
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
    /// The owner's sign-ins could not be copied in (CEF refused, or the copy did not end in
    /// time): the session does not start without them.
    SignInsNotCopied,
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
    /// Allowed with the owner's sign-ins: they are being copied into the session's container
    /// (`Effect::CopyCookies`) before the session goes on. A new session has no token yet (its
    /// `start_session` waits); a running one (`request_more`) works under `before` meanwhile and
    /// is answered `granted` once the copy is done.
    Copying {
        grant: Grant,
        token: Option<Token>,
        before: Option<Grant>,
        granted: Option<More>,
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
    /// The host's answer to `Effect::CopyCookies`: the copy ended (each cookie visited and set).
    CookiesCopied,
    /// The copy failed (CEF refused it) or did not end within the host's bound.
    CookiesNotCopied,
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
    /// Copy into the session's container the cookies of container `from` (Space `space`)
    /// that pages of `sites` would be sent ([`shares_cookie`]); answered with
    /// `Event::CookiesCopied`.
    CopyCookies {
        from: ContainerId,
        space: String,
        sites: Vec<String>,
    },
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
            }
            | State::Copying {
                before: Some(grant),
                token: Some(t),
                ..
            } if t.matches(presented) => Some(grant),
            _ => None,
        }
    }

    /// The grant a running session works under (also while it asks for more, and while its
    /// sign-ins are copied: the grant it had before).
    pub fn grant(&self) -> Option<&Grant> {
        match self {
            State::Running { grant, .. } | State::AskingMore { grant, .. } => Some(grant),
            State::Copying { grant, before, .. } => Some(before.as_ref().unwrap_or(grant)),
            _ => None,
        }
    }
}

/// The session whose Space and container hold `tab` now, by the model, with its grant: the one
/// owner of which navigation rule a tab's view carries. `sessions`: each running session's id,
/// Space, container and grant. A tab moved out of a session's Space is no longer held; one moved
/// in is.
pub fn holder<K: Clone>(
    model: &crate::model::Model,
    tab: crate::tab::TabId,
    sessions: &[(K, crate::space::SpaceId, crate::space::ContainerId, &Grant)],
) -> Option<(K, Grant)> {
    sessions.iter().find_map(|(id, space, container, grant)| {
        model
            .holds(*space, *container, tab)
            .then(|| (id.clone(), (*grant).clone()))
    })
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
            match copy_of(&grant, None) {
                // the sign-ins first: the handle (and so any navigation) waits for the copy
                Some(copy) => (
                    Copying {
                        grant,
                        token: None,
                        before: None,
                        granted: None,
                    },
                    vec![DismissApprovalSheet, OpenSpace(mode), copy],
                ),
                None => (
                    Running { grant, token: None },
                    vec![DismissApprovalSheet, OpenSpace(mode), MintToken],
                ),
            }
        }
        (
            Copying {
                grant,
                token: None,
                before: None,
                ..
            },
            Event::CookiesCopied,
        ) => (Running { grant, token: None }, vec![MintToken]),
        (
            Copying {
                grant,
                token: Some(token),
                granted,
                ..
            },
            Event::CookiesCopied,
        ) => (
            Running {
                grant,
                token: Some(token),
            },
            vec![AnswerMore(MoreAnswer::Granted(granted.unwrap_or_default()))],
        ),
        (Copying { token: None, .. }, Event::CookiesNotCopied) => {
            (Stopped(StopReason::SignInsNotCopied), vec![Teardown])
        }
        (
            Copying {
                token: Some(token),
                before: Some(before),
                ..
            },
            Event::CookiesNotCopied,
        ) => (
            Running {
                grant: before,
                token: Some(token),
            },
            vec![AnswerMore(MoreAnswer::NotCopied)],
        ),
        (state @ Copying { .. }, Event::AskMore { .. }) => {
            (state, vec![AnswerMore(MoreAnswer::Busy)])
        }
        (Copying { .. }, Event::Stop) => (Stopped(StopReason::Stopped), vec![Teardown]),
        (Copying { token: Some(_), .. }, Event::Release) => {
            (Stopped(StopReason::Released), vec![Release])
        }
        (Copying { .. }, Event::ClientDisconnects) => {
            (Stopped(StopReason::ClientGone), vec![Teardown])
        }
        (Copying { .. }, Event::BrowserQuits) => (Stopped(StopReason::BrowserQuit), vec![Teardown]),
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
            let widened = Grant {
                mode: grant.mode,
                ..widened
            };
            match copy_of(&widened, Some(&grant)) {
                // sign-ins for the new sites: the answer waits for the copy
                Some(copy) => (
                    Copying {
                        grant: widened,
                        token: Some(token),
                        before: Some(grant),
                        granted: Some(granted),
                    },
                    vec![DismissApprovalSheet, copy],
                ),
                None => (
                    Running {
                        grant: widened,
                        token: Some(token),
                    },
                    vec![
                        DismissApprovalSheet,
                        AnswerMore(MoreAnswer::Granted(granted)),
                    ],
                ),
            }
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

/// The copy `grant`'s shared sign-ins call for: of the sites shared now and not in `before`
/// (one approval shares them from one Space).
fn copy_of(grant: &Grant, before: Option<&Grant>) -> Option<Effect> {
    let new: Vec<&SharedSite> = grant
        .shared()
        .iter()
        .filter(|s| !before.is_some_and(|b| b.signin_sites().any(|t| t == s.site)))
        .collect();
    let first = new.first()?;
    Some(Effect::CopyCookies {
        from: first.from,
        space: first.space.clone(),
        sites: new.iter().map(|s| s.site.clone()).collect(),
    })
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

/// Where shared sign-ins come from, as the host resolved the sheet's Space: its container and
/// name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Source {
    pub container: ContainerId,
    pub space: String,
}

/// The owner's answer to a sheet for more (`State::AskingMore`): Allow widens `grant` by the
/// sites and the bound names the owner left on it (`Lists`), with the sign-ins of the new sites
/// when the owner shared them (from `from`); Deny keeps the grant. A site that is no pattern
/// denies.
pub fn answer_more(grant: &Grant, lists: &Lists, allow: bool, from: Option<&Source>) -> Event {
    match grant.widened(&lists.sites, lists.granted_secrets()) {
        Ok(widened) if allow => Event::Approve(match from.filter(|_| lists.share) {
            Some(src) => widened.with_signins(src.container, &src.space, &lists.sites),
            None => widened,
        }),
        _ => Event::Deny,
    }
}

/// The owner's answer to the sheet as the session's event: Allow grants the sites the owner
/// left, in the mode the sheet's choice says, the names bound to logins (an unbound one is not
/// granted), and the sign-ins of those sites when the owner shared them (from `from`; only for
/// a list of sites). A grant that cannot be built denies.
pub fn answer(lists: &Lists, allow: bool, headless: bool, from: Option<&Source>) -> Event {
    if !allow {
        return Event::Deny;
    }
    let mode = if headless {
        Mode::Headless
    } else {
        Mode::Windowed
    };
    match Grant::new(lists.granted_secrets(), &lists.sites, mode) {
        Ok(grant) => Event::Approve(match from.filter(|_| lists.share && lists.may_share()) {
            Some(src) => grant.with_signins(src.container, &src.space, &lists.sites),
            None => grant,
        }),
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
        let Event::Approve(g) = answer(&l, true, false, None) else {
            panic!("not approved");
        };
        assert!(!g.allows_site("https://github.com/"));
        assert_eq!(g.mode, Mode::Windowed);
        l.edit(ListEdit::RemoveSite("example.com")).unwrap();
        let Event::Approve(g) = answer(&l, true, true, None) else {
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
                    part: Part::Password,
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

    /// Turning the setting off does not close an endpoint this run opened: sessions stay
    /// refused until the restart, and a port only set for the next start refuses them too.
    #[test]
    fn an_open_endpoint_refuses_sessions_whatever_the_setting_now_says() {
        assert_eq!(refusing_port(Some(9222), None), Some(9222));
        assert_eq!(refusing_port(None, Some(9333)), Some(9333));
        assert_eq!(refusing_port(Some(9222), Some(9333)), Some(9222));
        assert_eq!(refusing_port(None, None), None);
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
            g.secret_for("GITHUB_PASSWORD", gh, gh).map(|b| &b.login),
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
        let sites = Lists::asked(&[], &["github.com".to_string()]);
        assert_eq!(answer(&sites, false, true, None), Event::Deny);
        let Event::Approve(g) = answer(&sites, true, true, None) else {
            panic!("allow approves");
        };
        assert_eq!(g.mode, Mode::Headless);
        assert!(g.allows_site("https://github.com/x"));
        assert!(!g.allows_site("https://evil.test/"));
        assert_eq!(g.secret_names().count(), 0);
        let Event::Approve(g) = answer(&Lists::default(), true, false, None) else {
            panic!("allow approves");
        };
        assert_eq!(g.mode, Mode::Windowed);
        // a site that is no pattern cannot be granted
        assert_eq!(
            answer(&Lists::asked(&[], &["a b".to_string()]), true, false, None),
            Event::Deny
        );
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
            answer_more(&grant, &Lists::asked(&[], &["b.test".into()]), false, None),
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
        let (s, e) = transition(
            asking.clone(),
            answer_more(&grant, &Lists::asked(&[], &left), true, None),
        );
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
        assert_eq!(
            answer_more(&grant, &Lists::asked(&[], &["a b".into()]), true, None),
            Event::Deny
        );
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
    fn a_session_tab_goes_only_where_its_grant_says() {
        let some = Grant::new([], &["a.localhost".to_string()], Mode::Headless).unwrap();
        assert_eq!(may_go(&some, "http://a.localhost:8080/x?y#z"), Ok(()));
        assert_eq!(may_go(&some, "HTTP://A.localhost/"), Ok(()));
        assert_eq!(may_go(&some, "about:blank"), Ok(()));
        assert_eq!(may_go(&some, "about:blank#blocked"), Ok(()));
        assert!(may_go(&some, "about:blankx").is_err());
        assert!(may_go(&some, "about:srcdoc").is_err());
        assert_eq!(
            may_go(&some, "http://b.localhost/"),
            Err(VisitRefusal::NotGranted)
        );
        for off in [
            "file:///etc/passwd",
            "chrome://version",
            "data:text/html,x",
            "about:config",
        ] {
            assert_eq!(may_go(&some, off), Err(VisitRefusal::NotWeb), "{off}");
        }
        // an empty list is any web site, still nothing off the web
        let any = Grant::new([], &[], Mode::Headless).unwrap();
        assert_eq!(may_go(&any, "https://anything.test/"), Ok(()));
        assert_eq!(may_go(&any, "file:///x"), Err(VisitRefusal::NotWeb));
        assert!(refused_note("http://b.localhost:9/p").contains("to go to b.localhost,"));
        assert!(refused_note("http://b.localhost:9/p").contains("request_more"));
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
            part: Part::Password,
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

    /// A tab dragged out of a session's Space is free; one dragged in is held to the grant.
    #[test]
    fn the_model_says_which_tabs_a_session_holds() {
        use crate::model::{Command, Dest, Effect as M, Model, apply};
        use crate::space::ContainerId;
        let open = |m: Model| {
            let (m, fx) = apply(
                m,
                Command::NewTab {
                    url: "https://a.test".into(),
                    container: None,
                    opener: None,
                },
            );
            let tab = fx
                .iter()
                .find_map(|e| match e {
                    M::CreateTab { tab, .. } => Some(*tab),
                    _ => None,
                })
                .unwrap();
            (m, tab)
        };
        let m = Model::new(std::time::Instant::now());
        let (m, agent_tab) = open(m);
        let (m, _) = apply(
            m,
            Command::NewSpace {
                name: "Agent".into(),
                tint: crate::space::Rgb(1, 2, 3),
                container: ContainerId::DEFAULT,
            },
        );
        let agent_space = m.spaces.last().unwrap().id;
        let user_space = m.spaces[0].id;
        let g = grant(Mode::Windowed);
        let sessions = [("s1", agent_space, ContainerId::DEFAULT, &g)];
        let into = |space| Dest::Normal { space, index: 0 };
        let (m, _) = apply(
            m,
            Command::MoveTab {
                tab: agent_tab,
                to: into(agent_space),
            },
        );
        assert_eq!(holder(&m, agent_tab, &sessions), Some(("s1", g.clone())));
        let (m, _) = apply(
            m,
            Command::MoveTab {
                tab: agent_tab,
                to: into(user_space),
            },
        );
        assert_eq!(holder(&m, agent_tab, &sessions), None, "dragged out: free");
    }

    fn src() -> Source {
        Source {
            container: ContainerId(0),
            space: "Personal".into(),
        }
    }

    fn login(id: &str, origin: &str, part: Part) -> SecretBinding {
        SecretBinding {
            login: LoginRef(id.into()),
            origin: origin.into(),
            part,
        }
    }

    #[test]
    fn a_name_says_which_part_of_a_login_it_types() {
        for name in [
            "GITHUB_USER",
            "github_username",
            "AMAZON_LOGIN",
            "WORK_EMAIL",
            "USER",
        ] {
            assert_eq!(suggested_part(name), Part::Username, "{name}");
        }
        for name in ["GITHUB_PASSWORD", "NPM_TOKEN", "PIN", "USER_PW"] {
            assert_eq!(suggested_part(name), Part::Password, "{name}");
        }
    }

    #[test]
    fn only_bound_listed_names_are_granted_and_only_with_sites_are_sign_ins_shared() {
        let mut l = Lists::asked(
            &["GH_USER".into(), "GH_PASSWORD".into(), "NPM".into()],
            &["github.com".into()],
        );
        let gh = "https://github.com";
        l.edit(ListEdit::Bind {
            name: "GH_USER",
            login: Some(&login("l1", gh, Part::Username)),
        })
        .unwrap();
        l.edit(ListEdit::Bind {
            name: "GH_PASSWORD",
            login: Some(&login("l1", gh, Part::Password)),
        })
        .unwrap();
        // a name nobody asked for cannot be bound
        l.edit(ListEdit::Bind {
            name: "OTHER",
            login: Some(&login("l2", gh, Part::Password)),
        })
        .unwrap();
        // removed after binding: not granted
        l.edit(ListEdit::RemoveSecret("GH_USER")).unwrap();
        let Event::Approve(g) = answer(&l, true, false, Some(&src())) else {
            panic!("allowed");
        };
        assert_eq!(g.secret_names().collect::<Vec<_>>(), ["GH_PASSWORD"]);
        let b = g.secret_for(
            "GH_PASSWORD",
            "https://github.com/login",
            "https://github.com/login",
        );
        assert_eq!(
            b.map(|b| (b.login.0.as_str(), b.part)),
            Some(("l1", Part::Password))
        );
        // sharing is off by default
        assert!(g.shared().is_empty());
        assert!(!g.signs_in("https://github.com/"));
        // on: the listed sites' sign-ins, from the Space resolved
        l.edit(ListEdit::Share(true)).unwrap();
        let Event::Approve(g) = answer(&l, true, false, Some(&src())) else {
            panic!("allowed");
        };
        assert_eq!(g.signin_sites().collect::<Vec<_>>(), ["github.com"]);
        assert_eq!(
            g.shared().first().map(|s| (s.from, s.space.as_str())),
            Some((ContainerId(0), "Personal"))
        );
        assert!(g.signs_in("https://gist.github.com/x") && !g.signs_in("https://evil.test/"));
        // never with any site: the sheet does not offer it and Allow ignores it
        let mut any = Lists::asked(&[], &[]);
        assert!(!any.may_share());
        any.edit(ListEdit::Share(true)).unwrap();
        let Event::Approve(g) = answer(&any, true, false, Some(&src())) else {
            panic!("allowed");
        };
        assert!(g.shared().is_empty());
        // with_signins takes only granted sites
        let g = Grant::new([], &["a.test".to_string()], Mode::Windowed)
            .unwrap()
            .with_signins(ContainerId(3), "Work", &["b.test".to_string()]);
        assert!(g.shared().is_empty());
    }

    #[test]
    fn shared_sign_ins_are_copied_before_the_session_gets_its_handle() {
        let mut l = Lists::asked(&[], &["a.test".into(), "b.test".into()]);
        l.edit(ListEdit::Share(true)).unwrap();
        let Event::Approve(g) = answer(&l, true, true, Some(&src())) else {
            panic!("allowed");
        };
        let (s, e) = transition(State::AwaitingApproval(request()), Event::Approve(g));
        assert_eq!(
            e,
            vec![
                Effect::DismissApprovalSheet,
                Effect::OpenSpace(Mode::Headless),
                Effect::CopyCookies {
                    from: ContainerId(0),
                    space: "Personal".into(),
                    sites: vec!["a.test".into(), "b.test".into()],
                },
            ]
        );
        assert!(matches!(s, State::Copying { token: None, .. }));
        // no token, no calls, until the copy answers; the grant already guards the tab
        assert!(s.authorize("t").is_none());
        assert!(s.grant().is_some_and(|g| g.allows_site("https://a.test/")));
        let (s, e) = transition(s, Event::CookiesCopied);
        assert_eq!(e, vec![Effect::MintToken]);
        let (s, _) = transition(s, Event::TokenMinted(Token::new("t")));
        assert!(
            s.authorize("t")
                .is_some_and(|g| g.signs_in("https://b.test/"))
        );
        // stopping while copying tears the session down (its container, with the copies)
        let (s, _) = transition(State::AwaitingApproval(request()), {
            let Event::Approve(g) = answer(&l, true, true, Some(&src())) else {
                panic!()
            };
            Event::Approve(g)
        });
        for (ev, why) in [
            (Event::Stop, StopReason::Stopped),
            (Event::ClientDisconnects, StopReason::ClientGone),
            (Event::BrowserQuits, StopReason::BrowserQuit),
        ] {
            assert_eq!(
                transition(s.clone(), ev),
                (State::Stopped(why), vec![Effect::Teardown])
            );
        }
    }

    #[test]
    fn asking_for_more_copies_the_new_sites_sign_ins_only() {
        // running with a.test shared
        let g = Grant::new([], &["a.test".to_string()], Mode::Windowed)
            .unwrap()
            .with_signins(ContainerId(0), "Personal", &["a.test".to_string()]);
        let running = State::Running {
            grant: g,
            token: Some(Token::new("t")),
        };
        let (asking, _) = transition(running, ask(&["b.test"], &["B_PASSWORD"]));
        let State::AskingMore { grant, .. } = &asking else {
            panic!("asking");
        };
        let grant = grant.clone();
        let mut l = Lists::asked(&["B_PASSWORD".into()], &["b.test".into()]);
        l.edit(ListEdit::Bind {
            name: "B_PASSWORD",
            login: Some(&login("l9", "https://b.test", Part::Password)),
        })
        .unwrap();
        // not shared: answered at once, nothing copied
        let (s, e) = transition(asking.clone(), answer_more(&grant, &l, true, Some(&src())));
        assert!(matches!(s, State::Running { .. }));
        assert!(matches!(
            e.last(),
            Some(Effect::AnswerMore(MoreAnswer::Granted(_)))
        ));
        assert!(!s.grant().unwrap().signs_in("https://b.test/"));
        // shared: only b.test is copied, the old grant serves meanwhile, then the answer
        l.edit(ListEdit::Share(true)).unwrap();
        let (s, e) = transition(asking, answer_more(&grant, &l, true, Some(&src())));
        assert_eq!(
            e,
            vec![
                Effect::DismissApprovalSheet,
                Effect::CopyCookies {
                    from: ContainerId(0),
                    space: "Personal".into(),
                    sites: vec!["b.test".into()],
                },
            ]
        );
        let old = s.authorize("t").unwrap();
        assert!(!old.allows_site("https://b.test/"));
        assert_eq!(
            transition(s.clone(), ask(&["c.test"], &[])).1,
            vec![Effect::AnswerMore(MoreAnswer::Busy)]
        );
        let (s, e) = transition(s, Event::CookiesCopied);
        assert_eq!(
            e,
            vec![Effect::AnswerMore(MoreAnswer::Granted(More {
                sites: vec!["b.test".into()],
                secrets: vec!["B_PASSWORD".into()],
            }))]
        );
        let g = s.authorize("t").unwrap();
        assert!(g.signs_in("https://a.test/") && g.signs_in("https://b.test/"));
        assert_eq!(g.secret_names().collect::<Vec<_>>(), ["B_PASSWORD"]);
    }

    #[test]
    fn the_cookie_matcher_is_the_grants_sites() {
        let scope = cookie_scope(&["github.com".into(), "https://accounts.google.com/*".into()]);
        assert!(shares_cookie(&scope, "github.com", "/", true));
        assert!(shares_cookie(&scope, ".github.com", "/", false));
        assert!(shares_cookie(&scope, ".google.com", "/", true));
        assert!(!shares_cookie(&scope, "mail.google.com", "/", true));
        assert!(!shares_cookie(&scope, "evil.test", "/", false));
        assert!(!shares_cookie(&cookie_scope(&[]), "github.com", "/", false));
    }

    #[test]
    fn sign_ins_are_never_shared_for_a_public_suffix() {
        let mut l = Lists::asked(
            &[],
            &["*.co.uk".into(), "github.com".into(), "github.io".into()],
        );
        l.edit(ListEdit::Share(true)).unwrap();
        assert_eq!(l.unshareable(), ["*.co.uk", "github.io"]);
        let Event::Approve(g) = answer(&l, true, false, Some(&src())) else {
            panic!("allowed: the suffixes are still sites to visit");
        };
        assert!(g.allows_site("https://bbc.co.uk/"));
        assert_eq!(g.signin_sites().collect::<Vec<_>>(), ["github.com"]);
        // only suffixes: nothing to share, nothing copied
        let mut only = Lists::asked(&[], &["com".into()]);
        only.edit(ListEdit::Share(true)).unwrap();
        let Event::Approve(g) = answer(&only, true, false, Some(&src())) else {
            panic!("allowed");
        };
        assert!(g.shared().is_empty());
        assert_eq!(copy_of(&g, None), None);
    }

    #[test]
    fn each_shared_site_keeps_the_space_it_came_from() {
        let sites = ["a.test".to_string(), "b.test".to_string()];
        let g = Grant::new([], &sites, Mode::Windowed)
            .unwrap()
            .with_signins(ContainerId(0), "Personal", &sites[..1])
            .with_signins(ContainerId(3), "Work", &sites[1..])
            // again from elsewhere: the first source stays
            .with_signins(ContainerId(5), "Other", &sites[..1]);
        let from: Vec<_> = g
            .shared()
            .iter()
            .map(|s| (s.site.as_str(), s.from, s.space.as_str()))
            .collect();
        assert_eq!(
            from,
            [
                ("a.test", ContainerId(0), "Personal"),
                ("b.test", ContainerId(3), "Work")
            ]
        );
    }

    #[test]
    fn a_copy_that_fails_or_never_ends_fails_the_start_or_keeps_the_grant() {
        let mut l = Lists::asked(&[], &["a.test".into()]);
        l.edit(ListEdit::Share(true)).unwrap();
        let Event::Approve(g) = answer(&l, true, true, Some(&src())) else {
            panic!("allowed");
        };
        let (copying, _) = transition(State::AwaitingApproval(request()), Event::Approve(g));
        // CEF refused, or the host's bound on the copy ran out: the start fails, the
        // container (with whatever was copied) goes
        assert_eq!(
            transition(copying.clone(), Event::CookiesNotCopied),
            (
                State::Stopped(StopReason::SignInsNotCopied),
                vec![Effect::Teardown]
            )
        );
        // a late answer to a stopped session changes nothing
        let (stopped, _) = transition(copying, Event::CookiesNotCopied);
        assert_eq!(
            transition(stopped.clone(), Event::CookiesCopied),
            (stopped, vec![])
        );
        // request_more: the session runs on with the grant it had, the answer says why
        let before = Grant::new([], &["a.test".to_string()], Mode::Windowed).unwrap();
        let widened = Grant::new(
            [],
            &["a.test".to_string(), "b.test".to_string()],
            Mode::Windowed,
        )
        .unwrap()
        .with_signins(ContainerId(0), "Personal", &["b.test".to_string()]);
        let more = State::Copying {
            grant: widened,
            token: Some(Token::new("t")),
            before: Some(before.clone()),
            granted: Some(More::default()),
        };
        assert_eq!(
            transition(more, Event::CookiesNotCopied),
            (
                State::Running {
                    grant: before,
                    token: Some(Token::new("t"))
                },
                vec![Effect::AnswerMore(MoreAnswer::NotCopied)]
            )
        );
    }

    #[test]
    fn a_fill_taints_its_tab_and_its_origin_for_scripts() {
        let mut t = Tainted::default();
        let (one, two) = (crate::tab::TabId(1), crate::tab::TabId(2));
        assert!(!t.refuses_scripts(one, "https://github.com/login"));
        t.filled(one, "https://github.com/login");
        // the tab, wherever it goes now
        assert!(t.refuses_scripts(one, "https://github.com/login"));
        assert!(t.refuses_scripts(one, "https://elsewhere.test/"));
        // another tab only while it is on the origin the secret was typed on
        assert!(t.refuses_scripts(two, "https://github.com/settings"));
        assert!(!t.refuses_scripts(two, "https://gist.github.com/"));
        assert!(!t.refuses_scripts(two, "http://github.com/"));
        assert!(!t.refuses_scripts(two, "https://elsewhere.test/"));
    }

    #[test]
    fn the_name_only_suggests_a_part() {
        assert_eq!(suggested_part("GITHUB_USER"), Part::Username);
        assert_eq!(suggested_part("GITHUB_ID"), Part::Password);
        // the owner's choice on the sheet is what the binding keeps
        let mut l = Lists::asked(&["GITHUB_ID".into()], &["github.com".into()]);
        l.edit(ListEdit::Bind {
            name: "GITHUB_ID",
            login: Some(&login("l1", "https://github.com", Part::Username)),
        })
        .unwrap();
        assert_eq!(l.bound["GITHUB_ID"].part, Part::Username);
    }

    #[test]
    fn missing_does_not_treat_exact_host_or_root_path_as_covering_domain() {
        let exact = Grant::new(
            [],
            &[
                "https://example.com/*".into(),
                "http://example.com/*".into(),
            ],
            Mode::Headless,
        )
        .unwrap();
        assert_eq!(
            exact.missing(&["*.example.com".into()], &[]).sites,
            vec!["*.example.com"]
        );
        assert_eq!(
            exact.missing(&["example.com".into()], &[]).sites,
            vec!["example.com"]
        );

        let root_only = Grant::new(
            [],
            &["https://example.com/".into(), "http://example.com/".into()],
            Mode::Headless,
        )
        .unwrap();
        assert_eq!(
            root_only.missing(&["example.com".into()], &[]).sites,
            vec!["example.com"]
        );
    }
}
