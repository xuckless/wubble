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
    Element(ElementRef),
    Load,
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
        button: ButtonKind,
        count: u32,
    },
    Type {
        session: SessionId,
        tab: TabId,
        target: ElementRef,
        text: String,
        submit: bool,
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
    },
    ListTabs {
        session: SessionId,
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
        timeout_ms: Option<u32>,
    },
    Evaluate {
        session: SessionId,
        tab: TabId,
        script: String,
    },
    FillSecret {
        session: SessionId,
        tab: TabId,
        target: ElementRef,
        secret: String,
    },
}

impl AgentCall {
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
