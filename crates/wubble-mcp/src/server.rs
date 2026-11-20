//! JSON-RPC 2.0 for MCP 2026-07-28 (stateless: no `initialize`, no session id). A line becomes an
//! [`Incoming`]; the host answers a `Call` later with [`tool_response`].

use serde_json::{Map, Value, json};

use crate::call::{AgentCall, parse_named};
use crate::tools::TOOLS;

pub const PROTOCOL_VERSION: &str = "2026-07-28";
/// `tools/list` is fixed by the server entry, so a long cache is right.
pub const LIST_TTL_MS: u64 = 3_600_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RequestId {
    Num(i64),
    Str(String),
}

impl RequestId {
    fn to_json(&self) -> Value {
        match self {
            RequestId::Num(n) => json!(n),
            RequestId::Str(s) => json!(s),
        }
    }
}

/// A typed JSON-RPC error: the code is the wire code.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RpcError {
    Parse,
    InvalidRequest(String),
    MethodNotFound(String),
    InvalidParams(String),
}

impl RpcError {
    pub fn code(&self) -> i64 {
        match self {
            RpcError::Parse => -32700,
            RpcError::InvalidRequest(_) => -32600,
            RpcError::MethodNotFound(_) => -32601,
            RpcError::InvalidParams(_) => -32602,
        }
    }

    pub fn message(&self) -> String {
        match self {
            RpcError::Parse => "parse error".into(),
            RpcError::InvalidRequest(m) | RpcError::InvalidParams(m) => m.clone(),
            RpcError::MethodNotFound(m) => format!("method not found: {m}"),
        }
    }

    /// The error response; `id` is null when the request's id could not be read.
    pub fn response(&self, id: Option<&RequestId>) -> Value {
        json!({
            "jsonrpc": "2.0",
            "id": id.map_or(Value::Null, RequestId::to_json),
            "error": {"code": self.code(), "message": self.message()},
        })
    }
}

/// What the host does with one inbound line.
#[derive(Debug, PartialEq)]
pub enum Incoming {
    /// Send this response now (discovery, listing, or an error).
    Reply(Value),
    /// Execute the call, then send `tool_response(&id, result)`.
    Call { id: RequestId, call: AgentCall },
    /// A notification or a response: nothing to send.
    Ignore,
}

/// Content of a tool result, serialised by [`tool_response`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Content {
    Text(String),
    /// Base64 data and its media type (the screenshot's PNG).
    Image {
        data: String,
        mime: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolResult {
    pub content: Vec<Content>,
    /// A failure the agent can read and react to (stale ref, session paused), not a protocol error.
    pub is_error: bool,
}

impl ToolResult {
    pub fn text(s: impl Into<String>) -> Self {
        Self {
            content: vec![Content::Text(s.into())],
            is_error: false,
        }
    }
    pub fn error(s: impl Into<String>) -> Self {
        Self {
            content: vec![Content::Text(s.into())],
            is_error: true,
        }
    }
    pub fn image(data: impl Into<String>, mime: impl Into<String>) -> Self {
        Self {
            content: vec![Content::Image {
                data: data.into(),
                mime: mime.into(),
            }],
            is_error: false,
        }
    }

    fn to_json(&self) -> Value {
        let content: Vec<Value> = self
            .content
            .iter()
            .map(|c| match c {
                Content::Text(t) => json!({"type": "text", "text": t}),
                Content::Image { data, mime } => {
                    json!({"type": "image", "data": data, "mimeType": mime})
                }
            })
            .collect();
        json!({"content": content, "isError": self.is_error})
    }
}

pub fn tool_response(id: &RequestId, result: &ToolResult) -> Value {
    json!({"jsonrpc": "2.0", "id": id.to_json(), "result": result.to_json()})
}

fn ok(id: &RequestId, result: Value) -> Incoming {
    Incoming::Reply(json!({"jsonrpc": "2.0", "id": id.to_json(), "result": result}))
}

fn discover() -> Value {
    json!({
        "supportedVersions": [PROTOCOL_VERSION],
        "capabilities": {"tools": {}},
        "serverInfo": {"name": "wubble", "version": env!("CARGO_PKG_VERSION")},
    })
}

fn list() -> Value {
    let tools: Vec<Value> = TOOLS.iter().map(|t| t.listing()).collect();
    json!({"tools": tools, "ttlMs": LIST_TTL_MS, "cacheScope": "public"})
}

fn read_id(v: &Value) -> Result<Option<RequestId>, RpcError> {
    match v.get("id") {
        None => Ok(None),
        Some(Value::String(s)) => Ok(Some(RequestId::Str(s.clone()))),
        Some(n) if n.is_i64() => Ok(n.as_i64().map(RequestId::Num)),
        Some(_) => Err(RpcError::InvalidRequest(
            "id must be a string or an integer".into(),
        )),
    }
}

/// Parses one line and answers what needs no host: discovery, the tool list, protocol errors.
pub fn handle_line(line: &str) -> Incoming {
    let Ok(v) = serde_json::from_str::<Value>(line) else {
        return Incoming::Reply(RpcError::Parse.response(None));
    };
    let id = match read_id(&v) {
        Ok(id) => id,
        Err(e) => return Incoming::Reply(e.response(None)),
    };
    let fail = |e: RpcError| Incoming::Reply(e.response(id.as_ref()));
    if !v.is_object() || v.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return fail(RpcError::InvalidRequest(
            "not a JSON-RPC 2.0 message".into(),
        ));
    }
    let Some(method) = v.get("method").and_then(Value::as_str) else {
        // A response to something we never asked, or garbage with an id.
        return if id.is_some() && v.get("result").is_none() && v.get("error").is_none() {
            fail(RpcError::InvalidRequest("missing method".into()))
        } else {
            Incoming::Ignore
        };
    };
    let Some(id) = id else {
        return Incoming::Ignore;
    };
    let fail = |e: RpcError| Incoming::Reply(e.response(Some(&id)));
    match method {
        "server/discover" => ok(&id, discover()),
        "tools/list" => ok(&id, list()),
        "tools/call" => {
            let params = v.get("params");
            let Some(name) = params.and_then(|p| p.get("name")).and_then(Value::as_str) else {
                return fail(RpcError::InvalidParams("missing tool name".into()));
            };
            let empty = Map::new();
            let args = match params.and_then(|p| p.get("arguments")) {
                None | Some(Value::Null) => &empty,
                Some(Value::Object(m)) => m,
                Some(_) => {
                    return fail(RpcError::InvalidParams(
                        "arguments must be an object".into(),
                    ));
                }
            };
            match parse_named(name, args) {
                None => fail(RpcError::InvalidParams(format!("unknown tool `{name}`"))),
                Some(Err(m)) => fail(RpcError::InvalidParams(m)),
                Some(Ok(call)) => Incoming::Call { id, call },
            }
        }
        other => fail(RpcError::MethodNotFound(other.into())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::call::{ButtonKind, NavTarget, SessionId, TabId, WaitCondition};

    fn reply(line: &str) -> Value {
        match handle_line(line) {
            Incoming::Reply(v) => v,
            other => panic!("{other:?}"),
        }
    }

    fn call(name: &str, args: Value) -> Incoming {
        let req = json!({"jsonrpc": "2.0", "id": 7, "method": "tools/call",
            "params": {"name": name, "arguments": args}});
        handle_line(&req.to_string())
    }

    fn parsed(name: &str, args: Value) -> AgentCall {
        match call(name, args) {
            Incoming::Call { id, call } => {
                assert_eq!(id, RequestId::Num(7));
                call
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn discover_and_list() {
        let d = reply(r#"{"jsonrpc":"2.0","id":"a","method":"server/discover"}"#);
        assert_eq!(d["id"], "a");
        assert_eq!(d["result"]["supportedVersions"][0], "2026-07-28");
        let l = reply(r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#);
        assert_eq!(l["result"]["tools"].as_array().unwrap().len(), TOOLS.len());
        assert_eq!(l["result"]["ttlMs"], LIST_TTL_MS);
        assert_eq!(l["result"]["cacheScope"], "public");
    }

    /// Every tool: build arguments from its own schema's required fields and parse them.
    #[test]
    fn every_tool_round_trips_through_its_schema() {
        for t in TOOLS {
            let mut args = Map::new();
            for a in t.args.iter().filter(|a| a.required) {
                let v = match a.ty {
                    crate::tools::Ty::Str => json!("x"),
                    crate::tools::Ty::Int => json!(1),
                    crate::tools::Ty::Bool => json!(true),
                    crate::tools::Ty::StrList => json!(["x"]),
                    crate::tools::Ty::Enum(vs) => json!(vs[0]),
                };
                args.insert(a.name.into(), v);
            }
            // The first of an exactly-one group is a string argument in every row.
            if let Some(first) = t.one_of.first() {
                args.insert((*first).into(), json!("x"));
            }
            match call(t.name, Value::Object(args)) {
                Incoming::Call { call, .. } => assert_eq!(call.tool_name(), t.name),
                other => panic!("{}: {other:?}", t.name),
            }
        }
    }

    #[test]
    fn parsed_values() {
        assert_eq!(
            parsed(
                "click",
                json!({"session": "s", "tab": "t", "ref": "e1.4", "button": "right", "count": 2})
            ),
            AgentCall::Click {
                session: SessionId("s".into()),
                tab: TabId("t".into()),
                target: crate::call::ElementRef("e1.4".into()),
                button: ButtonKind::Right,
                count: 2,
            }
        );
        assert!(matches!(
            parsed(
                "navigate",
                json!({"session": "s", "tab": "t", "action": "reload"})
            ),
            AgentCall::Navigate {
                target: NavTarget::Reload,
                ..
            }
        ));
        assert!(matches!(
            parsed(
                "wait_for",
                json!({"session": "s", "tab": "t", "load": true})
            ),
            AgentCall::WaitFor {
                condition: WaitCondition::Load,
                timeout_ms: None,
                ..
            }
        ));
        let c = parsed("start_session", json!({"purpose": "p", "secrets": ["GH"]}));
        assert_eq!(c.session(), None);
        assert!(matches!(
            c,
            AgentCall::StartSession {
                headless: false,
                ..
            }
        ));
    }

    #[test]
    fn typed_errors() {
        assert_eq!(reply("{nope")["error"]["code"], -32700);
        assert_eq!(reply("{\"id\":1}")["error"]["code"], -32600);
        assert_eq!(
            reply(r#"{"jsonrpc":"2.0","id":[1],"method":"x"}"#)["error"]["code"],
            -32600
        );
        let m = reply(r#"{"jsonrpc":"2.0","id":2,"method":"initialize"}"#);
        assert_eq!(m["error"]["code"], -32601);
        assert_eq!(m["id"], 2);
        let unknown = match call("rm_rf", json!({})) {
            Incoming::Reply(v) => v,
            o => panic!("{o:?}"),
        };
        assert_eq!(unknown["error"]["code"], -32602);
        for bad in [
            json!({"session": "s"}),
            json!({"session": 3, "tab": "t", "key": "k"}),
        ] {
            match call("press_key", bad) {
                Incoming::Reply(v) => assert_eq!(v["error"]["code"], -32602),
                o => panic!("{o:?}"),
            }
        }
        let no_name = reply(r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{}}"#);
        assert_eq!(no_name["error"]["code"], -32602);
        let bad_args = reply(
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"list_tabs","arguments":[]}}"#,
        );
        assert_eq!(bad_args["error"]["code"], -32602);
    }

    #[test]
    fn notifications_and_responses_are_ignored() {
        assert_eq!(
            handle_line(r#"{"jsonrpc":"2.0","method":"notifications/cancelled"}"#),
            Incoming::Ignore
        );
        assert_eq!(
            handle_line(r#"{"jsonrpc":"2.0","id":1,"result":{}}"#),
            Incoming::Ignore
        );
    }

    #[test]
    fn tool_results_serialise() {
        let id = RequestId::Str("q".into());
        let r = tool_response(&id, &ToolResult::error("stale ref: take a new snapshot"));
        assert_eq!(r["id"], "q");
        assert_eq!(r["result"]["isError"], true);
        assert_eq!(r["result"]["content"][0]["type"], "text");
        let img = tool_response(&id, &ToolResult::image("AAAA", "image/png"));
        assert_eq!(img["result"]["content"][0]["mimeType"], "image/png");
        assert_eq!(img["result"]["isError"], false);
    }
}
