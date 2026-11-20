//! The tool table as data (docs/research/mcp-server.md section 0 and 4): name, description,
//! arguments, annotations. The JSON Schema served by `tools/list` and the argument validation in
//! `tools/call` are both generated from these rows, so the two cannot drift.

use serde_json::{Map, Value, json};

use crate::call::{AgentCall, Args, ButtonKind, Direction, DragTo, NavTarget, WaitCondition};

#[derive(Clone, Copy, Debug)]
pub enum Ty {
    Str,
    /// An integer in `min..=max`.
    Int(u32, u32),
    Bool,
    StrList,
    Enum(&'static [&'static str]),
}

#[derive(Clone, Copy, Debug)]
pub struct Arg {
    pub name: &'static str,
    pub ty: Ty,
    pub required: bool,
    pub doc: &'static str,
}

/// MCP tool annotations: hints for the harness's own permission UI, never enforced by wubble.
#[derive(Clone, Copy, Debug)]
pub struct Annotations {
    pub read_only: bool,
    pub destructive: bool,
    pub idempotent: bool,
    pub open_world: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct Tool {
    pub name: &'static str,
    pub description: &'static str,
    pub args: &'static [Arg],
    pub annotations: Annotations,
    /// Groups of argument names of which exactly one must be present (e.g. `navigate`'s target).
    pub one_of: &'static [&'static str],
    /// Builds the call from arguments `validate` has accepted.
    pub(crate) build: fn(&Args) -> Result<AgentCall, String>,
}

const fn arg(name: &'static str, ty: Ty, required: bool, doc: &'static str) -> Arg {
    Arg {
        name,
        ty,
        required,
        doc,
    }
}

const SESSION: Arg = arg(
    "session",
    Ty::Str,
    true,
    "Session handle from start_session.",
);
const TAB: Arg = arg(
    "tab",
    Ty::Str,
    true,
    "Tab handle from list_tabs or open_tab.",
);
const REF: Arg = arg(
    "ref",
    Ty::Str,
    true,
    "Element ref from read_page (stale after a navigation).",
);

const READ: Annotations = Annotations {
    read_only: true,
    destructive: false,
    idempotent: true,
    open_world: false,
};
const READ_WEB: Annotations = Annotations {
    open_world: true,
    ..READ
};
const ACT: Annotations = Annotations {
    read_only: false,
    destructive: true,
    idempotent: false,
    open_world: true,
};
const UI: Annotations = Annotations {
    destructive: false,
    open_world: false,
    ..ACT
};

/// A canvas point: from the canvas's top-left, CSS px.
const X: Arg = arg(
    "x",
    Ty::Int(0, 100_000),
    false,
    "Only for a <canvas> ref: a point from its box's top-left (CSS px, as getBoundingClientRect has it), with y. Else the element's centre.",
);
const Y: Arg = arg("y", Ty::Int(0, 100_000), false, "With x.");

fn button(a: &Args) -> ButtonKind {
    match a.str("button").as_deref() {
        Some("right") => ButtonKind::Right,
        Some("middle") => ButtonKind::Middle,
        _ => ButtonKind::Left,
    }
}

pub static TOOLS: &[Tool] = &[
    Tool {
        name: "start_session",
        description: "Ask the user to approve an agent session in its own Space. Blocks until answered.",
        args: &[
            arg(
                "purpose",
                Ty::Str,
                true,
                "Why the session is needed, shown to the user.",
            ),
            arg(
                "sites",
                Ty::StrList,
                false,
                "Sites the session expects to visit.",
            ),
            arg(
                "secrets",
                Ty::StrList,
                false,
                "Names of secrets to fill (never their values).",
            ),
            arg("headless", Ty::Bool, false, "Run without a visible window."),
        ],
        annotations: UI,
        one_of: &[],
        build: |a| {
            Ok(AgentCall::StartSession {
                purpose: a.req("purpose")?,
                sites: a.list("sites"),
                secrets: a.list("secrets"),
                headless: a.bool("headless"),
            })
        },
    },
    Tool {
        name: "stop_session",
        description: "End a session. Its tabs close, unless keep_tabs leaves them to the user as \
            ordinary tabs of the session's Space. Closing the connection ends the session as \
            without keep_tabs: to hand work over, call this with keep_tabs first.",
        args: &[
            SESSION,
            arg(
                "keep_tabs",
                Ty::Bool,
                false,
                "Leave the tabs open for the user (the handle dies either way).",
            ),
        ],
        annotations: Annotations {
            read_only: false,
            destructive: true,
            idempotent: true,
            open_world: false,
        },
        one_of: &[],
        build: |a| {
            Ok(AgentCall::StopSession {
                session: a.session()?,
                keep_tabs: a.bool("keep_tabs"),
            })
        },
    },
    Tool {
        name: "request_more",
        description: "Ask the user for more sites or secret names mid-session. The approval \
            sheet lists only what the session does not have yet; blocks until the user answers. \
            Nothing is granted unless the user presses Allow; on Deny the session runs on as it was.",
        args: &[
            SESSION,
            arg("sites", Ty::StrList, false, "Sites to add."),
            arg(
                "secrets",
                Ty::StrList,
                false,
                "Secret names to add (never their values).",
            ),
        ],
        annotations: UI,
        one_of: &[],
        build: |a| {
            Ok(AgentCall::RequestMore {
                session: a.session()?,
                sites: a.list("sites"),
                secrets: a.list("secrets"),
            })
        },
    },
    Tool {
        name: "navigate",
        description: "Load a URL in a tab, or go back, forward or reload; returns once the new \
            page has loaded.",
        args: &[
            SESSION,
            TAB,
            arg("url", Ty::Str, false, "http(s) URL to load."),
            arg(
                "action",
                Ty::Enum(&["back", "forward", "reload"]),
                false,
                "History action.",
            ),
        ],
        annotations: ACT,
        one_of: &["url", "action"],
        build: |a| {
            Ok(AgentCall::Navigate {
                session: a.session()?,
                tab: a.tab()?,
                target: match (a.str("url"), a.str("action").as_deref()) {
                    (Some(u), _) => NavTarget::Url(u),
                    (_, Some("back")) => NavTarget::Back,
                    (_, Some("forward")) => NavTarget::Forward,
                    _ => NavTarget::Reload,
                },
            })
        },
    },
    Tool {
        name: "read_page",
        description: "The page's accessibility tree as flat items in reading order: role, name, \
            value, state, a link's target, and a ref for the other tools. Names are given whole. \
            A cut list ends with a note of how many items there are.",
        args: &[
            SESSION,
            TAB,
            arg("ref", Ty::Str, false, "Limit to this element's subtree."),
            arg(
                "max_items",
                Ty::Int(1, 10_000),
                false,
                "Cap on returned items (default 500).",
            ),
        ],
        annotations: READ_WEB,
        one_of: &[],
        build: |a| {
            Ok(AgentCall::ReadPage {
                session: a.session()?,
                tab: a.tab()?,
                within: a.eref("ref"),
                max_items: a.int("max_items"),
            })
        },
    },
    Tool {
        name: "read_text",
        description: "The page's readable text: the article (title, byline, body, by Mozilla's \
            Readability), else the whole page's text; or one element's text by ref. Capped, with \
            a note when cut.",
        args: &[
            SESSION,
            TAB,
            arg("ref", Ty::Str, false, "Only this element's text."),
            arg(
                "max_chars",
                Ty::Int(100, 200_000),
                false,
                "Cap on returned characters (default 20000).",
            ),
        ],
        annotations: READ_WEB,
        one_of: &[],
        build: |a| {
            Ok(AgentCall::ReadText {
                session: a.session()?,
                tab: a.tab()?,
                within: a.eref("ref"),
                max_chars: a.int("max_chars"),
            })
        },
    },
    Tool {
        name: "click",
        description: "Click an element by ref: scrolled into view first, clicked at the centre \
            of its visible box.",
        args: &[
            SESSION,
            TAB,
            REF,
            arg(
                "button",
                Ty::Enum(&["left", "right", "middle"]),
                false,
                "Default left.",
            ),
            arg(
                "count",
                Ty::Int(1, 3),
                false,
                "Click count (2 for a double click). Default 1.",
            ),
            X,
            Y,
        ],
        annotations: ACT,
        one_of: &[],
        build: |a| {
            Ok(AgentCall::Click {
                session: a.session()?,
                tab: a.tab()?,
                target: a.target()?,
                at: a.offset("x", "y")?,
                button: button(a),
                count: a.int("count").unwrap_or(1),
            })
        },
    },
    Tool {
        name: "hover",
        description: "Move the pointer over an element by ref (scrolled into view first).",
        args: &[SESSION, TAB, REF, X, Y],
        annotations: Annotations {
            destructive: false,
            ..ACT
        },
        one_of: &[],
        build: |a| {
            Ok(AgentCall::Hover {
                session: a.session()?,
                tab: a.tab()?,
                target: a.target()?,
                at: a.offset("x", "y")?,
            })
        },
    },
    Tool {
        name: "drag",
        description: "Press on an element by ref, move to another element (to_ref) or, inside \
            a <canvas>, to a point of it (to_x, to_y), and release: the pointer's own events \
            (mouse down, moves, up).",
        args: &[
            SESSION,
            TAB,
            REF,
            X,
            Y,
            arg("to_ref", Ty::Str, false, "The element to release on."),
            arg(
                "to_x",
                Ty::Int(0, 100_000),
                false,
                "Only when ref is a <canvas>: where to release, from its top-left, with to_y.",
            ),
            arg("to_y", Ty::Int(0, 100_000), false, "With to_x."),
        ],
        annotations: ACT,
        one_of: &["to_ref", "to_x"],
        build: |a| {
            let to = match (a.eref("to_ref"), a.offset("to_x", "to_y")?) {
                (Some(r), _) => DragTo::Element(r),
                (None, Some(o)) => DragTo::Offset(o),
                (None, None) => return Err("`to_ref` or `to_x` with `to_y` is required".into()),
            };
            Ok(AgentCall::Drag {
                session: a.session()?,
                tab: a.tab()?,
                target: a.target()?,
                at: a.offset("x", "y")?,
                to,
            })
        },
    },
    Tool {
        name: "scroll",
        description: "Scroll an element by ref into view; or, with a direction, scroll the page \
            (or the scrollable element under ref) by amount pixels, as a mouse wheel does.",
        args: &[
            SESSION,
            TAB,
            arg(
                "ref",
                Ty::Str,
                false,
                "The element to bring into view or scroll inside.",
            ),
            arg(
                "direction",
                Ty::Enum(&["up", "down", "left", "right"]),
                false,
                "Wheel direction.",
            ),
            arg(
                "amount",
                Ty::Int(1, 100_000),
                false,
                "Pixels to scroll (default: most of the viewport).",
            ),
        ],
        annotations: Annotations {
            destructive: false,
            ..ACT
        },
        one_of: &[],
        build: |a| {
            let direction = match a.str("direction").as_deref() {
                Some("up") => Some(Direction::Up),
                Some("down") => Some(Direction::Down),
                Some("left") => Some(Direction::Left),
                Some("right") => Some(Direction::Right),
                _ => None,
            };
            let target = a.eref("ref");
            if target.is_none() && direction.is_none() {
                return Err("`ref` or `direction` is required".into());
            }
            Ok(AgentCall::Scroll {
                session: a.session()?,
                tab: a.tab()?,
                target,
                direction,
                amount: a.int("amount"),
            })
        },
    },
    Tool {
        name: "type",
        description: "Type text into an element by ref (replacing what it holds). With submit, \
            press Enter and wait as navigate does when a page load follows.",
        args: &[
            SESSION,
            TAB,
            REF,
            arg("text", Ty::Str, true, "The text to type."),
            arg("submit", Ty::Bool, false, "Press Enter afterwards."),
        ],
        annotations: ACT,
        one_of: &[],
        build: |a| {
            Ok(AgentCall::Type {
                session: a.session()?,
                tab: a.tab()?,
                target: a.target()?,
                text: a.req("text")?,
                submit: a.bool("submit"),
            })
        },
    },
    Tool {
        name: "press_key",
        description: "Send a named key (Enter, Escape, ArrowDown, Control+A) to the page. \
            wubble's back, forward and reload chords act on this tab; its other shortcuts \
            (tabs, the address bar, closed tabs) never act for an agent, and the reply says so.",
        args: &[
            SESSION,
            TAB,
            arg("key", Ty::Str, true, "Key name with optional modifiers."),
        ],
        annotations: ACT,
        one_of: &[],
        build: |a| {
            Ok(AgentCall::PressKey {
                session: a.session()?,
                tab: a.tab()?,
                key: a.req("key")?,
            })
        },
    },
    Tool {
        name: "screenshot",
        description: "A PNG of what the tab shows, of one element (scrolled into view first) or \
            of the whole page.",
        args: &[
            SESSION,
            TAB,
            arg("ref", Ty::Str, false, "Crop to this element."),
            arg(
                "full_page",
                Ty::Bool,
                false,
                "The whole page, beyond the view.",
            ),
        ],
        annotations: Annotations {
            idempotent: false,
            ..READ_WEB
        },
        one_of: &[],
        build: |a| {
            let full_page = a.bool("full_page");
            let crop = a.eref("ref");
            if full_page && crop.is_some() {
                return Err("`ref` and `full_page` exclude each other".into());
            }
            Ok(AgentCall::Screenshot {
                session: a.session()?,
                tab: a.tab()?,
                crop,
                full_page,
            })
        },
    },
    Tool {
        name: "list_tabs",
        description: "The session's tabs: handle, title, URL, loading, whether it is its \
            Space's selected tab (active) and whether the window shows it now (shown).",
        args: &[SESSION],
        annotations: READ,
        one_of: &[],
        build: |a| {
            Ok(AgentCall::ListTabs {
                session: a.session()?,
            })
        },
    },
    Tool {
        name: "focus_tab",
        description: "Bring a session tab forward: the window shows its Space and selects it.",
        args: &[SESSION, TAB],
        annotations: UI,
        one_of: &[],
        build: |a| {
            Ok(AgentCall::FocusTab {
                session: a.session()?,
                tab: a.tab()?,
            })
        },
    },
    Tool {
        name: "open_tab",
        description: "Open a tab in the session's Space; with a URL, returns once it has loaded.",
        args: &[
            SESSION,
            arg("url", Ty::Str, false, "http(s) URL; blank when omitted."),
        ],
        annotations: ACT,
        one_of: &[],
        build: |a| {
            Ok(AgentCall::OpenTab {
                session: a.session()?,
                url: a.str("url"),
            })
        },
    },
    Tool {
        name: "close_tab",
        description: "Close a tab of the session.",
        args: &[SESSION, TAB],
        annotations: Annotations {
            open_world: false,
            ..ACT
        },
        one_of: &[],
        build: |a| {
            Ok(AgentCall::CloseTab {
                session: a.session()?,
                tab: a.tab()?,
            })
        },
    },
    Tool {
        name: "wait_for",
        description: "Wait until text appears or is gone, an element exists, the page has \
            loaded, or the address changes (url, navigation; in-page history changes count). \
            Text is matched in what the page shows, not in form fields' values unless in_values. \
            load waits for the document's own load only: a page that fetches after an in-page \
            navigation is not followed, so wait for its text or url instead.",
        args: &[
            SESSION,
            TAB,
            arg("text", Ty::Str, false, "Text to wait for."),
            arg("text_gone", Ty::Str, false, "Text to wait to disappear."),
            arg("ref", Ty::Str, false, "Element ref to wait for."),
            arg(
                "load",
                Ty::Bool,
                false,
                "Wait for the load to finish (true).",
            ),
            arg(
                "url",
                Ty::Str,
                false,
                "Wait until the tab's address contains this.",
            ),
            arg(
                "navigation",
                Ty::Bool,
                false,
                "Wait until the address or the document changes (true).",
            ),
            arg(
                "in_values",
                Ty::Bool,
                false,
                "Also match text in form fields' values.",
            ),
            arg(
                "timeout_ms",
                Ty::Int(1, 120_000),
                false,
                "Give up after this long (default 30000).",
            ),
        ],
        annotations: READ,
        one_of: &["text", "text_gone", "ref", "load", "url", "navigation"],
        build: |a| {
            Ok(AgentCall::WaitFor {
                session: a.session()?,
                tab: a.tab()?,
                condition: if let Some(t) = a.str("text") {
                    WaitCondition::Text(t)
                } else if let Some(t) = a.str("text_gone") {
                    WaitCondition::TextGone(t)
                } else if let Some(r) = a.eref("ref") {
                    WaitCondition::Element(r)
                } else if let Some(u) = a.str("url") {
                    WaitCondition::Url(u)
                } else if a.bool("navigation") {
                    WaitCondition::Navigation
                } else {
                    WaitCondition::Load
                },
                in_values: a.bool("in_values"),
                timeout_ms: a.int("timeout_ms"),
            })
        },
    },
    Tool {
        name: "evaluate",
        description: "Run JavaScript in the page: an expression or statements (the last \
            statement's value is returned; a top-level return or await works). A promise is \
            awaited. Returns JSON; a DOM node or a function is named, not serialised; a throw \
            returns the error with its stack.",
        args: &[
            SESSION,
            TAB,
            arg(
                "script",
                Ty::Str,
                true,
                "JavaScript expression or statements.",
            ),
            arg(
                "timeout_ms",
                Ty::Int(1, 120_000),
                false,
                "Stop the script and stop waiting for its promise after this long (default 30000).",
            ),
        ],
        annotations: ACT,
        one_of: &[],
        build: |a| {
            Ok(AgentCall::Evaluate {
                session: a.session()?,
                tab: a.tab()?,
                script: a.req("script")?,
                timeout_ms: a.int("timeout_ms"),
            })
        },
    },
    Tool {
        name: "fill_secret",
        description: "Fill a field with a secret the user approved, by name. The value never reaches the agent.",
        args: &[
            SESSION,
            TAB,
            REF,
            arg("secret", Ty::Str, true, "Secret variable name."),
        ],
        annotations: ACT,
        one_of: &[],
        build: |a| {
            Ok(AgentCall::FillSecret {
                session: a.session()?,
                tab: a.tab()?,
                target: a.target()?,
                secret: a.req("secret")?,
            })
        },
    },
];

pub fn find(name: &str) -> Option<&'static Tool> {
    TOOLS.iter().find(|t| t.name == name)
}

impl Ty {
    fn schema(self, doc: &str) -> Value {
        let mut m = match self {
            Ty::Str => json!({"type": "string"}),
            Ty::Int(lo, hi) => json!({"type": "integer", "minimum": lo, "maximum": hi}),
            Ty::Bool => json!({"type": "boolean"}),
            Ty::StrList => json!({"type": "array", "items": {"type": "string"}}),
            Ty::Enum(vs) => json!({"type": "string", "enum": vs}),
        };
        m["description"] = doc.into();
        m
    }

    fn accepts(self, v: &Value) -> bool {
        match self {
            Ty::Str => v.is_string(),
            Ty::Int(lo, hi) => v
                .as_u64()
                .is_some_and(|n| (u64::from(lo)..=u64::from(hi)).contains(&n)),
            Ty::Bool => v.is_boolean(),
            Ty::StrList => v.as_array().is_some_and(|a| a.iter().all(Value::is_string)),
            Ty::Enum(vs) => v.as_str().is_some_and(|s| vs.contains(&s)),
        }
    }
}

impl Tool {
    pub fn input_schema(&self) -> Value {
        let props: Map<String, Value> = self
            .args
            .iter()
            .map(|a| (a.name.to_string(), a.ty.schema(a.doc)))
            .collect();
        let required: Vec<&str> = self
            .args
            .iter()
            .filter(|a| a.required)
            .map(|a| a.name)
            .collect();
        let mut s = json!({
            "type": "object",
            "properties": props,
            "required": required,
            "additionalProperties": false,
        });
        if !self.one_of.is_empty() {
            let alts: Vec<Value> = self
                .one_of
                .iter()
                .map(|n| json!({"required": [n]}))
                .collect();
            s["oneOf"] = alts.into();
        }
        s
    }

    /// The `tools/list` entry.
    pub fn listing(&self) -> Value {
        let a = &self.annotations;
        json!({
            "name": self.name,
            "description": self.description,
            "inputSchema": self.input_schema(),
            "annotations": {
                "readOnlyHint": a.read_only,
                "destructiveHint": a.destructive,
                "idempotentHint": a.idempotent,
                "openWorldHint": a.open_world,
            },
        })
    }

    /// Checks `args` against the row: unknown names, missing required ones, wrong types, and the
    /// exactly-one groups. The error is the text for an `InvalidParams` reply.
    pub fn validate(&self, args: &Map<String, Value>) -> Result<(), String> {
        if let Some(k) = args
            .keys()
            .find(|k| !self.args.iter().any(|a| a.name == k.as_str()))
        {
            return Err(format!("unknown argument `{k}`"));
        }
        for a in self.args {
            match args.get(a.name) {
                None if a.required => return Err(format!("missing argument `{}`", a.name)),
                Some(v) if !a.ty.accepts(v) => {
                    return Err(format!("argument `{}` has the wrong type or value", a.name));
                }
                _ => {}
            }
        }
        if !self.one_of.is_empty() {
            let n = self
                .one_of
                .iter()
                .filter(|k| args.get(**k).is_some_and(|v| *v != Value::Bool(false)))
                .count();
            if n != 1 {
                return Err(format!(
                    "exactly one of {} is required (a boolean counts only when true)",
                    self.one_of.join(", ")
                ));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_unique_and_the_v0_set_is_complete() {
        let mut names: Vec<_> = TOOLS.iter().map(|t| t.name).collect();
        names.sort_unstable();
        let before = names.len();
        names.dedup();
        assert_eq!(before, names.len());
        for n in [
            "start_session",
            "stop_session",
            "navigate",
            "read_page",
            "click",
            "type",
            "press_key",
            "screenshot",
            "list_tabs",
            "open_tab",
            "close_tab",
            "wait_for",
            "evaluate",
            "fill_secret",
            "request_more",
            "read_text",
            "focus_tab",
            "scroll",
            "hover",
            "drag",
        ] {
            assert!(find(n).is_some(), "{n}");
        }
        assert_eq!(TOOLS.len(), 20);
    }

    #[test]
    fn every_schema_is_well_formed() {
        for t in TOOLS {
            let l = t.listing();
            let s = &l["inputSchema"];
            assert_eq!(s["type"], "object");
            let props = s["properties"].as_object().unwrap();
            for r in s["required"].as_array().unwrap() {
                assert!(props.contains_key(r.as_str().unwrap()), "{}", t.name);
            }
            assert!(l["annotations"]["readOnlyHint"].is_boolean());
        }
    }

    #[test]
    fn validate_cases() {
        let nav = find("navigate").unwrap();
        let ok = json!({"session": "s", "tab": "t", "url": "https://a.test"});
        assert!(nav.validate(ok.as_object().unwrap()).is_ok());
        let both = json!({"session": "s", "tab": "t", "url": "u", "action": "back"});
        assert!(nav.validate(both.as_object().unwrap()).is_err());
        let none = json!({"session": "s", "tab": "t"});
        assert!(nav.validate(none.as_object().unwrap()).is_err());
        let bad = json!({"session": "s", "tab": "t", "action": "sideways"});
        assert!(nav.validate(bad.as_object().unwrap()).is_err());
        let extra = json!({"session": "s", "tab": "t", "url": "u", "x": 1});
        assert!(
            nav.validate(extra.as_object().unwrap())
                .unwrap_err()
                .contains("unknown")
        );
        let neg = json!({"session": "s", "tab": "t", "ref": "e1.2", "count": -1});
        assert!(
            find("click")
                .unwrap()
                .validate(neg.as_object().unwrap())
                .is_err()
        );
    }
}
