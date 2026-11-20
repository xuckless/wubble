//! Agent sessions over MCP (docs/research/mcp-server.md section 0, the basic version): the
//! host's half. Connections arrive from `mcp.rs`; each session's lifecycle is
//! `wubble_core::agent::transition`, held here in the one session table, and its effects are
//! carried out here: the approval (the sheet, `Kind::AgentSheet` of the one overlay stack,
//! asking one session at a time), a new Space with its own container for the session's tabs,
//! the token, the teardown that closes the Space; and the owner's stop (`agents_stop`).
//!
//! A call presents its session's token (the handle `start_session` returned), on the
//! connection that started it, and names tabs by handle; a tab outside the session's Space is
//! unknown to it. Calls run through the engine paths the browser already has: loads through the
//! model, `read_page` and element refs from the accessibility mirror (`host/ax.rs`), clicks and
//! keys through the view's own pointer and key input, screenshots through the DevTools
//! capture the screenshot tool uses, `evaluate` through `View::eval`, and `fill_secret` from
//! the vault, only on the origin of the login the grant names (`agent::Grant::secret_for`).
//! A call that waits (a load, a condition, a script, a capture) is a [`Pending`] answered from
//! the page's events, or with an error at its deadline.

use std::collections::{BTreeMap, HashMap};
use std::time::{Duration, Instant};

use rand::RngCore;
use serde_json::{Value, json};
use slint::{ComponentHandle, Model};
use wubble_ax::{AxItem, Bounds, PageTree, Reach};
use wubble_core::agent::log::{Entry, Outcome};
use wubble_core::agent::refs::RefBook;
use wubble_core::agent::{self, Effect, Env, Event, Grant, Mode, Request, SessionId, State};
use wubble_core::agent::{Refusal, StopReason, Token};
use wubble_core::model::Command;
use wubble_core::overlay::Kind;
use wubble_core::secret::Secret;
use wubble_core::space::{ContainerId, SpaceId};
use wubble_core::tab::TabId;
use wubble_engine::input::{Buttons, Modifiers, MouseButton, Point};
use wubble_engine::nav::{Phase, WebEvent};
use wubble_engine::page::{EditCommand, PageEvent};
use wubble_mcp::{AgentCall, ButtonKind, NavTarget, RequestId, ToolResult, WaitCondition};
use wubble_ui::{AgentLogItem, AgentSheetView, AppWindow};
use wubble_vault::RecordId;

use super::{Host, now_ms};
use crate::mcp::{ConnId, Event as McpEvent, Outbox};
use crate::with_host;

/// How long a call may wait for the page unless it says otherwise (`wait_for`'s `timeout_ms`).
const WAIT: Duration = Duration::from_secs(30);
/// Items `read_page` returns unless asked for fewer.
const READ_ITEMS: usize = 500;
/// Items a ref is looked up among.
const ALL_ITEMS: usize = 10_000;
/// `--smoke mcp`'s tab switch: the step that hides the sidebar (`Host::mcp_smoke_switch`).
const SWITCH_RESIZE: usize = 4;
/// The page a session's first tab opens on.
const BLANK: &str = "about:blank";

struct Conn {
    out: Outbox,
    client: String,
}

/// One session: its lifecycle state (the one owner of what it may do), its connection, and
/// what the effects made for it.
struct Session {
    conn: ConnId,
    state: State,
    purpose: String,
    /// What the client asked for, as the owner leaves it on the sheet; Allow grants its sites.
    lists: agent::Lists,
    /// The `start_session` call, answered once the token exists (or the session ended).
    start: Option<RequestId>,
    /// The Space and container the approval opened.
    space: Option<(SpaceId, ContainerId)>,
    /// Why the session was refused, when not the owner's word.
    note: Option<&'static str>,
    /// When it began waiting for the sheet (`Agents::fresh`); the sheet takes the lowest.
    asked: u64,
}

/// What a call's answer is logged and sent with.
#[derive(Clone)]
struct Reply {
    conn: ConnId,
    id: RequestId,
    session: Option<SessionId>,
    tool: &'static str,
    target: Option<String>,
}

/// A click or text, at an element's centre.
enum Act {
    Click {
        button: MouseButton,
        count: u32,
    },
    Type {
        text: Secret,
        submit: bool,
    },
    /// `fill_secret`'s value for granted variable `name`: typed only once the page confirms
    /// the click focused the element there, in its own document, still on the login's site.
    Fill {
        text: Secret,
        name: String,
    },
}

/// How far the load a `navigate` asked for has come: the end of a load started before it
/// (a new tab's `about:blank`) is not its end.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Load {
    Asked,
    Started,
    Ended,
}

/// Asks the page whether the click at `(x, y)` focused the element there, in the top
/// document (not a frame): its URL if so, else `null`.
fn focus_script(x: f64, y: f64) -> String {
    format!(
        "(()=>{{const e=document.activeElement,h=document.elementFromPoint({x},{y});\
         return e&&e!==document.body&&!/^I?FRAME$/.test(e.tagName)&&h&&(e===h||e.contains(h))\
         ?location.href:null}})()"
    )
}

/// What a waiting call waits for.
enum Wait {
    /// `navigate`: its load to end and the new document's tree (other than `before`).
    Navigated {
        tab: TabId,
        before: Option<String>,
        load: Load,
    },
    /// `wait_for` load: no load running and a tree.
    Loaded {
        tab: TabId,
    },
    /// A new tab's first load to end (`about:blank` has no tree) before `start_session` or
    /// `open_tab` gives its handle, so a `navigate` starts on a settled page.
    Opened {
        tab: TabId,
        answer: String,
    },
    Text {
        tab: TabId,
        text: String,
    },
    Element {
        tab: TabId,
        target: String,
    },
    /// `read_page` before the page has a tree.
    Tree {
        tab: TabId,
        within: Option<String>,
        max: usize,
    },
    Eval {
        id: u64,
    },
    Shot {
        id: u64,
    },
    /// The element was out of view: the page was asked to scroll it to the middle.
    Scroll {
        id: u64,
        tab: TabId,
        bounds: Bounds,
        viewport: Bounds,
        act: Act,
    },
    /// `fill_secret`: the vault's answer.
    Vault {
        nonce: u64,
        tab: TabId,
        target: String,
        name: String,
    },
    /// `fill_secret` clicked its element: the page's word on where focus went
    /// ([`focus_script`]).
    Focus {
        id: u64,
        tab: TabId,
        text: Secret,
        name: String,
    },
}

struct Pending {
    reply: Reply,
    wait: Wait,
    deadline: Instant,
}

/// The session table and the calls in flight.
#[derive(Default)]
pub(super) struct Agents {
    conns: HashMap<ConnId, Conn>,
    sessions: BTreeMap<SessionId, Session>,
    /// Each tab's element refs.
    refs: BTreeMap<TabId, RefBook>,
    pending: Vec<Pending>,
    next: u64,
    timer: slint::Timer,
    /// The window the sheet shows in and the session it shows, while it does.
    sheet: Option<(slint::Weak<AppWindow>, SessionId)>,
    /// Counts the sheets shown, so each restarts the sheet's window choice.
    sheets: i32,
    /// The answer to the sheet's last Sites draft: its serial, and the note (empty: taken).
    answer: (i32, &'static str),
}

impl Agents {
    fn fresh(&mut self) -> u64 {
        self.next += 1;
        self.next
    }

    /// The sessions' Spaces: never saved with the session (`Model::for_session`).
    pub(super) fn spaces(&self) -> Vec<SpaceId> {
        self.sessions
            .values()
            .filter_map(|s| s.space.map(|(sp, _)| sp))
            .collect()
    }
}

fn mouse(button: ButtonKind) -> MouseButton {
    match button {
        ButtonKind::Left => MouseButton::Left,
        ButtonKind::Right => MouseButton::Right,
        ButtonKind::Middle => MouseButton::Middle,
    }
}

/// A random 256-bit token, hex.
fn mint() -> Token {
    let mut bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    Token::new(bytes.iter().map(|b| format!("{b:02x}")).collect::<String>())
}

/// One `read_page` line: role, level, name, value, state, ref.
fn line(item: &AxItem, r: &str) -> String {
    let mut out = format!("- {}", item.role);
    if item.level > 0 {
        out.push_str(&format!(" {}", item.level));
    }
    out.push_str(&format!(" {:?}", item.name));
    if !item.value.is_empty() {
        out.push_str(&format!(" value={:?}", item.value));
    }
    if let Some(c) = item.checked {
        out.push_str(if c { " checked" } else { " unchecked" });
    }
    if item.focused {
        out.push_str(" focused");
    }
    out.push_str(&format!(" [ref={r}]"));
    out
}

fn inside(b: Bounds, outer: Bounds) -> bool {
    b.x >= outer.x
        && b.y >= outer.y
        && b.x + b.width <= outer.x + outer.width
        && b.y + b.height <= outer.y + outer.height
}

/// The sheet's window view of what it shows.
fn sheet_view(
    request: &Request,
    lists: &agent::Lists,
    serial: i32,
    answer: (i32, &str),
) -> AgentSheetView {
    // one column each: the name, the site
    let rows = |v: &[String]| -> slint::ModelRc<wubble_ui::EntryItem> {
        let entries: Vec<wubble_core::pages::Entry> = v
            .iter()
            .map(|s| wubble_core::pages::Entry {
                label: s.clone(),
                key: s.clone(),
                cells: vec![s.clone()],
            })
            .collect();
        wubble_ui::settings::entry_items(&entries).as_slice().into()
    };
    AgentSheetView {
        open: true,
        serial,
        client: request.client.as_str().into(),
        secrets: rows(&lists.secrets),
        sites: rows(&lists.sites),
        answer: answer.0,
        note: answer.1.into(),
        headless: request.wants_mode == Mode::Headless,
    }
}

fn stopped_text(reason: StopReason, note: Option<&str>) -> String {
    if let Some(note) = note {
        return note.to_owned();
    }
    match reason {
        StopReason::Refused(Refusal::Disabled) => {
            "agent sessions are off (agent.mcp_enabled)".into()
        }
        StopReason::Refused(Refusal::RemoteDebugging) => {
            "agent sessions cannot start while remote debugging is on".into()
        }
        StopReason::Denied => "the user denied the session".into(),
        StopReason::Stopped => "the session was stopped".into(),
        StopReason::ClientGone => "the client went away".into(),
        StopReason::BrowserQuit => "the browser quit".into(),
    }
}

impl Host {
    /// What a connection reports (`mcp.rs`).
    pub(crate) fn mcp_event(&mut self, event: McpEvent) {
        match event {
            McpEvent::Connected { conn, out, client } => {
                self.agents.conns.insert(conn, Conn { out, client });
            }
            McpEvent::Call { conn, id, call } => self.agent_call(conn, id, call),
            McpEvent::Gone(conn) => {
                self.agents.conns.remove(&conn);
                self.agents.pending.retain(|p| p.reply.conn != conn);
                let gone: Vec<SessionId> = self
                    .agents
                    .sessions
                    .iter()
                    .filter(|(_, s)| s.conn == conn)
                    .map(|(id, _)| id.clone())
                    .collect();
                for id in gone {
                    self.agent_event(&id, Event::ClientDisconnects);
                }
            }
        }
    }

    /// Whether a session reaches `tab` (in its Space and its container): its view keeps an
    /// accessibility tree, and the user's closed tabs and history leave it out.
    pub(super) fn agent_drives(&self, tab: TabId) -> bool {
        self.agents.sessions.values().any(|s| {
            s.space
                .is_some_and(|(space, container)| self.model.holds(space, container, tab))
        })
    }

    /// The browser quits: every session ends and its Space closes.
    pub(super) fn agents_quit(&mut self) {
        let all: Vec<SessionId> = self.agents.sessions.keys().cloned().collect();
        let spaces = self.agents.spaces();
        for id in all {
            self.agent_event(&id, Event::BrowserQuits);
        }
        // `--smoke mcp` leaves a session running for this: its Space must be gone.
        if self.checks.mcp_smoke.is_some() {
            let left = spaces
                .iter()
                .any(|sp| self.model.spaces.iter().any(|s| s.id == *sp));
            if spaces.is_empty() || left || !self.agents.sessions.is_empty() {
                self.failure
                    .get_or_insert("the quit left an agent session's Space open".into());
            }
        }
    }

    fn agent_env(&self) -> Env {
        use wubble_core::apply::value_of;
        use wubble_core::settings as defs;
        let stored = &self.settings.stored;
        Env {
            mcp_enabled: value_of(&defs::AGENT_MCP_ENABLED, stored) == "1",
            remote_port: defs::remote_port(
                value_of(&defs::DEVTOOLS_REMOTE_PORT, stored)
                    .parse()
                    .unwrap_or(0),
            ),
        }
    }

    // ----- the lifecycle

    /// One event of session `id`, through the core machine; its effects run here.
    fn agent_event(&mut self, id: &SessionId, event: Event) {
        let Some(s) = self.agents.sessions.get_mut(id) else {
            return;
        };
        let state = std::mem::replace(&mut s.state, State::Requested);
        let (state, effects) = agent::transition(state, event);
        s.state = state;
        for effect in effects {
            self.agent_effect(id, effect);
        }
        if self
            .agents
            .sessions
            .get(id)
            .is_some_and(|s| s.state.is_stopped())
        {
            self.agent_forget(id);
        }
    }

    fn agent_effect(&mut self, id: &SessionId, effect: Effect) {
        match effect {
            Effect::ShowApprovalSheet(_) => {
                let asked = self.agents.fresh();
                if let Some(s) = self.agents.sessions.get_mut(id) {
                    s.asked = asked;
                }
                self.agent_sheet_pump();
            }
            Effect::DismissApprovalSheet => {
                if self
                    .agents
                    .sheet
                    .as_ref()
                    .is_some_and(|(_, shown)| shown == id)
                {
                    self.agent_sheet_clear();
                }
                self.agent_sheet_pump();
            }
            Effect::OpenSpace(mode) => {
                // `--smoke mcp`: the grant's mode is the one the sheet showed when Allow was pressed
                if let Some(expect) = self.checks.mcp_smoke.as_mut().and_then(|r| r.expect.take())
                    && expect != mode
                {
                    self.fail(format!(
                        "the sheet showed {expect:?}, the session opened {mode:?}"
                    ));
                }
                self.agent_open_space(id, mode);
            }
            Effect::MintToken => {
                let token = mint();
                self.agent_event(id, Event::TokenMinted(token.clone()));
                self.agent_started(id, &token);
            }
            Effect::Teardown => {
                if let Some((space, container)) = self
                    .agents
                    .sessions
                    .get_mut(id)
                    .and_then(|s| s.space.take())
                {
                    self.dispatch(Command::DeleteSpace(space));
                    self.dispatch(Command::DeleteContainer(container));
                }
            }
        }
    }

    // ----- the approval sheet

    /// Shows the first waiting session's sheet when the overlays let it (one modal at a time:
    /// a page's dialog or the user's sheet makes it wait; the pump asks again).
    pub(super) fn agent_sheet_pump(&mut self) {
        if self.agents.sheet.is_some() {
            return;
        }
        // the session that has waited longest (the session table is the one owner of who waits)
        let Some((id, request, lists)) = self
            .agents
            .sessions
            .iter()
            .filter_map(|(id, s)| match &s.state {
                State::AwaitingApproval(request) => {
                    Some((s.asked, id.clone(), request.clone(), s.lists.clone()))
                }
                _ => None,
            })
            .min_by_key(|(asked, ..)| *asked)
            .map(|(_, id, request, lists)| (id, request, lists))
        else {
            return;
        };
        let Some(window) = self.w.window.upgrade() else {
            return;
        };
        if !self.overlay_open(Kind::AgentSheet, None) {
            return;
        }
        self.agents.sheets += 1;
        self.agents.answer = (0, "");
        window.set_agent_sheet_view(sheet_view(&request, &lists, self.agents.sheets, (0, "")));
        self.agents.sheet = Some((window.as_weak(), id));
        self.render_surfaces(&window);
        if self.checks.mcp_smoke.is_some() {
            let _ = slint::invoke_from_event_loop(|| with_host(Host::mcp_smoke_answer));
        }
    }

    /// The sheet leaves (answered, or its session ended).
    fn agent_sheet_clear(&mut self) {
        if let Some(window) = self.agents.sheet.take().and_then(|(w, _)| w.upgrade()) {
            window.set_agent_sheet_view(AgentSheetView::default());
        }
    }

    /// The sheet's own answer: Allow or Deny (Escape and a click on Deny), with its window
    /// choice. It answers the session the sheet shows.
    pub(crate) fn agent_sheet_answered(&mut self, allow: bool, headless: bool) {
        let Some((_, id)) = self.agents.sheet.clone() else {
            return;
        };
        let sites = self
            .agents
            .sessions
            .get(&id)
            .map(|s| s.lists.sites.clone())
            .unwrap_or_default();
        // the session's transition dismisses the sheet and shows the next
        self.agent_event(&id, agent::answer(&sites, allow, headless));
    }

    /// A list editor's "−" on the sheet: a site, else a secret name, leaves what Allow grants.
    pub(crate) fn agent_sheet_removed(&mut self, site: bool, key: &str) {
        let edit = if site {
            agent::ListEdit::RemoveSite(key)
        } else {
            agent::ListEdit::RemoveSecret(key)
        };
        let _ = self.agent_sheet_edit(edit);
        self.agent_sheet_render();
    }

    /// The Sites editor's draft (`pages::submission`: one cell, and the site it replaces when
    /// it edits one): taken into the session's list, or refused with a note; either way the
    /// editor gets its answer.
    pub(crate) fn agent_sheet_site_submitted(&mut self, text: &str) {
        let draft = wubble_core::pages::submission(text);
        let site = draft.cells.first().copied().unwrap_or_default();
        let result = self.agent_sheet_edit(agent::ListEdit::PutSite {
            replacing: draft.replacing,
            site,
        });
        self.agents.answer = (
            self.agents.answer.0.wrapping_add(1),
            result.err().map_or("", agent::ListRefusal::note),
        );
        self.agent_sheet_render();
    }

    /// One edit of the shown session's lists (`agent::Lists::edit`).
    fn agent_sheet_edit(&mut self, edit: agent::ListEdit) -> Result<(), agent::ListRefusal> {
        let Some((_, id)) = &self.agents.sheet else {
            return Ok(());
        };
        let Some(s) = self.agents.sessions.get_mut(id) else {
            return Ok(());
        };
        s.lists.edit(edit)
    }

    /// The shown sheet's view again, after an edit (same serial: the window choice stays).
    fn agent_sheet_render(&mut self) {
        let Some((window, id)) = &self.agents.sheet else {
            return;
        };
        let (Some(window), Some(s)) = (window.upgrade(), self.agents.sessions.get(id)) else {
            return;
        };
        let State::AwaitingApproval(request) = &s.state else {
            return;
        };
        window.set_agent_sheet_view(sheet_view(
            request,
            &s.lists,
            self.agents.sheets,
            self.agents.answer,
        ));
    }

    /// The sheet is showing (the overlay stack's read).
    pub(super) fn agent_sheet_open(&self) -> bool {
        self.agents.sheet.is_some()
    }

    /// `--smoke gallery`: a session as a client would make it, asking for approval, or (when
    /// `approved`) approved in the window with a few tabs, which shows its marks. The scene
    /// after it stops it (`agents_stop`).
    pub(super) fn gallery_agent(&mut self, approved: bool) {
        let id = SessionId("gallery".into());
        self.agents.sessions.insert(
            id.clone(),
            Session {
                conn: 0,
                state: State::Requested,
                purpose: "Look up the release notes".into(),
                lists: agent::Lists::asked(
                    &["GITHUB_TOKEN".into(), "NPM_TOKEN".into()],
                    &["github.com".into(), "*.githubusercontent.com".into()],
                ),
                start: None,
                space: None,
                note: None,
                asked: 0,
            },
        );
        let request = Request {
            client: "Claude Code".into(),
            wants_secrets: vec!["GITHUB_PASSWORD".into()],
            wants_mode: Mode::Windowed,
        };
        let env = Env {
            mcp_enabled: true,
            remote_port: None,
        };
        self.agent_event(&id, Event::ClientAsks(request, env));
        if !approved {
            return;
        }
        self.agent_event(&id, agent::answer(&["github.com".to_string()], true, false));
        let Some((space, _)) = self.agents.sessions.get(&id).and_then(|s| s.space) else {
            return;
        };
        for url in ["https://github.com/", "https://github.com/releases"] {
            self.dispatch(Command::OpenInSpace {
                space,
                url: url.into(),
            });
        }
    }

    /// `--smoke gallery`: entries for the activity page.
    pub(super) fn gallery_agent_log(&mut self) {
        let id = SessionId("s1".into());
        let now = now_ms();
        let calls = [
            ("session_started", None, Outcome::Done, 540),
            ("navigate", Some("https://github.com/"), Outcome::Done, 520),
            ("read_page", Some("https://github.com/"), Outcome::Done, 515),
            ("click", Some("https://github.com/"), Outcome::Done, 480),
            (
                "navigate",
                Some("https://evil.example/"),
                Outcome::Refused,
                300,
            ),
            (
                "screenshot",
                Some("https://github.com/"),
                Outcome::Failed,
                120,
            ),
            ("session_stopped", None, Outcome::Done, 60),
        ];
        for (tool, target, outcome, secs_ago) in calls {
            self.store.push_agent_log(Entry {
                session: id.clone(),
                at_ms: now - secs_ago * 1000,
                tool: tool.into(),
                target: target.map(Into::into),
                outcome,
            });
        }
    }

    // ----- stopping

    /// The owner's stop: every session ends and its Space closes, headless or not. The
    /// command bar's "Stop All Agents", and the mark on a tab row or the Space header.
    pub(super) fn agents_stop(&mut self) {
        let all: Vec<SessionId> = self.agents.sessions.keys().cloned().collect();
        for id in all {
            self.agent_event(&id, Event::Stop);
        }
    }

    /// Marks the rows of tabs an agent session drives, and the session's Space.
    pub(super) fn agent_marks(&self, view: &mut crate::shell::ShellView) {
        let spaces = self.agents.spaces();
        for row in view.pinned.iter_mut().chain(view.tabs.iter_mut()) {
            row.agent = row.kind == crate::shell::RowKind::Tab
                && crate::shell::tab_id(row.id).is_some_and(|t| self.agent_drives(t));
        }
        for row in &mut view.spaces {
            row.agent = crate::shell::space_id(row.id).is_some_and(|s| spaces.contains(&s));
        }
    }

    // ----- the activity page

    /// The page is shown: its rows, newest first, from the store.
    pub(super) fn agent_log_shown(&mut self) {
        self.store
            .agent_log(wubble_core::agent::log::STORE_MAX, |entries| {
                let _ = slint::invoke_from_event_loop(move || {
                    with_host(|host| host.render_agent_log(&entries))
                });
            });
    }

    fn render_agent_log(&self, entries: &[Entry]) {
        let Some(window) = self.w.window.upgrade() else {
            return;
        };
        let items: Vec<AgentLogItem> = agent::log::rows(entries, now_ms())
            .into_iter()
            .map(|r| AgentLogItem {
                title: r.title.into(),
                note: r.note.into(),
                failed: r.failed,
            })
            .collect();
        crate::bind::sync(window.get_agent_log(), items, |m| window.set_agent_log(m));
    }

    /// The session's own Space, in a new container, with its first tab; shown in the window
    /// when windowed, while headless its tabs are hidden views no window shows.
    fn agent_open_space(&mut self, id: &SessionId, mode: Mode) {
        let Some(purpose) = self.agents.sessions.get(id).map(|s| s.purpose.clone()) else {
            return;
        };
        self.dispatch(Command::NewContainer {
            name: format!("Agent {}", id.0),
        });
        let Some(container) = self.model.containers.last().map(|c| c.id) else {
            return;
        };
        let Command::NewSpace { tint, .. } = self.model.new_space() else {
            return;
        };
        let name: String = format!("Agent: {purpose}").chars().take(40).collect();
        self.dispatch(Command::NewSpace {
            name,
            tint,
            container,
        });
        let Some(space) = self.model.spaces.last().map(|s| s.id) else {
            return;
        };
        if let Some(s) = self.agents.sessions.get_mut(id) {
            s.space = Some((space, container));
        }
        self.dispatch(Command::SetSpaceNeverSleep { space, on: true });
        self.dispatch(Command::OpenInSpace {
            space,
            url: BLANK.into(),
        });
        if mode == Mode::Windowed {
            self.dispatch(Command::SwitchSpace(space));
        }
    }

    /// The token exists: `start_session` gets the handle and the first tab.
    fn agent_started(&mut self, id: &SessionId, token: &Token) {
        let Some(s) = self.agents.sessions.get_mut(id) else {
            return;
        };
        let Some(request) = s.start.take() else {
            return;
        };
        let (conn, space) = (s.conn, s.space.map(|(sp, _)| sp));
        let headless =
            matches!(&s.state, State::Running { grant, .. } if grant.mode == Mode::Headless);
        let first = space.and_then(|sp| self.model.order(sp).first().copied());
        let reply = Reply {
            conn,
            id: request,
            session: Some(id.clone()),
            tool: "start_session",
            target: None,
        };
        let body = json!({
            "session": token.expose(),
            "tab": first.map(|t| t.0.to_string()),
            "headless": headless,
            "secrets": Vec::<String>::new(),
        });
        self.agent_log(id, "session_started", None, Outcome::Done);
        match first {
            Some(tab) => self.agent_opened(reply, tab, body.to_string()),
            None => self.agent_fail(reply, "the session's tab did not open"),
        }
    }

    /// `answer` once `tab`'s first load ended.
    fn agent_opened(&mut self, reply: Reply, tab: TabId, answer: String) {
        self.agent_wait(reply, Wait::Opened { tab, answer }, WAIT);
        self.agent_check();
    }

    /// A stopped session leaves the table: a `start_session` still waiting hears why, and its
    /// calls in flight fail.
    fn agent_forget(&mut self, id: &SessionId) {
        let Some(s) = self.agents.sessions.remove(id) else {
            return;
        };
        let State::Stopped(reason) = s.state else {
            return;
        };
        let why = stopped_text(reason, s.note);
        if let Some(request) = s.start {
            let reply = Reply {
                conn: s.conn,
                id: request,
                session: Some(id.clone()),
                tool: "start_session",
                target: None,
            };
            self.agent_send(reply, ToolResult::error(why.clone()), Outcome::Refused);
        }
        let (gone, kept): (Vec<_>, Vec<_>) = std::mem::take(&mut self.agents.pending)
            .into_iter()
            .partition(|p| p.reply.session.as_ref() == Some(id));
        self.agents.pending = kept;
        for p in gone {
            self.agent_send(p.reply, ToolResult::error(why.clone()), Outcome::Failed);
        }
        let tabs = &self.model.tabs;
        self.agents.refs.retain(|t, _| tabs.contains_key(t));
        self.agent_log(id, "session_stopped", None, Outcome::Done);
    }

    // ----- calls

    fn agent_call(&mut self, conn: ConnId, id: RequestId, call: AgentCall) {
        let mut reply = Reply {
            conn,
            id,
            session: None,
            tool: call.tool(),
            target: None,
        };
        if let AgentCall::StartSession {
            purpose,
            sites,
            secrets,
            headless,
        } = call
        {
            return self.agent_start(reply, purpose, sites, secrets, headless);
        }
        let token = call.session().map(|s| s.0.clone()).unwrap_or_default();
        let Some(session) = self
            .agents
            .sessions
            .iter()
            .find(|(_, s)| s.conn == conn && s.state.authorize(&token).is_some())
            .map(|(id, _)| id.clone())
        else {
            return self.agent_refuse(reply, "no such session: start one with start_session");
        };
        reply.session = Some(session.clone());
        let tab = match call.tab() {
            Some(handle) => match self.agent_tab(&session, &handle.0) {
                Some(tab) => Some(tab),
                None => return self.agent_refuse(reply, "no such tab in this session"),
            },
            None => None,
        };
        reply.target = tab.and_then(|t| {
            let url = &self.model.tabs.get(&t)?.url;
            Some(wubble_core::autofill::web_origin(url).unwrap_or_else(|| url.clone()))
        });
        let tab = tab.unwrap_or(TabId(u64::MAX));
        match call {
            AgentCall::StartSession { .. } => {}
            AgentCall::StopSession { .. } => {
                self.agent_send(reply, ToolResult::text("stopped"), Outcome::Done);
                self.agent_event(&session, Event::Stop);
            }
            AgentCall::Navigate { target, .. } => self.agent_navigate(reply, &session, tab, target),
            AgentCall::ReadPage {
                within, max_items, ..
            } => {
                let max = max_items.map_or(READ_ITEMS, |n| n as usize);
                let within = within.map(|r| r.0);
                if let Some(text) = self.agent_read(tab, within.as_deref(), max) {
                    return self.agent_send(reply, ToolResult::text(text), Outcome::Done);
                }
                self.agent_wait(reply, Wait::Tree { tab, within, max }, WAIT);
            }
            AgentCall::Click {
                target,
                button,
                count,
                ..
            } => {
                let act = Act::Click {
                    button: mouse(button),
                    count,
                };
                self.agent_act(reply, tab, &target.0, act);
            }
            AgentCall::Type {
                target,
                text,
                submit,
                ..
            } => {
                let act = Act::Type {
                    text: Secret::new(text),
                    submit,
                };
                self.agent_act(reply, tab, &target.0, act);
            }
            AgentCall::PressKey { key, .. } => match self.agent_keys(tab, &key) {
                Ok(()) => self.agent_send(reply, ToolResult::text("pressed"), Outcome::Done),
                Err(e) => self.agent_send(reply, ToolResult::error(e), Outcome::Failed),
            },
            AgentCall::Screenshot { crop, .. } => self.agent_shot(reply, tab, crop.map(|r| r.0)),
            AgentCall::ListTabs { .. } => {
                let text = self.agent_list(&session);
                self.agent_send(reply, ToolResult::text(text), Outcome::Done);
            }
            AgentCall::OpenTab { url, .. } => self.agent_open_tab(reply, &session, url),
            AgentCall::CloseTab { .. } => {
                self.dispatch(Command::CloseTab(tab));
                self.agent_send(reply, ToolResult::text("closed"), Outcome::Done);
            }
            AgentCall::WaitFor {
                condition,
                timeout_ms,
                ..
            } => {
                let wait = match condition {
                    WaitCondition::Load => Wait::Loaded { tab },
                    WaitCondition::Text(text) => Wait::Text { tab, text },
                    WaitCondition::Element(r) => Wait::Element { tab, target: r.0 },
                };
                let within = timeout_ms.map_or(WAIT, |ms| Duration::from_millis(ms.into()));
                self.agent_wait(reply, wait, within);
                self.agent_check();
            }
            AgentCall::Evaluate { script, .. } => {
                match self.tab_view(tab).and_then(|v| v.eval(&script)) {
                    Some(id) => self.agent_wait(reply, Wait::Eval { id: id.0 }, WAIT),
                    None => self.agent_fail(reply, "the page cannot run scripts now"),
                }
            }
            AgentCall::FillSecret { target, secret, .. } => {
                self.agent_fill(reply, &session, tab, target.0, &secret)
            }
        }
    }

    fn agent_start(
        &mut self,
        reply: Reply,
        purpose: String,
        sites: Vec<String>,
        secrets: Vec<String>,
        headless: bool,
    ) {
        let Some(client) = self.agents.conns.get(&reply.conn).map(|c| c.client.clone()) else {
            return;
        };
        let id = SessionId(format!("s{}", self.agents.fresh()));
        self.agents.sessions.insert(
            id.clone(),
            Session {
                conn: reply.conn,
                state: State::Requested,
                purpose,
                lists: agent::Lists::asked(&secrets, &sites),
                start: Some(reply.id),
                space: None,
                note: None,
                asked: 0,
            },
        );
        let request = Request {
            client,
            wants_secrets: secrets,
            wants_mode: if headless {
                Mode::Headless
            } else {
                Mode::Windowed
            },
        };
        let env = self.agent_env();
        self.agent_event(&id, Event::ClientAsks(request, env));
    }

    /// The tab `handle` names, if it is in the session's Space and container.
    fn agent_tab(&self, session: &SessionId, handle: &str) -> Option<TabId> {
        let (space, container) = self.agents.sessions.get(session)?.space?;
        let tab = TabId(handle.parse().ok()?);
        self.model.holds(space, container, tab).then_some(tab)
    }

    fn agent_grant(&self, session: &SessionId) -> Option<&Grant> {
        match &self.agents.sessions.get(session)?.state {
            State::Running { grant, .. } => Some(grant),
            _ => None,
        }
    }

    fn agent_navigate(&mut self, reply: Reply, session: &SessionId, tab: TabId, target: NavTarget) {
        let before = self
            .ax_tree(tab)
            .and_then(|t| t.document().map(str::to_owned));
        match target {
            NavTarget::Url(url) => {
                if let Err(why) = self.agent_may_visit(session, &url) {
                    return self.agent_refuse(reply, why);
                }
                self.dispatch(Command::Navigate { tab, url });
            }
            NavTarget::Back | NavTarget::Forward | NavTarget::Reload => {
                let Some(view) = self.tab_view(tab) else {
                    return self.agent_fail(reply, "the tab has no page");
                };
                match target {
                    NavTarget::Back => view.back(),
                    NavTarget::Forward => view.forward(),
                    _ => view.reload(),
                }
            }
        }
        let wait = Wait::Navigated {
            tab,
            before,
            load: Load::Asked,
        };
        self.agent_wait(reply, wait, WAIT);
    }

    /// Whether the session may load `url`: http(s) only, and on the grant's sites if it has any.
    fn agent_may_visit(&self, session: &SessionId, url: &str) -> Result<(), &'static str> {
        if !(url.starts_with("http://") || url.starts_with("https://")) {
            return Err("only http and https pages can be opened");
        }
        match self.agent_grant(session) {
            Some(g) if g.allows_site(url) => Ok(()),
            _ => Err("not a site this session may visit"),
        }
    }

    fn agent_open_tab(&mut self, reply: Reply, session: &SessionId, url: Option<String>) {
        if let Some(url) = &url
            && let Err(why) = self.agent_may_visit(session, url)
        {
            return self.agent_refuse(reply, why);
        }
        let Some((space, _)) = self.agents.sessions.get(session).and_then(|s| s.space) else {
            return self.agent_fail(reply, "the session has no Space");
        };
        self.dispatch(Command::OpenInSpace {
            space,
            url: url.unwrap_or_else(|| BLANK.into()),
        });
        match self.model.order(space).last().copied() {
            Some(tab) => {
                let body = json!({ "tab": tab.0.to_string() });
                self.agent_opened(reply, tab, body.to_string());
            }
            None => self.agent_fail(reply, "the tab did not open"),
        }
    }

    fn agent_list(&mut self, session: &SessionId) -> String {
        let space = self
            .agents
            .sessions
            .get(session)
            .and_then(|s| s.space)
            .map(|(sp, _)| sp);
        let tabs = space.map(|sp| self.model.order(sp)).unwrap_or_default();
        let rows: Vec<Value> = tabs
            .into_iter()
            .filter_map(|t| {
                let loading = self
                    .tab_view(t)
                    .is_some_and(|v| matches!(v.nav().phase, Phase::Loading { .. }));
                let tab = self.model.tabs.get(&t)?;
                Some(json!({
                    "tab": t.0.to_string(),
                    "title": tab.title,
                    "url": tab.url,
                    "loading": loading,
                }))
            })
            .collect();
        Value::Array(rows).to_string()
    }

    /// The page's items as `read_page` lines, once its tree exists.
    fn agent_read(&mut self, tab: TabId, within: Option<&str>, max: usize) -> Option<String> {
        let items = self
            .ax_tree(tab)
            .filter(|t| t.viewport().is_some())?
            .flatten(ALL_ITEMS);
        let book = self.agents.refs.entry(tab).or_default();
        let area = within.and_then(|r| {
            let (tree, node) = book.resolve(r)?;
            items
                .iter()
                .find(|i| i.tree == tree && i.id == node)
                .map(|i| i.bounds)
        });
        if within.is_some() && area.is_none() {
            return Some("stale ref: take a new snapshot".into());
        }
        let lines: Vec<String> = items
            .iter()
            .filter(|i| area.is_none_or(|a| inside(i.bounds, a)))
            .take(max)
            .map(|i| line(i, &book.name(&i.tree, i.id)))
            .collect();
        Some(lines.join("\n"))
    }

    /// The item `target` names on `tab`'s page now, and the page's viewport.
    fn agent_item(&self, tab: TabId, target: &str) -> Option<(AxItem, Bounds, bool)> {
        let (tree, node) = self.agents.refs.get(&tab)?.resolve(target)?;
        let page = self.ax_tree(tab)?;
        let viewport = page.viewport()?;
        let top = page.document() == Some(tree);
        let item = page
            .flatten(ALL_ITEMS)
            .into_iter()
            .find(|i| i.tree == tree && i.id == node)?;
        Some((item, viewport, top))
    }

    fn agent_act(&mut self, reply: Reply, tab: TabId, target: &str, act: Act) {
        let Some((item, viewport, _)) = self.agent_item(tab, target) else {
            return self.agent_fail(reply, "stale ref: take a new snapshot");
        };
        match wubble_ax::reach(item.bounds, viewport) {
            Reach::At { x, y } => self.agent_input(reply, tab, x, y, act),
            Reach::ScrollBy { dx, dy } => {
                let script = super::ax::scroll_script(dx, dy);
                match self.tab_view(tab).and_then(|v| v.eval(&script)) {
                    Some(id) => {
                        let wait = Wait::Scroll {
                            id: id.0,
                            tab,
                            bounds: item.bounds,
                            viewport,
                            act,
                        };
                        self.agent_wait(reply, wait, WAIT);
                    }
                    None => self.agent_fail(reply, "the page cannot scroll now"),
                }
            }
        }
    }

    /// The click or text at `(x, y)` of `tab`'s page (CSS px), through the view's pointer and
    /// key input as the user's would go, and the call's answer.
    fn agent_input(&mut self, reply: Reply, tab: TabId, x: f64, y: f64, act: Act) {
        let now = self.input.now();
        let Some(view) = self.tab_view(tab) else {
            return self.agent_fail(reply, "the tab has no page");
        };
        let point = Point {
            x: x as f32,
            y: y as f32,
        };
        view.focus(true);
        view.mouse_move(point, &Modifiers::default(), now);
        let (button, count) = match &act {
            Act::Click { button, count } => (*button, *count),
            Act::Type { .. } | Act::Fill { .. } => (MouseButton::Left, 1),
        };
        // each call's clicks are counted alone, not as the next of the last call's
        view.forget_clicks();
        for _ in 0..count {
            for pressed in [true, false] {
                let modifiers = Modifiers {
                    buttons: Buttons::default().with(button, pressed),
                    ..Modifiers::default()
                };
                view.mouse_button(button, pressed, point, &modifiers, now);
            }
        }
        let answer = match act {
            Act::Click { .. } => "clicked",
            Act::Type { text, submit } => {
                view.edit(EditCommand::SelectAll);
                view.ime_commit(text.expose());
                if submit && let Err(e) = self.agent_keys(tab, "Enter") {
                    return self.agent_fail(reply, &e);
                }
                "typed"
            }
            Act::Fill { text, name } => {
                let Some(id) = view.eval(&focus_script(x, y)) else {
                    return self.agent_fail(reply, "the page cannot run scripts now");
                };
                let wait = Wait::Focus {
                    id: id.0,
                    tab,
                    text,
                    name,
                };
                return self.agent_wait(reply, wait, WAIT);
            }
        };
        self.pace(wubble_gfx::pacing::PaceEvent::Wake);
        self.agent_send(reply, ToolResult::text(answer), Outcome::Done);
    }

    /// The page's answer to [`focus_script`]: the secret is typed only into the element that
    /// was clicked, in the top document, while the page is still on the login's site.
    fn agent_focused(&mut self, reply: Reply, tab: TabId, text: Secret, name: &str, href: &str) {
        let granted = reply
            .session
            .as_ref()
            .and_then(|s| self.agent_grant(s))
            .is_some_and(|g| g.secret_for(name, href, href).is_some());
        if !granted {
            return self.agent_refuse(
                reply,
                "the field did not take focus in the login site's own page",
            );
        }
        let Some(view) = self.tab_view(tab) else {
            return self.agent_fail(reply, "the tab has no page");
        };
        view.edit(EditCommand::SelectAll);
        view.ime_commit(text.expose());
        self.pace(wubble_gfx::pacing::PaceEvent::Wake);
        self.agent_send(reply, ToolResult::text("filled"), Outcome::Done);
    }

    /// A Playwright key name pressed and released on `tab`'s page.
    fn agent_keys(&mut self, tab: TabId, name: &str) -> Result<(), String> {
        let chord =
            agent::keys::chord_of(name).map_err(|e| format!("unknown key {name:?}: {e:?}"))?;
        let keys = crate::parity::key_inputs(chord).ok_or(format!("no key for {name:?}"))?;
        let view = self.tab_view(tab).ok_or("the tab has no page")?;
        view.focus(true);
        for key in &keys {
            view.key(key);
        }
        Ok(())
    }

    fn agent_shot(&mut self, reply: Reply, tab: TabId, crop: Option<String>) {
        let clip = match crop {
            Some(r) => match self.agent_item(tab, &r) {
                Some((item, _, _)) => Some(item.bounds),
                None => return self.agent_fail(reply, "stale ref: take a new snapshot"),
            },
            None => None,
        };
        let params = match clip {
            Some(b) => wubble_core::shot::params(wubble_core::shot::Capture::Clip(
                wubble_core::shot::RectF {
                    x: b.x,
                    y: b.y,
                    w: b.width,
                    h: b.height,
                },
            )),
            None => wubble_core::shot::params(wubble_core::shot::Capture::Visible),
        };
        match self
            .tab_view(tab)
            .and_then(|v| v.cdp("Page.captureScreenshot", &params))
        {
            Some(id) => self.agent_wait(reply, Wait::Shot { id: id.0 }, WAIT),
            None => self.agent_fail(reply, "the page cannot be captured now"),
        }
    }

    fn agent_fill(
        &mut self,
        reply: Reply,
        session: &SessionId,
        tab: TabId,
        target: String,
        name: &str,
    ) {
        let url = self
            .tab_view(tab)
            .map(|v| v.nav().page.url.clone())
            .unwrap_or_default();
        let Some(login) = self
            .agent_grant(session)
            .and_then(|g| g.secret_for(name, &url, &url))
            .cloned()
        else {
            return self.agent_refuse(reply, "not granted, or this page is not the login's site");
        };
        // Only into the page's own document: a frame's origin is not checked here.
        match self.agent_item(tab, &target) {
            Some((_, _, true)) => {}
            Some(_) => return self.agent_refuse(reply, "secrets are not filled into frames"),
            None => return self.agent_fail(reply, "stale ref: take a new snapshot"),
        }
        let nonce = self.agents.fresh();
        let wait = Wait::Vault {
            nonce,
            tab,
            target,
            name: name.to_owned(),
        };
        self.agent_wait(reply, wait, WAIT);
        self.store.vault(move |vault| {
            let value = vault
                .reveal(&RecordId(login.0))
                .map(|p| Secret::new(p.as_str()))
                .ok();
            let _ = slint::invoke_from_event_loop(move || {
                with_host(|host| host.agent_vault(nonce, value));
            });
        });
    }

    fn agent_vault(&mut self, nonce: u64, value: Option<Secret>) {
        let Some(i) = self
            .agents
            .pending
            .iter()
            .position(|p| matches!(p.wait, Wait::Vault { nonce: n, .. } if n == nonce))
        else {
            return;
        };
        let p = self.agents.pending.remove(i);
        let Wait::Vault {
            tab, target, name, ..
        } = p.wait
        else {
            return;
        };
        match value {
            Some(text) => self.agent_act(p.reply, tab, &target, Act::Fill { text, name }),
            None => self.agent_fail(p.reply, "the vault has no such login"),
        }
    }

    // ----- waiting

    fn agent_wait(&mut self, reply: Reply, wait: Wait, within: Duration) {
        self.agents.pending.push(Pending {
            reply,
            wait,
            deadline: Instant::now() + within,
        });
        self.agent_arm();
    }

    /// The deadline timer, at the first call's deadline.
    fn agent_arm(&mut self) {
        let Some(first) = self.agents.pending.iter().map(|p| p.deadline).min() else {
            self.agents.timer.stop();
            return;
        };
        let after = first.saturating_duration_since(Instant::now());
        self.agents
            .timer
            .start(slint::TimerMode::SingleShot, after, || {
                with_host(Host::agent_check)
            });
    }

    /// A web event of `tab`: the start, then the end, of a load lets `navigate` go on.
    pub(super) fn agent_web(&mut self, tab: TabId, event: &WebEvent) {
        let step = |load: Load| match (load, event) {
            (Load::Asked, WebEvent::LoadStarted) => Load::Started,
            (Load::Started, WebEvent::LoadingChanged { loading: false, .. }) => Load::Ended,
            (load, _) => load,
        };
        for p in &mut self.agents.pending {
            if let Wait::Navigated { tab: t, load, .. } = &mut p.wait
                && *t == tab
            {
                *load = step(*load);
            }
        }
    }

    /// The page's answer to a script or a capture a call sent; whether it was one.
    pub(super) fn agent_answer(&mut self, event: &PageEvent) -> bool {
        let id = match event {
            PageEvent::EvalResult { id, .. } => id.0,
            PageEvent::CdpReply { id, .. } => id.0,
            _ => return false,
        };
        let Some(i) = self.agents.pending.iter().position(|p| match p.wait {
            Wait::Eval { id: w }
            | Wait::Shot { id: w, .. }
            | Wait::Scroll { id: w, .. }
            | Wait::Focus { id: w, .. } => w == id,
            _ => false,
        }) else {
            return false;
        };
        let p = self.agents.pending.remove(i);
        self.agent_arm();
        match (p.wait, event) {
            (Wait::Eval { .. }, PageEvent::EvalResult { result, .. }) => match result {
                Ok(json) => self.agent_send(p.reply, ToolResult::text(json.clone()), Outcome::Done),
                Err(e) => self.agent_fail(p.reply, e),
            },
            (Wait::Shot { .. }, PageEvent::CdpReply { result, .. }) => {
                let data = result
                    .as_ref()
                    .ok()
                    .and_then(|v| v.get("data"))
                    .and_then(Value::as_str);
                match data {
                    Some(png) => {
                        let image = ToolResult::image(png, "image/png");
                        self.agent_send(p.reply, image, Outcome::Done);
                    }
                    None => self.agent_fail(p.reply, &format!("no picture: {result:?}")),
                }
            }
            (
                Wait::Scroll {
                    tab,
                    bounds,
                    viewport,
                    act,
                    ..
                },
                PageEvent::EvalResult { result, .. },
            ) => match super::ax::reach_after(bounds, viewport, result) {
                Some(Reach::At { x, y }) => self.agent_input(p.reply, tab, x, y, act),
                Some(Reach::ScrollBy { .. }) => {
                    self.agent_fail(p.reply, "the element stays out of view")
                }
                None => self.agent_fail(p.reply, "the page did not scroll"),
            },
            (
                Wait::Focus {
                    tab, text, name, ..
                },
                PageEvent::EvalResult { result, .. },
            ) => {
                let href = result
                    .as_deref()
                    .ok()
                    .and_then(|t| serde_json::from_str::<Option<String>>(t).ok())
                    .flatten()
                    .unwrap_or_default();
                self.agent_focused(p.reply, tab, text, &name, &href);
            }
            (_, _) => self.agent_fail(p.reply, "an unexpected answer"),
        }
        true
    }

    /// Answers every waiting call whose condition holds now, and those past their deadline.
    pub(super) fn agent_check(&mut self) {
        if self.agents.pending.is_empty() {
            return;
        }
        let now = Instant::now();
        let waiting = std::mem::take(&mut self.agents.pending);
        let mut kept = vec![];
        for p in waiting {
            if let Some(p) = self.agent_try(p, now) {
                kept.push(p);
            }
        }
        // calls answered meanwhile may have queued others
        kept.append(&mut self.agents.pending);
        self.agents.pending = kept;
        self.agent_arm();
    }

    /// `p` answered if it can be, else back.
    fn agent_try(&mut self, p: Pending, now: Instant) -> Option<Pending> {
        let late = now >= p.deadline;
        let tab = match &p.wait {
            Wait::Navigated { tab, .. }
            | Wait::Loaded { tab }
            | Wait::Opened { tab, .. }
            | Wait::Text { tab, .. }
            | Wait::Element { tab, .. }
            | Wait::Tree { tab, .. } => *tab,
            // answered by the page or the vault; only their deadline is checked here
            _ if late => {
                self.agent_fail(p.reply, "the page did not answer in time");
                return None;
            }
            _ => return Some(p),
        };
        if !self.model.tabs.contains_key(&tab) {
            self.agent_fail(p.reply, "the tab closed");
            return None;
        }
        let phase = self.tab_view(tab).map(|v| v.nav().phase);
        let loading = matches!(phase, Some(Phase::Loading { .. }));
        let page = self.ax_tree(tab).filter(|t| t.viewport().is_some());
        let document = page.and_then(PageTree::document).map(str::to_owned);
        let done: Option<String> = match &p.wait {
            Wait::Navigated { before, load, .. } => {
                let ended = *load == Load::Ended;
                let fresh = document.is_some() && document != *before;
                match (ended && fresh, ended && late) {
                    (true, _) => Some("loaded".into()),
                    (false, true) => Some("loaded (the page's tree did not change)".into()),
                    _ => None,
                }
            }
            Wait::Loaded { .. } => (!loading && document.is_some()).then(|| "loaded".into()),
            Wait::Opened { answer, .. } => (phase == Some(Phase::Loaded)).then(|| answer.clone()),
            Wait::Text { text, .. } => page
                .map(|t| t.flatten(ALL_ITEMS))
                .is_some_and(|items| {
                    items
                        .iter()
                        .any(|i| i.name.contains(text.as_str()) || i.value.contains(text.as_str()))
                })
                .then(|| "found".into()),
            Wait::Element { tab, target } => self.agent_item(*tab, target).map(|_| "found".into()),
            Wait::Tree { tab, within, max } => {
                let (tab, within, max) = (*tab, within.clone(), *max);
                self.agent_read(tab, within.as_deref(), max)
            }
            _ => None,
        };
        match done {
            Some(text) => {
                self.agent_send(p.reply, ToolResult::text(text), Outcome::Done);
                None
            }
            None if late => {
                self.agent_fail(p.reply, "timed out");
                None
            }
            None => Some(p),
        }
    }

    // ----- answers

    fn agent_refuse(&mut self, reply: Reply, why: &str) {
        self.agent_send(reply, ToolResult::error(why), Outcome::Refused);
    }

    fn agent_fail(&mut self, reply: Reply, why: &str) {
        self.agent_send(reply, ToolResult::error(why), Outcome::Failed);
    }

    /// The call's answer on its connection, logged.
    fn agent_send(&mut self, reply: Reply, result: ToolResult, outcome: Outcome) {
        let session = reply.session.clone().unwrap_or(SessionId("-".into()));
        self.agent_log(&session, reply.tool, reply.target.clone(), outcome);
        if let Some(conn) = self.agents.conns.get(&reply.conn) {
            conn.out.send(wubble_mcp::tool_response(&reply.id, &result));
        }
    }

    fn agent_log(
        &mut self,
        session: &SessionId,
        tool: &str,
        target: Option<String>,
        outcome: Outcome,
    ) {
        let entry = Entry {
            session: session.clone(),
            at_ms: now_ms(),
            tool: tool.into(),
            target,
            outcome,
        };
        self.store.push_agent_log(entry);
    }

    /// `--smoke mcp`: the sheet is up. Checks what it shows against what the client asked, then
    /// answers it with the pointer, as a person would (`mcp_smoke::sheet`): Deny for the session
    /// that asks for a secret; Allow for the next, headless as it asked; for the one after, a
    /// press on Windowed first, which the sheet must then show selected. Each press must answer
    /// the sheet at once; the Space the approval opens must be in the mode the sheet showed
    /// (`agent_open_space`).
    pub(crate) fn mcp_smoke_answer(&mut self) {
        use crate::mcp_smoke::sheet;
        let Some(window) = self.agents.sheet.as_ref().and_then(|(w, _)| w.upgrade()) else {
            return self.fail("the approval sheet is not up".into());
        };
        let view = window.get_agent_sheet_view();
        let secrets: Vec<String> = view.secrets.iter().map(|s| s.key.to_string()).collect();
        let sites: Vec<String> = view.sites.iter().map(|s| s.key.to_string()).collect();
        if !view.open || view.client.is_empty() {
            return self.fail("the approval sheet names no client".into());
        }
        let deny = !secrets.is_empty();
        if deny && (secrets != ["GITHUB_PASSWORD"] || sites != ["example.com"]) {
            return self.fail(format!("the sheet shows {secrets:?} on {sites:?}"));
        }
        if !deny && (!sites.is_empty() || !view.headless) {
            return self.fail(format!(
                "the sheet shows {sites:?}, headless {}",
                view.headless
            ));
        }
        let Some(run) = &mut self.checks.mcp_smoke else {
            return;
        };
        // the third sheet: the owner picks Windowed over the client's headless
        let windowed = !deny && run.allowed == 1;
        if !deny {
            run.allowed += 1;
            run.expect = Some(if windowed {
                Mode::Windowed
            } else {
                Mode::Headless
            });
        }
        let serial = view.serial;
        let headless = view.headless;
        let client = view.client.to_string();
        let weak = window.as_weak();
        // later, outside the host: the sheet's callbacks call back into it
        let _ = slint::invoke_from_event_loop(move || {
            let fail = |why: String| with_host(|h| h.fail(why));
            let shown = |w: &AppWindow| {
                let v = w.get_agent_sheet_view();
                v.open && v.serial == serial
            };
            let steps = || -> Result<(), String> {
                let w = weak.upgrade().ok_or("no window")?;
                let asked = if headless { "Headless" } else { "Windowed" };
                if sheet::chosen(&w)?.as_deref() != Some(asked) {
                    return Err(format!(
                        "the sheet's window choice shows {:?} selected, the client asked {asked}",
                        sheet::chosen(&w)?
                    ));
                }
                if windowed {
                    let at = sheet::window_choice(&w, 0)?;
                    sheet::click(&w, at);
                    if sheet::chosen(&w)?.as_deref() != Some("Windowed") {
                        return Err(format!(
                            "a press on Windowed at {at:?} left {:?} selected",
                            sheet::chosen(&w)?
                        ));
                    }
                }
                let label = if deny { "Deny" } else { "Allow" };
                let at = sheet::button(&w, label)?;
                sheet::click(&w, at);
                if shown(&w) {
                    return Err(format!(
                        "a press on {label} at {at:?} did not answer the sheet"
                    ));
                }
                println!(
                    "smoke: mcp sheet {client} asks {secrets:?} on {sites:?}, pressed {}{label}",
                    if windowed { "Windowed, then " } else { "" }
                );
                Ok(())
            };
            if let Err(why) = steps() {
                fail(why);
            }
        });
    }

    /// `--smoke mcp`'s client finished: its step count, or what failed. Then the agent's
    /// closed tab and its loads must be none of the user's (the store answers in order, so
    /// after what the session's calls filed).
    pub(crate) fn mcp_smoke_done(&mut self, result: Result<usize, String>) {
        let steps = match result {
            Ok(steps) => steps,
            Err(e) => return self.fail(e),
        };
        if let Some(run) = &mut self.checks.mcp_smoke {
            run.steps = steps;
        }
        self.mcp_smoke_switch(0);
    }

    /// `--smoke mcp`, after the client: the windowed session's two tabs in the window: the
    /// first shown, the second, the first again (the same size), the second, then the sidebar
    /// hidden (the second's pane widens, `SWITCH_RESIZE`), then the first again. A tab shown again must present a frame of its own at its pane's new size:
    /// an agent's view is never hidden for CEF, so nothing else replaces the small snapshot its
    /// pane stands in with, nor the size it had while unshown. Each step waits for its tab's
    /// frame (`frame_arrived`); a deadline says which never came.
    pub(super) fn mcp_smoke_switch(&mut self, step: usize) {
        let tabs: Vec<TabId> = self
            .agents
            .sessions
            .values()
            .find(|s| matches!(&s.state, State::Running { grant, .. } if grant.mode == Mode::Windowed))
            .and_then(|s| s.space)
            .map(|(space, _)| self.model.order(space))
            .unwrap_or_default();
        let [first, second] = tabs[..] else {
            return self.fail(format!("the windowed session has tabs {tabs:?}, not two"));
        };
        let Some(&tab) = [first, second, first, second, second, first].get(step) else {
            // shown again at full size: on to the user's closed tabs and history
            let steps = self.checks.mcp_smoke.as_ref().map_or(0, |r| r.steps);
            self.store.pop_closed(move |closed| {
                let _ = slint::invoke_from_event_loop(move || {
                    with_host(|host| match closed {
                        Some(c) => host.fail(format!("the user's closed tabs hold {}", c.url)),
                        None => host.mcp_smoke_history(steps),
                    })
                });
            });
            return;
        };
        if let Some(run) = &mut self.checks.mcp_smoke {
            run.switch = Some((step, tab));
        }
        if step == SWITCH_RESIZE {
            // the sidebar hides: the pane widens (the shell's callback calls back into the host:
            // later, outside it)
            let _ = self.w.window.upgrade_in_event_loop(|window| {
                window.invoke_sidebar_mode_requested(wubble_ui::SidebarMode::Hidden);
            });
        } else {
            self.dispatch(Command::ActivateTab(tab));
            // already on screen with a frame of its own (the first step, if it was shown)
            if self.w.frames.contains_key(&tab) && !self.snapshots.standing_in(tab) {
                return self.mcp_smoke_frame(tab);
            }
        }
        let deadline = slint::Timer::default();
        deadline.start(slint::TimerMode::SingleShot, WAIT, move || {
            with_host(|h| {
                if h.checks.mcp_smoke.as_ref().and_then(|r| r.switch) == Some((step, tab)) {
                    let shows = h.w.frames.get(&tab).map(|f| f.image.size());
                    let snapshot = if h.snapshots.standing_in(tab) {
                        ", its snapshot"
                    } else {
                        ""
                    };
                    h.fail(format!(
                        "agent tab {} (switch step {step}) got no frame at its pane's size: \
                         the pane shows {shows:?}{snapshot}",
                        tab.0
                    ));
                }
            });
        });
        if let Some(run) = &mut self.checks.mcp_smoke {
            run.deadline = deadline;
        }
    }

    /// `--smoke mcp`: a frame of `tab` reached its pane. The switch's step waiting for it moves
    /// on once the pane shows a frame of its own at its view's size (CEF may paint a pixel
    /// more), after the resize one at a new size; later, outside the frame loop.
    pub(super) fn mcp_smoke_frame(&mut self, tab: TabId) {
        let Some((step, waiting)) = self.checks.mcp_smoke.as_ref().and_then(|r| r.switch) else {
            return;
        };
        if waiting != tab {
            return;
        }
        let _ = slint::invoke_from_event_loop(move || {
            with_host(|h| {
                if h.checks.mcp_smoke.as_ref().and_then(|r| r.switch) != Some((step, tab)) {
                    return;
                }
                let wanted = h.tab_view(tab).and_then(|v| v.wanted_frame());
                let shows = h.w.frames.get(&tab).map(|f| f.image.size());
                let Some(run) = &mut h.checks.mcp_smoke else {
                    return;
                };
                let (Some(w), Some(s)) = (wanted, shows) else {
                    return;
                };
                let full = s.width + 1 >= w.width && s.height + 1 >= w.height;
                let resized = run
                    .sized
                    .is_some_and(|before| before != (s.width, s.height));
                // not yet: the resize's frame, or a frame painted before it, comes later
                if !full || (step >= SWITCH_RESIZE && !resized) {
                    return;
                }
                if step + 1 == SWITCH_RESIZE {
                    run.sized = Some((s.width, s.height));
                }
                run.switch = None;
                println!(
                    "smoke: mcp switch step {step} tab {}: {}x{}",
                    tab.0, s.width, s.height
                );
                h.mcp_smoke_switch(step + 1);
            })
        });
    }

    fn mcp_smoke_history(&mut self, steps: usize) {
        self.store.visits("", None, 100, move |page| {
            let left = page.rows.into_iter().find(|r| r.url.contains("127.0.0.1"));
            let _ = slint::invoke_from_event_loop(move || {
                with_host(|host| match left {
                    Some(r) => host.fail(format!("the user's history holds {}", r.url)),
                    None => {
                        host.mcp_passed = Some(steps);
                        crate::quit();
                    }
                })
            });
        });
    }
}
