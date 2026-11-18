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

pub(crate) struct Held<C: Answer> {
    last: RequestId,
    pending: HashMap<RequestId, C>,
}

impl<C: Answer> Default for Held<C> {
    fn default() -> Self {
        Self {
            last: 0,
            pending: HashMap::new(),
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
        // Skip 0 and any id still waiting after the counter wrapped.
        loop {
            self.last = self.last.wrapping_add(1);
            if self.last != 0 && !self.pending.contains_key(&self.last) {
                break;
            }
        }
        self.pending.insert(self.last, callback);
        Ok(self.last)
    }

    /// The callback waiting on `id`, once. `None` for an id that was already
    /// answered, drained, or never handed out.
    pub(crate) fn take(&mut self, id: RequestId) -> Option<C> {
        self.pending.remove(&id)
    }

    /// Everything still waiting, removed. Ids are not reused afterwards.
    /// Answer them after releasing any lock around the table: a CEF callback
    /// may call straight back into a handler that takes it.
    pub(crate) fn drain(&mut self) -> Vec<C> {
        self.pending.drain().map(|(_, callback)| callback).collect()
    }

    /// Answer everything still waiting with its default.
    pub(crate) fn drain_default(&mut self) {
        for callback in self.drain() {
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
    fn ids_stay_fresh_after_a_drain() {
        let log = Log::default();
        let mut held = Held::default();
        let first = held.hold(Fake(1, log.clone())).ok().unwrap();
        held.drain_default();
        let second = held.hold(Fake(2, log.clone())).ok().unwrap();
        assert_ne!(first, second, "a stale id could answer a newer request");
    }
}
