//! wubble change 50: CEF's accessibility handler for windowless browsers. With
//! `BrowserHost::set_accessibility_state(ENABLED)` (the host turns it on;
//! nothing here does) CEF runs in TreeOnly mode: the renderer computes the
//! whole tree and hands its updates and location changes to this handler as
//! `cef::Value` trees, on the browser's UI thread. Each one is written out as
//! JSON (`cef_write_json`) and queued as a navigation event, so welding names
//! no accessibility type of its own and the host maps the values itself.

use std::sync::Arc;

use cef::*;

use crate::surface::{CefSurfaceEvent, NavigationEvent, WebEventQueue};

cef::wrap_accessibility_handler! {
    pub(crate) struct WeldAccessibilityHandler {
        events: Arc<WebEventQueue>,
    }

    impl AccessibilityHandler {
        fn on_accessibility_tree_change(&self, value: Option<&mut cef::Value>) {
            if let Some(json) = json(value) {
                self.events
                    .push(CefSurfaceEvent::Navigation(NavigationEvent::AccessibilityTree { json }));
            }
        }

        fn on_accessibility_location_change(&self, value: Option<&mut cef::Value>) {
            if let Some(json) = json(value) {
                self.events.push(CefSurfaceEvent::Navigation(
                    NavigationEvent::AccessibilityLocation { json },
                ));
            }
        }
    }
}

/// The handler every platform's render handler returns.
pub(crate) fn handler(events: &Arc<WebEventQueue>) -> cef::AccessibilityHandler {
    WeldAccessibilityHandler::new(events.clone())
}

fn json(value: Option<&mut cef::Value>) -> Option<String> {
    let value = value?;
    let text = crate::handlers::cef_string(cef::write_json(
        Some(value),
        cef::JsonWriterOptions::default(),
    ));
    (!text.is_empty()).then_some(text)
}
