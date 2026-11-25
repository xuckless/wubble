//! The process's `NSApplication`, as CEF needs it on macOS.
//!
//! Ported from cef-rs `examples/cefsimple/src/mac/mod.rs` (`SimpleApplication`,
//! `setup_simple_application`; tauri-apps/cef-rs commit b84b7caa, MIT OR Apache-2.0); CEF's
//! `cefclient` `SimpleApplication` is the original (BSD-3-Clause). Chromium's nested run loops
//! (a DevTools or other Chrome-style window, a native dialog) ask the application
//! `isHandlingSendEvent`; a plain `NSApplication` raises "unrecognized selector" and the
//! process dies. winit 0.30 swizzles `sendEvent:` on whatever class `sharedApplication` made, so
//! creating ours first keeps both working.

use cef::application_mac::{CefAppProtocol, CrAppControlProtocol, CrAppProtocol};
use objc2::{
    ClassType, DefinedClass, MainThreadMarker, define_class, extern_methods, msg_send,
    rc::Retained,
    runtime::{AnyObject, Bool, NSObjectProtocol},
};
use objc2_app_kit::{NSApp, NSApplication, NSEvent, NSEventType};
use std::cell::Cell;
use std::sync::Mutex;

static QUIT_HANDLER: Mutex<Option<fn()>> = Mutex::new(None);
static QUIT_KEY_HANDLER: Mutex<Option<fn()>> = Mutex::new(None);

/// Registers or clears the callback invoked by `-[WubbleApplication terminate:]` instead of
/// AppKit's default `exit(0)`, so `Cmd+Q` / Dock Quit runs orderly CEF shutdown.
pub fn set_quit_handler(handler: Option<fn()>) {
    if let Ok(mut slot) = QUIT_HANDLER.lock() {
        *slot = handler;
    }
}

/// Registers or clears the callback a `terminate:` sent by the keys (Cmd+Q, a key-down event is
/// the application's current one) runs instead of the quit handler: "Warn before quitting" asks
/// for a second press there. Chrome tells the keys from the menu and the Dock the same way
/// (`ConfirmQuitPanelController +eventTriggersFeature:`, BSD-3-Clause).
pub fn set_quit_key_handler(handler: Option<fn()>) {
    if let Ok(mut slot) = QUIT_KEY_HANDLER.lock() {
        *slot = handler;
    }
}

#[derive(Default)]
pub struct Ivars {
    handling_send_event: Cell<Bool>,
}

define_class!(
    /// An `NSApplication` that tells Chromium whether it is inside `sendEvent:`.
    #[unsafe(super(NSApplication))]
    #[ivars = Ivars]
    pub struct WubbleApplication;

    impl WubbleApplication {
        #[unsafe(method(sendEvent:))]
        unsafe fn send_event(&self, event: &NSEvent) {
            let was = self.is_handling_send_event();
            if !was {
                self.set_handling_send_event(true);
            }
            let _: () = msg_send![super(self), sendEvent: event];
            if !was {
                self.set_handling_send_event(false);
            }
        }

        // `terminate:` is Cocoa's one entry for an orderly quit (the Quit item and Cmd+Q, the
        // Dock, Activity Monitor's Quit, logout, restart and shut down). AppKit's own ends the
        // process with exit() inside the run loop, so Chromium could not shut down; as cefclient
        // (cef `tests/cefclient/cefclient_mac.mm`, `-[ClientApplication terminate:]`) and Chrome
        // (`chrome/browser/chrome_browser_application_mac.mm`, `-[BrowserCrApplication
        // terminate:]`) do, it starts the browser's own close and returns: "Return, don't exit.
        // The application is responsible for exiting on its own." The handler ends the event
        // loop (winit stops `NSApp` with `stop:`), the host drains CEF and `main` returns, which
        // ends the process; a logout waits for that exit, as it does for Chrome. Without a
        // handler (before the loop runs, after the drain), AppKit's own terminate runs.
        #[unsafe(method(terminate:))]
        unsafe fn terminate(&self, sender: Option<&AnyObject>) {
            let by_keys = self
                .currentEvent()
                .is_some_and(|event| event.r#type() == NSEventType::KeyDown);
            let key_handler = QUIT_KEY_HANDLER.lock().ok().and_then(|g| *g);
            let handler = QUIT_HANDLER.lock().ok().and_then(|g| *g);
            if let (true, Some(_), Some(keys)) = (by_keys, handler, key_handler) {
                keys();
            } else if let Some(quit) = handler {
                quit();
            } else {
                let _: () = msg_send![super(self), terminate: sender];
            }
        }
    }

    unsafe impl CrAppControlProtocol for WubbleApplication {
        #[unsafe(method(setHandlingSendEvent:))]
        unsafe fn _set_handling_send_event(&self, handling: Bool) {
            self.ivars().handling_send_event.set(handling);
        }
    }

    unsafe impl CrAppProtocol for WubbleApplication {
        #[unsafe(method(isHandlingSendEvent))]
        unsafe fn _is_handling_send_event(&self) -> Bool {
            self.ivars().handling_send_event.get()
        }
    }

    unsafe impl CefAppProtocol for WubbleApplication {}
);

impl WubbleApplication {
    extern_methods! {
        #[unsafe(method(sharedApplication))]
        fn shared_application() -> Retained<Self>;

        #[unsafe(method(setHandlingSendEvent:))]
        fn set_handling_send_event(&self, handling: bool);

        #[unsafe(method(isHandlingSendEvent))]
        fn is_handling_send_event(&self) -> bool;
    }
}

/// Makes the shared application a [`WubbleApplication`]. Before anything else touches
/// `NSApp` (winit's event loop, `cef::initialize`); main thread only.
pub fn install() {
    let Some(mtm) = MainThreadMarker::new() else {
        eprintln!("wubble-engine: NSApplication must be set up on the main thread");
        return;
    };
    let _ = WubbleApplication::shared_application();
    if !NSApp(mtm).isKindOfClass(WubbleApplication::class()) {
        eprintln!("wubble-engine: NSApplication was created before wubble's; DevTools may crash");
    }
}
