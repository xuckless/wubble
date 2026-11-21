// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! wubble change, see `WUBBLE_CHANGES.md`: CEF's keyboard handler, shared
//! by the Linux, macOS and Windows clients. `OnKeyEvent` is called for key
//! events the page did not handle (also in windowless mode); each key-down
//! among them is queued for the host as [`CefSurfaceEvent::KeyUnhandled`].

use std::sync::Arc;

// The `wrap_*` macros name the handler traits unqualified.
use cef::*;

use crate::surface::{CefSurfaceEvent, WebEventQueue};

/// The platform's native event pointer in `OnKeyEvent` (cef 154 bindings).
#[cfg(target_os = "linux")]
type OsEvent<'a> = Option<&'a mut cef::sys::XEvent>;
#[cfg(windows)]
type OsEvent<'a> = Option<&'a mut cef::sys::MSG>;
#[cfg(target_os = "macos")]
type OsEvent<'a> = *mut u8;

cef::wrap_keyboard_handler! {
    pub(crate) struct WeldKeyboardHandler {
        events: Arc<WebEventQueue>,
    }

    impl KeyboardHandler {
        fn on_key_event(
            &self,
            _browser: Option<&mut cef::Browser>,
            event: Option<&cef::KeyEvent>,
            _os_event: OsEvent<'_>,
        ) -> ::std::os::raw::c_int {
            if let Some(event) = event
                && (event.type_ == cef::KeyEventType::RAWKEYDOWN
                    || event.type_ == cef::KeyEventType::KEYDOWN)
            {
                self.events.push(CefSurfaceEvent::KeyUnhandled {
                    windows_key_code: event.windows_key_code,
                    modifiers: event.modifiers,
                    // wubble change (56): the page's focus on an editable field
                    focus_on_editable_field: event.focus_on_editable_field != 0,
                });
            }
            // Not handled here: CEF's own default handling still runs.
            0
        }
    }
}

impl WeldKeyboardHandler {
    pub(crate) fn build(events: Arc<WebEventQueue>) -> cef::KeyboardHandler {
        Self::new(events)
    }
}
