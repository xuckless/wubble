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
use wubble_core::agent::forms;
use wubble_core::agent::keys::{KeyPlan, SessionNav};
use wubble_core::agent::log::{Entry, Outcome};
use wubble_core::agent::refs::RefBook;
use wubble_core::agent::spot;
use wubble_core::agent::{self, Effect, Env, Event, Grant, Mode, Request, SessionId, State};
use wubble_core::agent::{MoreAnswer, Refusal, StopReason, Token};
use wubble_core::model::Command;
use wubble_core::overlay::Kind;
use wubble_core::secret::Secret;
use wubble_core::space::{ContainerId, SpaceId};
use wubble_core::tab::TabId;
use wubble_engine::input::{Buttons, Modifiers, MouseButton, Point};
use wubble_engine::nav::{Phase, WebEvent};
use wubble_engine::page::{EditCommand, PageEvent};
use wubble_mcp::{
    AgentCall, ButtonKind, CallId, Direction, DragTo, NavTarget, Offset, ToolResult, WaitCondition,
};
use wubble_ui::{AgentLogItem, AgentSheetView, AppWindow};
use wubble_vault::RecordId;

use super::{Host, now_ms};
use crate::mcp::{ConnId, Event as McpEvent, Outbox};
use crate::with_host;

/// How long a call may wait for the page unless it says otherwise (`wait_for`'s `timeout_ms`).
const WAIT: Duration = Duration::from_secs(30);
/// Items `read_page` returns unless asked for fewer.
const READ_ITEMS: usize = 500;
/// Characters `read_text` returns unless asked otherwise.
const READ_CHARS: usize = 20_000;
/// Characters of a script's value `evaluate` returns.
const EVAL_CHARS: usize = 100_000;
/// After `type` with `submit`, how long a load has to begin before none is taken to follow.
const SUBMIT_GRACE: Duration = Duration::from_secs(1);
/// Ticks a `scroll` turns the wheel in, and the time between them.
const WHEEL_TICKS: u32 = 5;
const WHEEL_TICK: Duration = Duration::from_millis(20);
/// How often a call waiting on the owner reports progress.
const BEAT: Duration = Duration::from_secs(5);
/// How long an action that may start a navigation (a click, a key, a script) waits before it
/// answers, so a navigation its page started and the session's sites refused is its error.
const SETTLE: Duration = Duration::from_millis(300);
/// Steps a drag's pointer moves in between its press and its release.
const DRAG_STEPS: u32 = 8;
/// How long after a script's own `timeout_ms` the call gives up waiting for the page's answer.
const EVAL_GRACE: Duration = Duration::from_millis(1000);
/// The DevTools object group `evaluate`'s values are held in until answered.
const EVAL_GROUP: &str = "wubble-agent";
/// The DevTools object group `select`'s resolved node is held in until answered.
const SELECT_GROUP: &str = "wubble-agent-select";
/// Turns `evaluate`'s value into plain JSON (called on it with `returnByValue`): nodes and
/// functions named, cycles and depth cut, maps and sets as objects and arrays.
const SERIALISE: &str = "function(){const seen=new WeakSet();let n=0;\
const walk=(v,d)=>{if(v===undefined)return null;\
if(typeof v==='bigint'||typeof v==='symbol')return v.toString();\
if(typeof v==='number'&&!isFinite(v))return String(v);\
if(v===null||(typeof v!=='object'&&typeof v!=='function'))return v;\
if(typeof v==='function')return '[function '+(v.name||'anonymous')+']';\
if(typeof Node!=='undefined'&&v instanceof Node)return '[node '+String(v.nodeName).toLowerCase()+(v.id?'#'+v.id:'')+']';\
if(seen.has(v))return '[circular]';if(d>20||++n>20000)return '[cut]';seen.add(v);\
if(v instanceof Date)return isNaN(v)?null:v.toISOString();if(v instanceof RegExp)return String(v);\
if(v instanceof Error)return v.stack||String(v);\
if(v instanceof Map)return Object.fromEntries(Array.from(v,([k,x])=>[String(k),walk(x,d+1)]));\
if(v instanceof Set||Array.isArray(v)||ArrayBuffer.isView(v))return Array.from(v,x=>walk(x,d+1));\
const o={};for(const k of Object.keys(v))o[k]=walk(v[k],d+1);return o};return walk(this,0)}";
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
    /// The calling program, as the connection found it (the sheets name it).
    client: String,
    state: State,
    purpose: String,
    /// What the client asked for, as the owner leaves it on the sheet; Allow grants its sites.
    lists: agent::Lists,
    /// The `start_session` call, answered once the token exists (or the session ended).
    start: Option<CallId>,
    /// The Space and container the approval opened.
    space: Option<(SpaceId, ContainerId)>,
    /// Why the session was refused, when not the owner's word.
    note: Option<&'static str>,
    /// When it began waiting for the sheet (`Agents::fresh`); the sheet takes the lowest.
    asked: u64,
    /// The `request_more` call waiting for the owner's answer.
    more: Option<Reply>,
    /// Where a filled secret could be read back by a script (`agent::Tainted`).
    tainted: agent::Tainted,
    /// The sign-ins copy in flight (`Effect::CopyCookies`), while one is.
    copy: Option<CopyRun>,
}

/// A copy of the owner's sign-ins in flight: its serial (a late answer to an older one is
/// dropped), its stop, and its bound ([`COPY_BOUND`]).
struct CopyRun {
    serial: u64,
    cancel: wubble_engine::storage::CopyCancel,
    _bound: slint::Timer,
}

/// How long a sign-ins copy may take before the session's start (or its `request_more`)
/// fails: CEF answers a copy of a few thousand cookies well inside it.
const COPY_BOUND: Duration = Duration::from_secs(20);

/// What a call's answer is logged and sent with.
#[derive(Clone)]
struct Reply {
    conn: ConnId,
    id: CallId,
    session: Option<SessionId>,
    tool: &'static str,
    target: Option<String>,
}

/// What a call does at an element once it is located on screen.
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
    Hover,
    /// Press here, then release at `to`.
    Drag {
        to: DragTo,
    },
    /// Release, here, the drag pressed at `from`.
    Drop {
        from: (f64, f64),
    },
    /// The wheel, by `(dx, dy)` CSS px (positive: right, down), over the element.
    Wheel {
        dx: f64,
        dy: f64,
    },
    /// Into view only.
    Reveal,
    /// A picture of the element.
    Shot,
}

/// How far the load a call asked for has come: the end of a load started before it (a new
/// tab's `about:blank`) is not its end.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Load {
    Asked,
    Started,
    Ended,
}

/// Asks the page whether the click at `(x, y)` focused the element there, in the top
/// document (not a frame): `P` (a password field) or `T`, then its URL, if so; else `null`.
fn focus_script(x: f64, y: f64) -> String {
    format!(
        "(()=>{{const e=document.activeElement,h=document.elementFromPoint({x},{y});\
         return e&&e!==document.body&&!/^I?FRAME$/.test(e.tagName)&&h&&(e===h||e.contains(h))\
         ?(e.type==='password'?'P':'T')+location.href:null}})()"
    )
}

/// Whether the focused field may take `part`: a password only into a password field (one a
/// show-password toggle turned into text is refused: turn it back), a username never into one.
fn field_takes(part: agent::Part, password_field: bool) -> Result<(), &'static str> {
    match (part, password_field) {
        (agent::Part::Password, true) | (agent::Part::Username, false) => Ok(()),
        (agent::Part::Password, false) => Err(
            "a password is typed only into a password field (if a show-password toggle made it \
             text, turn it back)",
        ),
        (agent::Part::Username, true) => Err("a username is not typed into a password field"),
    }
}

/// Where locating an element is (DevTools: `DOM.scrollIntoViewIfNeeded`, then
/// `DOM.getContentQuads`, by the node's backend id, which is its accessibility id).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Locating {
    Revealing,
    Measuring,
}

/// Where `select` is: scrolling the `<select>` into view, resolving its DOM node, or picking the
/// option in the page.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Selecting {
    Revealing,
    Resolving,
    Applying,
}

/// Where `evaluate` is: the script as given, the script again as an async function body (it
/// had a top-level `return`), the promise it gave being awaited, or its value being serialised.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Evaluating {
    First,
    AsBody,
    Awaiting,
    Serialising,
}

/// What a waiting call waits for.
enum Wait {
    /// `navigate` (and `type` with `submit`, `open_tab` with a URL): its load to end and the
    /// new document's tree (other than `before`). With `grace`: no load begun by then means
    /// none follows.
    Navigated {
        tab: TabId,
        before: Option<String>,
        load: Load,
        answer: String,
        grace: Option<(Instant, String)>,
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
        /// Form fields' values count too.
        values: bool,
        /// Waits for the text to be gone.
        gone: bool,
    },
    Element {
        tab: TabId,
        target: String,
    },
    /// The tab's address to contain `part`.
    Url {
        tab: TabId,
        part: String,
    },
    /// The tab's address or document to differ from these.
    Moved {
        tab: TabId,
        url: String,
        document: Option<String>,
    },
    /// `read_page` before the page has a tree.
    Tree {
        tab: TabId,
        within: Option<String>,
        max: usize,
    },
    Eval {
        id: u64,
        tab: TabId,
        script: String,
        timeout: u32,
        stage: Evaluating,
    },
    /// `read_text`'s article script.
    Article {
        id: u64,
        max: usize,
    },
    Shot {
        id: u64,
    },
    /// An element of a frame was out of view: the page was asked to scroll it to the middle.
    Scroll {
        id: u64,
        tab: TabId,
        bounds: Bounds,
        viewport: Bounds,
        act: Act,
    },
    /// An element of the page's own document, being scrolled into view and measured.
    Locate {
        id: u64,
        tab: TabId,
        node: i32,
        stage: Locating,
        at: Option<Offset>,
        viewport: Bounds,
        act: Act,
    },
    /// `screenshot` of an element: where the viewport is in the document, for the clip.
    Metrics {
        id: u64,
        tab: TabId,
        rect: spot::Rect,
    },
    /// `fill_secret`: the vault's answer.
    Vault {
        nonce: u64,
        tab: TabId,
        target: String,
        name: String,
    },
    /// A click, a key or a script done: its answer after [`SETTLE`], unless a navigation it
    /// started is refused meanwhile (`agent_refused`).
    Settle {
        tab: TabId,
        answer: String,
    },
    /// `fill_secret` clicked its element at `at`: the page's word on where focus went
    /// ([`focus_script`]).
    Focus {
        id: u64,
        tab: TabId,
        at: (f64, f64),
        text: Secret,
        name: String,
    },
    /// `type` clicked its element at `at`: the page's word on whether focus landed on an
    /// editable target ([`forms::type_focus_script`]).
    TypeFocus {
        id: u64,
        tab: TabId,
        at: (f64, f64),
        text: Secret,
        submit: bool,
        before: Option<String>,
    },
    /// `select`: scrolling the `<select>` into view, resolving its DOM node, and choosing
    /// `option`.
    Select {
        id: u64,
        tab: TabId,
        node: i32,
        option: String,
        stage: Selecting,
    },
    /// `focus_tab`: the tab shows in its pane a frame of its own at its pane's size
    /// ([`Host::agent_shows`]).
    Shown {
        tab: TabId,
    },
    /// An action's input was sent: the [`Fence`] after it, answered once the page has
    /// handled all of it; then the action goes on.
    Input {
        id: u64,
        tab: TabId,
        /// The page's requested navigations before the input (`View::navigations_requested`).
        asked: u64,
        /// A load the input began may begin before the fence answers.
        load: Load,
        then: Then,
    },
    /// An action's input asked for a navigation: its answer once the load began (or the
    /// navigation was refused: `agent_refused`).
    Started {
        tab: TabId,
        load: Load,
        answer: String,
    },
}

/// The DevTools input event sent after an action's own input, through the same widget's input
/// router: `Input.dispatch*Event` replies only once the renderer has acknowledged the event
/// (`InputHandler::OnInputEventAck`, after the page's handlers ran), and the renderer handles a
/// widget's input in order, so the reply means the page has handled everything before it
/// (the pointer, keys, the select-all and the text commit share that pipe). A script or a
/// DevTools call sent at once instead overtakes the input (another pipe, straight to the main
/// thread) and sees the page as it was before.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Fence {
    /// A pointer move where the action left the pointer (viewport CSS px).
    Pointer(f64, f64),
    /// A wheel of no distance there, behind the action's wheel ticks (wheel events queue
    /// apart from other input, one at a time); DevTools first waits for a frame.
    Wheel(f64, f64),
    /// A key release of no key, after keys.
    Key,
}

impl Fence {
    /// The DevTools call that sends it.
    fn call(self) -> (&'static str, Value) {
        match self {
            Fence::Pointer(x, y) => (
                "Input.dispatchMouseEvent",
                json!({ "type": "mouseMoved", "x": x, "y": y }),
            ),
            Fence::Wheel(x, y) => (
                "Input.dispatchMouseEvent",
                json!({ "type": "mouseWheel", "x": x, "y": y, "deltaX": 0, "deltaY": 0 }),
            ),
            Fence::Key => ("Input.dispatchKeyEvent", json!({ "type": "keyUp" })),
        }
    }
}

/// What an action does once the page has handled its input (its [`Fence`] answered).
enum Then {
    /// The answer once a navigation the input asked for began; with none asked for, after
    /// [`SETTLE`] (a page's script may still navigate, and be refused).
    Settle(String),
    /// `type` with `submit`: waits as `navigate` does when the Enter asked for a navigation;
    /// else a load begun within [`SUBMIT_GRACE`] (a script's) still counts.
    Submitted { before: Option<String> },
    /// `fill_secret`'s click at `at`: the page is asked where focus went.
    Focus {
        at: (f64, f64),
        text: Secret,
        name: String,
    },
    /// `type`'s click at `at`: the page is asked whether focus landed on an editable target.
    TypeFocus {
        at: (f64, f64),
        text: Secret,
        submit: bool,
        before: Option<String>,
    },
    /// A drag let go.
    Dragged,
    /// The answer now.
    Answer(&'static str),
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
    /// Progress for the calls that wait on the owner (`start_session`, `request_more`), while
    /// any does; and how many beats it has sent.
    beat: slint::Timer,
    beats: u64,
    /// The window the sheet shows in and the session it shows, while it does.
    sheet: Option<(slint::Weak<AppWindow>, SessionId)>,
    /// Counts the sheets shown, so each restarts the sheet's window choice.
    sheets: i32,
    /// Each session tab's last address its grant allowed: where a tab that got somewhere else
    /// is taken back to.
    pub(super) last_at: HashMap<TabId, String>,
    /// The grant each view's navigation rule was last set from (`agent_rules`).
    rules: HashMap<wubble_engine::app::ViewId, Option<agent::Grant>>,
    /// The answer to the sheet's last Sites draft: its serial, and the note (empty: taken).
    answer: (i32, &'static str),
    /// DevTools calls whose answers mean nothing (an object group released).
    ignored: Vec<u64>,
    /// What the sheet knows of the vault for its login pickers (fetched as a sheet asking for
    /// secrets shows).
    sheet_vault: SheetVault,
    /// The values `fill_secret` typed, per session: redacted from every reply it gets.
    filled: BTreeMap<SessionId, Vec<Secret>>,
}

/// The vault as the agent sheet's login pickers see it.
#[derive(Default)]
enum SheetVault {
    /// Not asked yet (no sheet asked for secrets).
    #[default]
    Unknown,
    /// Locked, or unusable: the note says which; the sheet offers its Unlock.
    Closed(String),
    /// Unlocked: the saved logins (no password), as the vault listed them.
    Open(Vec<wubble_core::pages::LoginView>),
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

/// The tab a call waits on, if it waits on one.
fn wait_tab(wait: &Wait) -> Option<TabId> {
    match wait {
        Wait::Navigated { tab, .. }
        | Wait::Loaded { tab }
        | Wait::Opened { tab, .. }
        | Wait::Text { tab, .. }
        | Wait::Element { tab, .. }
        | Wait::Url { tab, .. }
        | Wait::Moved { tab, .. }
        | Wait::Tree { tab, .. }
        | Wait::Eval { tab, .. }
        | Wait::Scroll { tab, .. }
        | Wait::Locate { tab, .. }
        | Wait::Metrics { tab, .. }
        | Wait::Vault { tab, .. }
        | Wait::Focus { tab, .. }
        | Wait::TypeFocus { tab, .. }
        | Wait::Select { tab, .. }
        | Wait::Input { tab, .. }
        | Wait::Shown { tab }
        | Wait::Started { tab, .. }
        | Wait::Settle { tab, .. } => Some(*tab),
        _ => None,
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

/// One `read_page` line: role, level, name, value, a link's target, state, ref. Text is given
/// whole.
fn line(item: &AxItem, r: &str) -> String {
    let mut out = format!("- {}", item.role);
    if item.level > 0 {
        out.push_str(&format!(" {}", item.level));
    }
    out.push_str(&format!(" {:?}", item.name));
    if !item.value.is_empty() {
        out.push_str(&format!(" value={:?}", item.value));
    }
    if !item.url.is_empty() {
        out.push_str(&format!(" url={:?}", item.url));
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

/// Whether a frame of `shows` (px) fills a pane that wants `wanted`: CEF may paint a pixel more
/// or less.
fn fills(shows: (u32, u32), wanted: (u32, u32)) -> bool {
    shows.0 + 1 >= wanted.0 && shows.1 + 1 >= wanted.1
}

fn inside(b: Bounds, outer: Bounds) -> bool {
    b.x >= outer.x
        && b.y >= outer.y
        && b.x + b.width <= outer.x + outer.width
        && b.y + b.height <= outer.y + outer.height
}

fn point_of((x, y): (f64, f64)) -> Point {
    Point {
        x: x as f32,
        y: y as f32,
    }
}

/// The left button down at `point` (a drag's start).
fn press(view: &mut wubble_engine::view::View, point: Point, now: Duration) {
    view.focus(true);
    view.mouse_move(point, &Modifiers::default(), now);
    view.forget_clicks();
    let held = Modifiers {
        buttons: Buttons::default().with(MouseButton::Left, true),
        ..Modifiers::default()
    };
    view.mouse_button(MouseButton::Left, true, point, &held, now);
}

/// The pointer, its left button held, moved from `from` to `to` in [`DRAG_STEPS`] steps (a
/// page that starts a drag after a few pixels sees them).
fn glide(view: &mut wubble_engine::view::View, from: (f64, f64), to: (f64, f64), now: Duration) {
    let held = Modifiers {
        buttons: Buttons::default().with(MouseButton::Left, true),
        ..Modifiers::default()
    };
    for i in 1..=DRAG_STEPS {
        let t = f64::from(i) / f64::from(DRAG_STEPS);
        let at = (from.0 + (to.0 - from.0) * t, from.1 + (to.1 - from.1) * t);
        view.mouse_move(point_of(at), &held, now);
    }
}

/// `read_text`'s answer from the page's article script (`pagescripts::reader_text`): the
/// title, the byline and the text, or the page's whole text when it has no article.
fn article_text(json: &str) -> Option<String> {
    let v: Value = serde_json::from_str(json).ok()?;
    let field = |k: &str| v[k].as_str().unwrap_or_default().trim().to_owned();
    let text = field("text");
    let mut out = String::new();
    if v["article"] == true {
        for head in [field("title"), field("byline")] {
            if !head.is_empty() {
                out.push_str(&head);
                out.push('\n');
            }
        }
        out.push('\n');
    } else {
        out.push_str("[no article found: the page's whole text]\n\n");
    }
    out.push_str(&text);
    Some(out)
}

/// `text` cut to `max` characters, with a note saying so.
fn capped(text: &str, max: usize) -> String {
    let total = text.chars().count();
    if total <= max {
        return text.to_owned();
    }
    let mut out: String = text.chars().take(max).collect();
    out.push_str(&format!(
        "\n[cut at {max} of {total} characters: max_chars raises the cap, ref reads one part]"
    ));
    out
}

/// Why `evaluate` (and `wait_for` in field values) is refused after a fill.
const TAINTED: &str = "not in this tab: a secret was filled into it, or into a page of its \
    site, in this session, and a script there could read it back (evaluate and wait_for with \
    in_values stay off here for the rest of the session)";

/// The text a filled secret is replaced with in a reply.
const REDACTED: &str = "[secret]";

/// The shortest value `fill_secret` types: a shorter one could not be told apart from
/// ordinary text in a reply, so it is not typed at all (the call says why).
const MIN_SECRET: usize = 3;

/// The spellings a reply can carry `value` in: as typed, JSON-escaped (an `evaluate` result,
/// once or as a JSON string inside JSON), and Rust-escaped (`read_page` quotes names and values
/// with `{:?}`). A value with nothing to escape is spelt the same each way.
fn spellings(value: &str) -> Vec<String> {
    let inner = |quoted: String| quoted[1..quoted.len() - 1].to_owned();
    let json = inner(serde_json::to_string(value).unwrap_or_default());
    let json2 = inner(serde_json::to_string(&json).unwrap_or_default());
    let debug = inner(format!("{value:?}"));
    let mut all = vec![value.to_owned(), json, json2, debug];
    all.sort_by_key(|v| std::cmp::Reverse(v.len()));
    all.dedup();
    all
}

/// `text` with every spelling of each of `values` replaced by [`REDACTED`], longest first.
fn redact_text(text: &str, values: &[&str]) -> String {
    let mut all: Vec<String> = values.iter().flat_map(|v| spellings(v)).collect();
    all.sort_by_key(|v| std::cmp::Reverse(v.len()));
    let mut text = text.to_owned();
    for v in all.iter().filter(|v| !v.is_empty()) {
        if text.contains(v.as_str()) {
            text = text.replace(v.as_str(), REDACTED);
        }
    }
    text
}

/// Takes every value `fill_secret` typed for the session out of `result`'s text, in each
/// spelling a reply carries ([`spellings`]). What it cannot catch: a value a page or script
/// changed (split, reversed, encoded otherwise), and pixels (a screenshot of a field made
/// visible); `evaluate` is refused where a secret was typed (`agent::Tainted`).
fn redact(result: &mut ToolResult, values: &[Secret]) {
    let values: Vec<&str> = values.iter().map(Secret::expose).collect();
    for content in &mut result.content {
        if let wubble_mcp::Content::Text(text) = content {
            *text = redact_text(text, &values);
        }
    }
}

/// What a sheet asks about a session: who asks, the window it asked for, and whether it is a
/// sheet for more (`request_more`: only what is new, no window choice).
struct Ask {
    client: String,
    headless: bool,
    more: bool,
}

/// The owner's Spaces sign-ins may come from (never an agent's): (id, name, container).
fn source_spaces(model: &wubble_core::model::Model) -> Vec<(SpaceId, String, ContainerId)> {
    model
        .spaces
        .iter()
        .filter(|s| !s.agent)
        .map(|s| (s.id, s.name.clone(), s.container))
        .collect()
}

/// The Space "Use my sign-ins" takes them from: the one picked, else the one the owner is in,
/// else the first of the owner's.
fn source_of(
    spaces: &[(SpaceId, String, ContainerId)],
    picked: Option<SpaceId>,
    active: SpaceId,
) -> Option<usize> {
    let at = |id: SpaceId| spaces.iter().position(|(s, ..)| *s == id);
    picked
        .and_then(at)
        .or_else(|| at(active))
        .or((!spaces.is_empty()).then_some(0))
}

/// The logins a name may stand for, in the pickers' order: those of the sheet's sites first
/// (by the grant's own patterns), then the rest, each group as the vault listed them.
fn picker_logins<'a>(
    logins: &'a [wubble_core::pages::LoginView],
    sites: &[String],
) -> Vec<&'a wubble_core::pages::LoginView> {
    let scope = agent::cookie_scope(sites);
    let on_site = |l: &wubble_core::pages::LoginView| {
        scope.iter().any(|p| p.matches(&format!("{}/", l.origin)))
    };
    let (mut first, rest): (Vec<_>, Vec<_>) = logins
        .iter()
        .filter(|l| l.username.is_some())
        .partition(|l| on_site(l));
    first.extend(rest);
    first
}

/// A name's picker after "Not bound": each login twice, its username and its password, the
/// part the name suggests first ([`agent::suggested_part`]); the owner's pick is the part.
fn binding_choices<'a>(
    logins: &[&'a wubble_core::pages::LoginView],
    name: &str,
) -> Vec<(&'a wubble_core::pages::LoginView, agent::Part)> {
    let first = agent::suggested_part(name);
    let other = match first {
        agent::Part::Username => agent::Part::Password,
        agent::Part::Password => agent::Part::Username,
    };
    logins
        .iter()
        .flat_map(|l| [(*l, first), (*l, other)])
        .collect()
}

/// What the sheet shows beyond the lists: the Spaces to share from, the logins to bind to.
struct SheetData<'a> {
    spaces: &'a [(SpaceId, String, ContainerId)],
    source: Option<usize>,
    vault: &'a SheetVault,
}

/// The sheet's window view of what it shows.
fn sheet_view(
    ask: &Ask,
    lists: &agent::Lists,
    data: &SheetData,
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
    let entry = |label: String| wubble_ui::MenuEntry {
        label: label.into(),
        enabled: true,
        ..Default::default()
    };
    let logins = match data.vault {
        SheetVault::Open(all) => picker_logins(all, &lists.sites),
        _ => vec![],
    };
    let bindings: Vec<wubble_ui::AgentBindingItem> = lists
        .secrets
        .iter()
        .map(|name| {
            let choices = binding_choices(&logins, name);
            let options: Vec<wubble_ui::MenuEntry> = std::iter::once(entry("Not bound".into()))
                .chain(choices.iter().map(|(l, part)| {
                    let part = match part {
                        agent::Part::Username => "username",
                        agent::Part::Password => "password",
                    };
                    entry(format!(
                        "The {part} of {} on {}",
                        l.username.as_deref().unwrap_or_default(),
                        l.site()
                    ))
                }))
                .collect();
            let index = lists
                .bound
                .get(name)
                .and_then(|b| {
                    choices
                        .iter()
                        .position(|(l, p)| l.id == b.login.0 && *p == b.part)
                })
                .map_or(0, |i| i + 1);
            wubble_ui::AgentBindingItem {
                name: name.as_str().into(),
                title: format!("{name} stands for").into(),
                options: options.as_slice().into(),
                index: i32::try_from(index).unwrap_or(0),
            }
        })
        .collect();
    let (vault_locked, vault_note) = match data.vault {
        SheetVault::Closed(note) if !lists.secrets.is_empty() => (true, note.clone()),
        _ => (false, String::new()),
    };
    let from = data.source.and_then(|i| data.spaces.get(i));
    let unshareable = lists.unshareable();
    let shared: Vec<&str> = lists
        .sites
        .iter()
        .map(String::as_str)
        .filter(|s| !unshareable.contains(s))
        .collect();
    let never = if unshareable.is_empty() {
        String::new()
    } else {
        format!(
            " Never shared for {}: each is a whole domain ending, not one site.",
            unshareable.join(", ")
        )
    };
    let share_note = match from {
        Some(_) if shared.is_empty() => format!("No sign-ins can be shared.{never}"),
        Some((_, name, _)) => format!(
            "Cookies for {} are copied from your Space {name} into the agent's own Space before \
             it starts, with the cookies their parent domain sets for all its sites (sharing \
             accounts.google.com shares the google.com sign-in); your own tabs stay signed in. \
             Stopping the session deletes the copies; a Space it hands to you keeps them until \
             you delete it. Sign-ins kept in a site's local storage are not copied.{never}",
            shared.join(", ")
        ),
        None => "You have no Space to take sign-ins from.".into(),
    };
    let spaces: Vec<wubble_ui::MenuEntry> = data
        .spaces
        .iter()
        .map(|(_, n, _)| entry(n.clone()))
        .collect();
    AgentSheetView {
        open: true,
        serial,
        client: ask.client.as_str().into(),
        secrets: rows(&lists.secrets),
        sites: rows(&lists.sites),
        answer: answer.0,
        note: answer.1.into(),
        headless: ask.headless,
        more: ask.more,
        may_share: lists.may_share() && !data.spaces.is_empty(),
        share: lists.share,
        share_from: spaces.as_slice().into(),
        share_from_index: data.source.and_then(|i| i32::try_from(i).ok()).unwrap_or(0),
        share_note: share_note.into(),
        bindings: bindings.as_slice().into(),
        vault_locked,
        vault_note: vault_note.into(),
    }
}

/// What the sheet asks about `s`, while it waits for one.
fn asking(s: &Session) -> Option<Ask> {
    match &s.state {
        State::AwaitingApproval(request) => Some(Ask {
            client: request.client.clone(),
            headless: request.wants_mode == Mode::Headless,
            more: false,
        }),
        State::AskingMore { grant, .. } => Some(Ask {
            client: s.client.clone(),
            headless: grant.mode == Mode::Headless,
            more: true,
        }),
        _ => None,
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
        StopReason::Released => "the session ended; its tabs are the user's now".into(),
        StopReason::ClientGone => "the client went away".into(),
        StopReason::BrowserQuit => "the browser quit".into(),
        StopReason::SignInsNotCopied => {
            "the user's sign-ins could not be copied into the session: it did not start".into()
        }
    }
}

impl Host {
    /// What a connection reports (`mcp.rs`).
    pub(crate) fn mcp_event(&mut self, event: McpEvent) {
        match event {
            McpEvent::Connected { conn, out, client } => {
                self.agents.conns.insert(conn, Conn { out, client });
            }
            // hosting turned off while the connection was open: nothing is served on it
            McpEvent::Call { conn, id, .. } if !self.mcp_serving() => {
                if let Some(c) = self.agents.conns.get(&conn) {
                    c.out.send(wubble_mcp::tool_response(
                        &id,
                        &ToolResult::error(crate::mcp::NOT_RUNNING),
                    ));
                }
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
        // `--smoke mcp` leaves a session running for this: its Space must be gone (unless the
        // run turned hosting off first, which ended it the same way: `mcp_smoke_off`)
        if self
            .checks
            .mcp_smoke
            .as_ref()
            .is_some_and(|r| !r.hosting_off)
        {
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
        Env {
            mcp_enabled: self.mcp_serving(),
            remote_port: self.debug_port(),
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
        self.agent_rules();
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
                // a copy in flight sets nothing more into the container going now
                if let Some(run) = self.agents.sessions.get_mut(id).and_then(|s| s.copy.take()) {
                    run.cancel.cancel();
                }
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
            // the Space and its container stay, the owner's now: once the session leaves the
            // table its tabs are no longer driven, and the Space is saved like any other
            Effect::Release => {
                if let Some((space, _)) = self
                    .agents
                    .sessions
                    .get_mut(id)
                    .and_then(|s| s.space.take())
                {
                    for tab in self.model.order(space) {
                        self.agents.refs.remove(&tab);
                    }
                    self.dispatch(Command::SetSpaceNeverSleep { space, on: false });
                }
            }
            Effect::ShowMoreSheet(more) => {
                let asked = self.agents.fresh();
                if let Some(s) = self.agents.sessions.get_mut(id) {
                    s.asked = asked;
                    // the sheet lists only what is new; the owner may edit it as at the start
                    s.lists = agent::Lists::asked(&more.secrets, &more.sites);
                }
                self.agent_sheet_pump();
            }
            Effect::CopyCookies { from, space, sites } => {
                self.agent_copy_cookies(id, from, space, sites)
            }
            Effect::AnswerMore(answer) => {
                let Some(reply) = self.agents.sessions.get_mut(id).and_then(|s| s.more.take())
                else {
                    return;
                };
                let sites: Vec<String> = self
                    .agents
                    .sessions
                    .get(id)
                    .and_then(|s| s.state.grant())
                    .map(|g| g.sites().map(str::to_owned).collect())
                    .unwrap_or_default();
                match answer {
                    MoreAnswer::Granted(more) => {
                        let body = json!({
                            "granted": {"sites": more.sites, "secrets": more.secrets},
                            "sites": sites,
                            "note": "a secret name is granted only when the user bound it to a \
                                saved login on the sheet; fill_secret types it on that login's site",
                        });
                        self.agent_send(reply, ToolResult::text(body.to_string()), Outcome::Done);
                    }
                    MoreAnswer::Denied => self.agent_refuse(
                        reply,
                        "the user denied it: the session runs on with what it had",
                    ),
                    MoreAnswer::NothingNew => self.agent_send(
                        reply,
                        ToolResult::text("nothing new: the session has all of it already"),
                        Outcome::Done,
                    ),
                    MoreAnswer::Busy => {
                        self.agent_refuse(reply, "the user is already being asked for more")
                    }
                    MoreAnswer::NotCopied => self.agent_fail(
                        reply,
                        "the user allowed it with their sign-ins, but those could not be copied: \
                         nothing was granted, the session runs on with what it had",
                    ),
                }
            }
        }
    }

    /// "Use my sign-ins": copies container `from`'s cookies for `sites` into the session's
    /// container (`wubble_engine::app::Engine::copy_cookies`, the edge), within
    /// [`COPY_BOUND`]; logs the copy (the sites, the Space and a count: never a name or value)
    /// and tells the session, once: copied, or not (refused, or out of time).
    fn agent_copy_cookies(
        &mut self,
        id: &SessionId,
        from: ContainerId,
        space: String,
        sites: Vec<String>,
    ) {
        let Some((_, to)) = self.agents.sessions.get(id).and_then(|s| s.space) else {
            let why = "the session has no Space to copy into".to_owned();
            return self.agent_cookies_copied(id, None, &sites, &space, Err(why));
        };
        let serial = self.agents.fresh();
        let scope = agent::cookie_scope(&sites);
        let filter: wubble_engine::storage::CookieFilter = Box::new(move |domain, path, secure| {
            agent::shares_cookie(&scope, domain, path, secure)
        });
        let (session, names, named) = (id.clone(), sites.clone(), space.clone());
        let done = Box::new(move |report: wubble_engine::storage::CopyReport| {
            let _ = slint::invoke_from_event_loop(move || {
                with_host(|host| {
                    host.mcp_smoke_copied(from, to, &report);
                    host.agent_cookies_copied(&session, Some(serial), &names, &named, Ok(report));
                });
            });
        });
        match self.engine.copy_cookies(from, to, filter, done) {
            Ok(cancel) => {
                let bound = slint::Timer::default();
                let session = id.clone();
                bound.start(slint::TimerMode::SingleShot, COPY_BOUND, move || {
                    let why = format!("it did not finish in {} s", COPY_BOUND.as_secs());
                    with_host(|h| {
                        h.agent_cookies_copied(&session, Some(serial), &sites, &space, Err(why))
                    });
                });
                if let Some(s) = self.agents.sessions.get_mut(id) {
                    s.copy = Some(CopyRun {
                        serial,
                        cancel,
                        _bound: bound,
                    });
                }
            }
            Err(e) => self.agent_cookies_copied(id, None, &sites, &space, Err(e.to_string())),
        }
    }

    /// The end of a copy: `serial` names the run (`None`: one that never started); an answer
    /// to a run no longer in flight (out of time already, or the session gone) is dropped.
    fn agent_cookies_copied(
        &mut self,
        id: &SessionId,
        serial: Option<u64>,
        sites: &[String],
        space: &str,
        report: Result<wubble_engine::storage::CopyReport, String>,
    ) {
        if let Some(serial) = serial {
            let Some(s) = self.agents.sessions.get_mut(id) else {
                return;
            };
            match s.copy.take() {
                Some(run) if run.serial == serial => run.cancel.cancel(),
                other => {
                    s.copy = other;
                    return;
                }
            }
        }
        let sites = sites.join(", ");
        let (target, outcome, event) = match &report {
            Ok(r) => (
                format!(
                    "cookies for {sites} copied from Space {space} ({} of {})",
                    r.set, r.taken
                ),
                Outcome::Done,
                Event::CookiesCopied,
            ),
            Err(why) => (
                format!("cookies for {sites} from Space {space} not copied: {why}"),
                Outcome::Failed,
                Event::CookiesNotCopied,
            ),
        };
        self.agent_log(id, "share_sign_ins", Some(target), outcome);
        self.agent_event(id, event);
    }

    /// `--smoke mcp`: a copy ended. Keeps its report and container, and counts the owner's
    /// cookies for the shared site, which the copy must have left where they were.
    fn mcp_smoke_copied(
        &mut self,
        from: ContainerId,
        to: ContainerId,
        report: &wubble_engine::storage::CopyReport,
    ) {
        let Some(run) = self.checks.mcp_smoke.as_mut() else {
            return;
        };
        run.copied.push(*report);
        run.signin_container = Some(to);
        let seen = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = seen.clone();
        // a copy into itself that takes nothing: a count
        let filter: wubble_engine::storage::CookieFilter = Box::new(move |domain, _, _| {
            if domain.trim_start_matches('.') == crate::mcp_smoke::SIGNIN_HOST {
                counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
            false
        });
        let done = Box::new(move |_| {
            let n = seen.load(std::sync::atomic::Ordering::Relaxed);
            let _ = slint::invoke_from_event_loop(move || {
                with_host(|h| {
                    if let Some(run) = h.checks.mcp_smoke.as_mut() {
                        run.owner_kept = Some(n);
                    }
                });
            });
        });
        if let Err(e) = self.engine.copy_cookies(from, from, filter, done) {
            self.fail(format!("the owner's cookies cannot be counted: {e}"));
        }
    }

    // ----- the approval sheet

    /// Shows the first waiting session's sheet when the overlays let it (one modal at a time:
    /// a page's dialog or the user's sheet makes it wait; the pump asks again).
    pub(super) fn agent_sheet_pump(&mut self) {
        self.agent_beat_arm();
        if self.agents.sheet.is_some() {
            return;
        }
        // the session that has waited longest (the session table is the one owner of who waits)
        let Some((id, lists)) = self
            .agents
            .sessions
            .iter()
            .filter_map(|(id, s)| Some((s.asked, id.clone(), asking(s).map(|_| s.lists.clone())?)))
            .min_by_key(|(asked, ..)| *asked)
            .map(|(_, id, lists)| (id, lists))
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
        self.agents.sheet = Some((window.as_weak(), id));
        // the login pickers need the vault's list (nothing while it is locked)
        if !lists.secrets.is_empty() {
            self.agent_sheet_vault(wubble_vault::VaultCommand::List);
        }
        self.agent_sheet_render();
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
        let Some(s) = self.agents.sessions.get(&id) else {
            return;
        };
        // the Space the sign-ins come from, resolved now (an approval switches the window to
        // the session's Space)
        let spaces = source_spaces(&self.model);
        let from = source_of(&spaces, s.lists.from, self.model.active_space()).map(|i| {
            let (_, space, container) = &spaces[i];
            agent::Source {
                container: *container,
                space: space.clone(),
            }
        });
        let event = match &s.state {
            // a sheet for more: Allow widens the grant by what the owner left on it
            State::AskingMore { grant, .. } => {
                agent::answer_more(grant, &s.lists, allow, from.as_ref())
            }
            _ => agent::answer(&s.lists, allow, headless, from.as_ref()),
        };
        // the session's transition dismisses the sheet and shows the next
        self.agent_event(&id, event);
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
        let Some(ask) = asking(s) else {
            return;
        };
        let spaces = source_spaces(&self.model);
        let data = SheetData {
            source: source_of(&spaces, s.lists.from, self.model.active_space()),
            spaces: &spaces,
            vault: &self.agents.sheet_vault,
        };
        window.set_agent_sheet_view(sheet_view(
            &ask,
            &s.lists,
            &data,
            self.agents.sheets,
            self.agents.answer,
        ));
    }

    /// "Use my sign-ins for these sites", flipped on the sheet.
    pub(crate) fn agent_sheet_share(&mut self, on: bool) {
        let _ = self.agent_sheet_edit(agent::ListEdit::Share(on));
        self.agent_sheet_render();
    }

    /// The Space the sign-ins come from, picked on the sheet (its index among the owner's).
    pub(crate) fn agent_sheet_share_from(&mut self, index: i32) {
        let spaces = source_spaces(&self.model);
        if let Some((space, ..)) = usize::try_from(index).ok().and_then(|i| spaces.get(i)) {
            let _ = self.agent_sheet_edit(agent::ListEdit::ShareFrom(*space));
        }
        self.agent_sheet_render();
    }

    /// A secret name's picker: option `index` (0: none) of the logins the sheet lists.
    pub(crate) fn agent_sheet_bound(&mut self, name: &str, index: i32) {
        let Some((_, id)) = &self.agents.sheet else {
            return;
        };
        let sites = self
            .agents
            .sessions
            .get(id)
            .map(|s| s.lists.sites.clone())
            .unwrap_or_default();
        let binding = match (&self.agents.sheet_vault, usize::try_from(index)) {
            (SheetVault::Open(all), Ok(i)) if i > 0 => {
                binding_choices(&picker_logins(all, &sites), name)
                    .get(i - 1)
                    .map(|(l, part)| agent::SecretBinding {
                        login: agent::LoginRef(l.id.clone()),
                        origin: l.origin.clone(),
                        part: *part,
                    })
            }
            _ => None,
        };
        let _ = self.agent_sheet_edit(agent::ListEdit::Bind {
            name,
            login: binding.as_ref(),
        });
        self.agent_sheet_render();
    }

    /// The sheet's Unlock: the owner unlocks the vault (the OS keyring may ask), then its logins
    /// are listed for the pickers.
    pub(crate) fn agent_sheet_unlock_vault(&mut self) {
        self.agent_sheet_vault(wubble_vault::VaultCommand::Unlock);
    }

    /// Asks the vault for the sheet's pickers; an unlock is followed by the list.
    fn agent_sheet_vault(&mut self, command: wubble_vault::VaultCommand) {
        self.store.vault_command(command, move |answer| {
            let _ = slint::invoke_from_event_loop(move || {
                with_host(|host| host.agent_sheet_vault_answered(answer));
            });
        });
    }

    fn agent_sheet_vault_answered(&mut self, answer: wubble_vault::VaultAnswer) {
        use wubble_core::pages::VaultStatus;
        use wubble_vault::VaultAnswer;
        match answer {
            VaultAnswer::Entries(rows) => self.agents.sheet_vault = SheetVault::Open(rows),
            VaultAnswer::Status(VaultStatus::Unlocked | VaultStatus::Unreadable { .. }) => {
                return self.agent_sheet_vault(wubble_vault::VaultCommand::List);
            }
            VaultAnswer::Status(VaultStatus::Locked) => {
                self.agents.sheet_vault =
                    SheetVault::Closed("Unlock them to pick the login each name stands for".into());
            }
            VaultAnswer::Status(status @ VaultStatus::KeyLost { .. }) => {
                let (_, body) = wubble_core::pages::status_text(&status);
                self.agents.sheet_vault = SheetVault::Closed(body);
            }
            VaultAnswer::Status(VaultStatus::Unavailable(why)) | VaultAnswer::Failed(why) => {
                self.agents.sheet_vault = SheetVault::Closed(format!("They cannot be used: {why}"));
            }
            VaultAnswer::Revealed(_) | VaultAnswer::Done | VaultAnswer::Imported(_) => {}
        }
        self.agent_sheet_render();
        if let Some(run) = self.checks.mcp_smoke.as_mut()
            && std::mem::take(&mut run.waiting_vault)
        {
            let _ = slint::invoke_from_event_loop(|| with_host(Host::mcp_smoke_answer));
        }
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
                client: "Claude Code".into(),
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
                more: None,
                tainted: agent::Tainted::default(),
                copy: None,
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
        let lists = agent::Lists::asked(&[], &[GALLERY_SITE.to_string()]);
        self.agent_event(&id, agent::answer(&lists, true, false, None));
        let Some((space, _)) = self.agents.sessions.get(&id).and_then(|s| s.space) else {
            return;
        };
        for url in [GALLERY_PAGES[0], GALLERY_PAGES[1]] {
            self.dispatch(Command::OpenInSpace {
                space,
                url: url.into(),
            });
        }
    }

    /// `--smoke gallery`: a Space a session made and left with its tabs (an agent Space for
    /// good, no session holding it).
    pub(super) fn gallery_agent_kept(&mut self) {
        const NAME: &str = "Agent: draft the issue";
        // one scene after another on one profile: made once
        if self.model.spaces.iter().any(|s| s.name == NAME) {
            return;
        }
        let Command::NewSpace { tint, .. } = self.model.new_space() else {
            return;
        };
        self.dispatch(Command::NewSpace {
            name: NAME.into(),
            tint,
            container: ContainerId::DEFAULT,
        });
        let Some(space) = self.model.spaces.last().map(|s| s.id) else {
            return;
        };
        self.dispatch(Command::MarkAgentSpace(space));
        self.dispatch(Command::OpenInSpace {
            space,
            url: GALLERY_PAGES[2].into(),
        });
    }

    /// `--smoke gallery`: the approved session asks for a site and a secret it lacks, and one
    /// it has: the sheet lists only the two new ones.
    pub(super) fn gallery_agent_more(&mut self) {
        self.gallery_agent(true);
        let id = SessionId("gallery".into());
        self.agent_event(
            &id,
            Event::AskMore {
                sites: vec!["github.com".into(), "registry.npmjs.org".into()],
                secrets: vec!["NPM_TOKEN".into()],
            },
        );
    }

    /// `--smoke gallery`: the sheet with the owner's sign-ins shared and GITHUB_TOKEN bound to
    /// a saved github.com login (saved in the run's throwaway vault, then listed for the
    /// pickers).
    pub(super) fn gallery_agent_signins(&mut self) {
        self.gallery_agent(false);
        let _ = self.agent_sheet_edit(agent::ListEdit::Share(true));
        self.agent_sheet_render();
        self.store.vault(|vault| {
            for (origin, username) in [
                ("https://github.com", "octocat"),
                ("https://example.org", "me@example.org"),
            ] {
                let Ok(origin) = wubble_vault::Origin::parse(origin) else {
                    continue;
                };
                let login = wubble_vault::NewLogin {
                    origin,
                    form_action_origin: None,
                    username: username.into(),
                    password: zeroize::Zeroizing::new("gallery-pw".into()),
                };
                let _ = vault.save(login, 0);
            }
            let answer = vault.run(wubble_vault::VaultCommand::List, now_ms());
            let _ = slint::invoke_from_event_loop(move || {
                with_host(|h| {
                    h.agent_sheet_vault_answered(answer);
                    h.agent_sheet_bound("GITHUB_TOKEN", 1);
                });
            });
        });
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

    /// The dustbin on an agent Space's row in the Spaces popup (`agent::clear`): the session
    /// holding the Space is stopped (that one alone), its teardown deleting the Space; a Space
    /// no session holds any more (its tabs were left to the owner) is deleted as a teardown
    /// does. A user's Space has no dustbin, and is never deleted here.
    pub(super) fn agent_space_cleared(&mut self, space: SpaceId) {
        let Some(container) = self
            .model
            .spaces
            .iter()
            .find(|s| s.id == space && s.agent)
            .map(|s| s.container)
        else {
            return;
        };
        let holder = self
            .agents
            .sessions
            .iter()
            .find(|(_, s)| s.space.is_some_and(|(sp, _)| sp == space))
            .map(|(id, s)| (id.clone(), s.state.clone()));
        match agent::clear(holder.as_ref().map(|(_, state)| state)) {
            agent::Clear::Stop => {
                if let Some((id, _)) = holder {
                    self.agent_event(&id, Event::Stop);
                }
            }
            agent::Clear::Delete => {
                self.dispatch(Command::DeleteSpace(space));
                // the session's own container; never the Default one, nor one another Space uses
                let shared = self.model.spaces.iter().any(|s| s.container == container);
                if container != ContainerId::DEFAULT && !shared {
                    self.dispatch(Command::DeleteContainer(container));
                }
            }
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
        self.dispatch(Command::MarkAgentSpace(space));
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
        let grant = s.state.grant();
        let headless = grant.is_some_and(|g| g.mode == Mode::Headless);
        let secrets: Vec<String> = grant
            .map(|g| g.secret_names().map(str::to_owned).collect())
            .unwrap_or_default();
        let signed_in: Vec<String> = grant
            .map(|g| g.signin_sites().map(str::to_owned).collect())
            .unwrap_or_default();
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
            "secrets": secrets,
            "signed_in_sites": signed_in,
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
        self.agents.filled.remove(id);
        let State::Stopped(reason) = s.state else {
            return;
        };
        let why = stopped_text(reason, s.note);
        if let Some(reply) = s.more {
            self.agent_send(reply, ToolResult::error(why.clone()), Outcome::Failed);
        }
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

    fn agent_call(&mut self, conn: ConnId, id: CallId, call: AgentCall) {
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
            AgentCall::StopSession { keep_tabs, .. } => {
                let (text, event) = if keep_tabs {
                    (
                        "stopped; the session's tabs are the user's now",
                        Event::Release,
                    )
                } else {
                    ("stopped", Event::Stop)
                };
                self.agent_send(reply, ToolResult::text(text), Outcome::Done);
                self.agent_event(&session, event);
            }
            AgentCall::RequestMore { sites, secrets, .. } => {
                let Some(s) = self.agents.sessions.get_mut(&session) else {
                    return;
                };
                if s.more.is_some() {
                    return self.agent_refuse(reply, "the user is already being asked for more");
                }
                // answered by the transition's `AnswerMore`, at once or from the sheet
                s.more = Some(reply);
                self.agent_event(&session, Event::AskMore { sites, secrets });
            }
            AgentCall::Navigate { target, .. } => self.agent_navigate(reply, &session, tab, target),
            AgentCall::ReadPage {
                within, max_items, ..
            } => {
                let max = max_items.map_or(READ_ITEMS, |n| n as usize);
                let within = within.map(|r| r.0);
                match self.agent_read(tab, within.as_deref(), max) {
                    Some(Ok(text)) => self.agent_send(reply, ToolResult::text(text), Outcome::Done),
                    Some(Err(e)) => self.agent_fail(reply, e),
                    None => self.agent_wait(reply, Wait::Tree { tab, within, max }, WAIT),
                }
            }
            AgentCall::ReadText {
                within, max_chars, ..
            } => {
                let max = max_chars.map_or(READ_CHARS, |n| n as usize);
                self.agent_read_text(reply, tab, within.map(|r| r.0), max);
            }
            AgentCall::Click {
                target,
                at,
                button,
                count,
                ..
            } => {
                let act = Act::Click {
                    button: mouse(button),
                    count,
                };
                self.agent_act(reply, tab, &target.0, at, act);
            }
            AgentCall::Hover { target, at, .. } => {
                self.agent_act(reply, tab, &target.0, at, Act::Hover)
            }
            AgentCall::Drag { target, at, to, .. } => {
                self.agent_act(reply, tab, &target.0, at, Act::Drag { to })
            }
            AgentCall::Scroll {
                target,
                direction,
                amount,
                ..
            } => self.agent_scroll(reply, tab, target.map(|r| r.0), direction, amount),
            AgentCall::Type {
                target,
                text,
                submit,
                ..
            } => {
                self.agent_type(reply, tab, &target.0, Secret::new(text), submit);
            }
            AgentCall::Select { target, option, .. } => {
                self.agent_select(reply, tab, &target.0, option);
            }
            AgentCall::PressKey { key, .. } => self.agent_press(reply, &session, tab, &key),
            AgentCall::Screenshot {
                crop, full_page, ..
            } => match crop {
                Some(r) => self.agent_act(reply, tab, &r.0, None, Act::Shot),
                None => {
                    let capture = if full_page {
                        wubble_core::shot::Capture::FullPage
                    } else {
                        wubble_core::shot::Capture::Visible
                    };
                    self.agent_capture(reply, tab, capture);
                }
            },
            AgentCall::ListTabs { .. } => {
                let text = self.agent_list(&session);
                self.agent_send(reply, ToolResult::text(text), Outcome::Done);
            }
            AgentCall::FocusTab { .. } => {
                let Some((space, _)) = self.agents.sessions.get(&session).and_then(|s| s.space)
                else {
                    return self.agent_fail(reply, "the session has no Space");
                };
                self.dispatch(Command::SwitchSpace(space));
                self.dispatch(Command::ActivateTab(tab));
                // answered once the window shows the tab's own frame at its pane's size, so
                // what the agent does next meets the page the owner sees
                self.agent_wait(reply, Wait::Shown { tab }, WAIT);
            }
            AgentCall::OpenTab { url, .. } => self.agent_open_tab(reply, &session, url),
            AgentCall::CloseTab { .. } => {
                self.dispatch(Command::CloseTab(tab));
                self.agent_send(reply, ToolResult::text("closed"), Outcome::Done);
            }
            AgentCall::WaitFor {
                condition,
                in_values,
                timeout_ms,
                ..
            } => {
                // a field's value matched against a guess is a way to read a secret back
                if in_values && self.agent_tainted(&session, tab) {
                    return self.agent_refuse(reply, TAINTED);
                }
                let text = |text: String, gone: bool| Wait::Text {
                    tab,
                    text,
                    values: in_values,
                    gone,
                };
                let wait = match condition {
                    WaitCondition::Load => Wait::Loaded { tab },
                    WaitCondition::Text(t) => text(t, false),
                    WaitCondition::TextGone(t) => text(t, true),
                    WaitCondition::Element(r) => Wait::Element { tab, target: r.0 },
                    WaitCondition::Url(part) => Wait::Url { tab, part },
                    WaitCondition::Navigation => Wait::Moved {
                        tab,
                        url: self.agent_url(tab),
                        document: self.agent_document(tab),
                    },
                };
                let within = timeout_ms.map_or(WAIT, |ms| Duration::from_millis(ms.into()));
                self.agent_wait(reply, wait, within);
                self.agent_check();
            }
            AgentCall::Evaluate {
                script, timeout_ms, ..
            } => {
                if self.agent_tainted(&session, tab) {
                    return self.agent_refuse(reply, TAINTED);
                }
                let timeout = timeout_ms.unwrap_or(WAIT.as_millis() as u32);
                self.agent_evaluate(reply, tab, script, timeout, Evaluating::First);
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
                client: client.clone(),
                state: State::Requested,
                purpose,
                lists: agent::Lists::asked(&secrets, &sites),
                start: Some(reply.id),
                space: None,
                note: None,
                asked: 0,
                more: None,
                tainted: agent::Tainted::default(),
                copy: None,
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

    /// Whether a secret was typed where `tab` is ([`agent::Tainted`]).
    fn agent_tainted(&self, session: &SessionId, tab: TabId) -> bool {
        let url = self.agent_url(tab);
        self.agents
            .sessions
            .get(session)
            .is_some_and(|s| s.tainted.refuses_scripts(tab, &url))
    }

    fn agent_grant(&self, session: &SessionId) -> Option<&Grant> {
        self.agents.sessions.get(session)?.state.grant()
    }

    fn agent_url(&self, tab: TabId) -> String {
        self.model
            .tabs
            .get(&tab)
            .map(|t| t.url.clone())
            .unwrap_or_default()
    }

    fn agent_document(&self, tab: TabId) -> Option<String> {
        self.ax_tree(tab)
            .and_then(|t| t.document().map(str::to_owned))
    }

    fn agent_navigate(&mut self, reply: Reply, session: &SessionId, tab: TabId, target: NavTarget) {
        let before = self.agent_document(tab);
        match target {
            NavTarget::Url(url) => {
                let url = match self.agent_may_visit(session, &url) {
                    Ok(url) => url,
                    Err(why) => return self.agent_refuse(reply, why),
                };
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
            answer: "loaded".into(),
            grace: None,
        };
        self.agent_wait(reply, wait, WAIT);
    }

    /// The address the session may load for `url` (the parser's spelling): http(s) only, and
    /// on the grant's sites if it has any (`agent::may_visit`).
    fn agent_may_visit(&self, session: &SessionId, url: &str) -> Result<String, &'static str> {
        let grant = self
            .agent_grant(session)
            .ok_or("the session is not running")?;
        agent::may_visit(grant, url).map_err(agent::VisitRefusal::note)
    }

    fn agent_open_tab(&mut self, reply: Reply, session: &SessionId, url: Option<String>) {
        let url = match url.map(|u| self.agent_may_visit(session, &u)).transpose() {
            Ok(url) => url,
            Err(why) => return self.agent_refuse(reply, why),
        };
        let Some((space, _)) = self.agents.sessions.get(session).and_then(|s| s.space) else {
            return self.agent_fail(reply, "the session has no Space");
        };
        let loads = url.is_some();
        self.dispatch(Command::OpenInSpace {
            space,
            url: url.unwrap_or_else(|| BLANK.into()),
        });
        let Some(tab) = self.model.order(space).last().copied() else {
            return self.agent_fail(reply, "the tab did not open");
        };
        let answer = json!({ "tab": tab.0.to_string() }).to_string();
        if loads {
            // as `navigate`: once its page has loaded
            let wait = Wait::Navigated {
                tab,
                before: None,
                load: Load::Asked,
                answer,
                grace: None,
            };
            self.agent_wait(reply, wait, WAIT);
        } else {
            self.agent_opened(reply, tab, answer);
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
        let active = space.and_then(|sp| self.model.spaces.iter().find(|s| s.id == sp)?.active);
        let shown = self.shown();
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
                    "active": active == Some(t),
                    "shown": shown == Some(t),
                }))
            })
            .collect();
        Value::Array(rows).to_string()
    }

    /// The page's items as `read_page` lines, once its tree exists; an error for a ref that
    /// names nothing on it. A list cut at `max` ends with a note of the total.
    fn agent_read(
        &mut self,
        tab: TabId,
        within: Option<&str>,
        max: usize,
    ) -> Option<Result<String, &'static str>> {
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
            return Some(Err("stale ref: take a new snapshot"));
        }
        let all: Vec<&AxItem> = items
            .iter()
            .filter(|i| area.is_none_or(|a| inside(i.bounds, a)))
            .collect();
        let mut lines: Vec<String> = all
            .iter()
            .take(max)
            .map(|i| line(i, &book.name(&i.tree, i.id)))
            .collect();
        if all.len() > max {
            let total = if items.len() >= ALL_ITEMS {
                format!("at least {}", all.len())
            } else {
                all.len().to_string()
            };
            lines.push(format!(
                "[{max} of {total} items: max_items raises the cap (default {READ_ITEMS}), \
                 ref reads one part]"
            ));
        }
        Some(Ok(lines.join("\n")))
    }

    /// `read_text`: one element's text from the accessibility mirror, or the page's article
    /// (Readability, in the page) or its whole text.
    fn agent_read_text(&mut self, reply: Reply, tab: TabId, within: Option<String>, max: usize) {
        let Some(r) = within else {
            let script = crate::pagescripts::reader_text();
            return match self.tab_view(tab).and_then(|v| v.eval(&script)) {
                Some(id) => self.agent_wait(reply, Wait::Article { id: id.0, max }, WAIT),
                None => self.agent_fail(reply, "the page cannot run scripts now"),
            };
        };
        let Some((item, _, _)) = self.agent_item(tab, &r) else {
            return self.agent_fail(reply, "stale ref: take a new snapshot");
        };
        let items = self
            .ax_tree(tab)
            .map(|t| t.flatten(ALL_ITEMS))
            .unwrap_or_default();
        let words: Vec<&str> = items
            .iter()
            .filter(|i| inside(i.bounds, item.bounds))
            .map(|i| i.name.as_str())
            .filter(|n| !n.is_empty())
            .collect();
        let text = if words.is_empty() {
            item.name.clone()
        } else {
            words.join("\n")
        };
        self.agent_send(reply, ToolResult::text(capped(&text, max)), Outcome::Done);
    }

    /// The item `target` names on `tab`'s page now, the page's viewport, and whether it is in
    /// the page's own document (not a frame's).
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

    /// `act` on the element `target` names. An element of the page's own document is
    /// scrolled into view and measured as the page lays it out now (DevTools, by its node:
    /// Blink's accessibility id is the node's own id), never from the mirror's boxes, which
    /// the page's scroll leaves behind; a frame's element is reached from the mirror's box.
    /// Raw coordinates (`at`) are taken only inside a canvas.
    fn agent_act(&mut self, reply: Reply, tab: TabId, target: &str, at: Option<Offset>, act: Act) {
        let Some((item, viewport, top)) = self.agent_item(tab, target) else {
            return self.agent_unpress(reply, tab, &act, "stale ref: take a new snapshot");
        };
        let canvas_point = at.is_some()
            || matches!(
                act,
                Act::Drag {
                    to: DragTo::Offset(_)
                }
            );
        if canvas_point && item.role != "canvas" {
            return self.agent_refuse(
                reply,
                "coordinates are taken only inside a <canvas> element: name the element by ref",
            );
        }
        if top && item.id > 0 {
            let params = json!({ "backendNodeId": item.id });
            return match self
                .tab_view(tab)
                .and_then(|v| v.cdp("DOM.scrollIntoViewIfNeeded", &params))
            {
                Some(id) => {
                    let wait = Wait::Locate {
                        id: id.0,
                        tab,
                        node: item.id,
                        stage: Locating::Revealing,
                        at,
                        viewport,
                        act,
                    };
                    self.agent_wait(reply, wait, WAIT)
                }
                None => self.agent_unpress(reply, tab, &act, "the page cannot be reached now"),
            };
        }
        if canvas_point || matches!(act, Act::Shot) {
            return self.agent_refuse(
                reply,
                "this works only on the page's own document, not in a frame",
            );
        }
        match wubble_ax::reach(item.bounds, viewport) {
            Reach::At { x, y } => self.agent_at(reply, tab, (x, y), act),
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
                    None => self.agent_unpress(reply, tab, &act, "the page cannot scroll now"),
                }
            }
        }
    }

    /// The element of a `Locate` was scrolled into view (`DOM.scrollIntoViewIfNeeded`) and
    /// measured (`DOM.getContentQuads`): the next step, or the action where it shows.
    fn agent_located(&mut self, p: Pending, result: &Result<Value, String>) {
        let Wait::Locate {
            tab,
            node,
            stage,
            at,
            viewport,
            act,
            ..
        } = p.wait
        else {
            return;
        };
        let answer = match result {
            Ok(v) => v,
            Err(e) => {
                let why = format!("the element cannot be reached: {e}");
                return self.agent_unpress(p.reply, tab, &act, &why);
            }
        };
        match stage {
            Locating::Revealing if matches!(act, Act::Reveal) => {
                self.agent_send(
                    p.reply,
                    ToolResult::text("scrolled into view"),
                    Outcome::Done,
                );
            }
            Locating::Revealing => {
                let params = json!({ "backendNodeId": node });
                match self
                    .tab_view(tab)
                    .and_then(|v| v.cdp("DOM.getContentQuads", &params))
                {
                    Some(id) => {
                        let wait = Wait::Locate {
                            id: id.0,
                            tab,
                            node,
                            stage: Locating::Measuring,
                            at,
                            viewport,
                            act,
                        };
                        self.agent_wait(p.reply, wait, WAIT);
                    }
                    None => {
                        self.agent_unpress(p.reply, tab, &act, "the page cannot be reached now")
                    }
                }
            }
            Locating::Measuring => {
                let quads = spot::quads(answer);
                let size = (viewport.width, viewport.height);
                if matches!(act, Act::Shot) {
                    return match spot::visible(&quads, size) {
                        Some(rect) => self.agent_measure(p.reply, tab, rect),
                        None => self.agent_fail(p.reply, spot::Unreachable::Hidden.note()),
                    };
                }
                let at = at.map(|o| (f64::from(o.x), f64::from(o.y)));
                let point = match spot::point(&quads, size, at) {
                    Ok(point) => point,
                    Err(why) => return self.agent_unpress(p.reply, tab, &act, why.note()),
                };
                // a drag to a point of the same canvas: its end is measured now too
                if let Act::Drag {
                    to: DragTo::Offset(o),
                } = &act
                {
                    let to = (f64::from(o.x), f64::from(o.y));
                    return match spot::point(&quads, size, Some(to)) {
                        Ok(end) => self.agent_drag(p.reply, tab, point, end),
                        Err(why) => self.agent_fail(p.reply, why.note()),
                    };
                }
                self.agent_at(p.reply, tab, point, act);
            }
        }
    }

    /// A failed step of an action: a drag's pressed button is let go first.
    fn agent_unpress(&mut self, reply: Reply, tab: TabId, act: &Act, why: &str) {
        if let Act::Drop { from } = act {
            let now = self.input.now();
            if let Some(view) = self.tab_view(tab) {
                let point = point_of(*from);
                view.mouse_button(MouseButton::Left, false, point, &Modifiers::default(), now);
            }
        }
        self.agent_fail(reply, why);
    }

    /// `act` at `(x, y)` of `tab`'s page (viewport CSS px), through the view's pointer and key
    /// input as the user's would go, and the call's answer.
    fn agent_at(&mut self, reply: Reply, tab: TabId, (x, y): (f64, f64), act: Act) {
        let now = self.input.now();
        let point = point_of((x, y));
        let Some(view) = self.tab_view(tab) else {
            return self.agent_unpress(reply, tab, &act, "the tab has no page");
        };
        match act {
            Act::Click { .. } | Act::Type { .. } | Act::Fill { .. } => {
                self.agent_input(reply, tab, x, y, act)
            }
            Act::Hover => {
                view.mouse_move(point, &Modifiers::default(), now);
                self.pace(wubble_gfx::pacing::PaceEvent::Wake);
                self.agent_fence(reply, tab, Fence::Pointer(x, y), Then::Answer("hovering"));
            }
            Act::Wheel { dx, dy } => {
                view.mouse_move(point, &Modifiers::default(), now);
                let step = (dx / f64::from(WHEEL_TICKS), dy / f64::from(WHEEL_TICKS));
                self.agent_wheel(reply, tab, point, step, WHEEL_TICKS);
            }
            Act::Drag {
                to: DragTo::Element(target),
            } => {
                // pressed here, then the end is found (and scrolled to, if it must be)
                press(view, point, now);
                self.agent_act(reply, tab, &target.0, None, Act::Drop { from: (x, y) });
            }
            Act::Drag {
                to: DragTo::Offset(_),
            } => self.agent_fail(reply, "a drag to a point is measured with its canvas"),
            Act::Drop { from } => {
                glide(view, from, (x, y), now);
                let held = Modifiers {
                    buttons: Buttons::default(),
                    ..Modifiers::default()
                };
                view.mouse_button(MouseButton::Left, false, point, &held, now);
                self.pace(wubble_gfx::pacing::PaceEvent::Wake);
                self.agent_fence(reply, tab, Fence::Pointer(x, y), Then::Dragged);
            }
            Act::Reveal => self.agent_send(reply, ToolResult::text("in view"), Outcome::Done),
            Act::Shot => self.agent_fail(
                reply,
                "a picture of an element of a frame cannot be cropped",
            ),
        }
    }

    /// The wheel's next `left` ticks of `step` (CSS px, positive: right, down) at `point`, one
    /// per [`WHEEL_TICK`] as a mouse wheel turns (Chromium takes a burst of ticks at once as one
    /// gesture it may drop); the answer after the last.
    fn agent_wheel(&mut self, reply: Reply, tab: TabId, point: Point, step: (f64, f64), left: u32) {
        let Some(view) = self.tab_view(tab) else {
            return self.agent_fail(reply, "the tab has no page");
        };
        // the view's wheel takes positive as up and left
        view.wheel(point, -step.0 as f32, -step.1 as f32, &Modifiers::default());
        self.pace(wubble_gfx::pacing::PaceEvent::Wake);
        if left <= 1 {
            let (x, y) = (f64::from(point.x), f64::from(point.y));
            return self.agent_fence(reply, tab, Fence::Wheel(x, y), Then::Answer("scrolled"));
        }
        slint::Timer::single_shot(WHEEL_TICK, move || {
            with_host(|h| h.agent_wheel(reply, tab, point, step, left - 1));
        });
    }

    /// A drag from `from` to `to` in one go (both measured already).
    fn agent_drag(&mut self, reply: Reply, tab: TabId, from: (f64, f64), to: (f64, f64)) {
        let now = self.input.now();
        let Some(view) = self.tab_view(tab) else {
            return self.agent_fail(reply, "the tab has no page");
        };
        press(view, point_of(from), now);
        glide(view, from, to, now);
        view.mouse_button(
            MouseButton::Left,
            false,
            point_of(to),
            &Modifiers::default(),
            now,
        );
        self.pace(wubble_gfx::pacing::PaceEvent::Wake);
        self.agent_fence(reply, tab, Fence::Pointer(to.0, to.1), Then::Dragged);
    }

    /// A drag was let go: a page's own drag and drop the press may have started (an HTML
    /// `draggable`, which runs in the window's pointer, not the agent's) is dropped with
    /// nothing, so the window is not left in it.
    fn agent_dragged(&mut self, reply: Reply) {
        let page_drag = self.drag_cancelled();
        let text = if page_drag {
            "dragged; the page began an HTML drag and drop, which agent drags do not carry: it was dropped with nothing"
        } else {
            "dragged"
        };
        self.agent_send(reply, ToolResult::text(text), Outcome::Done);
    }

    /// `scroll`: an element into view (`ref` alone), or the wheel over it or over the middle
    /// of the page.
    fn agent_scroll(
        &mut self,
        reply: Reply,
        tab: TabId,
        target: Option<String>,
        direction: Option<Direction>,
        amount: Option<u32>,
    ) {
        let viewport = self.ax_tree(tab).and_then(PageTree::viewport);
        let Some(direction) = direction else {
            let Some(target) = target else {
                return self.agent_fail(reply, "ref or direction is required");
            };
            return self.agent_act(reply, tab, &target, None, Act::Reveal);
        };
        let (w, h) = viewport.map_or((800.0, 600.0), |v| (v.width, v.height));
        let by = |size: f64| amount.map_or(size * 0.8, f64::from);
        let (dx, dy) = match direction {
            Direction::Up => (0.0, -by(h)),
            Direction::Down => (0.0, by(h)),
            Direction::Left => (-by(w), 0.0),
            Direction::Right => (by(w), 0.0),
        };
        match target {
            Some(target) => self.agent_act(reply, tab, &target, None, Act::Wheel { dx, dy }),
            None => self.agent_at(reply, tab, (w / 2.0, h / 2.0), Act::Wheel { dx, dy }),
        }
    }

    /// A click or text at `(x, y)` of `tab`'s page (CSS px), and the call's answer.
    fn agent_input(&mut self, reply: Reply, tab: TabId, x: f64, y: f64, act: Act) {
        let now = self.input.now();
        let before = self.agent_document(tab);
        let Some(view) = self.tab_view(tab) else {
            return self.agent_fail(reply, "the tab has no page");
        };
        let point = point_of((x, y));
        view.focus(true);
        view.mouse_move(point, &Modifiers::default(), now);
        let (button, count) = match &act {
            Act::Click { button, count } => (*button, *count),
            _ => (MouseButton::Left, 1),
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
        let then = match act {
            Act::Type { text, submit } => Then::TypeFocus {
                at: (x, y),
                text,
                submit,
                before,
            },
            Act::Fill { text, name } => Then::Focus {
                at: (x, y),
                text,
                name,
            },
            _ => Then::Settle("clicked".into()),
        };
        self.pace(wubble_gfx::pacing::PaceEvent::Wake);
        self.agent_fence(reply, tab, Fence::Pointer(x, y), then);
    }

    /// The [`Fence`] after an action's input to `tab`, and `then` once the page answers it.
    fn agent_fence(&mut self, reply: Reply, tab: TabId, fence: Fence, then: Then) {
        let (method, params) = fence.call();
        let Some(view) = self.tab_view(tab) else {
            return self.agent_fail(reply, "the tab has no page");
        };
        let asked = view.navigations_requested();
        match view.cdp(method, &params) {
            Some(id) => {
                let wait = Wait::Input {
                    id: id.0,
                    tab,
                    asked,
                    load: Load::Asked,
                    then,
                };
                self.agent_wait(reply, wait, WAIT);
            }
            None => self.agent_fail(reply, "the page cannot be reached now"),
        }
    }

    /// The page has handled an action's input (its [`Fence`] answered; `navigated`: the input
    /// asked for a navigation of the page): the action goes on.
    fn agent_then(&mut self, reply: Reply, tab: TabId, navigated: bool, load: Load, then: Then) {
        match then {
            Then::Settle(answer) if navigated => {
                let wait = Wait::Started { tab, load, answer };
                self.agent_wait(reply, wait, WAIT);
            }
            Then::Settle(answer) => self.agent_settle(reply, tab, answer),
            Then::Submitted { before } => {
                // as `navigate` when a load follows; a page that stays shows no load
                let grace = (!navigated).then(|| {
                    (
                        Instant::now() + SUBMIT_GRACE,
                        "typed and submitted; no page load followed".into(),
                    )
                });
                let wait = Wait::Navigated {
                    tab,
                    before,
                    load,
                    answer: "typed and submitted; the page loaded".into(),
                    grace,
                };
                self.agent_wait(reply, wait, WAIT);
            }
            Then::Focus { at, text, name } => {
                let Some(id) = self
                    .tab_view(tab)
                    .and_then(|v| v.eval(&focus_script(at.0, at.1)))
                else {
                    return self.agent_fail(reply, "the page cannot run scripts now");
                };
                let wait = Wait::Focus {
                    id: id.0,
                    tab,
                    at,
                    text,
                    name,
                };
                self.agent_wait(reply, wait, WAIT);
            }
            Then::TypeFocus {
                at,
                text,
                submit,
                before,
            } => {
                let Some(id) = self
                    .tab_view(tab)
                    .and_then(|v| v.eval(&forms::type_focus_script(at.0, at.1)))
                else {
                    return self.agent_fail(reply, "the page cannot run scripts now");
                };
                let wait = Wait::TypeFocus {
                    id: id.0,
                    tab,
                    at,
                    text,
                    submit,
                    before,
                };
                self.agent_wait(reply, wait, WAIT);
            }
            Then::Dragged => self.agent_dragged(reply),
            Then::Answer(answer) => self.agent_send(reply, ToolResult::text(answer), Outcome::Done),
        }
    }

    /// The page's answer to [`focus_script`]: the secret is typed only into the element that
    /// was clicked, in the top document, while the page is still on the login's site.
    #[allow(clippy::too_many_arguments)]
    fn agent_focused(
        &mut self,
        reply: Reply,
        tab: TabId,
        at: (f64, f64),
        text: Secret,
        name: &str,
        href: &str,
        password_field: bool,
    ) {
        let part = reply
            .session
            .as_ref()
            .and_then(|s| self.agent_grant(s))
            .and_then(|g| g.secret_for(name, href, href))
            .map(|b| b.part);
        let Some(part) = part else {
            return self.agent_refuse(
                reply,
                "the field did not take focus in the login site's own page",
            );
        };
        if let Err(why) = field_takes(part, password_field) {
            return self.agent_refuse(reply, why);
        }
        let Some(view) = self.tab_view(tab) else {
            return self.agent_fail(reply, "the tab has no page");
        };
        view.edit(EditCommand::SelectAll);
        view.ime_commit(text.expose());
        self.pace(wubble_gfx::pacing::PaceEvent::Wake);
        if let Some(session) = &reply.session {
            if let Some(s) = self.agents.sessions.get_mut(session) {
                s.tainted.filled(tab, href);
            }
            self.agents
                .filled
                .entry(session.clone())
                .or_default()
                .push(text);
        }
        self.agent_fence(
            reply,
            tab,
            Fence::Pointer(at.0, at.1),
            Then::Answer("filled"),
        );
    }

    /// `type`: refuses dropdown roles up front, then clicks and verifies focus before typing.
    fn agent_type(&mut self, reply: Reply, tab: TabId, target: &str, text: Secret, submit: bool) {
        let Some((item, _, _)) = self.agent_item(tab, target) else {
            return self.agent_fail(reply, "stale ref: take a new snapshot");
        };
        if let Some(why) = forms::type_role_refusal(&item.role) {
            return self.agent_refuse(reply, why);
        }
        self.agent_act(reply, tab, target, None, Act::Type { text, submit });
    }

    /// The page's answer to [`forms::type_focus_script`]: text is typed only once focus landed on an
    /// editable target element (or iframe) at `at`.
    #[allow(clippy::too_many_arguments)]
    fn agent_type_focused(
        &mut self,
        reply: Reply,
        tab: TabId,
        at: (f64, f64),
        text: Secret,
        submit: bool,
        before: Option<String>,
        result: &Result<String, String>,
    ) {
        let json = match result {
            Ok(json) => json,
            Err(e) => {
                return self
                    .agent_fail(reply, &format!("could not verify focus before typing: {e}"));
            }
        };
        if let Err(why) = forms::parse_type_focus(json) {
            return self.agent_refuse(reply, &why);
        }
        let Some(view) = self.tab_view(tab) else {
            return self.agent_fail(reply, "the tab has no page");
        };
        view.edit(EditCommand::SelectAll);
        view.ime_commit(text.expose());
        let then = if submit {
            if let Err(e) = self.agent_keys(tab, "Enter") {
                return self.agent_fail(reply, &e);
            }
            Then::Submitted { before }
        } else {
            Then::Settle("typed".into())
        };
        self.pace(wubble_gfx::pacing::PaceEvent::Wake);
        self.agent_fence(reply, tab, Fence::Pointer(at.0, at.1), then);
    }

    /// `select`: resolves a `<select>` in the page's own document and picks `option`.
    fn agent_select(&mut self, reply: Reply, tab: TabId, target: &str, option: String) {
        let Some((item, _, top)) = self.agent_item(tab, target) else {
            return self.agent_fail(reply, "stale ref: take a new snapshot");
        };
        if !top {
            return self.agent_refuse(
                reply,
                "select works only on the page's own document, not in a frame",
            );
        }
        if item.id <= 0 {
            return self.agent_fail(reply, "the element cannot be reached");
        }
        let params = json!({ "backendNodeId": item.id });
        match self
            .tab_view(tab)
            .and_then(|v| v.cdp("DOM.scrollIntoViewIfNeeded", &params))
        {
            Some(id) => {
                let wait = Wait::Select {
                    id: id.0,
                    tab,
                    node: item.id,
                    option,
                    stage: Selecting::Revealing,
                };
                self.agent_wait(reply, wait, WAIT);
            }
            None => self.agent_fail(reply, "the page cannot be reached now"),
        }
    }

    /// DevTools' answer to one stage of `select`.
    fn agent_selected(&mut self, p: Pending, result: &Result<Value, String>) {
        let Wait::Select {
            tab,
            node,
            option,
            stage,
            ..
        } = p.wait
        else {
            return;
        };
        match stage {
            Selecting::Revealing => {
                if let Err(e) = result {
                    return self
                        .agent_fail(p.reply, &format!("the element cannot be reached: {e}"));
                }
                let params = json!({ "backendNodeId": node, "objectGroup": SELECT_GROUP });
                match self
                    .tab_view(tab)
                    .and_then(|v| v.cdp("DOM.resolveNode", &params))
                {
                    Some(id) => {
                        let wait = Wait::Select {
                            id: id.0,
                            tab,
                            node,
                            option,
                            stage: Selecting::Resolving,
                        };
                        self.agent_wait(p.reply, wait, WAIT);
                    }
                    None => self.agent_fail(p.reply, "the page cannot be reached now"),
                }
            }
            Selecting::Resolving => {
                let v = match result {
                    Ok(v) => v,
                    Err(e) => {
                        self.agent_release_group(tab, SELECT_GROUP);
                        return self
                            .agent_fail(p.reply, &format!("the element cannot be resolved: {e}"));
                    }
                };
                let Some(object_id) = v.pointer("/object/objectId").and_then(Value::as_str) else {
                    self.agent_release_group(tab, SELECT_GROUP);
                    return self.agent_fail(p.reply, "the element could not be resolved to a node");
                };
                let params = json!({
                    "objectId": object_id,
                    "functionDeclaration": forms::SELECT_OPTION,
                    "arguments": [{ "value": option }],
                    "returnByValue": true,
                });
                match self
                    .tab_view(tab)
                    .and_then(|v| v.cdp("Runtime.callFunctionOn", &params))
                {
                    Some(id) => {
                        let wait = Wait::Select {
                            id: id.0,
                            tab,
                            node,
                            option,
                            stage: Selecting::Applying,
                        };
                        self.agent_wait(p.reply, wait, WAIT);
                    }
                    None => {
                        self.agent_release_group(tab, SELECT_GROUP);
                        self.agent_fail(p.reply, "the page cannot run scripts now");
                    }
                }
            }
            Selecting::Applying => {
                self.agent_release_group(tab, SELECT_GROUP);
                let v = match result {
                    Ok(v) => v,
                    Err(e) => {
                        return self.agent_fail(p.reply, &format!("could not select option: {e}"));
                    }
                };
                if let Some(thrown) = v.get("exceptionDetails") {
                    let what = thrown["exception"]["description"]
                        .as_str()
                        .or_else(|| thrown["text"].as_str())
                        .unwrap_or("an exception");
                    return self.agent_fail(p.reply, &format!("could not select option: {what}"));
                }
                match forms::format_select_result(&v["result"]["value"]) {
                    Ok(msg) => self.agent_settle(p.reply, tab, msg),
                    Err(why) => self.agent_fail(p.reply, &why),
                }
            }
        }
    }

    /// `press_key`: the chord to the page, unless it is one of wubble's shortcuts
    /// (`agent::keys::plan`): back, forward and reload act on this tab as `navigate`; the
    /// others never act for an agent, and the answer says so.
    fn agent_press(&mut self, reply: Reply, session: &SessionId, tab: TabId, name: &str) {
        let chord = match agent::keys::chord_of(name) {
            Ok(chord) => chord,
            Err(e) => return self.agent_fail(reply, &format!("unknown key {name:?}: {e:?}")),
        };
        let named = |host: &Host, action| host.keymap.tooltip_text(action).0;
        match agent::keys::plan(self.keymap.action_for(chord)) {
            KeyPlan::Page => match self.agent_keys(tab, name) {
                Ok(()) => self.agent_fence(reply, tab, Fence::Key, Then::Settle("pressed".into())),
                Err(e) => self.agent_fail(reply, &e),
            },
            KeyPlan::PageNotBrowser(action) => {
                let text = format!(
                    "pressed, to the page only: {name} is also wubble's \"{}\" shortcut, which \
                     does not run for an agent",
                    named(self, action)
                );
                match self.agent_keys(tab, name) {
                    Ok(()) => self.agent_fence(reply, tab, Fence::Key, Then::Settle(text)),
                    Err(e) => self.agent_fail(reply, &e),
                }
            }
            KeyPlan::Navigate(nav) => {
                let target = match nav {
                    SessionNav::Back => NavTarget::Back,
                    SessionNav::Forward => NavTarget::Forward,
                    SessionNav::Reload => NavTarget::Reload,
                };
                self.agent_navigate(reply, session, tab, target);
            }
            KeyPlan::Refused(action) => {
                let why = format!(
                    "not pressed: {name} is wubble's \"{}\" shortcut, which acts on the window, \
                     not on this session's page (open_tab, close_tab, focus_tab and navigate \
                     do what the session may)",
                    named(self, action)
                );
                self.agent_refuse(reply, &why);
            }
        }
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

    /// `screenshot` of an element: where the viewport is in the document (the clip is in
    /// document px), then the capture.
    fn agent_measure(&mut self, reply: Reply, tab: TabId, rect: spot::Rect) {
        match self
            .tab_view(tab)
            .and_then(|v| v.cdp("Page.getLayoutMetrics", &json!({})))
        {
            Some(id) => self.agent_wait(
                reply,
                Wait::Metrics {
                    id: id.0,
                    tab,
                    rect,
                },
                WAIT,
            ),
            None => self.agent_fail(reply, "the page cannot be captured now"),
        }
    }

    fn agent_capture(&mut self, reply: Reply, tab: TabId, capture: wubble_core::shot::Capture) {
        let params = wubble_core::shot::params(capture);
        match self
            .tab_view(tab)
            .and_then(|v| v.cdp("Page.captureScreenshot", &params))
        {
            Some(id) => self.agent_wait(reply, Wait::Shot { id: id.0 }, WAIT),
            None => self.agent_fail(reply, "the page cannot be captured now"),
        }
    }

    /// `evaluate` through DevTools (`Runtime.evaluate` in the page's main world, REPL mode:
    /// statements give their completion value, `await` works at the top), the promise awaited.
    fn agent_evaluate(
        &mut self,
        reply: Reply,
        tab: TabId,
        script: String,
        timeout: u32,
        stage: Evaluating,
    ) {
        // REPL mode gives statements their completion value and `await` at the top; a body
        // with `return` is a function's, whose promise plain evaluation awaits
        let params = match stage {
            Evaluating::AsBody => json!({
                "expression": format!("(async () => {{\n{script}\n}})()"),
                "awaitPromise": true,
                "objectGroup": EVAL_GROUP,
                "timeout": timeout,
            }),
            _ => json!({
                "expression": script,
                "replMode": true,
                "awaitPromise": true,
                "objectGroup": EVAL_GROUP,
                "timeout": timeout,
            }),
        };
        match self
            .tab_view(tab)
            .and_then(|v| v.cdp("Runtime.evaluate", &params))
        {
            Some(id) => {
                let wait = Wait::Eval {
                    id: id.0,
                    tab,
                    script,
                    timeout,
                    stage,
                };
                // the page's own stop comes first (DevTools' `timeout`), then the call's
                let within = Duration::from_millis(timeout.into()) + EVAL_GRACE;
                self.agent_wait(reply, wait, within);
            }
            None => self.agent_fail(reply, "the page cannot run scripts now"),
        }
    }

    /// DevTools' answer to `evaluate`'s call: a value, the error with its stack, or a note
    /// for what JSON cannot carry.
    fn agent_evaluated(&mut self, p: Pending, result: &Result<Value, String>) {
        let Wait::Eval {
            tab,
            script,
            timeout,
            stage,
            ..
        } = p.wait
        else {
            return;
        };
        let v = match result {
            Ok(v) => v,
            Err(e) => {
                self.agent_release_values(tab);
                return self.agent_fail(
                    p.reply,
                    &format!(
                        "the script did not finish ({e}): a script still running at timeout_ms \
                         ({timeout} ms) is stopped"
                    ),
                );
            }
        };
        if let Some(thrown) = v.get("exceptionDetails") {
            let what = thrown["exception"]["description"]
                .as_str()
                .or_else(|| thrown["text"].as_str())
                .unwrap_or("an exception")
                .to_owned();
            if stage == Evaluating::First && what.starts_with("SyntaxError: Illegal return") {
                return self.agent_evaluate(p.reply, tab, script, timeout, Evaluating::AsBody);
            }
            self.agent_release_values(tab);
            let at = format!(
                " (line {}, column {})",
                thrown["lineNumber"].as_i64().unwrap_or(0) + 1,
                thrown["columnNumber"].as_i64().unwrap_or(0) + 1
            );
            let text = if what.contains("\n    at ") {
                format!("threw: {what}")
            } else {
                format!("threw: {what}{at}")
            };
            return self.agent_fail(p.reply, &capped(&text, EVAL_CHARS));
        }
        let r = &v["result"];
        if stage == Evaluating::Serialising {
            self.agent_release_values(tab);
            let text = serde_json::to_string(&r["value"]).unwrap_or_else(|_| "null".into());
            return self.agent_settle(p.reply, tab, capped(&text, EVAL_CHARS));
        }
        // a value that is itself a promise (REPL mode awaits only its own): awaited too
        if r["subtype"] == "promise" && stage != Evaluating::Awaiting {
            let params = json!({ "promiseObjectId": r["objectId"] });
            return match self
                .tab_view(tab)
                .and_then(|v| v.cdp("Runtime.awaitPromise", &params))
            {
                Some(id) => {
                    let wait = Wait::Eval {
                        id: id.0,
                        tab,
                        script,
                        timeout,
                        stage: Evaluating::Awaiting,
                    };
                    let left = p.deadline.saturating_duration_since(Instant::now());
                    self.agent_wait(p.reply, wait, left)
                }
                None => self.agent_fail(p.reply, "the page cannot run scripts now"),
            };
        }
        let text = match (r["type"].as_str(), r["subtype"].as_str()) {
            (Some("undefined"), _) => Some("undefined".to_owned()),
            _ if r.get("unserializableValue").is_some() => {
                r["unserializableValue"].as_str().map(str::to_owned)
            }
            _ if r.get("value").is_some() => serde_json::to_string(&r["value"]).ok(),
            (_, Some("node")) => Some(format!(
                "[not JSON] a DOM node ({}): return what you need of it, such as .textContent, \
                 .outerHTML or .href",
                r["description"].as_str().unwrap_or("a node")
            )),
            (Some("function"), _) => Some(format!(
                "[not JSON] a function ({}): call it, or return what it returns",
                r["description"]
                    .as_str()
                    .and_then(|d| d.lines().next())
                    .unwrap_or("anonymous")
            )),
            (Some("symbol"), _) => r["description"].as_str().map(str::to_owned),
            (_, Some("error")) => r["description"].as_str().map(str::to_owned),
            _ => None,
        };
        if let Some(text) = text {
            self.agent_release_values(tab);
            return self.agent_settle(p.reply, tab, capped(&text, EVAL_CHARS));
        }
        // an object: made plain JSON in the page
        let params = json!({
            "objectId": r["objectId"],
            "functionDeclaration": SERIALISE,
            "returnByValue": true,
        });
        match self
            .tab_view(tab)
            .and_then(|v| v.cdp("Runtime.callFunctionOn", &params))
        {
            Some(id) => {
                let wait = Wait::Eval {
                    id: id.0,
                    tab,
                    script,
                    timeout,
                    stage: Evaluating::Serialising,
                };
                self.agent_wait(p.reply, wait, WAIT);
            }
            None => self.agent_fail(p.reply, "the page cannot run scripts now"),
        }
    }

    /// Lets the page drop the values `evaluate` held (its object group); the answer means
    /// nothing.
    fn agent_release_values(&mut self, tab: TabId) {
        self.agent_release_group(tab, EVAL_GROUP);
    }

    /// Lets the page drop a DevTools object group; the answer means nothing.
    fn agent_release_group(&mut self, tab: TabId, group: &str) {
        let params = json!({ "objectGroup": group });
        if let Some(id) = self
            .tab_view(tab)
            .and_then(|v| v.cdp("Runtime.releaseObjectGroup", &params))
        {
            self.agents.ignored.push(id.0);
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
        let mut reply = reply;
        reply.target = Some(match wubble_core::autofill::web_origin(&url) {
            Some(origin) => format!("{name} on {origin}"),
            None => name.to_owned(),
        });
        let Some(binding) = self
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
            let id = RecordId(binding.login.0);
            let value = match binding.part {
                agent::Part::Password => vault.reveal(&id).map(|p| Secret::new(p.as_str())).ok(),
                agent::Part::Username => vault
                    .entries()
                    .ok()
                    .and_then(|rows| rows.into_iter().find(|(m, _)| m.id == id))
                    .and_then(|(_, user)| user)
                    .map(Secret::new),
            };
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
            Some(text) if text.expose().chars().count() < MIN_SECRET => self.agent_refuse(
                p.reply,
                "the saved value is under 3 characters: too short to keep out of replies, so it \
                 is not typed",
            ),
            Some(text) => self.agent_act(p.reply, tab, &target, None, Act::Fill { text, name }),
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

    /// A web event of `tab`: the start, then the end, of a load lets `navigate` go on. A
    /// session's tab that commits an address off the web (a script's `location`, a redirect
    /// Chromium let through) is taken back to a blank page.
    pub(super) fn agent_web(&mut self, tab: TabId, event: &WebEvent) {
        let step = |load: Load| match (load, event) {
            (
                Load::Asked,
                WebEvent::LoadStarted | WebEvent::LoadingChanged { loading: true, .. },
            ) => Load::Started,
            (Load::Started, WebEvent::LoadingChanged { loading: false, .. }) => Load::Ended,
            (load, _) => load,
        };
        for p in &mut self.agents.pending {
            if let Wait::Navigated { tab: t, load, .. }
            | Wait::Started { tab: t, load, .. }
            | Wait::Input { tab: t, load, .. } = &mut p.wait
                && *t == tab
            {
                *load = step(*load);
            }
        }
        if let WebEvent::UrlChanged(url) = event
            && self.agent_drives(tab)
        {
            match self.agent_grant_of(tab) {
                // where the rule could not ask (a page from the back-forward cache, a script's
                // address Chromium let through): taken back, the agent or the log told
                Some((_, grant)) if agent::may_go(&grant, url).is_err() => {
                    eprintln!("wubble: agent: a session tab reached {url}: taken back");
                    self.agent_refused(tab, url);
                    self.agent_take_back(tab);
                }
                Some(_) => {
                    self.agents.last_at.insert(tab, url.clone());
                }
                None if !agent::is_web(url) && !agent::is_blank(url) => {
                    self.agent_backstop(tab, url);
                }
                None => {}
            }
        }
    }

    /// Whether the window shows `tab` in its pane with a frame of its own at its view's size:
    /// not the snapshot standing in for it, nor a frame of the size it had while unshown.
    pub(super) fn agent_shows(&mut self, tab: TabId) -> bool {
        if self.model.window_of(tab) != Some(self.w.id) || self.snapshots.standing_in(tab) {
            return false;
        }
        let shows = self.w.frames.get(&tab).map(|f| f.image.size());
        let wanted = self.tab_view(tab).and_then(|v| v.wanted_frame());
        match (shows, wanted) {
            (Some(s), Some(w)) => fills((s.width, s.height), (w.width, w.height)),
            _ => false,
        }
    }

    /// An action's answer once a navigation it started had [`SETTLE`] to be refused.
    fn agent_settle(&mut self, reply: Reply, tab: TabId, answer: String) {
        self.agent_wait(reply, Wait::Settle { tab, answer }, SETTLE);
    }

    /// The progress beat runs while a call waits on the owner's sheet, so a client that passed a
    /// progress token sees the call is alive (Claude Code's idle timer counts them).
    fn agent_beat_arm(&mut self) {
        let waiting = self
            .agents
            .sessions
            .values()
            .any(|s| s.start.is_some() || s.more.is_some());
        if !waiting {
            self.agents.beat.stop();
            return;
        }
        if !self.agents.beat.running() {
            self.agents
                .beat
                .start(slint::TimerMode::Repeated, BEAT, || {
                    with_host(Host::agent_beat)
                });
        }
    }

    fn agent_beat(&mut self) {
        self.agents.beats += 1;
        let step = self.agents.beats;
        let mut sent = false;
        for s in self.agents.sessions.values() {
            let (id, what) = match (&s.start, &s.more) {
                (Some(id), _) => (id, "waiting for the owner to approve the session"),
                (_, Some(more)) => (&more.id, "waiting for the owner to answer"),
                _ => continue,
            };
            sent = true;
            if let (Some(note), Some(conn)) = (
                wubble_mcp::progress(id, step, what),
                self.agents.conns.get(&s.conn),
            ) {
                conn.out.send(note);
            }
        }
        if !sent {
            self.agents.beat.stop();
        }
    }

    /// The grant of the session that drives `tab`.
    pub(super) fn agent_grant_of(&self, tab: TabId) -> Option<(SessionId, agent::Grant)> {
        let sessions: Vec<_> = self
            .agents
            .sessions
            .iter()
            .filter_map(|(id, s)| {
                let (space, container) = s.space?;
                Some((id.clone(), space, container, s.state.grant()?))
            })
            .collect();
        agent::holder(&self.model, tab, &sessions)
    }

    /// The owner's autofill filled `values` into `tab` on `origin`: in a session's tab they are
    /// the session's as a `fill_secret`'s are (scripts refused there, the values taken out of
    /// every reply).
    pub(super) fn agent_autofilled(
        &mut self,
        tab: TabId,
        origin: &str,
        values: impl Iterator<Item = Secret>,
    ) {
        let Some((session, _)) = self.agent_grant_of(tab) else {
            return;
        };
        if let Some(s) = self.agents.sessions.get_mut(&session) {
            s.tainted.filled(tab, origin);
        }
        // a value under MIN_SECRET would take ordinary text out of replies; the taint holds
        let values = values.filter(|v| v.expose().chars().count() >= MIN_SECRET);
        self.agents
            .filled
            .entry(session)
            .or_default()
            .extend(values);
    }

    /// Each view's navigation rule (`agent::may_go`), derived from the model (`agent::holder`):
    /// a running session's tabs go only where its grant says, every other tab anywhere. Run
    /// after every model change (`Host::dispatch`: a tab dragged into or out of a session's
    /// Space), a session's state change, and a view's opening; a view is told only when its
    /// grant changed.
    pub(super) fn agent_rules(&mut self) {
        // a closed tab, or one no longer a session's, has no last allowed address
        let held: Vec<TabId> = self
            .agents
            .last_at
            .keys()
            .copied()
            .filter(|t| self.agent_grant_of(*t).is_some())
            .collect();
        self.agents.last_at.retain(|t, _| held.contains(t));
        let tabs: Vec<(TabId, wubble_engine::app::ViewId)> =
            self.views.iter().map(|(t, v)| (*t, *v)).collect();
        self.agents
            .rules
            .retain(|view, _| tabs.iter().any(|(_, v)| v == view));
        for (tab, view_id) in tabs {
            let grant = self.agent_grant_of(tab).map(|(_, grant)| grant);
            if self.agents.rules.get(&view_id) == Some(&grant) {
                continue;
            }
            let rule = grant.clone().map(|grant| {
                let rule: wubble_engine::view::Rule =
                    std::sync::Arc::new(move |url: &str| agent::may_go(&grant, url).is_ok());
                rule
            });
            if let Some(view) = self.tab_view(tab) {
                view.set_navigation_rule(rule);
                self.agents.rules.insert(view_id, grant);
            }
        }
    }

    /// A session's tab was kept from going to `url` (its rule refused the navigation before it
    /// was sent, a pop-up there, or it got there and was taken back): the call in flight on the
    /// tab fails with the reason; with none, the page went on its own and the activity log says
    /// so.
    pub(super) fn agent_refused(&mut self, tab: TabId, url: &str) {
        let Some((session, _)) = self.agent_grant_of(tab) else {
            return;
        };
        if let Some(run) = &mut self.checks.mcp_smoke {
            run.navigations_refused += 1;
        }
        let note = agent::refused_note(url);
        let (on_tab, rest): (Vec<Pending>, Vec<Pending>) = std::mem::take(&mut self.agents.pending)
            .into_iter()
            .partition(|p| wait_tab(&p.wait) == Some(tab));
        self.agents.pending = rest;
        if on_tab.is_empty() {
            self.agent_log(
                &session,
                "navigation_refused",
                Some(url.to_owned()),
                Outcome::Refused,
            );
        }
        for p in on_tab {
            self.agent_fail(p.reply, &note);
        }
        self.agent_arm();
        // the tab's address is still the page it never left (a `navigate` wrote the new one)
        if let Some(at) = self.agents.last_at.get(&tab).cloned()
            && self.agent_url(tab) != at
        {
            self.dispatch(Command::UrlChanged { tab, url: at });
        }
    }

    /// A session's tab is somewhere a session may not be: back to a blank page, logged.
    fn agent_backstop(&mut self, tab: TabId, url: &str) {
        eprintln!("wubble: agent: a session tab reached {url}: blanked");
        if let Some(run) = &mut self.checks.mcp_smoke {
            run.blanked += 1;
        }
        let session = self.agents.sessions.iter().find_map(|(id, s)| {
            let (space, container) = s.space?;
            self.model.holds(space, container, tab).then(|| id.clone())
        });
        if let Some(session) = session {
            self.agent_log(
                &session,
                "left_the_web",
                Some(url.to_owned()),
                Outcome::Refused,
            );
        }
        self.agent_take_back(tab);
    }

    /// `tab` back to its last address the session's grant allowed (a blank page if none).
    fn agent_take_back(&mut self, tab: TabId) {
        let back = self
            .agents
            .last_at
            .get(&tab)
            .cloned()
            .unwrap_or(BLANK.into());
        self.dispatch(Command::Navigate { tab, url: back });
    }

    /// A page's pop-up from a session's tab: one off the web (`window.open('file:...')`,
    /// `chrome:`, `mailto:`) opens nothing (wubble would load it as the browser, which a page
    /// itself may not); whether it was refused.
    pub(super) fn agent_popup_refused(&mut self, tab: TabId, url: &str) -> bool {
        if !self.agent_drives(tab) || agent::is_blank(url) {
            return false;
        }
        // a web pop-up goes where the session's sites allow (`agent::may_go`), as the tab
        if agent::is_web(url) {
            let allowed = self
                .agent_grant_of(tab)
                .is_none_or(|(_, grant)| agent::may_go(&grant, url).is_ok());
            if !allowed {
                self.agent_refused(tab, url);
            }
            return !allowed;
        }
        let session = self.agents.sessions.iter().find_map(|(id, s)| {
            let (space, container) = s.space?;
            self.model.holds(space, container, tab).then(|| id.clone())
        });
        if let Some(session) = session {
            self.agent_log(
                &session,
                "popup_off_the_web",
                Some(url.to_owned()),
                Outcome::Refused,
            );
        }
        if let Some(run) = &mut self.checks.mcp_smoke {
            run.popups_refused += 1;
        }
        true
    }

    /// The page's answer to a script or a DevTools call a call sent; whether it was one.
    pub(super) fn agent_answer(&mut self, event: &PageEvent) -> bool {
        let id = match event {
            PageEvent::EvalResult { id, .. } => id.0,
            PageEvent::CdpReply { id, .. } => id.0,
            _ => return false,
        };
        if let Some(i) = self.agents.ignored.iter().position(|w| *w == id) {
            self.agents.ignored.swap_remove(i);
            return true;
        }
        let Some(i) = self.agents.pending.iter().position(|p| match p.wait {
            Wait::Eval { id: w, .. }
            | Wait::Article { id: w, .. }
            | Wait::Shot { id: w }
            | Wait::Scroll { id: w, .. }
            | Wait::Locate { id: w, .. }
            | Wait::Metrics { id: w, .. }
            | Wait::Focus { id: w, .. }
            | Wait::TypeFocus { id: w, .. }
            | Wait::Select { id: w, .. }
            | Wait::Input { id: w, .. } => w == id,
            _ => false,
        }) else {
            return false;
        };
        let p = self.agents.pending.remove(i);
        self.agent_arm();
        match (&p.wait, event) {
            (Wait::Eval { .. }, PageEvent::CdpReply { result, .. }) => {
                self.agent_evaluated(p, result)
            }
            (Wait::Locate { .. }, PageEvent::CdpReply { result, .. }) => {
                self.agent_located(p, result)
            }
            (Wait::Article { max, .. }, PageEvent::EvalResult { result, .. }) => {
                let max = *max;
                match result.as_deref().map(article_text) {
                    Ok(Some(text)) => {
                        let text = ToolResult::text(capped(&text, max));
                        self.agent_send(p.reply, text, Outcome::Done);
                    }
                    Ok(None) => self.agent_fail(p.reply, "the page gave no text"),
                    Err(e) => self.agent_fail(p.reply, e),
                }
            }
            (Wait::Metrics { tab, rect, .. }, PageEvent::CdpReply { result, .. }) => {
                let (tab, rect) = (*tab, *rect);
                self.agent_clip(p.reply, tab, rect, result);
            }
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
            (Wait::Scroll { .. }, PageEvent::EvalResult { result, .. }) => {
                let Wait::Scroll {
                    tab,
                    bounds,
                    viewport,
                    act,
                    ..
                } = p.wait
                else {
                    return true;
                };
                match super::ax::reach_after(bounds, viewport, result) {
                    Some(Reach::At { x, y }) => self.agent_at(p.reply, tab, (x, y), act),
                    Some(Reach::ScrollBy { .. }) => {
                        self.agent_unpress(p.reply, tab, &act, "the element stays out of view")
                    }
                    None => self.agent_unpress(p.reply, tab, &act, "the page did not scroll"),
                }
            }
            (Wait::Input { .. }, PageEvent::CdpReply { result, .. }) => {
                let Wait::Input {
                    tab,
                    asked,
                    load,
                    then,
                    ..
                } = p.wait
                else {
                    return true;
                };
                let navigated = self
                    .tab_view(tab)
                    .is_some_and(|v| v.navigations_requested() > asked);
                match result {
                    Ok(_) => self.agent_then(p.reply, tab, navigated, load, then),
                    Err(e) => {
                        let why = format!("the page did not take the input: {e}");
                        self.agent_fail(p.reply, &why);
                    }
                }
            }
            (Wait::Focus { .. }, PageEvent::EvalResult { result, .. }) => {
                let Wait::Focus {
                    tab,
                    at,
                    text,
                    name,
                    ..
                } = p.wait
                else {
                    return true;
                };
                // `P` or `T` (a password field or not), then the page's address
                let answer = result
                    .as_deref()
                    .ok()
                    .and_then(|t| serde_json::from_str::<Option<String>>(t).ok())
                    .flatten()
                    .unwrap_or_default();
                let (password, href) = match answer.split_at_checked(1) {
                    Some(("P", href)) => (true, href.to_owned()),
                    Some(("T", href)) => (false, href.to_owned()),
                    _ => (false, String::new()),
                };
                self.agent_focused(p.reply, tab, at, text, &name, &href, password);
            }
            (Wait::TypeFocus { .. }, PageEvent::EvalResult { result, .. }) => {
                let Wait::TypeFocus {
                    tab,
                    at,
                    text,
                    submit,
                    before,
                    ..
                } = p.wait
                else {
                    return true;
                };
                self.agent_type_focused(p.reply, tab, at, text, submit, before, result);
            }
            (Wait::Select { .. }, PageEvent::CdpReply { result, .. }) => {
                self.agent_selected(p, result);
            }
            (_, _) => self.agent_fail(p.reply, "an unexpected answer"),
        }
        true
    }

    /// `screenshot` of an element: its visible box (viewport px) moved by where the viewport
    /// is in the document (`Page.getLayoutMetrics`), as the capture's clip wants it
    /// (`shot::clip`).
    fn agent_clip(
        &mut self,
        reply: Reply,
        tab: TabId,
        rect: spot::Rect,
        result: &Result<Value, String>,
    ) {
        let view = result.as_ref().ok().map(|v| &v["cssVisualViewport"]);
        let num = |k: &str| view.and_then(|v| v[k].as_f64());
        let (Some(page_left), Some(page_top), Some(width), Some(height)) = (
            num("pageX"),
            num("pageY"),
            num("clientWidth"),
            num("clientHeight"),
        ) else {
            return self.agent_fail(reply, &format!("no page metrics: {result:?}"));
        };
        let metrics = wubble_core::shot::Metrics {
            page_left,
            page_top,
            width,
            height,
            element: None,
        };
        let r = wubble_core::shot::RectF {
            x: rect.x,
            y: rect.y,
            w: rect.w,
            h: rect.h,
        };
        match wubble_core::shot::clip(r, &metrics) {
            Some(clip) => self.agent_capture(reply, tab, wubble_core::shot::Capture::Clip(clip)),
            None => self.agent_fail(reply, spot::Unreachable::Hidden.note()),
        }
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
            | Wait::Url { tab, .. }
            | Wait::Moved { tab, .. }
            | Wait::Settle { tab, .. }
            | Wait::Shown { tab }
            | Wait::Started { tab, .. }
            | Wait::Tree { tab, .. } => *tab,
            Wait::Input { .. } if late => {
                self.agent_fail(p.reply, "the page did not handle the input in time");
                return None;
            }
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
        let url = self.agent_url(tab);
        let done: Option<Result<String, &str>> = match &p.wait {
            Wait::Navigated {
                before,
                load,
                answer,
                grace,
                ..
            } => {
                let ended = *load == Load::Ended;
                let fresh = document.is_some() && document != *before;
                let none_began = *load == Load::Asked && grace.as_ref().is_some_and(|g| now >= g.0);
                if ended && fresh {
                    Some(Ok(answer.clone()))
                } else if none_began {
                    grace.as_ref().map(|g| Ok(g.1.clone()))
                } else if ended && late {
                    Some(Ok(format!("{answer} (the page's tree did not change)")))
                } else {
                    None
                }
            }
            Wait::Loaded { .. } => (!loading && document.is_some()).then(|| Ok("loaded".into())),
            Wait::Settle { answer, .. } => late.then(|| Ok(answer.clone())),
            Wait::Shown { .. } => self.agent_shows(tab).then(|| Ok("focused".into())),
            Wait::Started { load, answer, .. } => {
                if *load != Load::Asked {
                    Some(Ok(answer.clone()))
                } else if late {
                    Some(Ok(format!(
                        "{answer}; the navigation it asked for did not begin"
                    )))
                } else {
                    None
                }
            }
            Wait::Opened { answer, .. } => {
                (phase == Some(Phase::Loaded)).then(|| Ok(answer.clone()))
            }
            Wait::Text {
                text, values, gone, ..
            } => page.map(|t| t.flatten(ALL_ITEMS)).and_then(|items| {
                let shows = items.iter().any(|i| {
                    i.name.contains(text.as_str()) || (*values && i.value.contains(text.as_str()))
                });
                match (shows, *gone) {
                    (true, false) => Some(Ok("found".into())),
                    (false, true) => Some(Ok("gone".into())),
                    _ => None,
                }
            }),
            Wait::Element { tab, target } => {
                self.agent_item(*tab, target).map(|_| Ok("found".into()))
            }
            Wait::Url { part, .. } => url
                .contains(part.as_str())
                .then(|| Ok(json!({ "url": url }).to_string())),
            Wait::Moved {
                url: before,
                document: was,
                ..
            } => {
                let moved = url != *before || (document.is_some() && document != *was);
                // a new document: once its load ended
                (moved && !loading).then(|| Ok(json!({ "url": url }).to_string()))
            }
            Wait::Tree { tab, within, max } => {
                let (tab, within, max) = (*tab, within.clone(), *max);
                self.agent_read(tab, within.as_deref(), max)
            }
            _ => None,
        };
        match done {
            Some(Ok(text)) => {
                self.agent_send(p.reply, ToolResult::text(text), Outcome::Done);
                None
            }
            Some(Err(why)) => {
                self.agent_fail(p.reply, why);
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
    fn agent_send(&mut self, reply: Reply, mut result: ToolResult, outcome: Outcome) {
        let session = reply.session.clone().unwrap_or(SessionId("-".into()));
        if let Some(values) = self.agents.filled.get(&session) {
            redact(&mut result, values);
        }
        self.agent_log(&session, reply.tool, reply.target.clone(), outcome);
        if let Some(conn) = self.agents.conns.get(&reply.conn) {
            conn.out.send(wubble_mcp::tool_response(&reply.id, &result));
        }
    }

    pub(super) fn agent_log(
        &mut self,
        session: &SessionId,
        tool: &str,
        target: Option<String>,
        outcome: Outcome,
    ) {
        if let Some(run) = self.checks.mcp_smoke.as_mut() {
            let text = target.as_deref().unwrap_or_default();
            if [
                crate::mcp_smoke::SIGNIN_USER,
                crate::mcp_smoke::SIGNIN_PASSWORD,
            ]
            .iter()
            .any(|v| text.contains(v))
            {
                return self.fail(format!("the activity log holds a filled value: {text}"));
            }
            run.copy_logged |= tool == "share_sign_ins" && text.starts_with("cookies for ");
        }
        let entry = Entry {
            session: session.clone(),
            at_ms: now_ms(),
            tool: tool.into(),
            target,
            outcome,
        };
        self.store.push_agent_log(entry);
        // the activity page, if it is open, shows the call at once (the store answers a read
        // after the write it follows)
        if self.shown_page() == Some(wubble_core::pages::InternalPage::AgentLog) {
            self.agent_log_shown();
        }
    }

    /// `--smoke mcp`: the sheet is up. Checks what it shows against the next row of
    /// `mcp_smoke::SHEETS`, then answers it with the pointer (and keys), as a person would
    /// (`mcp_smoke::sheet`). Each press must answer the sheet at once; the Space an approval
    /// opens must be in the mode the sheet showed (`agent_open_space`).
    pub(crate) fn mcp_smoke_answer(&mut self) {
        use crate::mcp_smoke::{Press, SHEETS, sheet};
        let Some(window) = self.agents.sheet.as_ref().and_then(|(w, _)| w.upgrade()) else {
            return self.fail("the approval sheet is not up".into());
        };
        let view = window.get_agent_sheet_view();
        let secrets: Vec<String> = view.secrets.iter().map(|s| s.key.to_string()).collect();
        let sites: Vec<String> = view.sites.iter().map(|s| s.key.to_string()).collect();
        if !view.open || view.client.is_empty() {
            return self.fail("the approval sheet names no client".into());
        }
        let Some(run) = &mut self.checks.mcp_smoke else {
            return;
        };
        let Some(want) = SHEETS.get(run.sheets) else {
            return self.fail(format!("an approval sheet more than the {}", SHEETS.len()));
        };
        // its pickers list the vault's logins once the vault answered
        if want.press == Press::ShareBindAllow
            && !matches!(self.agents.sheet_vault, SheetVault::Open(_))
        {
            run.waiting_vault = true;
            return;
        }
        run.sheets += 1;
        let n = run.sheets;
        if view.more != want.more
            || secrets != want.secrets
            || sites != want.sites
            || view.headless != want.headless
        {
            return self.fail(format!(
                "sheet {n} shows more {}, {secrets:?} on {sites:?}, headless {}",
                view.more, view.headless
            ));
        }
        let press = want.press;
        let windowed = press == Press::WindowedAllow;
        if !want.more && press != Press::Deny {
            run.expect = Some(if windowed || !want.headless {
                Mode::Windowed
            } else {
                Mode::Headless
            });
        }
        let serial = view.serial;
        let headless = view.headless;
        let more = view.more;
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
                // (the sign-ins sheet scrolls: its window choice is below the fold of the
                // smoke's window, where the query does not reach; the other sheets check it)
                if !more && press != Press::ShareBindAllow {
                    let asked = if headless { "Headless" } else { "Windowed" };
                    if sheet::chosen(&w)?.as_deref() != Some(asked) {
                        return Err(format!(
                            "the sheet's window choice shows {:?} selected, the client asked {asked} ({})",
                            sheet::chosen(&w)?,
                            sheet::tabs(&w)
                        ));
                    }
                } else if more && sheet::chosen(&w)?.is_some() {
                    return Err("a sheet for more shows a window choice".into());
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
                if press == Press::ShareBindAllow {
                    let share = "Use my sign-ins for these sites";
                    if !w.get_agent_sheet_view().may_share {
                        return Err("the sheet offers no sign-ins for its sites".into());
                    }
                    sheet::click(&w, sheet::switch(&w, share)?);
                    let v = w.get_agent_sheet_view();
                    if !v.share || !v.share_note.contains(crate::mcp_smoke::SIGNIN_HOST) {
                        return Err(format!(
                            "a press on {share:?} left it {} ({:?})",
                            v.share, v.share_note
                        ));
                    }
                    for name in ["SIGNIN_USER", "SIGNIN_PASSWORD"] {
                        with_host(|h| h.agent_sheet_bound(name, 1));
                    }
                    let v = w.get_agent_sheet_view();
                    let bound = v.bindings.iter().all(|b| {
                        b.index == 1
                            && b.options.row_data(1).is_some_and(|o| {
                                o.label.contains(crate::mcp_smoke::SIGNIN_USER)
                                    && o.label.contains(crate::mcp_smoke::SIGNIN_HOST)
                            })
                    });
                    if !bound || v.bindings.row_count() != 2 {
                        return Err("the names are not bound to the saved login".into());
                    }
                }
                if press == Press::EscapeDraftThenAllow {
                    sheet::click(&w, sheet::add_site(&w)?);
                    sheet::type_text(&w, "x.test");
                    if !sheet::drafting(&w)? {
                        return Err("the Sites editor opened no draft".into());
                    }
                    // Escape as the keyboard sends it: through the host's key path first
                    let mut taken = false;
                    with_host(|h| taken = h.mcp_smoke_escape(&w));
                    if taken {
                        return Err("Escape in the Sites draft was taken by the window (it denies the sheet)".into());
                    }
                    // what the host let through reaches the window
                    sheet::escape(&w);
                    if !shown(&w) || sheet::drafting(&w)? {
                        return Err("Escape in the Sites draft did not drop just the draft".into());
                    }
                }
                let label = if press == Press::Deny {
                    "Deny"
                } else {
                    "Allow"
                };
                let at = sheet::button(&w, label)?;
                sheet::click(&w, at);
                if shown(&w) {
                    return Err(format!(
                        "a press on {label} at {at:?} did not answer the sheet"
                    ));
                }
                println!(
                    "smoke: mcp sheet {client}{} asks {secrets:?} on {sites:?}, pressed {}{label}",
                    if more { " (more)" } else { "" },
                    match press {
                        Press::WindowedAllow => "Windowed, then ",
                        Press::EscapeDraftThenAllow =>
                            "+, typed x.test, Escape (the draft went, the sheet stayed), then ",
                        Press::ShareBindAllow =>
                            "Use my sign-ins, bound both names to the saved login, then ",
                        _ => "",
                    }
                );
                Ok(())
            };
            if let Err(why) = steps() {
                fail(why);
            }
        });
    }

    /// `--smoke mcp`: Escape pressed and released as the keyboard sends it, to the host's own
    /// key handling (which goes before the window's); whether the host took the press.
    pub(crate) fn mcp_smoke_escape(&mut self, window: &AppWindow) -> bool {
        let Some(keys) = wubble_core::keys::Chord::parse("Escape")
            .ok()
            .and_then(crate::parity::key_inputs)
        else {
            return true;
        };
        let mut taken = false;
        for key in &keys {
            let took = self.key(window, key) == slint::winit_030::EventResult::PreventDefault;
            taken |= took && key.pressed;
        }
        taken
    }

    /// `--smoke mcp`'s client finished: its step count, or what failed. The tab its first
    /// session left to the owner must be the owner's: open, in a Space no session holds, not
    /// driven, its Space allowed to sleep. Then the agent's closed tab and its loads must be
    /// none of the user's (the store answers in order, so after what the session's calls
    /// filed).
    pub(crate) fn mcp_smoke_done(&mut self, result: Result<crate::mcp_smoke::Done, String>) {
        let done = match result {
            Ok(done) => done,
            Err(e) => return self.fail(e),
        };
        if let Some(run) = &self.checks.mcp_smoke {
            let gone = run
                .signin_container
                .is_some_and(|c| !self.model.containers.iter().any(|k| k.id == c));
            let want = [wubble_engine::storage::CopyReport { taken: 2, set: 2 }];
            if run.copied != want || run.owner_kept != Some(2) || !run.copy_logged || !gone {
                return self.fail(format!(
                    "sign-ins: copies {:?} (want {want:?}), the owner keeps {:?} of 2, logged {}, \
                     the session's container gone {gone}",
                    run.copied, run.owner_kept, run.copy_logged
                ));
            }
            // the key chip on the shared site only (both have a saved login)
            let on = |host: &str| {
                run.key_chips
                    .iter()
                    .any(|o| o.contains(&format!("//{host}:")))
            };
            if !on(crate::mcp_smoke::SIGNIN_HOST) || on(crate::mcp_smoke::OTHER_HOST) {
                return self.fail(format!(
                    "sign-ins: the key chip showed in session tabs on {:?}: want {} only",
                    run.key_chips,
                    crate::mcp_smoke::SIGNIN_HOST
                ));
            }
            println!(
                "smoke: mcp sign-ins ok 2 of the owner's cookies copied (not {}), the owner \
                 keeps both, the copy logged, the stopped session's container (and its \
                 copies) gone; the key chip offered the owner's login in the session's tab on \
                 {} only",
                crate::mcp_smoke::OTHER_HOST,
                crate::mcp_smoke::SIGNIN_HOST
            );
        }
        let kept = TabId(done.kept);
        let space = self
            .model
            .spaces
            .iter()
            .find(|s| self.model.order(s.id).contains(&kept));
        // the owner's tabs now, in a Space that stays an agent Space
        let ok = space
            .is_some_and(|s| !self.agents.spaces().contains(&s.id) && !s.never_sleep && s.agent)
            && !self.agent_drives(kept);
        if !ok {
            return self.fail(format!(
                "the tab the session kept ({}) is not the owner's: Space {:?}",
                kept.0,
                space.map(|s| (&s.name, s.never_sleep))
            ));
        }
        println!(
            "smoke: mcp hand-off ok tab {} kept in {:?}, no longer driven, still an agent Space",
            kept.0,
            space.map(|s| s.name.clone()).unwrap_or_default()
        );
        let kept_space = space.map(|s| s.id);
        if let Some(run) = &mut self.checks.mcp_smoke {
            run.kept_space = kept_space;
        }
        // what the session's page tried for (`mcp_smoke::safety`): no tab anywhere reached it
        let off: Vec<String> = self
            .model
            .tabs
            .values()
            .map(|t| t.url.clone())
            .filter(|u| {
                ["file:", "chrome:", "javascript:"]
                    .iter()
                    .any(|s| u.starts_with(s))
            })
            .collect();
        if let Some(run) = &mut self.checks.mcp_smoke {
            run.steps = done.steps;
            let (refused, blanked, held) =
                (run.popups_refused, run.blanked, run.navigations_refused);
            if !off.is_empty() {
                return self.fail(format!("tabs off the web: {off:?}"));
            }
            // `mcp_smoke::sites`: a link, a script, a form, a pop-up and a redirect
            if held < 5 {
                return self.fail(format!("{held} navigations held to the sites, not 5"));
            }
            // Chromium refuses most of them itself; these count what reached wubble's own guards
            println!(
                "smoke: mcp safety ok no tab at file:, chrome: or javascript:; pop-ups refused by \
                 wubble {refused}, session tabs blanked by wubble {blanked}, navigations held to \
                 the sites {held}"
            );
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
            // shown again at full size: on to the dustbins
            return self.mcp_smoke_bins();
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
                let full = h.agent_shows(tab);
                let shows = h.w.frames.get(&tab).map(|f| f.image.size());
                let Some(run) = &mut h.checks.mcp_smoke else {
                    return;
                };
                let Some(s) = shows else {
                    return;
                };
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

    /// `--smoke mcp`, the Spaces popup's dustbins pressed by pointer: the kept Space's (no
    /// session) and the headless session's (running: only it stops); the user's Space has none;
    /// the popup stays open. Then the user's closed tabs and history.
    fn mcp_smoke_bins(&mut self) {
        let running = self
            .agents
            .sessions
            .iter()
            .find(|(_, s)| matches!(&s.state, State::Running { grant, .. } if grant.mode == Mode::Headless))
            .and_then(|(id, s)| Some((id.clone(), s.space?.0)));
        let kept = self.checks.mcp_smoke.as_ref().and_then(|r| r.kept_space);
        let (Some((session, live)), Some(kept)) = (running.clone(), kept) else {
            return self.fail(format!(
                "no Spaces to clear: running {running:?}, kept {kept:?}"
            ));
        };
        let name = |h: &Host, sp: SpaceId| {
            h.model
                .spaces
                .iter()
                .find(|s| s.id == sp)
                .map(|s| s.name.clone())
                .unwrap_or_default()
        };
        let (kept_name, live_name) = (name(self, kept), name(self, live));
        let users: Vec<String> = self
            .model
            .spaces
            .iter()
            .filter(|s| !s.agent)
            .map(|s| s.name.clone())
            .collect();
        self.spaces_menu_requested();
        let weak = self.w.window.clone();
        // pressed outside the host, as winit's events would come, once the popup is laid out
        slint::Timer::single_shot(Duration::from_millis(500), move || {
            let Some(window) = weak.upgrade() else {
                return;
            };
            let ask = |f: &dyn Fn(&Host) -> Result<(), String>| {
                let mut r = Ok(());
                with_host(|h| r = f(h));
                r
            };
            let press = |label: &str| -> Result<(), String> {
                let at = crate::mcp_smoke::sheet::labelled(&window, label)?;
                crate::mcp_smoke::sheet::click(&window, at);
                Ok(())
            };
            let check = || -> Result<(), String> {
                if !window.get_spaces_open() {
                    return Err("the Spaces popup did not open".into());
                }
                for user in &users {
                    if crate::mcp_smoke::sheet::labelled(&window, &format!("Clear {user}")).is_ok()
                    {
                        return Err(format!("the user's Space {user:?} has a dustbin"));
                    }
                }
                press(&format!("Clear {kept_name}"))?;
                ask(&|h| {
                    if h.model.spaces.iter().any(|s| s.id == kept) {
                        return Err(format!("{kept_name:?} is still there"));
                    }
                    Ok(())
                })?;
                if !window.get_spaces_open() {
                    return Err("the popup closed on the dustbin".into());
                }
                press(&format!("Clear {live_name}"))?;
                ask(&|h| {
                    if h.agents.sessions.contains_key(&session) {
                        return Err("the session runs on".into());
                    }
                    if h.model.spaces.iter().any(|s| s.id == live) {
                        return Err(format!("{live_name:?} is still there"));
                    }
                    if !h.agents.sessions.values().any(|s| !s.state.is_stopped()) {
                        return Err("the dustbin stopped the other session too".into());
                    }
                    Ok(())
                })?;
                if !window.get_spaces_open() {
                    return Err("the popup closed on the dustbin".into());
                }
                Ok(())
            };
            let result = check();
            with_host(|h| match result {
                Err(e) => h.fail(format!("dustbin: {e}")),
                Ok(()) => {
                    println!(
                        "smoke: mcp dustbin ok pressed on {kept_name:?} (kept, no session): gone; \
                         on {live_name:?} (running): that session stopped, its Space gone, the \
                         other runs on; {users:?} has none; the popup stayed open"
                    );
                    window.set_spaces_open(false);
                    if let Err(e) = h.mcp_smoke_off() {
                        return h.fail(format!("hosting off: {e}"));
                    }
                    let steps = h.checks.mcp_smoke.as_ref().map_or(0, |r| r.steps);
                    h.store.pop_closed(move |closed| {
                        let _ = slint::invoke_from_event_loop(move || {
                            with_host(|host| match closed {
                                Some(c) => {
                                    host.fail(format!("the user's closed tabs hold {}", c.url))
                                }
                                None => host.mcp_smoke_history(steps),
                            })
                        });
                    });
                }
            });
        });
    }

    /// `--smoke mcp`: agent apps in the smoke's scratch home (Codex without a file yet, opencode
    /// with comments, VS Code, which wubble does not write), then hosting turned on through its
    /// setting: wubble is in the first two, a backup beside the one that had a file; Remove and
    /// Add on Codex take it out (and keep it out) and put it back.
    pub(crate) fn mcp_smoke_on(&mut self) -> Result<(), String> {
        use wubble_core::agent::harness;
        let home = self
            .hosting
            .place
            .as_ref()
            .and_then(|p| p.home.clone())
            .ok_or("no MCP place")?;
        let io = |e: std::io::Error| e.to_string();
        std::fs::create_dir_all(home.join(".codex")).map_err(io)?;
        std::fs::create_dir_all(home.join(".config/opencode")).map_err(io)?;
        std::fs::create_dir_all(home.join(".config/Code/User")).map_err(io)?;
        let opencode = home.join(".config/opencode/opencode.jsonc");
        std::fs::write(&opencode, SMOKE_OPENCODE).map_err(io)?;
        // an entry named wubble the owner wrote: wubble never touches it
        std::fs::create_dir_all(home.join(".cursor")).map_err(io)?;
        let cursor = home.join(".cursor/mcp.json");
        std::fs::write(&cursor, SMOKE_CURSOR).map_err(io)?;
        self.write_setting(wubble_core::settings::AGENT_MCP_ENABLED.key, "1");
        if !self.mcp_serving() {
            return Err(format!("not hosting: {:?}", self.hosting.state));
        }
        // the switch turned on at run time is drawn on, from the hosting state (Settings is
        // not open here: drawn as it would be when it opens)
        if let Some(window) = self.w.window.upgrade() {
            self.render_agents(&window);
            let shown = window.get_settings_agents();
            if !shown.on || !shown.status.contains("Hosting") {
                return Err(format!("the switch shows {} ({})", shown.on, shown.status));
            }
        }
        let codex = home.join(".codex/config.toml");
        let has = |id: &str, file: &std::path::Path| {
            let h = harness::find(id).ok_or("no such app")?;
            let text = std::fs::read_to_string(file).unwrap_or_default();
            h.has(&text).map_err(|e| e.to_string())
        };
        if !has("codex", &codex)? || !has("opencode", &opencode)? {
            return Err("not added to Codex and opencode".into());
        }
        let backup = harness::backup_of(&opencode);
        if std::fs::read_to_string(&backup).ok().as_deref() != Some(SMOKE_OPENCODE) {
            return Err("no backup of opencode's file".into());
        }
        if home.join(".config/Code/User/mcp.json").exists() {
            return Err("VS Code's file was written".into());
        }
        if std::fs::read_to_string(&cursor).ok().as_deref() != Some(SMOKE_CURSOR) {
            return Err("the owner's own wubble entry in Cursor's file was touched".into());
        }
        self.agent_app_removed("codex");
        if has("codex", &codex)? {
            return Err("Remove left Codex's entry".into());
        }
        self.agent_app_added("codex");
        if !has("codex", &codex)? {
            return Err("Add did not put Codex's entry back".into());
        }
        println!(
            "smoke: mcp apps ok on: added to Codex (a new config.toml) and opencode (comments \
             kept, backup beside it), VS Code only offered, the owner's own wubble entry in \
             Cursor left alone; Remove and Add on Codex"
        );
        Ok(())
    }

    /// `--smoke mcp`: hosting turned off through its setting while a session runs: the session
    /// ends and its Space closes, the socket and the manifest go, a new connection is refused.
    fn mcp_smoke_off(&mut self) -> Result<(), String> {
        let (socket, manifest) = self.mcp_files().ok_or("no MCP place")?;
        if !socket.exists() || !manifest.exists() {
            return Err(format!("not hosting: {socket:?} {manifest:?}"));
        }
        let listed = crate::mcp::hosting_browsers()
            .iter()
            .any(|m| std::path::Path::new(&m.socket) == socket);
        if !listed {
            return Err("the manifest does not list this browser".into());
        }
        let spaces = self.agents.spaces();
        if spaces.is_empty() {
            return Err("no session runs".into());
        }
        self.write_setting(wubble_core::settings::AGENT_MCP_ENABLED.key, "0");
        if let Some(window) = self.w.window.upgrade()
            && {
                self.render_agents(&window);
                window.get_settings_agents().on
            }
        {
            return Err("the switch is drawn on after hosting stopped".into());
        }
        if let Some(run) = &mut self.checks.mcp_smoke {
            run.hosting_off = true;
        }
        if !self.agents.sessions.is_empty() {
            return Err("a session runs on".into());
        }
        if spaces
            .iter()
            .any(|sp| self.model.spaces.iter().any(|s| s.id == *sp))
        {
            return Err("a session's Space stayed open".into());
        }
        if socket.exists() || manifest.exists() {
            return Err(format!("left behind: {socket:?} {manifest:?}"));
        }
        // the agent apps: wubble's entries out, the rest of each file as it was
        if let Some(home) = self.hosting.place.as_ref().and_then(|p| p.home.clone()) {
            let opencode = std::fs::read_to_string(home.join(".config/opencode/opencode.jsonc"))
                .unwrap_or_default();
            let codex =
                std::fs::read_to_string(home.join(".codex/config.toml")).unwrap_or_default();
            if opencode != SMOKE_OPENCODE || codex.contains("wubble") {
                return Err(format!(
                    "the apps' files kept wubble: {opencode:?} {codex:?}"
                ));
            }
            let cursor = std::fs::read_to_string(home.join(".cursor/mcp.json")).unwrap_or_default();
            if cursor != SMOKE_CURSOR {
                return Err("turning off removed the owner's own wubble entry".into());
            }
        }
        use interprocess::local_socket::prelude::*;
        let name = socket
            .as_path()
            .to_fs_name::<interprocess::local_socket::GenericFilePath>()
            .map_err(|e| e.to_string())?;
        if interprocess::local_socket::Stream::connect(name).is_ok() {
            return Err("a new connection was taken".into());
        }
        println!(
            "smoke: mcp hosting ok on through its setting at run time, off through it with a \
             session running: the session ended and its Space closed, socket and manifest gone, \
             a new connection refused, wubble out of the apps' files (opencode's byte for byte)"
        );
        Ok(())
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

/// `--smoke mcp`: opencode's file as a person keeps it (a comment, an empty server table).
const SMOKE_OPENCODE: &str =
    "{\n  \"$schema\": \"https://opencode.ai/config.json\",\n  // mine\n  \"mcp\": {}\n}\n";

/// `--smoke mcp`: Cursor's file with a wubble entry the owner wrote by hand.
const SMOKE_CURSOR: &str =
    "{\"mcpServers\": {\"wubble\": {\"command\": \"/home/me/bin/wubble\", \"args\": [\"mcp\"]}}}\n";

/// `--smoke gallery`: the agent scenes' site and pages. A `.localhost` host is loopback in
/// Chromium (no DNS, nothing leaves the machine): a gallery run asks the network for nothing.
const GALLERY_SITE: &str = "github.localhost";
const GALLERY_PAGES: [&str; 3] = [
    "http://github.localhost/",
    "http://github.localhost/releases",
    "http://github.localhost/issues/new",
];

#[cfg(test)]
mod tests {
    use super::*;

    fn text(result: &ToolResult) -> &str {
        match &result.content[0] {
            wubble_mcp::Content::Text(t) => t,
            _ => panic!("not text"),
        }
    }

    #[test]
    fn a_filled_value_is_redacted_in_every_spelling_a_reply_carries() {
        for value in [
            "pa\"ss",
            "back\\slash",
            "tab\there",
            "new\nline",
            "plain-pw",
            "ünï\u{1}",
        ] {
            let secret = Secret::new(value);
            // as `read_page` prints a value (Rust's escaping), as `evaluate` returns it (JSON,
            // also a JSON string inside JSON), and as typed
            let json = serde_json::to_string(value).unwrap();
            for reply in [
                format!("- textField \"User\" value={value:?}"),
                format!("{{\"result\":{json}}}"),
                serde_json::to_string(&format!("{{\"result\":{json}}}")).unwrap(),
                format!("typed {value} here"),
            ] {
                let mut r = ToolResult::text(reply.clone());
                redact(&mut r, std::slice::from_ref(&secret));
                let out = text(&r);
                assert!(out.contains(REDACTED), "{value:?} in {reply:?}: {out}");
                for spelling in spellings(value) {
                    assert!(!out.contains(&spelling), "{value:?} left in {out}");
                }
            }
        }
        // other text is left alone
        let mut r = ToolResult::text("nothing secret");
        redact(&mut r, &[Secret::new("pa\"ss")]);
        assert_eq!(text(&r), "nothing secret");
    }

    #[test]
    fn a_frame_fills_its_pane_within_a_pixel() {
        assert!(fills((800, 600), (800, 600)));
        assert!(fills((799, 601), (800, 600)));
        assert!(!fills((798, 600), (800, 600)));
        assert!(!fills((800, 300), (800, 600)));
    }

    #[test]
    fn each_fence_is_the_devtools_input_event_that_follows_its_input() {
        let (method, params) = Fence::Pointer(12.0, 34.5).call();
        assert_eq!(method, "Input.dispatchMouseEvent");
        assert_eq!(params, json!({"type": "mouseMoved", "x": 12.0, "y": 34.5}));
        let (method, params) = Fence::Wheel(1.0, 2.0).call();
        assert_eq!(method, "Input.dispatchMouseEvent");
        assert_eq!(params["type"], "mouseWheel");
        assert_eq!(
            (params["deltaX"].as_f64(), params["deltaY"].as_f64()),
            (Some(0.0), Some(0.0))
        );
        assert_eq!(
            Fence::Key.call(),
            ("Input.dispatchKeyEvent", json!({"type": "keyUp"}))
        );
    }

    #[test]
    fn a_part_goes_only_into_its_kind_of_field() {
        assert!(field_takes(agent::Part::Password, true).is_ok());
        assert!(field_takes(agent::Part::Password, false).is_err());
        assert!(field_takes(agent::Part::Username, false).is_ok());
        assert!(field_takes(agent::Part::Username, true).is_err());
    }

    #[test]
    fn a_name_is_offered_each_part_of_each_login_its_suggestion_first() {
        let login = |id: &str| wubble_core::pages::LoginView {
            id: id.into(),
            origin: "https://a.test".into(),
            username: Some("me".into()),
            created_ms: 0,
            last_used_ms: None,
            times_used: 0,
        };
        let (a, b) = (login("a"), login("b"));
        let parts = |name| -> Vec<(String, agent::Part)> {
            binding_choices(&[&a, &b], name)
                .into_iter()
                .map(|(l, p)| (l.id.clone(), p))
                .collect()
        };
        use agent::Part::{Password, Username};
        assert_eq!(
            parts("GH_USER"),
            [
                ("a".into(), Username),
                ("a".into(), Password),
                ("b".into(), Username),
                ("b".into(), Password)
            ]
        );
        assert_eq!(parts("GH_ID")[0], ("a".into(), Password));
    }
}
