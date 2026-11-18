// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! wubble change, see `WUBBLE_CHANGES.md`: the page's clipboard through the
//! host, for windowless browsers whose own clipboard cannot reach the
//! system's.
//!
//! On Wayland, Chromium's clipboard is its own Wayland client: it may set
//! the selection only with the serial of an input event on one of its own
//! surfaces, and the compositor offers the selection only to the client with
//! keyboard focus. A windowless browser has no surface, so every copy stays
//! inside Chromium and every paste reads nothing. The host's window has both,
//! so the page's clipboard is carried to it:
//!
//! - Copy and cut: a trusted `copy` / `cut` event (keys, the context menu,
//!   `execCommand`) or `navigator.clipboard.writeText` reports the text the
//!   page put on the clipboard (`kind` `copy`); a selection change reports the
//!   selected text for the primary selection (`primary`).
//! - Paste: Chromium's own trusted `paste` event (keys, the menu) is
//!   cancelled before the page sees it, and the host is asked for the text
//!   (`paste`; with `primary` for a middle click in a field). Its answer is
//!   pasted as Chromium pastes plain text: a `paste` event carrying it, then,
//!   unless the page cancels that, the text inserted as one undoable edit. `navigator.clipboard.readText` keeps its
//!   permission check and resolves with the host's text (`read`).
//!
//! The hook is installed in every frame's main world when the browser
//! process launched the renderer with [`SWITCH`]. Its functions are private to
//! a closure: the page cannot call the host.

/// The renderer switch that turns the bridge on.
pub const SWITCH: &str = "weld-clipboard-bridge";
/// Renderer to browser: `[kind, text]`.
pub(crate) const REQUEST: &str = "weld.clipboard";
/// Browser to renderer: `[text, read id or ""]`.
pub(crate) const ANSWER: &str = "weld.paste";

/// Called with the native `send(kind, text)`; returns `answer(text, id)`.
pub(crate) const HOOK: &str = r#"(function (send) {
  'use strict';
  const own = new WeakSet();
  const reads = new Map();
  let nextRead = 1;
  const selected = () => {
    const a = document.activeElement;
    if (a && (a instanceof HTMLInputElement || a instanceof HTMLTextAreaElement)) {
      try {
        if (typeof a.selectionStart === 'number') return a.value.slice(a.selectionStart, a.selectionEnd);
      } catch (e) {}
    }
    return String(getSelection());
  };
  for (const type of ['copy', 'cut']) {
    addEventListener(type, (e) => {
      if (!e.isTrusted) return;
      let set;
      const data = e.clipboardData;
      if (data) {
        const setData = data.setData;
        data.setData = function (format, value) {
          if (format === 'text/plain' || format === 'text' || format === 'Text') set = String(value);
          return setData.call(this, format, value);
        };
      }
      const selection = selected();
      setTimeout(() => {
        const text = e.defaultPrevented ? set : selection;
        if (text) send('copy', text);
      });
    }, true);
  }
  // A middle click in a field pastes the primary selection, as on X11 (Blink's
  // `PasteGlobalSelection`). Chromium's own runs only where its clipboard has a
  // selection buffer, which a windowless Wayland one does not; when it does
  // run, its paste below is answered from the primary selection too.
  let middle = false;
  const editable = (t) => t instanceof Element && (t.isContentEditable ||
    t instanceof HTMLTextAreaElement || (t instanceof HTMLInputElement && !t.readOnly && !t.disabled));
  addEventListener('mousedown', (e) => { middle = e.isTrusted && e.button === 1; }, true);
  addEventListener('mouseup', (e) => {
    if (!e.isTrusted || e.button !== 1 || !middle || !editable(e.target)) return;
    setTimeout(() => { if (middle) { middle = false; send('paste', 'primary'); } });
  }, true);
  addEventListener('keydown', () => { middle = false; }, true);
  addEventListener('paste', (e) => {
    if (!e.isTrusted || own.has(e)) return;
    e.preventDefault();
    e.stopImmediatePropagation();
    send('paste', middle ? 'primary' : '');
    middle = false;
  }, true);
  let primaryTimer = 0;
  document.addEventListener('selectionchange', () => {
    clearTimeout(primaryTimer);
    primaryTimer = setTimeout(() => {
      const text = selected();
      if (text) send('primary', text);
    }, 60);
  }, true);
  const proto = window.Clipboard && Clipboard.prototype;
  if (proto && proto.writeText) {
    const writeText = proto.writeText;
    proto.writeText = function (text) {
      return writeText.call(this, text).then((v) => { send('copy', String(text)); return v; });
    };
  }
  if (proto && proto.readText) {
    const readText = proto.readText;
    proto.readText = function () {
      return readText.call(this).then(() => new Promise((resolve) => {
        const id = nextRead++;
        reads.set(id, resolve);
        send('read', String(id));
      }));
    };
  }
  return function (text, id) {
    if (id) {
      const resolve = reads.get(Number(id));
      reads.delete(Number(id));
      if (resolve) resolve(text);
      return;
    }
    const target = document.activeElement || document.body;
    if (!target || !text) return;
    const data = new DataTransfer();
    data.setData('text/plain', text);
    const event = new ClipboardEvent('paste', { clipboardData: data, bubbles: true, cancelable: true, composed: true });
    own.add(event);
    if (target.dispatchEvent(event)) document.execCommand('insertText', false, text);
  };
})"#;

#[cfg(feature = "cef-runtime")]
pub(crate) mod renderer {
    use std::cell::RefCell;
    use std::collections::HashMap;

    use cef::*;

    use super::{ANSWER, HOOK, REQUEST, SWITCH};

    thread_local! {
        /// Each frame's `answer` function, by frame identifier (renderer main thread).
        static ANSWERS: RefCell<HashMap<String, cef::V8Value>> = RefCell::new(HashMap::new());
    }

    pub(crate) fn enabled() -> bool {
        let name: cef::CefString = SWITCH.into();
        cef::command_line_get_global().is_some_and(|c| c.has_switch(Some(&name)) != 0)
    }

    fn frame_id(frame: &cef::Frame) -> String {
        cef::CefString::from(&frame.identifier()).to_string()
    }

    cef::wrap_v8_handler! {
        struct Send {}

        impl V8Handler {
            fn execute(
                &self,
                _name: Option<&cef::CefString>,
                _object: Option<&mut cef::V8Value>,
                arguments: Option<&[Option<cef::V8Value>]>,
                _retval: Option<&mut Option<cef::V8Value>>,
                _exception: Option<&mut cef::CefString>,
            ) -> ::std::os::raw::c_int {
                let text = |i: usize| {
                    arguments
                        .and_then(|a| a.get(i))
                        .and_then(|v| v.as_ref())
                        .filter(|v| v.is_string() != 0)
                        .map(|v| cef::CefString::from(&v.string_value()).to_string())
                        .unwrap_or_default()
                };
                let Some(frame) = cef::v8_context_get_current_context().and_then(|c| c.frame()) else {
                    return 1;
                };
                let name: cef::CefString = REQUEST.into();
                if let Some(mut message) = cef::process_message_create(Some(&name)) {
                    if let Some(list) = message.argument_list() {
                        for (i, value) in [text(0), text(1)].iter().enumerate() {
                            let value: cef::CefString = value.as_str().into();
                            list.set_string(i, Some(&value));
                        }
                    }
                    frame.send_process_message(cef::ProcessId::BROWSER, Some(&mut message));
                }
                1
            }
        }
    }

    /// Installs the hook in a new main-world context.
    pub(crate) fn context_created(frame: &cef::Frame, context: &cef::V8Context) {
        if !enabled() || context.enter() == 0 {
            return;
        }
        let code: cef::CefString = HOOK.into();
        let url: cef::CefString = "weld://clipboard".into();
        let mut hook: Option<cef::V8Value> = None;
        let mut exception: Option<cef::V8Exception> = None;
        let ran = context.eval(Some(&code), Some(&url), 0, Some(&mut hook), Some(&mut exception));
        let name: cef::CefString = "send".into();
        let mut handler = Send::new();
        let send = cef::v8_value_create_function(Some(&name), Some(&mut handler));
        if ran != 0
            && let Some(hook) = hook.filter(|h| h.is_function() != 0)
            && let Some(answer) = hook
                .execute_function(None, Some(&[send]))
                .filter(|a| a.is_function() != 0)
        {
            ANSWERS.with(|a| a.borrow_mut().insert(frame_id(frame), answer));
        }
        context.exit();
    }

    pub(crate) fn context_released(frame: &cef::Frame) {
        ANSWERS.with(|a| a.borrow_mut().remove(&frame_id(frame)));
    }

    /// The host's answer to a `paste` or `read` request. Returns whether the message was one.
    pub(crate) fn answer(frame: &cef::Frame, message: &cef::ProcessMessage) -> bool {
        if cef::CefString::from(&message.name()).to_string() != ANSWER {
            return false;
        }
        let Some(args) = message.argument_list() else {
            return true;
        };
        let text = cef::CefString::from(&args.string(0)).to_string();
        let id = cef::CefString::from(&args.string(1)).to_string();
        let Some(answer) = ANSWERS.with(|a| a.borrow().get(&frame_id(frame)).cloned()) else {
            return true;
        };
        let Some(context) = frame.v8_context() else {
            return true;
        };
        if context.enter() != 0 {
            let text: cef::CefString = text.as_str().into();
            let id: cef::CefString = id.as_str().into();
            let args = [cef::v8_value_create_string(Some(&text)), cef::v8_value_create_string(Some(&id))];
            answer.execute_function(None, Some(&args));
            context.exit();
        }
        true
    }
}

/// Browser side: sends the host's answer to the frame that asked.
#[cfg(feature = "cef-runtime")]
pub fn answer_frame(browser: &cef::Browser, frame: &str, text: &str, read: Option<u32>) -> bool {
    use cef::{ImplBrowser, ImplFrame, ImplListValue, ImplProcessMessage};
    let id: cef::CefString = frame.into();
    let Some(frame) = browser.frame_by_identifier(Some(&id)) else {
        return false;
    };
    let name: cef::CefString = ANSWER.into();
    let Some(mut message) = cef::process_message_create(Some(&name)) else {
        return false;
    };
    if let Some(list) = message.argument_list() {
        let text: cef::CefString = text.into();
        let read: cef::CefString = read.map(|r| r.to_string()).unwrap_or_default().as_str().into();
        list.set_string(0, Some(&text));
        list.set_string(1, Some(&read));
    }
    frame.send_process_message(cef::ProcessId::RENDERER, Some(&mut message));
    true
}
