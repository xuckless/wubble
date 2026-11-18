// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! welding's `CefApp`: the renderer-side half of the runtime.
//!
//! CEF splits work across processes, and a `CefApp` is the only hook that
//! exists in *both*. Without one, a host is confined to what the browser
//! process can answer on its own, which is why script results were previously
//! impossible rather than merely unwired: JavaScript evaluates in the renderer,
//! and nothing was listening there.
//!
//! The app is constructed identically in the browser process (passed to
//! `cef_initialize`) and in every subprocess (passed to `cef_execute_process`),
//! because CEF hands the same object to both roles and takes the handlers each
//! one needs.

use std::sync::{Arc, Mutex};

use crate::{
    error::WeldError,
    surface::{CefSurfaceEvent, WebEventQueue, WebRequestId},
};

/// Accepted script requests that have not settled yet.
#[derive(Debug)]
pub(crate) struct PendingScripts {
    ids: Mutex<Vec<WebRequestId>>,
    events: Arc<WebEventQueue>,
}

impl PendingScripts {
    pub(crate) fn new(events: Arc<WebEventQueue>) -> Self {
        Self {
            ids: Mutex::new(Vec::new()),
            events,
        }
    }

    pub(crate) fn begin(&self, id: WebRequestId) -> Result<(), WeldError> {
        let mut ids = self.ids.lock().unwrap();
        if ids.contains(&id) {
            return Err(WeldError::BrowserOp(
                "that script request id is already in flight".into(),
            ));
        }
        ids.push(id);
        Ok(())
    }

    /// Retract a request when a platform reports synchronous send refusal.
    /// Windows' cef-rs binding exposes this send as unit, while Linux and
    /// macOS preserve CEF's integer acceptance result.
    #[allow(dead_code)]
    pub(crate) fn abort(&self, id: WebRequestId) {
        let mut ids = self.ids.lock().unwrap();
        if let Some(index) = ids.iter().position(|pending| *pending == id) {
            ids.remove(index);
        }
    }

    pub(crate) fn complete(&self, id: WebRequestId, result: Result<String, String>) {
        let accepted = {
            let mut ids = self.ids.lock().unwrap();
            ids.iter()
                .position(|pending| *pending == id)
                .map(|index| ids.remove(index))
                .is_some()
        };
        if accepted {
            self.events
                .push(CefSurfaceEvent::ScriptCompleted { id, result });
        }
    }

    pub(crate) fn fail_all(&self, reason: &str) {
        let ids = std::mem::take(&mut *self.ids.lock().unwrap());
        for id in ids {
            self.events.push(CefSurfaceEvent::ScriptCompleted {
                id,
                result: Err(reason.to_owned()),
            });
        }
    }
}

/// Message names on the browser/renderer channel.
pub(crate) const EVAL_REQUEST: &str = "weld.eval";
pub(crate) const EVAL_RESULT: &str = "weld.eval.result";
/// A context script's message, both ways (renderer `send`, browser
/// `send_frame_message`). wubble change, see `WUBBLE_CHANGES.md` (26).
pub(crate) const FRAME_MESSAGE: &str = "wubble.frame";
/// The names of the two native functions handed to a context script.
const NATIVE_SEND: &str = "send";
const NATIVE_REGISTER: &str = "register";

/// Wrap the caller's script so the renderer always returns a JSON string.
///
/// The indirection through a function keeps a bare expression working (`2+2`)
/// alongside statements, and `JSON.stringify(undefined)` is itself `undefined`
/// rather than a string, so that case is normalised to JSON `null`.
pub(crate) fn wrap_script(script: &str) -> String {
    format!(
        "(function() {{ var __weld = (function() {{ return ({script}); }})(); \
         return __weld === undefined ? \"null\" : JSON.stringify(__weld); }})()"
    )
}

#[cfg(feature = "cef-runtime")]
mod cef_backed {
    use super::{EVAL_REQUEST, EVAL_RESULT, FRAME_MESSAGE, NATIVE_REGISTER, NATIVE_SEND};
    use cef::*;
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::sync::Arc;

    // wubble change (26): a context script in every frame.
    //
    // The script is a function expression, evaluated in each frame's V8
    // context as it is created (before the page's own scripts run) and called
    // once with two native functions: `send(text)` posts a `FRAME_MESSAGE` to
    // the browser from that frame, `register(fn)` names the function the
    // browser's `FRAME_MESSAGE`s are handed to. Neither is put on `window`,
    // so page code has no name to reach them by; the script keeps them in its
    // closure. This is closure capture, not an isolated world (CEF 156 has
    // none): the page shares the heap and the DOM with the script, and can
    // patch builtins the script did not capture first. The browser takes the
    // frame and its URL from CEF, not from the text, so a page can only ever
    // speak for its own frame. Pattern: cefclient `client_renderer.cc`
    // `OnContextCreated` with a `CefV8Handler` (BSD-3-Clause), as cef-rs's
    // `wrapper/message_router.rs` does it (MIT OR Apache-2.0).
    thread_local! {
        /// Per frame id: the context the script ran in and the function it registered.
        static REGISTERED: RefCell<HashMap<String, (cef::V8Context, cef::V8Value)>> =
            RefCell::new(HashMap::new());
    }

    wrap_v8_handler! {
        struct ContextNatives {}

        impl V8Handler {
            fn execute(
                &self,
                name: Option<&CefString>,
                _object: Option<&mut V8Value>,
                arguments: Option<&[Option<V8Value>]>,
                _retval: Option<&mut Option<V8Value>>,
                exception: Option<&mut CefString>,
            ) -> i32 {
                let name = name.map(|n| n.to_string()).unwrap_or_default();
                let arg = arguments.and_then(|a| a.first()).and_then(Option::as_ref);
                let Some(ctx) = v8_context_get_current_context() else {
                    return 0;
                };
                let Some(frame) = ctx.frame() else {
                    return 0;
                };
                let fail = |exception: Option<&mut CefString>, why: &str| {
                    if let Some(e) = exception {
                        *e = CefString::from(why);
                    }
                    1
                };
                match name.as_str() {
                    NATIVE_SEND => {
                        let Some(text) = arg.filter(|v| v.is_string() != 0) else {
                            return fail(exception, "send takes a string");
                        };
                        let text = CefString::from(&text.string_value()).to_string();
                        if text.len() > crate::surface::MAX_FRAME_MESSAGE_BYTES {
                            return 1;
                        }
                        let name: CefString = FRAME_MESSAGE.into();
                        if let Some(mut message) = process_message_create(Some(&name)) {
                            if let Some(list) = message.argument_list() {
                                let text: CefString = text.as_str().into();
                                list.set_string(0, Some(&text));
                            }
                            frame.send_process_message(ProcessId::BROWSER, Some(&mut message));
                        }
                        1
                    }
                    NATIVE_REGISTER => {
                        let Some(function) = arg.filter(|v| v.is_function() != 0) else {
                            return fail(exception, "register takes a function");
                        };
                        let id = crate::handlers::cef_string(frame.identifier());
                        REGISTERED.with(|r| r.borrow_mut().insert(id, (ctx, function.clone())));
                        1
                    }
                    _ => 0,
                }
            }
        }
    }

    /// Runs the context script in a new frame context: evaluates it to its
    /// function and calls that with the natives.
    fn run_context_script(script: &str, ctx: &mut V8Context) {
        let code: CefString = script.into();
        let url: CefString = "wubble://context-script".into();
        let mut retval: Option<V8Value> = None;
        let mut exception: Option<V8Exception> = None;
        let ran = ctx.eval(Some(&code), Some(&url), 1, Some(&mut retval), Some(&mut exception));
        let Some(function) = retval.filter(|v| ran != 0 && v.is_function() != 0) else {
            return;
        };
        let mut natives = ContextNatives::new();
        let send: CefString = NATIVE_SEND.into();
        let register: CefString = NATIVE_REGISTER.into();
        let args = [
            v8_value_create_function(Some(&send), Some(&mut natives)),
            v8_value_create_function(Some(&register), Some(&mut natives)),
        ];
        function.execute_function_with_context(Some(ctx), None, Some(&args));
    }

    /// The browser's message for the script in `frame`: handed to the
    /// function it registered, inside its context.
    fn deliver(frame: &Frame, text: &str) {
        let id = crate::handlers::cef_string(frame.identifier());
        let Some((mut ctx, function)) = REGISTERED.with(|r| r.borrow().get(&id).cloned()) else {
            return;
        };
        if ctx.enter() == 0 {
            return;
        }
        let text: CefString = text.into();
        let arg = [v8_value_create_string(Some(&text))];
        function.execute_function_with_context(Some(&mut ctx), None, Some(&arg));
        ctx.exit();
    }

    cef::wrap_render_process_handler! {
        pub(crate) struct WeldRenderProcessHandler {
            context_script: Option<&'static str>,
        }

        impl RenderProcessHandler {
            fn on_context_created(
                &self,
                _browser: Option<&mut Browser>,
                _frame: Option<&mut Frame>,
                context: Option<&mut V8Context>,
            ) {
                if let (Some(script), Some(ctx)) = (self.context_script, context) {
                    run_context_script(script, ctx);
                }
            }

            fn on_context_released(
                &self,
                _browser: Option<&mut Browser>,
                frame: Option<&mut Frame>,
                context: Option<&mut V8Context>,
            ) {
                let (Some(frame), Some(ctx)) = (frame, context) else {
                    return;
                };
                let id = crate::handlers::cef_string(frame.identifier());
                REGISTERED.with(|r| {
                    let mut r = r.borrow_mut();
                    if r.get(&id).is_some_and(|(c, _)| c.is_same(Some(&mut *ctx)) != 0) {
                        r.remove(&id);
                    }
                });
            }

            fn on_process_message_received(
                &self,
                _browser: Option<&mut cef::Browser>,
                frame: Option<&mut cef::Frame>,
                _source_process: cef::ProcessId,
                message: Option<&mut cef::ProcessMessage>,
            ) -> ::std::os::raw::c_int {
                let (Some(frame), Some(message)) = (frame, message) else {
                    return 0;
                };
                let name = cef::CefString::from(&message.name()).to_string();
                if name == FRAME_MESSAGE {
                    if let Some(args) = message.argument_list() {
                        let text = cef::CefString::from(&args.string(0)).to_string();
                        deliver(frame, &text);
                    }
                    return 1;
                }
                if name != EVAL_REQUEST {
                    return 0;
                }
                let Some(args) = message.argument_list() else {
                    return 0;
                };
                let id = cef::CefString::from(&args.string(0)).to_string();
                let script = cef::CefString::from(&args.string(1)).to_string();

                // Evaluation has to happen inside the frame's V8 context, and
                // that context only exists here, in the renderer process.
                let (ok, payload) = match frame.v8_context() {
                    Some(ctx) => {
                        let code: cef::CefString = super::wrap_script(&script).as_str().into();
                        let url: cef::CefString = "weld://eval".into();
                        let mut retval: Option<cef::V8Value> = None;
                        let mut exception: Option<cef::V8Exception> = None;
                        let ran = ctx.eval(
                            Some(&code),
                            Some(&url),
                            0,
                            Some(&mut retval),
                            Some(&mut exception),
                        );
                        if ran != 0 {
                            let value = retval
                                .as_ref()
                                .filter(|v| v.is_string() != 0)
                                .map(|v| cef::CefString::from(&v.string_value()).to_string())
                                .unwrap_or_else(|| "null".to_owned());
                            (1, value)
                        } else {
                            let message = exception
                                .as_ref()
                                .map(|e| cef::CefString::from(&e.message()).to_string())
                                .unwrap_or_else(|| "script evaluation failed".to_owned());
                            (0, message)
                        }
                    }
                    None => (0, "frame has no V8 context".to_owned()),
                };

                let name: cef::CefString = EVAL_RESULT.into();
                if let Some(mut reply) = cef::process_message_create(Some(&name)) {
                    if let Some(list) = reply.argument_list() {
                        let id: cef::CefString = id.as_str().into();
                        list.set_string(0, Some(&id));
                        list.set_int(1, ok);
                        let payload: cef::CefString = payload.as_str().into();
                        list.set_string(2, Some(&payload));
                    }
                    frame.send_process_message(cef::ProcessId::BROWSER, Some(&mut reply));
                }
                1
            }
        }
    }

    // wubble change: forwards the external-pump schedule to the host.
    cef::wrap_browser_process_handler! {
        pub(crate) struct WeldBrowserProcessHandler {
            scheduler: crate::runtime::PumpScheduler,
            // wubble change (29): switches only child processes get.
            child_switches: Arc<Vec<String>>,
        }

        impl BrowserProcessHandler {
            fn on_schedule_message_pump_work(&self, delay_ms: i64) {
                (self.scheduler.0)(delay_ms);
            }

            fn on_before_child_process_launch(&self, command_line: Option<&mut cef::CommandLine>) {
                use cef::ImplCommandLine;
                let Some(cmd) = command_line else { return };
                for name in self.child_switches.iter() {
                    let name: cef::CefString = name.as_str().into();
                    cmd.append_switch(Some(&name));
                }
            }
        }
    }

    cef::wrap_app! {
        pub(crate) struct WeldApp {
            switches: Arc<Vec<(String, Option<String>)>>,
            render_process_handler: cef::RenderProcessHandler,
            browser_process_handler: Option<cef::BrowserProcessHandler>,
        }

        impl App {
            fn render_process_handler(&self) -> Option<cef::RenderProcessHandler> {
                Some(self.render_process_handler.clone())
            }

            fn browser_process_handler(&self) -> Option<cef::BrowserProcessHandler> {
                self.browser_process_handler.clone()
            }

            fn on_before_command_line_processing(
                &self,
                _process_type: Option<&cef::CefString>,
                command_line: Option<&mut cef::CommandLine>,
            ) {
                use cef::ImplCommandLine;
                let Some(cmd) = command_line else {
                    return;
                };
                for (name, value) in self.switches.iter() {
                    let name: cef::CefString = name.as_str().into();
                    match value {
                        Some(value) => {
                            let value: cef::CefString = value.as_str().into();
                            cmd.append_switch_with_value(Some(&name), Some(&value));
                        }
                        None => cmd.append_switch(Some(&name)),
                    }
                }
            }
        }
    }

    impl WeldApp {
        /// `scheduler` is `Some` only in the browser process; `context_script`
        /// only matters in a renderer (wubble change 26); `child_switches` go
        /// on every child process's command line (wubble change 29).
        pub(crate) fn build(
            switches: Arc<Vec<(String, Option<String>)>>,
            scheduler: Option<crate::runtime::PumpScheduler>,
            context_script: Option<&'static str>,
            child_switches: Arc<Vec<String>>,
        ) -> cef::App {
            Self::new(
                switches,
                WeldRenderProcessHandler::new(context_script),
                scheduler.map(|s| WeldBrowserProcessHandler::new(s, child_switches)),
            )
        }
    }
}

#[cfg(feature = "cef-runtime")]
pub(crate) use cef_backed::WeldApp;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bare_expression_survives_wrapping() {
        assert!(wrap_script("2+2").contains("return (2+2)"));
    }

    #[test]
    fn undefined_is_normalised_to_json_null() {
        // JSON.stringify(undefined) is undefined, not a string, so without this
        // the renderer would have nothing to send back.
        assert!(wrap_script("whatever").contains("__weld === undefined"));
    }

    #[test]
    fn accepted_scripts_settle_once_and_duplicate_ids_are_refused() {
        let events = Arc::new(WebEventQueue::default());
        let scripts = PendingScripts::new(events.clone());
        let id = WebRequestId::new(u64::MAX);
        scripts.begin(id).unwrap();
        assert!(scripts.begin(id).is_err());
        scripts.complete(id, Ok("4".into()));
        scripts.complete(id, Ok("duplicate".into()));
        assert!(matches!(
            events.poll(),
            Some(CefSurfaceEvent::ScriptCompleted { id: seen, result: Ok(value) })
                if seen == id && value == "4"
        ));
        assert!(events.poll().is_none());
    }

    #[test]
    fn renderer_loss_fails_pending_scripts_in_acceptance_order() {
        let events = Arc::new(WebEventQueue::default());
        let scripts = PendingScripts::new(events.clone());
        scripts.begin(WebRequestId::new(8)).unwrap();
        scripts.begin(WebRequestId::new(3)).unwrap();
        scripts.fail_all("renderer exited");
        for expected in [8, 3] {
            assert!(matches!(
                events.poll(),
                Some(CefSurfaceEvent::ScriptCompleted { id, result: Err(reason) })
                    if id.get() == expected && reason == "renderer exited"
            ));
        }
        assert!(events.poll().is_none());
    }

    #[test]
    fn synchronous_send_refusal_emits_no_completion() {
        let events = Arc::new(WebEventQueue::default());
        let scripts = PendingScripts::new(events.clone());
        let id = WebRequestId::new(21);
        scripts.begin(id).unwrap();
        scripts.abort(id);
        scripts.fail_all("renderer exited");
        assert!(events.poll().is_none());
        scripts.begin(id).unwrap();
    }
}
