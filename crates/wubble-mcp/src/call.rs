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
    /// The session this call acts in; `None` for `StartSession`.
    pub fn session(&self) -> Option<&SessionId> {
        use AgentCall::*;
        match self {
            StartSession { .. } => None,
            StopSession { session }
            | Navigate { session, .. }
            | ReadPage { session, .. }
            | Click { session, .. }
            | Type { session, .. }
            | PressKey { session, .. }
            | Screenshot { session, .. }
            | ListTabs { session }
            | OpenTab { session, .. }
            | CloseTab { session, .. }
            | WaitFor { session, .. }
            | Evaluate { session, .. }
            | FillSecret { session, .. } => Some(session),
        }
    }

    /// The tool this call came from (its `tools/list` name).
    pub fn tool_name(&self) -> &'static str {
        use AgentCall::*;
        match self {
            StartSession { .. } => "start_session",
            StopSession { .. } => "stop_session",
            Navigate { .. } => "navigate",
            ReadPage { .. } => "read_page",
            Click { .. } => "click",
            Type { .. } => "type",
            PressKey { .. } => "press_key",
            Screenshot { .. } => "screenshot",
            ListTabs { .. } => "list_tabs",
            OpenTab { .. } => "open_tab",
            CloseTab { .. } => "close_tab",
            WaitFor { .. } => "wait_for",
            Evaluate { .. } => "evaluate",
            FillSecret { .. } => "fill_secret",
        }
    }

    /// Validates `args` against the tool table, then builds the call. The error is the text for
    /// an `InvalidParams` reply; an unknown tool name is the caller's to report first.
    pub fn parse(tool: &Tool, args: &Map<String, Value>) -> Result<AgentCall, String> {
        tool.validate(args)?;
        let a = Args(args);
        let session = || SessionId(a.str("session").unwrap_or_default());
        let tab = || TabId(a.str("tab").unwrap_or_default());
        let eref = |k| a.str(k).map(ElementRef);
        let target = || eref("ref").unwrap_or(ElementRef(String::new()));
        Ok(match tool.name {
            "start_session" => AgentCall::StartSession {
                purpose: a.str("purpose").unwrap_or_default(),
                sites: a.list("sites"),
                secrets: a.list("secrets"),
                headless: a.bool("headless"),
            },
            "stop_session" => AgentCall::StopSession { session: session() },
            "navigate" => AgentCall::Navigate {
                session: session(),
                tab: tab(),
                target: match (a.str("url"), a.str("action").as_deref()) {
                    (Some(u), _) => NavTarget::Url(u),
                    (_, Some("back")) => NavTarget::Back,
                    (_, Some("forward")) => NavTarget::Forward,
                    _ => NavTarget::Reload,
                },
            },
            "read_page" => AgentCall::ReadPage {
                session: session(),
                tab: tab(),
                within: eref("ref"),
                max_items: a.int("max_items"),
            },
            "click" => AgentCall::Click {
                session: session(),
                tab: tab(),
                target: target(),
                button: match a.str("button").as_deref() {
                    Some("right") => ButtonKind::Right,
                    Some("middle") => ButtonKind::Middle,
                    _ => ButtonKind::Left,
                },
                count: a.int("count").unwrap_or(1),
            },
            "type" => AgentCall::Type {
                session: session(),
                tab: tab(),
                target: target(),
                text: a.str("text").unwrap_or_default(),
                submit: a.bool("submit"),
            },
            "press_key" => AgentCall::PressKey {
                session: session(),
                tab: tab(),
                key: a.str("key").unwrap_or_default(),
            },
            "screenshot" => AgentCall::Screenshot {
                session: session(),
                tab: tab(),
                crop: eref("ref"),
            },
            "list_tabs" => AgentCall::ListTabs { session: session() },
            "open_tab" => AgentCall::OpenTab {
                session: session(),
                url: a.str("url"),
            },
            "close_tab" => AgentCall::CloseTab {
                session: session(),
                tab: tab(),
            },
            "wait_for" => AgentCall::WaitFor {
                session: session(),
                tab: tab(),
                condition: match (a.str("text"), eref("ref")) {
                    (Some(t), _) => WaitCondition::Text(t),
                    (_, Some(r)) => WaitCondition::Element(r),
                    _ => WaitCondition::Load,
                },
                timeout_ms: a.int("timeout_ms"),
            },
            "evaluate" => AgentCall::Evaluate {
                session: session(),
                tab: tab(),
                script: a.str("script").unwrap_or_default(),
            },
            "fill_secret" => AgentCall::FillSecret {
                session: session(),
                tab: tab(),
                target: target(),
                secret: a.str("secret").unwrap_or_default(),
            },
            other => return Err(format!("no parser for tool `{other}`")),
        })
    }
}

/// Typed reads of arguments `Tool::validate` already checked.
struct Args<'a>(&'a Map<String, Value>);

impl Args<'_> {
    fn str(&self, k: &str) -> Option<String> {
        self.0.get(k)?.as_str().map(str::to_owned)
    }
    fn int(&self, k: &str) -> Option<u32> {
        self.0
            .get(k)?
            .as_u64()
            .map(|n| u32::try_from(n).unwrap_or(u32::MAX))
    }
    fn bool(&self, k: &str) -> bool {
        self.0.get(k).and_then(Value::as_bool).unwrap_or(false)
    }
    fn list(&self, k: &str) -> Vec<String> {
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
