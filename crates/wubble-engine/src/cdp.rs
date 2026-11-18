//! The DevTools protocol (CDP) and page scripts as wubble asks them: one
//! message out per call, one reply in per id. Pure: plain JSON text, no CEF
//! types. [`crate::view::View::cdp`] sends a call over welding's raw DevTools
//! channel (`send_devtools_message`), [`crate::view::View::eval`] runs a
//! script in the main frame (`request_script_result`); their answers come
//! back as [`crate::page::PageEvent::CdpReply`] and
//! [`crate::page::PageEvent::EvalResult`].
//!
//! Wire format: Chromium's `ChromeDevTools/devtools-protocol` (BSD-3-Clause),
//! `{"id", "method", "params"}` out, `{"id", "result"}` or
//! `{"id", "error": {"message"}}` back; events carry no `id`.

use std::collections::BTreeMap;

use serde_json::{Value, json};

/// The first id wubble gives a call or a script. welding numbers its own
/// DevTools calls (the snapshot's `Page.captureScreenshot`) from 1, so ours
/// never meet them in the shared reply queue.
pub const FIRST_ID: u64 = 1_000_000;

/// One DevTools call, for as long as the engine runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CdpId(pub u64);

/// One page script's answer, for as long as the engine runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct EvalId(pub u64);

/// The next id for a call or a script: one counter for both and for every
/// view, so a reply never names another's request.
pub fn next_id() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(FIRST_ID);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// The message for one call.
pub fn message(id: CdpId, method: &str, params: &Value) -> String {
    json!({ "id": id.0, "method": method, "params": params }).to_string()
}

/// A reply to one of wubble's calls: its id and its result, or the
/// protocol's error message. Events (no `id`), welding's own replies (ids
/// under [`FIRST_ID`]) and text that is no JSON object are `None`.
pub fn reply(text: &str) -> Option<(CdpId, Result<Value, String>)> {
    let mut value: Value = serde_json::from_str(text).ok()?;
    let id = value.get("id")?.as_u64().filter(|&id| id >= FIRST_ID)?;
    let result = match value.get_mut("error") {
        Some(error) => Err(error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("DevTools error")
            .to_string()),
        None => Ok(value
            .get_mut("result")
            .map(Value::take)
            .unwrap_or(Value::Null)),
    };
    Some((CdpId(id), result))
}

/// A call that keeps a view's document-start scripts as wanted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DocCall {
    /// `Page.enable`, once per view: without it the renderer keeps the scripts but never runs
    /// them (seen with CEF 156: the add is answered with an identifier, no document runs it).
    Enable,
    /// `Page.addScriptToEvaluateOnNewDocument` with this source.
    Add(String),
    /// `Page.removeScriptToEvaluateOnNewDocument` of this identifier.
    Remove(String),
}

impl DocCall {
    /// The call's method and parameters.
    pub fn method(&self) -> (&'static str, Value) {
        match self {
            DocCall::Enable => ("Page.enable", json!({})),
            DocCall::Add(source) => (
                "Page.addScriptToEvaluateOnNewDocument",
                json!({ "source": source }),
            ),
            DocCall::Remove(identifier) => (
                "Page.removeScriptToEvaluateOnNewDocument",
                json!({ "identifier": identifier }),
            ),
        }
    }
}

/// The scripts a view runs at the start of every document, before the page's own
/// (`Page.addScriptToEvaluateOnNewDocument`: evaluated by the inspector, so no page
/// Content-Security-Policy applies; the Page domain is enabled first). DevTools names each by
/// an identifier that comes back later; a script replaced before its identifier came is
/// removed once it does. The Page domain's events are drained with every other DevTools
/// message and dropped (`reply` ignores events).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DocScripts {
    /// `Page.enable` went out (its id, until answered). Set when it is sent, not when it is
    /// asked for, so a call the channel refused is asked for again by the next `set`.
    enabled: bool,
    enabling: Option<u64>,
    /// The identifiers of the scripts in place.
    registered: Vec<String>,
    /// Add calls not answered yet, and whether their script is still wanted.
    adding: BTreeMap<u64, bool>,
}

impl DocScripts {
    /// The calls that replace every script with `sources`.
    pub fn set(&mut self, sources: &[String]) -> Vec<DocCall> {
        for wanted in self.adding.values_mut() {
            *wanted = false;
        }
        let mut calls = vec![];
        if !self.enabled {
            calls.push(DocCall::Enable);
        }
        calls.extend(self.registered.drain(..).map(DocCall::Remove));
        calls.extend(sources.iter().cloned().map(DocCall::Add));
        calls
    }

    /// `call` went out as `id`.
    pub fn sent(&mut self, id: CdpId, call: &DocCall) {
        match call {
            DocCall::Enable => {
                self.enabled = true;
                self.enabling = Some(id.0);
            }
            DocCall::Add(_) => {
                self.adding.insert(id.0, true);
            }
            DocCall::Remove(_) => {}
        }
    }

    /// The answer to call `id`: `None` when it is no add call of ours; else the calls it
    /// leads to (the removal of a script no longer wanted).
    pub fn reply(&mut self, id: CdpId, result: &Result<Value, String>) -> Option<Vec<DocCall>> {
        if self.enabling == Some(id.0) {
            self.enabling = None;
            return Some(vec![]);
        }
        let wanted = self.adding.remove(&id.0)?;
        let identifier = result
            .as_ref()
            .ok()
            .and_then(|v| v.get("identifier"))
            .and_then(Value::as_str)
            .map(str::to_owned);
        Some(match (identifier, wanted) {
            (Some(identifier), true) => {
                self.registered.push(identifier);
                vec![]
            }
            (Some(identifier), false) => vec![DocCall::Remove(identifier)],
            (None, _) => vec![],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn document_scripts_are_replaced_even_while_their_identifiers_are_on_the_way() {
        let mut d = DocScripts::default();
        let calls = d.set(&["a".into(), "b".into()]);
        assert_eq!(
            calls,
            vec![
                DocCall::Enable,
                DocCall::Add("a".into()),
                DocCall::Add("b".into())
            ]
        );
        d.sent(CdpId(10), &calls[0]);
        d.sent(CdpId(1), &calls[1]);
        d.sent(CdpId(2), &calls[2]);
        assert_eq!(
            d.reply(CdpId(10), &Ok(json!({}))),
            Some(vec![]),
            "enable is ours"
        );
        assert_eq!(
            d.reply(CdpId(1), &Ok(json!({"identifier": "1"}))),
            Some(vec![])
        );
        // Replaced before the second answered: the first is removed now, the second when its
        // identifier arrives.
        assert_eq!(
            d.set(&["c".into()]),
            vec![DocCall::Remove("1".into()), DocCall::Add("c".into())]
        );
        d.sent(CdpId(3), &DocCall::Add("c".into()));
        assert_eq!(
            d.reply(CdpId(2), &Ok(json!({"identifier": "2"}))),
            Some(vec![DocCall::Remove("2".into())])
        );
        assert_eq!(d.reply(CdpId(3), &Err("refused".into())), Some(vec![]));
        assert_eq!(d.reply(CdpId(9), &Ok(json!({}))), None, "not ours");
        assert_eq!(d.set(&[]), vec![]);
        let (method, params) = DocCall::Add("x".into()).method();
        assert_eq!(method, "Page.addScriptToEvaluateOnNewDocument");
        assert_eq!(params, json!({"source": "x"}));
    }

    #[test]
    fn an_enable_that_never_went_out_is_asked_for_again() {
        let mut d = DocScripts::default();
        // The channel refused every call: none was `sent`.
        assert_eq!(d.set(&["a".into()])[0], DocCall::Enable);
        let calls = d.set(&["a".into()]);
        assert_eq!(calls[0], DocCall::Enable);
        d.sent(CdpId(1), &calls[0]);
        assert_eq!(d.set(&["a".into()]), vec![DocCall::Add("a".into())]);
    }

    #[test]
    fn a_call_is_one_json_message() {
        let text = message(
            CdpId(FIRST_ID),
            "Emulation.setAutoDarkModeOverride",
            &json!({ "enabled": true }),
        );
        let v: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(
            v,
            json!({
                "id": FIRST_ID,
                "method": "Emulation.setAutoDarkModeOverride",
                "params": { "enabled": true }
            })
        );
    }

    #[test]
    fn ids_start_above_weldings_and_never_repeat() {
        let a = next_id();
        let b = next_id();
        assert!(a >= FIRST_ID && b > a);
    }

    #[test]
    fn replies_carry_their_result_or_error() {
        let id = FIRST_ID + 7;
        assert_eq!(
            reply(&format!(r#"{{"id":{id},"result":{{"data":"iVBO"}}}}"#)),
            Some((CdpId(id), Ok(json!({ "data": "iVBO" }))))
        );
        assert_eq!(
            reply(&format!(r#"{{"id":{id},"result":{{}}}}"#)),
            Some((CdpId(id), Ok(json!({}))))
        );
        assert_eq!(
            reply(&format!(
                r#"{{"id":{id},"error":{{"code":-32601,"message":"'X.y' wasn't found"}}}}"#
            )),
            Some((CdpId(id), Err("'X.y' wasn't found".into())))
        );
    }

    #[test]
    fn events_foreign_ids_and_junk_are_ignored() {
        for text in [
            r#"{"method":"Page.loadEventFired","params":{}}"#,
            r#"{"id":3,"result":{"data":""}}"#,
            r#"[1,2]"#,
            "not json",
        ] {
            assert_eq!(reply(text), None, "{text}");
        }
    }
}
