//! A validated `tools/call` as one value the host executes later.

use serde_json::{Map, Value};

use crate::tools::{self, Tool};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SessionId(pub String);
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct TabId(pub String);
/// An element ref from `read_page`, opaque to this crate (`e<doc>.<node>`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ElementRef(pub String);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonKind {
    Left,
    Right,
    Middle,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NavTarget {
    Url(String),
    Back,
    Forward,
    Reload,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WaitCondition {
    Text(String),
    /// No item shows the text any more.
    TextGone(String),
    Element(ElementRef),
    Load,
    /// The tab's address contains this.
    Url(String),
    /// The tab's address or document is no longer the one it had when the call came.
    Navigation,
}

/// A point inside a `<canvas>` the call names by ref, from the canvas's top-left (CSS px):
/// the only raw coordinates a session can give.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Offset {
    pub x: u32,
    pub y: u32,
}

/// Where a drag ends.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DragTo {
    /// On this element's centre.
    Element(ElementRef),
    /// At this point inside the canvas the drag starts in.
    Offset(Offset),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Up,
    Down,
    Left,
    Right,
}

/// Every call the v0 tool set can make. `session` is absent only on `StartSession`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AgentCall {
    StartSession {
        purpose: String,
        sites: Vec<String>,
        secrets: Vec<String>,
        headless: bool,
    },
    StopSession {
        session: SessionId,
        /// Leave the session's tabs to the owner instead of closing them.
        keep_tabs: bool,
    },
    RequestMore {
        session: SessionId,
        sites: Vec<String>,
        secrets: Vec<String>,
    },
    Navigate {
        session: SessionId,
        tab: TabId,
        target: NavTarget,
    },
    ReadPage {
        session: SessionId,
        tab: TabId,
        within: Option<ElementRef>,
        max_items: Option<u32>,
    },
    Click {
        session: SessionId,
        tab: TabId,
        target: ElementRef,
        at: Option<Offset>,
        button: ButtonKind,
        count: u32,
    },
    Hover {
        session: SessionId,
        tab: TabId,
        target: ElementRef,
        at: Option<Offset>,
    },
    Drag {
        session: SessionId,
        tab: TabId,
        target: ElementRef,
        at: Option<Offset>,
        to: DragTo,
    },
    Scroll {
        session: SessionId,
        tab: TabId,
        /// Into view, or (with a direction) the scroller under it.
        target: Option<ElementRef>,
        direction: Option<Direction>,
        amount: Option<u32>,
    },
    ReadText {
        session: SessionId,
        tab: TabId,
        within: Option<ElementRef>,
        max_chars: Option<u32>,
    },
    Type {
        session: SessionId,
        tab: TabId,
        target: ElementRef,
        text: String,
        submit: bool,
    },
    Select {
        session: SessionId,
        tab: TabId,
        target: ElementRef,
        option: String,
    },
    PressKey {
        session: SessionId,
        tab: TabId,
        key: String,
    },
    Screenshot {
        session: SessionId,
        tab: TabId,
        crop: Option<ElementRef>,
        full_page: bool,
    },
    ListTabs {
        session: SessionId,
    },
    FocusTab {
        session: SessionId,
        tab: TabId,
    },
    OpenTab {
        session: SessionId,
        url: Option<String>,
    },
    CloseTab {
        session: SessionId,
        tab: TabId,
    },
    WaitFor {
        session: SessionId,
        tab: TabId,
        condition: WaitCondition,
        /// Text conditions also read form fields' values.
        in_values: bool,
        timeout_ms: Option<u32>,
    },
    Evaluate {
        session: SessionId,
        tab: TabId,
        script: String,
        timeout_ms: Option<u32>,
    },
    FillSecret {
        session: SessionId,
        tab: TabId,
        target: ElementRef,
        secret: String,
    },
    UploadFile {
        session: SessionId,
        tab: TabId,
        target: ElementRef,
        paths: Vec<String>,
    },
}

impl AgentCall {
    /// The tool's name, as `tools/list` gives it.
    pub fn tool(&self) -> &'static str {
        match self {
            AgentCall::StartSession { .. } => "start_session",
            AgentCall::StopSession { .. } => "stop_session",
            AgentCall::RequestMore { .. } => "request_more",
            AgentCall::Hover { .. } => "hover",
            AgentCall::Drag { .. } => "drag",
            AgentCall::Scroll { .. } => "scroll",
            AgentCall::ReadText { .. } => "read_text",
            AgentCall::FocusTab { .. } => "focus_tab",
            AgentCall::Navigate { .. } => "navigate",
            AgentCall::ReadPage { .. } => "read_page",
            AgentCall::Click { .. } => "click",
            AgentCall::Type { .. } => "type",
            AgentCall::Select { .. } => "select",
            AgentCall::PressKey { .. } => "press_key",
            AgentCall::Screenshot { .. } => "screenshot",
            AgentCall::ListTabs { .. } => "list_tabs",
            AgentCall::OpenTab { .. } => "open_tab",
            AgentCall::CloseTab { .. } => "close_tab",
            AgentCall::WaitFor { .. } => "wait_for",
            AgentCall::Evaluate { .. } => "evaluate",
            AgentCall::FillSecret { .. } => "fill_secret",
            AgentCall::UploadFile { .. } => "upload_file",
        }
    }

    /// The session handle the call presents; `None` only for `start_session`.
    pub fn session(&self) -> Option<&SessionId> {
        match self {
            AgentCall::StartSession { .. } => None,
            AgentCall::StopSession { session, .. }
            | AgentCall::RequestMore { session, .. }
            | AgentCall::Hover { session, .. }
            | AgentCall::Drag { session, .. }
            | AgentCall::Scroll { session, .. }
            | AgentCall::ReadText { session, .. }
            | AgentCall::FocusTab { session, .. }
            | AgentCall::Navigate { session, .. }
            | AgentCall::ReadPage { session, .. }
            | AgentCall::Click { session, .. }
            | AgentCall::Type { session, .. }
            | AgentCall::Select { session, .. }
            | AgentCall::PressKey { session, .. }
            | AgentCall::Screenshot { session, .. }
            | AgentCall::ListTabs { session }
            | AgentCall::OpenTab { session, .. }
            | AgentCall::CloseTab { session, .. }
            | AgentCall::WaitFor { session, .. }
            | AgentCall::Evaluate { session, .. }
            | AgentCall::FillSecret { session, .. }
            | AgentCall::UploadFile { session, .. } => Some(session),
        }
    }

    /// The tab the call acts on, if it names one.
    pub fn tab(&self) -> Option<&TabId> {
        match self {
            AgentCall::Navigate { tab, .. }
            | AgentCall::Hover { tab, .. }
            | AgentCall::Drag { tab, .. }
            | AgentCall::Scroll { tab, .. }
            | AgentCall::ReadText { tab, .. }
            | AgentCall::FocusTab { tab, .. }
            | AgentCall::ReadPage { tab, .. }
            | AgentCall::Click { tab, .. }
            | AgentCall::Type { tab, .. }
            | AgentCall::Select { tab, .. }
            | AgentCall::PressKey { tab, .. }
            | AgentCall::Screenshot { tab, .. }
            | AgentCall::CloseTab { tab, .. }
            | AgentCall::WaitFor { tab, .. }
            | AgentCall::Evaluate { tab, .. }
            | AgentCall::FillSecret { tab, .. }
            | AgentCall::UploadFile { tab, .. } => Some(tab),
            AgentCall::StartSession { .. }
            | AgentCall::StopSession { .. }
            | AgentCall::RequestMore { .. }
            | AgentCall::ListTabs { .. }
            | AgentCall::OpenTab { .. } => None,
        }
    }

    /// Validates `args` against the tool table, then builds the call with the row's constructor.
    /// The error is the text for an `InvalidParams` reply; an unknown tool name is the caller's
    /// to report first.
    pub fn parse(tool: &Tool, args: &Map<String, Value>) -> Result<AgentCall, String> {
        tool.validate(args)?;
        (tool.build)(&Args(args))
    }
}

/// Typed reads of arguments `Tool::validate` already checked.
pub(crate) struct Args<'a>(&'a Map<String, Value>);

impl Args<'_> {
    /// A required string; validation guarantees it, so a miss is a table bug reported as an error.
    pub(crate) fn req(&self, k: &str) -> Result<String, String> {
        self.str(k).ok_or_else(|| format!("missing argument `{k}`"))
    }
    pub(crate) fn session(&self) -> Result<SessionId, String> {
        self.req("session").map(SessionId)
    }
    pub(crate) fn tab(&self) -> Result<TabId, String> {
        self.req("tab").map(TabId)
    }
    pub(crate) fn target(&self) -> Result<ElementRef, String> {
        self.req("ref").map(ElementRef)
    }
    pub(crate) fn eref(&self, k: &str) -> Option<ElementRef> {
        self.str(k).map(ElementRef)
    }
    pub(crate) fn str(&self, k: &str) -> Option<String> {
        self.0.get(k)?.as_str().map(str::to_owned)
    }
    pub(crate) fn int(&self, k: &str) -> Option<u32> {
        // Validation bounded it to u32.
        self.0.get(k)?.as_u64().and_then(|n| u32::try_from(n).ok())
    }
    pub(crate) fn bool(&self, k: &str) -> bool {
        self.0.get(k).and_then(Value::as_bool).unwrap_or(false)
    }
    /// A canvas point given as `x` and `y` (`x_name`, `y_name`): both or neither.
    pub(crate) fn offset(&self, x_name: &str, y_name: &str) -> Result<Option<Offset>, String> {
        match (self.int(x_name), self.int(y_name)) {
            (Some(x), Some(y)) => Ok(Some(Offset { x, y })),
            (None, None) => Ok(None),
            _ => Err(format!("`{x_name}` and `{y_name}` go together")),
        }
    }
    pub(crate) fn list(&self, k: &str) -> Vec<String> {
        self.0
            .get(k)
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// Parses by tool name; `None` when the table has no such tool.
pub fn parse_named(name: &str, args: &Map<String, Value>) -> Option<Result<AgentCall, String>> {
    tools::find(name).map(|t| AgentCall::parse(t, args))
}
