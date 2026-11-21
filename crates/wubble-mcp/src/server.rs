//! JSON-RPC 2.0 for MCP, dual era (2026-07-28 `basic/versioning.mdx`: "A server that wishes to
//! support both legacy clients ... MAY implement both behaviors"): a client that opens with
//! `server/discover` or a modern request is served statelessly as 2026-07-28; one that opens with
//! `initialize` (the TypeScript SDK's clients, at 2025-11-25) is served as the revision it
//! negotiated, for the rest of the connection ([`Peer`]). A line becomes an [`Incoming`]; the host
//! answers a `Call` later with [`tool_response`], and may send [`progress`] while it waits.

use serde_json::{Map, Value, json};

use crate::call::{AgentCall, parse_named};
use crate::tools::TOOLS;

pub const PROTOCOL_VERSION: &str = "2026-07-28";
/// The earlier revisions served through `initialize`, newest first: their `initialize`,
/// `tools/list` and `tools/call` shapes are the same for what wubble uses (2025-11-25
/// `schema.ts`; 2025-06-18 has no `tasks`, which wubble does not offer).
pub const LEGACY_VERSIONS: &[&str] = &["2025-11-25", "2025-06-18", "2025-03-26"];

/// What the server tells a client how to use it (`InitializeResult.instructions`; the tool
/// descriptions say the same).
pub const INSTRUCTIONS: &str = "wubble is a web browser on this computer. Call start_session first: \
    it asks the browser's owner to approve the session on screen, so it may block for minutes \
    until they answer (progress notifications are sent meanwhile if you pass a progressToken). \
    It returns a session handle and the handle of a first tab; every other tool takes the \
    session handle (and a tab handle where it acts on a page). The session's tabs stay on the \
    sites it was granted; ask for more with request_more (the owner approves again). End with \
    stop_session (keep_tabs to leave the tabs to the owner).";

/// The protocol a connection is served by.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Era {
    /// 2026-07-28: stateless, every result says `resultType`.
    #[default]
    Modern,
    /// Through `initialize`, at this revision.
    Legacy(String),
}

/// One connection's protocol state: which era it opened in.
#[derive(Clone, Debug, Default)]
pub struct Peer {
    era: Option<Era>,
}

/// A `tools/call` as the host answers it: its id, the era the answer is shaped for, and the
/// client's progress token, if it asked for progress.
#[derive(Clone, Debug, PartialEq)]
pub struct CallId {
    pub id: RequestId,
    pub era: Era,
    pub progress: Option<Value>,
}
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
    /// The server cannot answer at all (no browser runs): JSON-RPC's implementation-defined
    /// server error.
    Unavailable(String),
}

impl RpcError {
    pub fn code(&self) -> i64 {
        match self {
            RpcError::Parse => -32700,
            RpcError::InvalidRequest(_) => -32600,
            RpcError::MethodNotFound(_) => -32601,
            RpcError::InvalidParams(_) => -32602,
            RpcError::Unavailable(_) => -32000,
        }
    }

    pub fn message(&self) -> String {
        match self {
            RpcError::Parse => "parse error".into(),
            RpcError::InvalidRequest(m) | RpcError::InvalidParams(m) | RpcError::Unavailable(m) => {
                m.clone()
            }
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
    Call { id: CallId, call: AgentCall },
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

pub fn tool_response(id: &CallId, result: &ToolResult) -> Value {
    json!({"jsonrpc": "2.0", "id": id.id.to_json(), "result": shaped(result.to_json(), &id.era)})
}

/// `notifications/progress` for a call whose client passed a progress token (every revision
/// has it): `step` must grow from one notification to the next. `None` without a token.
pub fn progress(id: &CallId, step: u64, message: &str) -> Option<Value> {
    let token = id.progress.clone()?;
    Some(json!({
        "jsonrpc": "2.0",
        "method": "notifications/progress",
        "params": {"progressToken": token, "progress": step, "message": message},
    }))
}

/// A result as the era shapes it: 2026-07-28 says `resultType` on every result.
fn shaped(mut result: Value, era: &Era) -> Value {
    if *era == Era::Modern
        && let Some(m) = result.as_object_mut()
    {
        m.insert("resultType".into(), json!("complete"));
    }
    result
}

fn ok(id: &RequestId, result: Value) -> Incoming {
    Incoming::Reply(json!({"jsonrpc": "2.0", "id": id.to_json(), "result": result}))
}

fn initialized(asked: Option<&str>) -> (String, Value) {
    // the client's revision when served, else the newest earlier one (the client decides)
    let version = asked
        .filter(|v| LEGACY_VERSIONS.contains(v))
        .unwrap_or(LEGACY_VERSIONS[0])
        .to_owned();
    let result = json!({
        "protocolVersion": version,
        "capabilities": {"tools": {}},
        "serverInfo": {"name": "wubble", "version": env!("CARGO_PKG_VERSION")},
        "instructions": INSTRUCTIONS,
    });
    (version, result)
}

fn discover() -> Value {
    json!({
        "supportedVersions": [PROTOCOL_VERSION],
        "capabilities": {"tools": {}},
        "serverInfo": {"name": "wubble", "version": env!("CARGO_PKG_VERSION")},
        "instructions": INSTRUCTIONS,
        "resultType": "complete",
    })
}

fn list(era: &Era) -> Value {
    let tools: Vec<Value> = TOOLS.iter().map(|t| t.listing()).collect();
    match era {
        Era::Modern => json!({
            "tools": tools, "ttlMs": LIST_TTL_MS, "cacheScope": "public", "resultType": "complete",
        }),
        Era::Legacy(_) => json!({"tools": tools}),
    }
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

/// The answer to `line` when no server is there to take it (`wubble mcp` with no browser
/// running): every request gets [`RpcError::Unavailable`] with `why`; notifications and
/// responses get nothing.
pub fn unavailable(line: &str, why: &str) -> Option<Value> {
    let v: Value = serde_json::from_str(line).ok()?;
    v.get("method")?;
    let id = read_id(&v).ok()??;
    Some(RpcError::Unavailable(why.into()).response(Some(&id)))
}

/// One line of a fresh connection, served as 2026-07-28 unless it is `initialize`
/// ([`Peer::handle`]).
pub fn handle_line(line: &str) -> Incoming {
    Peer::default().handle(line)
}

impl Peer {
    /// The era this connection is served in (2026-07-28 until it opens with `initialize`).
    pub fn era(&self) -> Era {
        self.era.clone().unwrap_or_default()
    }

    /// Parses one line and answers what needs no host: the handshake of either era, the tool
    /// list, protocol errors.
    pub fn handle(&mut self, line: &str) -> Incoming {
        let v: Option<Value> = serde_json::from_str(line).ok();
        let method = v
            .as_ref()
            .and_then(|v| v.get("method"))
            .and_then(Value::as_str);
        if method == Some("initialize")
            && let Some(id) = v.as_ref().and_then(|v| read_id(v).ok().flatten())
        {
            let asked = v
                .as_ref()
                .and_then(|v| v.pointer("/params/protocolVersion"))
                .and_then(Value::as_str);
            let (version, result) = initialized(asked);
            self.era = Some(Era::Legacy(version));
            return ok(&id, result);
        }
        if self.era.is_none() && method.is_some_and(|m| !m.starts_with("notifications/")) {
            self.era = Some(Era::Modern);
        }
        let era = self.era();
        match dispatch(line) {
            Incoming::Call { id, call } => {
                let progress = v
                    .as_ref()
                    .and_then(|v| v.pointer("/params/_meta/progressToken"))
                    .cloned();
                Incoming::Call {
                    id: CallId {
                        era,
                        progress,
                        ..id
                    },
                    call,
                }
            }
            Incoming::Reply(r) if method == Some("tools/list") && r.get("result").is_some() => {
                Incoming::Reply(json!({"jsonrpc": "2.0", "id": r["id"], "result": list(&era)}))
            }
            other => other,
        }
    }
}

/// The line answered as 2026-07-28, statelessly.
fn dispatch(line: &str) -> Incoming {
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
        "tools/list" => ok(&id, list(&Era::Modern)),
        "ping" => ok(&id, json!({})),
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
                Some(Ok(call)) => Incoming::Call {
                    id: CallId {
                        id,
                        era: Era::Modern,
                        progress: None,
                    },
                    call,
                },
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
                assert_eq!(id.id, RequestId::Num(7));
                call
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn an_earlier_client_opens_with_initialize_and_is_served_its_revision() {
        let mut peer = Peer::default();
        let line = |v: Value| v.to_string();
        let init = match peer.handle(&line(
            json!({"jsonrpc": "2.0", "id": 0, "method": "initialize",
            "params": {"protocolVersion": "2025-06-18", "capabilities": {},
                "clientInfo": {"name": "t", "version": "1"}}}),
        )) {
            Incoming::Reply(v) => v,
            other => panic!("{other:?}"),
        };
        assert_eq!(init["result"]["protocolVersion"], "2025-06-18");
        assert_eq!(init["result"]["serverInfo"]["name"], "wubble");
        assert!(init["result"]["capabilities"]["tools"].is_object());
        assert!(
            init["result"]["instructions"]
                .as_str()
                .unwrap()
                .contains("start_session")
        );
        assert_eq!(peer.era(), Era::Legacy("2025-06-18".into()));
        assert_eq!(
            peer.handle(&line(
                json!({"jsonrpc": "2.0", "method": "notifications/initialized"})
            )),
            Incoming::Ignore
        );
        let Incoming::Reply(l) = peer.handle(&line(
            json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}),
        )) else {
            panic!("a list");
        };
        assert_eq!(l["result"]["tools"].as_array().unwrap().len(), TOOLS.len());
        assert!(l["result"].get("ttlMs").is_none() && l["result"].get("resultType").is_none());
        let Incoming::Call { id, .. } = peer.handle(&line(json!({"jsonrpc": "2.0", "id": 2,
            "method": "tools/call", "params": {"name": "start_session",
            "arguments": {"purpose": "p"}, "_meta": {"progressToken": "tk"}}})))
        else {
            panic!("a call");
        };
        assert_eq!(id.era, Era::Legacy("2025-06-18".into()));
        let answer = tool_response(&id, &ToolResult::text("ok"));
        assert_eq!(answer["id"], 2);
        assert!(answer["result"].get("resultType").is_none());
        let note = progress(&id, 1, "waiting for the owner").unwrap();
        assert_eq!(note["method"], "notifications/progress");
        assert_eq!(note["params"]["progressToken"], "tk");
        // a revision not served: the newest served one, the client decides
        let mut later = Peer::default();
        let Incoming::Reply(v) = later.handle(&line(json!({"jsonrpc": "2.0", "id": 0,
            "method": "initialize", "params": {"protocolVersion": "2099-01-01"}})))
        else {
            panic!("a reply");
        };
        assert_eq!(v["result"]["protocolVersion"], LEGACY_VERSIONS[0]);
        assert_eq!(
            peer.handle(&line(json!({"jsonrpc": "2.0", "id": 3, "method": "ping"}))),
            Incoming::Reply(json!({"jsonrpc": "2.0", "id": 3, "result": {}}))
        );
    }

    #[test]
    fn a_modern_client_gets_result_types_and_no_progress_without_a_token() {
        let mut peer = Peer::default();
        let Incoming::Call { id, .. } = peer.handle(
            &json!({"jsonrpc": "2.0", "id": 4, "method": "tools/call",
                "params": {"name": "start_session", "arguments": {"purpose": "p"}}})
            .to_string(),
        ) else {
            panic!("a call");
        };
        assert_eq!(id.era, Era::Modern);
        assert_eq!(
            tool_response(&id, &ToolResult::text("ok"))["result"]["resultType"],
            "complete"
        );
        assert!(progress(&id, 1, "x").is_none());
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
                    crate::tools::Ty::Int(lo, _) => json!(lo),
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
            // an optional ref outside such a group (`scroll` needs a ref or a direction)
            if t.args.iter().any(|a| a.name == "ref") && !t.one_of.contains(&"ref") {
                args.insert("ref".into(), json!("e1.1"));
            }
            match call(t.name, Value::Object(args)) {
                Incoming::Call { call, .. } => {
                    assert_eq!(call.tool(), t.name);
                    assert_eq!(call.session().is_none(), t.name == "start_session");
                    let names_tab = t.args.iter().any(|a| a.name == "tab");
                    assert_eq!(call.tab().is_some(), names_tab, "{}", t.name);
                }
                other => panic!("{}: {other:?}", t.name),
            }
        }
    }

    #[test]
    fn integers_are_range_checked_and_false_booleans_do_not_select() {
        let click = |n: Value| {
            call(
                "click",
                json!({"session": "s", "tab": "t", "ref": "e1.1", "count": n}),
            )
        };
        assert!(matches!(click(json!(3)), Incoming::Call { .. }));
        for bad in [json!(0), json!(4), json!(4_294_967_295u64), json!(u64::MAX)] {
            assert!(!matches!(click(bad), Incoming::Call { .. }));
        }
        let wait = |args: Value| call("wait_for", args);
        let base = json!({"session": "s", "tab": "t"});
        let with = |k: &str, v: Value| {
            let mut m = base.clone();
            m[k] = v;
            m
        };
        assert!(!matches!(
            wait(with("load", json!(false))),
            Incoming::Call { .. }
        ));
        assert!(!matches!(
            wait(with("timeout_ms", json!(0))),
            Incoming::Call { .. }
        ));
        let mut m = with("text", json!("x"));
        m["load"] = json!(false);
        assert!(matches!(wait(m), Incoming::Call { .. }));
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
                at: None,
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
        assert!(matches!(
            c,
            AgentCall::StartSession {
                headless: false,
                ..
            }
        ));
    }

    #[test]
    fn coordinates_come_in_pairs_and_drags_need_an_end() {
        let base = json!({"session": "s", "tab": "t", "ref": "e1.1"});
        let with = |extra: Value| {
            let mut m = base.clone();
            for (k, v) in extra.as_object().unwrap() {
                m[k] = v.clone();
            }
            m
        };
        assert!(matches!(
            parsed("hover", with(json!({"x": 3, "y": 4}))),
            AgentCall::Hover {
                at: Some(crate::call::Offset { x: 3, y: 4 }),
                ..
            }
        ));
        assert!(!matches!(
            call("hover", with(json!({"x": 3}))),
            Incoming::Call { .. }
        ));
        assert!(matches!(
            parsed("drag", with(json!({"to_x": 5, "to_y": 6}))),
            AgentCall::Drag {
                to: crate::call::DragTo::Offset(crate::call::Offset { x: 5, y: 6 }),
                ..
            }
        ));
        for bad in [
            json!({}),
            json!({"to_x": 5}),
            json!({"to_ref": "e1.2", "to_x": 1, "to_y": 1}),
        ] {
            assert!(
                !matches!(call("drag", with(bad.clone())), Incoming::Call { .. }),
                "{bad}"
            );
        }
        assert!(!matches!(
            call("scroll", json!({"session": "s", "tab": "t"})),
            Incoming::Call { .. }
        ));
        assert!(!matches!(
            call("screenshot", with(json!({"full_page": true}))),
            Incoming::Call { .. }
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
        let m = reply(r#"{"jsonrpc":"2.0","id":2,"method":"resources/list"}"#);
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
    fn without_a_server_requests_get_an_error_and_notifications_nothing() {
        let r = unavailable(
            r#"{"jsonrpc":"2.0","id":4,"method":"tools/list"}"#,
            "no browser",
        )
        .unwrap();
        assert_eq!(r["id"], 4);
        assert_eq!(r["error"]["code"], -32000);
        assert_eq!(r["error"]["message"], "no browser");
        for quiet in [
            r#"{"jsonrpc":"2.0","method":"notifications/cancelled"}"#,
            r#"{"jsonrpc":"2.0","id":1,"result":{}}"#,
            "{nope",
        ] {
            assert_eq!(unavailable(quiet, "x"), None, "{quiet}");
        }
    }

    #[test]
    fn tool_results_serialise() {
        let id = CallId {
            id: RequestId::Str("q".into()),
            era: Era::Modern,
            progress: None,
        };
        let r = tool_response(&id, &ToolResult::error("stale ref: take a new snapshot"));
        assert_eq!(r["id"], "q");
        assert_eq!(r["result"]["isError"], true);
        assert_eq!(r["result"]["content"][0]["type"], "text");
        let img = tool_response(&id, &ToolResult::image("AAAA", "image/png"));
        assert_eq!(img["result"]["content"][0]["mimeType"], "image/png");
        assert_eq!(img["result"]["isError"], false);
    }
}
