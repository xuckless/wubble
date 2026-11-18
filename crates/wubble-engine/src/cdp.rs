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

use crate::page::PageEvent;

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
    next: u32,
    waiting: BTreeMap<u32, Request>,
}

impl Pending {
    fn add(&mut self, request: Request) -> u32 {
        // High ids, so they never meet welding's own snapshot requests (small ids).
        self.next += 1;
        let id = 0x4000_0000 + self.next;
        self.waiting.insert(id, request);
        id
    }

    /// A new freeze request: its id and the message to send.
    pub(crate) fn request(&mut self, frozen: bool) -> (u32, String) {
        let id = self.add(Request::Freeze(frozen));
        let state = if frozen { "frozen" } else { "active" };
        let message = format!(
            r#"{{"id":{id},"method":"Page.setWebLifecycleState","params":{{"state":"{state}"}}}}"#
        );
        (id, message)
    }

    /// A new target id request (a page session answers with its own target).
    pub(crate) fn request_target_id(&mut self) -> (u32, String) {
        let id = self.add(Request::TargetId);
        (
            id,
            format!(r#"{{"id":{id},"method":"Target.getTargetInfo"}}"#),
        )
    }

    /// The request could not be sent: no reply will come.
    pub(crate) fn cancel(&mut self, id: u32) {
        self.waiting.remove(&id);
    }

    /// A message from the channel: the answer to one of ours becomes a [`PageEvent`];
    /// anything else (events, other replies) is ignored.
    pub(crate) fn reply(&mut self, message: &str) -> Option<PageEvent> {
        let value: serde_json::Value = serde_json::from_str(message).ok()?;
        let id = u32::try_from(value.get("id")?.as_u64()?).ok()?;
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

    /// Whether replies are still expected (the channel is polled only then).
    pub(crate) fn is_empty(&self) -> bool {
        self.waiting.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(v["id"].as_u64(), Some(u64::from(target_id)));
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
        p.cancel(u32::try_from(id).unwrap());
        assert!(p.is_empty());
    }
}
