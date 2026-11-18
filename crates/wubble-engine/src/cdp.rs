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

#[cfg(test)]
mod tests {
    use super::*;

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
