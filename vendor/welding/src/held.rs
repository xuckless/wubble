// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! wubble change (see `WUBBLE_CHANGES.md`): one table for every CEF request a
//! host answers later.
//!
//! CEF hands a reference-counted callback to each question it asks the host
//! (auth, permission, JS dialog, file chooser, certificate error, context
//! menu, download destination). The callback is held here under an id the
//! host quotes back. Every kind has a defined default answer
//! ([`Answer::answer_default`]: cancel, deny, decline), given when the table
//! is full, when the browser closes (`drain_default` from `OnBeforeClose`)
//! and when the table is dropped, so an unanswered request never leaves a
//! page waiting.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};

/// Identifies one held request until it is answered. Never 0: 0 reads like
/// "no request" in host code, and an id the host sees for a request welding
/// already answered itself (an opt-out notice) is 0.
pub type RequestId = u32;

/// At most this many requests of one kind wait per browser. A page that asks
/// faster than the host answers gets the default for the rest.
pub(crate) const MAX_HELD: usize = 64;

/// A held CEF callback that knows its default answer.
pub(crate) trait Answer {
    /// Answer as a user who dismissed the question would: cancel, deny,
    /// decline. Consumes the callback, so it is answered at most once.
    fn answer_default(self);
}

/// wubble change: one counter for every table, so an id names one request
/// across all kinds (a host may key its open prompts on the id alone).
/// Process-wide, so ids are also unique across browsers.
static LAST_ID: AtomicU32 = AtomicU32::new(0);

/// The next id, never 0. After 2^32 requests it wraps; a table still holding
/// that id skips it.
fn next_id() -> RequestId {
    loop {
        let id = LAST_ID.fetch_add(1, Ordering::Relaxed).wrapping_add(1);
        if id != 0 {
            return id;
        }
    }
}

pub(crate) struct Held<C: Answer> {
    pending: HashMap<RequestId, C>,
    /// wubble change: callbacks CEF answered itself ([`Held::retire`]),
    /// unanswerable but still referenced until [`Held::take_retired`].
    retired: Vec<C>,
}

impl<C: Answer> Default for Held<C> {
    fn default() -> Self {
        Self {
            pending: HashMap::new(),
            retired: Vec::new(),
        }
    }
}

impl<C: Answer> Held<C> {
    /// Hold `callback` and return its id, or hand it back when the table is
    /// full; the caller then answers it with the default at once.
    pub(crate) fn hold(&mut self, callback: C) -> Result<RequestId, C> {
        if self.pending.len() >= MAX_HELD {
            return Err(callback);
        }
        let id = loop {
            let id = next_id();
            if !self.pending.contains_key(&id) {
                break id;
            }
        };
        self.pending.insert(id, callback);
        Ok(id)
    }

    /// The callback waiting on `id`, once. `None` for an id that was already
    /// answered, drained, or never handed out.
    pub(crate) fn take(&mut self, id: RequestId) -> Option<C> {
        self.pending.remove(&id)
    }

    /// wubble change: the first waiting request `matches` accepts, removed,
    /// with its id. For CEF dismissing a request itself.
    pub(crate) fn take_where(&mut self, matches: impl Fn(&C) -> bool) -> Option<(RequestId, C)> {
        let id = *self.pending.iter().find(|(_, c)| matches(c))?.0;
        self.pending.remove(&id).map(|c| (id, c))
    }

    /// Everything still waiting, removed, with its id. Ids are not reused
    /// afterwards. Answer them after releasing any lock around the table: a
    /// CEF callback may call straight back into a handler that takes it.
    pub(crate) fn drain(&mut self) -> Vec<(RequestId, C)> {
        self.pending.drain().collect()
    }

    /// wubble change: CEF closed every waiting request itself and already
    /// ran its callback. Their ids go stale and nothing is answered, but the
    /// handles are kept, not dropped: CEF may be reporting the close from
    /// inside that very callback's `Continue` (the context menu: CEF cancels
    /// through a raw pointer and `OnContextMenuDismissed` runs before
    /// `Continue` returns), and releasing what is then the last reference
    /// frees the object under it. Release them later, from a call CEF did not
    /// make from inside a callback, with [`Held::take_retired`].
    pub(crate) fn retire(&mut self) -> Vec<RequestId> {
        let (ids, callbacks): (Vec<_>, Vec<_>) = self.pending.drain().unzip();
        self.retired.extend(callbacks);
        ids
    }

    /// wubble change: the retired handles, to drop outside any lock.
    pub(crate) fn take_retired(&mut self) -> Vec<C> {
        std::mem::take(&mut self.retired)
    }

    /// Answer everything still waiting with its default.
    pub(crate) fn drain_default(&mut self) {
        for (_, callback) in self.drain() {
            callback.answer_default();
        }
    }

    /// How many requests are waiting on the host.
    // Diagnostic accessor: exercised by unit tests, kept for host-side leak checks.
    #[allow(dead_code)]
    pub(crate) fn outstanding(&self) -> usize {
        self.pending.len()
    }
}

impl<C: Answer> Drop for Held<C> {
    fn drop(&mut self) {
        self.drain_default();
    }
}

/// The `CefSurfaceProducer` methods that answer held requests, written once
/// for every platform producer: each delegates to the producer's
/// `requests: Arc<crate::handlers::Requests>` (cef-runtime only). Invoke
/// inside `impl CefSurfaceProducer for ...`.
macro_rules! request_answers {
    () => {
        fn grant_permission(&mut self, id: crate::PermissionId) -> Result<(), crate::WeldError> {
            crate::held::request_answers!(@run self, requests => requests.answer_permission_all(id, true), id)
        }

        fn deny_permission(&mut self, id: crate::PermissionId) -> Result<(), crate::WeldError> {
            crate::held::request_answers!(@run self, requests => requests.answer_permission_all(id, false), id)
        }

        fn answer_permission(
            &mut self,
            id: crate::PermissionId,
            granted: &[crate::PermissionKind],
        ) -> Result<(), crate::WeldError> {
            crate::held::request_answers!(@run self, requests => requests.answer_permission(id, granted), id, granted)
        }

        fn answer_auth(
            &mut self,
            id: crate::AuthId,
            username: &str,
            password: &str,
        ) -> Result<(), crate::WeldError> {
            // Deliberately not logged, here or anywhere.
            crate::held::request_answers!(@run self, requests => requests.answer_auth(id, Some((username, password))), id, username, password)
        }

        fn cancel_auth(&mut self, id: crate::AuthId) -> Result<(), crate::WeldError> {
            crate::held::request_answers!(@run self, requests => requests.answer_auth(id, None), id)
        }

        fn cancel_download(&mut self, id: crate::DownloadId) -> Result<(), crate::WeldError> {
            crate::held::request_answers!(@run self, requests => requests.steer_download(id, crate::downloads::DownloadOp::Cancel), id)
        }

        fn pause_download(&mut self, id: crate::DownloadId) -> Result<(), crate::WeldError> {
            crate::held::request_answers!(@run self, requests => requests.steer_download(id, crate::downloads::DownloadOp::Pause), id)
        }

        fn resume_download(&mut self, id: crate::DownloadId) -> Result<(), crate::WeldError> {
            crate::held::request_answers!(@run self, requests => requests.steer_download(id, crate::downloads::DownloadOp::Resume), id)
        }

        fn answer_dialog(
            &mut self,
            id: crate::RequestId,
            accept: bool,
            text: &str,
        ) -> Result<(), crate::WeldError> {
            crate::held::request_answers!(@run self, requests => requests.answer_dialog(id, accept, text), id, accept, text)
        }

        fn answer_file_chooser(
            &mut self,
            id: crate::RequestId,
            paths: Option<&[std::path::PathBuf]>,
        ) -> Result<(), crate::WeldError> {
            crate::held::request_answers!(@run self, requests => requests.answer_file_chooser(id, paths), id, paths)
        }

        fn answer_certificate(&mut self, id: crate::RequestId, proceed: bool) -> Result<(), crate::WeldError> {
            crate::held::request_answers!(@run self, requests => requests.answer_certificate(id, proceed), id, proceed)
        }

        fn answer_context_menu(
            &mut self,
            id: crate::RequestId,
            command: Option<i32>,
        ) -> Result<(), crate::WeldError> {
            crate::held::request_answers!(@run self, requests => requests.answer_context_menu(id, command), id, command)
        }

        fn answer_download_path(
            &mut self,
            id: crate::RequestId,
            path: Option<&std::path::Path>,
        ) -> Result<(), crate::WeldError> {
            crate::held::request_answers!(@run self, requests => requests.answer_download_path(id, path), id, path)
        }

        fn set_drag_source(&mut self, enabled: bool) -> Result<(), crate::WeldError> {
            crate::held::request_answers!(@run self, requests => {
                requests.set_drag_source(enabled);
                Ok(())
            }, enabled)
        }
    };
    (@run $self:ident, $requests:ident => $call:expr $(, $arg:ident)*) => {{
        #[cfg(feature = "cef-runtime")]
        let result = {
            let $requests = &$self.requests;
            $call
        };
        #[cfg(not(feature = "cef-runtime"))]
        let result = {
            let _ = ($(&$arg,)*);
            Err(crate::WeldError::PlatformUnsupported(
                "held requests require the cef-runtime feature",
            ))
        };
        result
    }};
}
pub(crate) use request_answers;

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    /// Records which fakes were answered with their default.
    #[derive(Clone, Default)]
    pub(crate) struct Log(Rc<RefCell<Vec<u32>>>);

    impl Log {
        pub(crate) fn answered(&self) -> Vec<u32> {
            let mut v = self.0.borrow().clone();
            v.sort_unstable();
            v
        }
    }

    pub(crate) struct Fake(pub(crate) u32, pub(crate) Log);

    impl Answer for Fake {
        fn answer_default(self) {
            self.1.0.borrow_mut().push(self.0);
        }
    }

    #[test]
    fn ids_are_unique_and_never_zero() {
        let log = Log::default();
        let mut held = Held::default();
        let ids: Vec<_> = (0..10).map(|n| held.hold(Fake(n, log.clone())).ok().unwrap()).collect();
        assert!(!ids.contains(&0));
        let mut sorted = ids.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), ids.len(), "ids repeated: {ids:?}");
        assert_eq!(held.outstanding(), 10);
    }

    #[test]
    fn take_hands_a_callback_out_once() {
        let log = Log::default();
        let mut held = Held::default();
        let id = held.hold(Fake(7, log.clone())).ok().unwrap();
        assert_eq!(held.take(id).map(|f| f.0), Some(7));
        assert!(held.take(id).is_none(), "answered twice");
        assert!(held.take(id + 100).is_none(), "an id never handed out");
        drop(held);
        assert!(log.answered().is_empty(), "a taken callback was defaulted too");
    }

    #[test]
    fn a_full_table_hands_the_callback_back() {
        let log = Log::default();
        let mut held = Held::default();
        for n in 0..MAX_HELD as u32 {
            assert!(held.hold(Fake(n, log.clone())).is_ok());
        }
        let back = held.hold(Fake(999, log.clone()));
        assert!(matches!(back, Err(Fake(999, _))), "the cap let one more in");
        assert_eq!(held.outstanding(), MAX_HELD);
    }

    #[test]
    fn drain_answers_every_held_callback_exactly_once() {
        let log = Log::default();
        let mut held = Held::default();
        let ids: Vec<_> = (0..5).map(|n| held.hold(Fake(n, log.clone())).ok().unwrap()).collect();
        held.take(ids[1]);
        held.drain_default();
        assert_eq!(log.answered(), vec![0, 2, 3, 4]);
        assert_eq!(held.outstanding(), 0);
        held.drain_default();
        assert_eq!(log.answered(), vec![0, 2, 3, 4], "drained twice");
        assert!(held.take(ids[0]).is_none(), "a drained id is stale");
    }

    #[test]
    fn dropping_the_table_answers_what_is_left() {
        let log = Log::default();
        let mut held = Held::default();
        held.hold(Fake(1, log.clone())).ok().unwrap();
        held.hold(Fake(2, log.clone())).ok().unwrap();
        drop(held);
        assert_eq!(log.answered(), vec![1, 2]);
    }

    #[test]
    fn ids_are_unique_across_tables() {
        // A host keys open prompts on the id: a dialog and a permission
        // request in one view must never share one.
        let log = Log::default();
        let mut dialogs = Held::default();
        let mut permissions = Held::default();
        let a = dialogs.hold(Fake(1, log.clone())).ok().unwrap();
        let b = permissions.hold(Fake(2, log.clone())).ok().unwrap();
        let c = dialogs.hold(Fake(3, log.clone())).ok().unwrap();
        assert!(a != b && b != c && a != c, "{a} {b} {c}");
    }

    #[test]
    fn take_where_removes_one_match_without_answering_it() {
        let log = Log::default();
        let mut held = Held::default();
        held.hold(Fake(1, log.clone())).ok().unwrap();
        let two = held.hold(Fake(2, log.clone())).ok().unwrap();
        let (id, fake) = held.take_where(|f| f.0 == 2).unwrap();
        assert_eq!((id, fake.0), (two, 2));
        assert!(held.take_where(|f| f.0 == 2).is_none());
        drop(fake);
        assert_eq!(held.outstanding(), 1);
        drop(held);
        assert_eq!(log.answered(), vec![1], "the dismissed one was answered too");
    }

    /// How many fakes sharing `log` are still alive (held, retired or out).
    fn alive(log: &Log) -> usize {
        Rc::strong_count(&log.0) - 1
    }

    #[test]
    fn retire_answers_nothing_and_keeps_the_handles() {
        let log = Log::default();
        let mut held = Held::default();
        let one = held.hold(Fake(1, log.clone())).ok().unwrap();
        let two = held.hold(Fake(2, log.clone())).ok().unwrap();
        let mut ids = held.retire();
        ids.sort_unstable();
        let mut want = vec![one, two];
        want.sort_unstable();
        assert_eq!(ids, want);
        assert_eq!(held.outstanding(), 0);
        assert_eq!(alive(&log), 2, "a retired handle was released during the dismissal");
        assert!(held.take(one).is_none(), "a retired request answered after CEF closed it");
        held.drain_default();
        assert!(log.answered().is_empty(), "CEF already answered a retired request");
        assert_eq!(alive(&log), 2);
        let retired = held.take_retired();
        assert_eq!(retired.len(), 2);
        drop(retired);
        assert_eq!(alive(&log), 0);
        assert!(log.answered().is_empty());
    }

    #[test]
    fn every_request_is_answered_exactly_once_whoever_closes_it() {
        // The host answers one, CEF closes the next, the browser closes with
        // the third waiting: three requests, three answers, one each.
        let log = Log::default();
        let mut held = Held::default();
        let host = held.hold(Fake(1, log.clone())).ok().unwrap();
        held.take(host).unwrap().answer_default();
        assert!(held.retire().is_empty(), "the host's answered request was retired too");
        let cef = held.hold(Fake(2, log.clone())).ok().unwrap();
        // CEF ran callback 2 itself: record it as CEF's answer.
        log.0.borrow_mut().push(2);
        assert_eq!(held.retire(), vec![cef]);
        assert!(held.take(cef).is_none());
        held.hold(Fake(3, log.clone())).ok().unwrap();
        drop(held);
        assert_eq!(log.answered(), vec![1, 2, 3]);
        assert_eq!(alive(&log), 0, "a retired handle leaked past the table");
    }

    #[test]
    fn ids_stay_fresh_after_a_drain() {
        let log = Log::default();
        let mut held = Held::default();
        let first = held.hold(Fake(1, log.clone())).ok().unwrap();
        held.drain_default();
        let second = held.hold(Fake(2, log.clone())).ok().unwrap();
        assert_ne!(first, second, "a stale id could answer a newer request");
    }
}
