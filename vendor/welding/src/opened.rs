// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! wubble change 57: windows a page opens (`window.open`, `target=_blank`)
//! that CEF creates as windowless browsers of their own, so the opener
//! relation holds: `window.opener`, the handle `window.open` returns, named
//! targets, `postMessage` both ways and `window.close()`.
//!
//! The shape is cefclient's off-screen path (chromiumembedded/cef,
//! BSD-3-Clause; no code copied): `ClientHandler::OnBeforePopup` makes the
//! new browser windowless and gives it its own client, then returns false so
//! Chromium creates it beside its opener; `OnAfterCreated` delivers it and
//! `RootWindowManager::CreateRootWindowAsPopup` adopts it. Here the opener's
//! [`PopupFactory`] decides per request (no config: welding cancels it and
//! reports `NavigationEvent::NewWindowRequested`, as before); the new
//! browser's producer waits in its opener's [`Children`] until the host
//! takes it (`CefSurfaceProducer::take_popup_browser`), announced by
//! `NavigationEvent::PopupBrowserCreated`.

use std::sync::{Arc, Mutex, PoisonError};

use crate::surface::{CefSurfaceConfig, NewWindowKind};

/// What a page asked for when it opened a window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenedWindow {
    /// CEF's id for this pop-up, unique per opener.
    pub popup_id: i32,
    /// The URL it opens; empty for `window.open('')` (the page writes or
    /// navigates it through the handle it got).
    pub url: String,
    /// The `window.open` target name (`_blank` for none).
    pub target: String,
    pub kind: NewWindowKind,
    pub user_gesture: bool,
    /// The `width`/`height` window features, when both were given (CSS px).
    pub size: Option<(u32, u32)>,
    /// The `left`/`top` window features, when both were given (screen CSS px).
    pub position: Option<(i32, i32)>,
}

/// Decides, on CEF's UI thread inside `OnBeforePopup`, whether a request
/// becomes a CEF browser: the new browser's config (its size, frame timing,
/// request filter, navigation guard, pop-up factory), or `None` to cancel it
/// and report it as `NavigationEvent::NewWindowRequested`. The config's
/// `initial_url`, `user_data_dir` and `profile` are ignored: Chromium opens
/// the target itself, in the opener's request context. It must not panic
/// (a panic cancels).
#[derive(Clone)]
pub struct PopupFactory(Arc<dyn Fn(&OpenedWindow) -> Option<CefSurfaceConfig> + Send + Sync>);

impl PopupFactory {
    pub fn new(
        factory: impl Fn(&OpenedWindow) -> Option<CefSurfaceConfig> + Send + Sync + 'static,
    ) -> Self {
        Self(Arc::new(factory))
    }

    /// The new browser's config, or `None` (cancel). A panicking factory cancels.
    pub fn config(&self, opened: &OpenedWindow) -> Option<CefSurfaceConfig> {
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| (self.0)(opened)))
            .ok()
            .flatten()
    }
}

impl std::fmt::Debug for PopupFactory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PopupFactory")
    }
}

/// The pop-up browsers CEF created for one opener, until the host takes
/// them. Once the opener closes, nothing more is taken: a late arrival is
/// handed back to be closed.
pub(crate) struct Children<P> {
    inner: Mutex<(bool, Vec<(i32, P)>)>,
}

impl<P> Default for Children<P> {
    fn default() -> Self {
        Self {
            inner: Mutex::new((false, Vec::new())),
        }
    }
}

impl<P> Children<P> {
    /// Keeps `child` for the host; `Err(child)` once the opener closed.
    pub(crate) fn put(&self, popup_id: i32, child: P) -> Result<(), P> {
        let mut inner = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        if inner.0 {
            return Err(child);
        }
        inner.1.push((popup_id, child));
        Ok(())
    }

    pub(crate) fn take(&self, popup_id: i32) -> Option<P> {
        let mut inner = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        let at = inner.1.iter().position(|(id, _)| *id == popup_id)?;
        Some(inner.1.remove(at).1)
    }

    /// The opener closed: the children nobody took, to be closed.
    pub(crate) fn close(&self) -> Vec<P> {
        let mut inner = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        inner.0 = true;
        std::mem::take(&mut inner.1).into_iter().map(|(_, p)| p).collect()
    }
}

/// What the new browser's own life-span handler needs to hand it over in
/// `OnAfterCreated`: its opener's children and event queue, and the request.
#[cfg(feature = "cef-runtime")]
pub(crate) struct Adoption<P> {
    pub(crate) children: Arc<Children<P>>,
    pub(crate) events: Arc<crate::surface::WebEventQueue>,
    pub(crate) opened: OpenedWindow,
}

/// The request as welding reports it, from `OnBeforePopup`'s arguments.
#[cfg(feature = "cef-runtime")]
pub(crate) fn opened_window(
    popup_id: i32,
    url: Option<&cef::CefString>,
    target: Option<&cef::CefString>,
    disposition: cef::WindowOpenDisposition,
    user_gesture: bool,
    features: Option<&cef::PopupFeatures>,
) -> OpenedWindow {
    let position = features.and_then(|f| (f.x_set != 0 && f.y_set != 0).then_some((f.x, f.y)));
    OpenedWindow {
        popup_id,
        url: url.map(|u| u.to_string()).unwrap_or_default(),
        target: target.map(|t| t.to_string()).unwrap_or_default(),
        kind: crate::handlers::new_window_kind(disposition),
        user_gesture,
        size: features.and_then(crate::handlers::popup_size),
        position,
    }
}

/// Makes the pop-up CEF is about to create a windowless browser painted
/// like its opener's (`config`'s frame timing and background).
#[cfg(feature = "cef-runtime")]
pub(crate) fn make_windowless(
    config: &CefSurfaceConfig,
    window_info: &mut cef::WindowInfo,
    settings: &mut cef::BrowserSettings,
) {
    window_info.windowless_rendering_enabled = 1;
    window_info.shared_texture_enabled = 1;
    window_info.external_begin_frame_enabled = config.external_begin_frame.into();
    settings.windowless_frame_rate = config.cef_windowless_frame_rate();
    settings.background_color = config.cef_background_color();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn children_are_taken_once_and_refused_after_the_close() {
        let children = Children::default();
        assert!(children.put(1, "a").is_ok());
        assert!(children.put(2, "b").is_ok());
        assert_eq!(children.take(3), None);
        assert_eq!(children.take(2), Some("b"));
        assert_eq!(children.take(2), None);
        assert_eq!(children.close(), vec!["a"], "what nobody took is closed");
        assert_eq!(children.put(4, "c"), Err("c"), "a late one is handed back");
        assert_eq!(children.take(1), None);
    }

    #[test]
    fn a_panicking_factory_cancels() {
        let opened = OpenedWindow {
            popup_id: 1,
            url: String::new(),
            target: "_blank".into(),
            kind: NewWindowKind::Popup,
            user_gesture: true,
            size: None,
            position: None,
        };
        let panics = PopupFactory::new(|_| panic!("no"));
        assert!(panics.config(&opened).is_none());
        let some = PopupFactory::new(|o| o.user_gesture.then(CefSurfaceConfig::default));
        assert!(some.config(&opened).is_some());
    }
}
