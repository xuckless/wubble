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
//!
//! Two uses ride on it: the generic calls above, and [`Pending`], the
//! bookkeeping of the engine's own requests (freeze, target id).
//!
//! Freezing a page through the DevTools protocol, the one freeze prebuilt CEF 156 offers.
//!
//! CEF has no freeze API. Chromium's `Page.setWebLifecycleState` (experimental; handler
//! `content/browser/devtools/protocol/page_handler.cc`, `PageHandler::SetWebLifecycleState`,
//! BSD-3-Clause, called, nothing copied) hides the page (`WasHidden`) and then freezes it
//! (`SetPageFrozen(true)`): no JS, no timers. `active` unfreezes it but does not show it; the
//! view's `set_visible(true)` stays the show path. The call is refused for a page that is not
//! the active top-level one (prerendered, in the back/forward cache).
//!
//! The message goes through welding's CDP pass-through (`send_devtools_message`); its reply
//! comes back on the same channel. [`Pending`] is the pure bookkeeping between the two. It also
//! asks a page for its target id (`Target.getTargetInfo`, handler `target_handler.cc`, BSD-3-Clause,
//! called, nothing copied), which names the page to the DevTools frontend (`wubble_core::devtools`).

use std::collections::BTreeMap;

use serde_json::{Value, json};

use crate::page::PageEvent;

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

/// The main frame's navigations the page asked for, counted from the Page domain's events
/// (enabled on every view for its document scripts): `Page.frameRequestedNavigation` is sent,
/// flushed, as the page asks (a form submitted, a link followed, a script's `location`), before
/// the input or script that asked has returned, so a reply that comes after that input was
/// handled (an input fence) finds it counted. Only the current tab's: a new tab or window, a
/// download is not this page's navigation. The main frame is the one `Page.frameNavigated`
/// names with no parent; before one is seen, any frame counts.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NavRequests {
    main: Option<String>,
    count: u64,
}

impl NavRequests {
    /// Reads one DevTools message; anything but those two events is ignored.
    pub fn read(&mut self, text: &str) {
        let Ok(value) = serde_json::from_str::<Value>(text) else {
            return;
        };
        let params = &value["params"];
        match value["method"].as_str() {
            Some("Page.frameNavigated") if params["frame"]["parentId"].is_null() => {
                self.main = params["frame"]["id"].as_str().map(str::to_owned);
            }
            Some("Page.frameRequestedNavigation")
                if params["disposition"] == "currentTab"
                    && self
                        .main
                        .as_deref()
                        .is_none_or(|m| params["frameId"].as_str() == Some(m)) =>
            {
                self.count += 1;
            }
            _ => {}
        }
    }

    /// How many the page has asked for so far.
    pub fn count(&self) -> u64 {
        self.count
    }
}

/// `Page.fileChooserOpened`, sent instead of a file chooser while the page's DevTools session
/// intercepts them (`Page.setInterceptFileChooserDialog`; handler `page_handler.cc`,
/// `PageHandler::FileChooserOpened`, BSD-3-Clause, called, nothing copied): the chooser was not
/// shown, and the element that opened it takes files through `DOM.setFileInputFiles` by its
/// backend node id, as Puppeteer's and Playwright's file choosers do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChooserOpened {
    /// The frame whose element opened it.
    pub frame: String,
    /// `selectMultiple` (an `<input multiple>`), else one file.
    pub multiple: bool,
    /// The `<input type=file>` that opened it, when DevTools names one.
    pub node: Option<i64>,
}

/// The [`ChooserOpened`] one DevTools message carries, if it is that event.
pub fn chooser_opened(text: &str) -> Option<ChooserOpened> {
    let value: Value = serde_json::from_str(text).ok()?;
    if value["method"] != "Page.fileChooserOpened" {
        return None;
    }
    let params = &value["params"];
    Some(ChooserOpened {
        frame: params["frameId"].as_str().unwrap_or_default().to_owned(),
        multiple: params["mode"] == "selectMultiple",
        node: params["backendNodeId"].as_i64(),
    })
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

/// What a request in flight asks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Request {
    /// `Page.setWebLifecycleState`, to freeze (true) or thaw.
    Freeze(bool),
    /// `Target.getTargetInfo`: the page's own target id, for the docked DevTools frontend.
    TargetId,
}

/// Requests sent and not yet answered, by CDP message id.
#[derive(Debug, Default)]
pub(crate) struct Pending {
    waiting: BTreeMap<u64, Request>,
}

impl Pending {
    fn add(&mut self, request: Request) -> u64 {
        // From the one counter of [`next_id`], so they never meet welding's own snapshot
        // requests (small ids) nor `View::cdp`'s calls.
        let id = next_id();
        self.waiting.insert(id, request);
        id
    }

    /// A new freeze request: its id and the message to send.
    pub(crate) fn request(&mut self, frozen: bool) -> (u64, String) {
        let id = self.add(Request::Freeze(frozen));
        let state = if frozen { "frozen" } else { "active" };
        let message = format!(
            r#"{{"id":{id},"method":"Page.setWebLifecycleState","params":{{"state":"{state}"}}}}"#
        );
        (id, message)
    }

    /// A new target id request (a page session answers with its own target).
    pub(crate) fn request_target_id(&mut self) -> (u64, String) {
        let id = self.add(Request::TargetId);
        (
            id,
            format!(r#"{{"id":{id},"method":"Target.getTargetInfo"}}"#),
        )
    }

    /// The request could not be sent: no reply will come.
    pub(crate) fn cancel(&mut self, id: u64) {
        self.waiting.remove(&id);
    }

    /// A message from the channel: the answer to one of ours becomes a [`PageEvent`];
    /// anything else (events, other replies) is ignored.
    pub(crate) fn reply(&mut self, message: &str) -> Option<PageEvent> {
        let value: serde_json::Value = serde_json::from_str(message).ok()?;
        let id = value.get("id")?.as_u64()?;
        let request = self.waiting.remove(&id)?;
        let error = value.get("error");
        if let Some(error) = error {
            eprintln!("wubble-engine: CDP {request:?}: {error}");
        }
        Some(match request {
            Request::Freeze(frozen) => PageEvent::Frozen {
                frozen,
                ok: error.is_none(),
            },
            Request::TargetId => PageEvent::TargetId(
                value["result"]["targetInfo"]["targetId"]
                    .as_str()
                    .filter(|_| error.is_none())
                    .map(str::to_string)
                    .ok_or(()),
            ),
        })
    }

    /// Whether replies are still expected.
    #[cfg(test)]
    pub(crate) fn is_empty(&self) -> bool {
        self.waiting.is_empty()
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
    fn the_main_frames_requested_navigations_are_counted() {
        let mut n = NavRequests::default();
        let asked = |frame: &str, disposition: &str| {
            format!(
                r#"{{"method":"Page.frameRequestedNavigation","params":{{"frameId":"{frame}","reason":"formSubmissionGet","url":"http://a/","disposition":"{disposition}"}}}}"#
            )
        };
        // before the main frame is known, any frame's counts
        n.read(&asked("SUB", "currentTab"));
        assert_eq!(n.count(), 1);
        n.read(r#"{"method":"Page.frameNavigated","params":{"frame":{"id":"MAIN","loaderId":"L","url":"http://a/"},"type":"Navigation"}}"#);
        n.read(r#"{"method":"Page.frameNavigated","params":{"frame":{"id":"SUB","parentId":"MAIN","url":"http://b/"},"type":"Navigation"}}"#);
        n.read(&asked("MAIN", "currentTab"));
        n.read(&asked("SUB", "currentTab"));
        n.read(&asked("MAIN", "newTab"));
        n.read(r#"{"id":12,"result":{}}"#);
        n.read("not json");
        assert_eq!(n.count(), 2);
    }

    #[test]
    fn a_chooser_devtools_intercepted_names_its_frame_mode_and_element() {
        assert_eq!(
            chooser_opened(
                r#"{"method":"Page.fileChooserOpened","params":{"frameId":"F1","mode":"selectMultiple","backendNodeId":42}}"#
            ),
            Some(ChooserOpened {
                frame: "F1".into(),
                multiple: true,
                node: Some(42)
            })
        );
        assert_eq!(
            chooser_opened(
                r#"{"method":"Page.fileChooserOpened","params":{"frameId":"F1","mode":"selectSingle"}}"#
            ),
            Some(ChooserOpened {
                frame: "F1".into(),
                multiple: false,
                node: None
            })
        );
        for other in [
            r#"{"method":"Page.frameNavigated","params":{}}"#,
            r#"{"id":1000001,"result":{}}"#,
            "not json",
        ] {
            assert_eq!(chooser_opened(other), None, "{other}");
        }
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

    fn id_of(message: &str) -> u64 {
        let v: serde_json::Value = serde_json::from_str(message).unwrap();
        v["id"].as_u64().unwrap()
    }

    #[test]
    fn replies_map_to_their_requests() {
        let mut p = Pending::default();
        let (_, freeze) = p.request(true);
        let (_, thaw) = p.request(false);
        let v: serde_json::Value = serde_json::from_str(&freeze).unwrap();
        assert_eq!(v["method"], "Page.setWebLifecycleState");
        assert_eq!(v["params"]["state"], "frozen");
        assert!(thaw.contains(r#""state":"active""#));
        let (a, b) = (id_of(&freeze), id_of(&thaw));
        assert_ne!(a, b);
        assert!(!p.is_empty());
        // Out of order, one refused.
        assert_eq!(
            p.reply(&format!(
                r#"{{"id":{b},"error":{{"code":-32000,"message":"no"}}}}"#
            )),
            Some(PageEvent::Frozen {
                frozen: false,
                ok: false
            })
        );
        assert_eq!(
            p.reply(&format!(r#"{{"id":{a},"result":{{}}}}"#)),
            Some(PageEvent::Frozen {
                frozen: true,
                ok: true
            })
        );
        assert!(p.is_empty());
        // Answered once only.
        assert_eq!(p.reply(&format!(r#"{{"id":{a},"result":{{}}}}"#)), None);
    }

    #[test]
    fn the_target_id_request_has_its_own_id_and_reply() {
        let mut p = Pending::default();
        let (freeze_id, _) = p.request(true);
        let (target_id, message) = p.request_target_id();
        assert_ne!(freeze_id, target_id);
        let v: serde_json::Value = serde_json::from_str(&message).unwrap();
        assert_eq!(v["method"], "Target.getTargetInfo");
        assert_eq!(v["id"].as_u64(), Some(target_id));
        // out of order: the target reply first, the freeze still waiting
        assert_eq!(
            p.reply(&format!(
                r#"{{"id":{target_id},"result":{{"targetInfo":{{"targetId":"ABC123","type":"page"}}}}}}"#
            )),
            Some(PageEvent::TargetId(Ok("ABC123".into())))
        );
        assert!(!p.is_empty());
        assert_eq!(
            p.reply(&format!(r#"{{"id":{freeze_id},"result":{{}}}}"#)),
            Some(PageEvent::Frozen {
                frozen: true,
                ok: true
            })
        );
        // an error, or a reply with no id in it, is a failure
        for reply in [
            r#"{"id":ID,"error":{"code":-32601,"message":"no"}}"#,
            r#"{"id":ID,"result":{}}"#,
        ] {
            let (id, _) = p.request_target_id();
            assert_eq!(
                p.reply(&reply.replace("ID", &id.to_string())),
                Some(PageEvent::TargetId(Err(())))
            );
        }
        assert!(p.is_empty());
    }

    #[test]
    fn unrelated_messages_are_ignored() {
        let mut p = Pending::default();
        let id = id_of(&p.request(true).1);
        for other in [
            r#"{"method":"Page.lifecycleEvent","params":{}}"#.to_string(),
            r#"{"id":7,"result":{}}"#.to_string(),
            "not json".to_string(),
            format!(r#"{{"id":"{id}"}}"#),
        ] {
            assert_eq!(p.reply(&other), None, "{other}");
        }
        assert!(!p.is_empty());
        p.cancel(id);
        assert!(p.is_empty());
    }
}
