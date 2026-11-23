// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

use std::{collections::VecDeque, sync::{Arc, Mutex}};

use dpi::PhysicalSize;

use crate::{
    auth::AuthId,
    downloads::DownloadId,
    error::WeldError,
    held::RequestId,
    native_frame::{HostWgpuContext, ImportedTexture, NativeFrame},
    permissions::{PermissionId, PermissionKind},
};

/// Opaque identity for one Chromium PNG snapshot request.
///
/// A request ID is minted by [`CefSurfaceProducer::request_snapshot_png`] and
/// returned unchanged by [`CefSurfaceProducer::poll_snapshot_png`]. Keep it
/// with host-side capture context rather than inferring provenance from the
/// order Chromium happened to finish captures.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SnapshotRequestId(i32);

#[cfg(feature = "cef-runtime")]
impl SnapshotRequestId {
    pub(crate) const fn from_cef_message_id(id: i32) -> Self {
        Self(id)
    }

    pub(crate) const fn cef_message_id(self) -> i32 {
        self.0
    }
}

impl std::fmt::Display for SnapshotRequestId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(formatter)
    }
}

/// A completed PNG snapshot, paired with the request that caused it.
#[derive(Debug)]
pub struct SnapshotPngCompletion {
    /// The request identity returned when Chromium accepted the capture.
    pub id: SnapshotRequestId,
    /// Validated PNG bytes, or Chromium's capture/encoding error.
    pub result: Result<Vec<u8>, WeldError>,
}

/// Caller-minted identity for one asynchronous web operation.
///
/// Welding never allocates or interprets this value. A host can use one ID
/// space across surfaces and recover its own request context when the matching
/// completion arrives through [`CefSurfaceProducer::poll_web_event`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct WebRequestId(u64);

impl WebRequestId {
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

impl std::fmt::Display for WebRequestId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(formatter)
    }
}

/// Browser callbacks in the order welding observed them.
///
/// Script and cookie requests that return `Ok(())` settle exactly once in
/// this stream. A synchronous `Err` means the request was not accepted and no
/// completion will follow.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum CefSurfaceEvent {
    Navigation(NavigationEvent),
    WebMessage(String),
    ScriptCompleted {
        id: WebRequestId,
        /// `Ok` carries JSON; `Err` carries the exception message.
        result: Result<String, String>,
    },
    CookiesCompleted {
        id: WebRequestId,
        result: Result<Vec<Cookie>, String>,
    },
    /// CEF has destroyed the browser (`OnBeforeClose`); no further callbacks
    /// follow for it. wubble change, see `WUBBLE_CHANGES.md`.
    BrowserClosed,
    /// A key-down the page did not handle (CEF `OnKeyEvent`), as CEF reports
    /// it: its Windows key code, `EVENTFLAG_*` modifiers and whether the
    /// page's focus was on an editable field. wubble change, see
    /// `WUBBLE_CHANGES.md`.
    KeyUnhandled {
        windows_key_code: i32,
        modifiers: u32,
        focus_on_editable_field: bool,
    },
    /// The context script in one frame called its `send` with `text`. The
    /// frame's id, URL and kind are the browser process's own view of the
    /// sending frame (`CefFrame`), never anything the page wrote. The text
    /// may hold page data (form values): do not log it. wubble change, see
    /// `WUBBLE_CHANGES.md` (27).
    FrameMessage {
        frame_id: String,
        frame_url: String,
        is_main: bool,
        /// The pid of the renderer process that sent it, as that process reported it; 0 when
        /// unknown. wubble change (34).
        pid: u32,
        text: String,
    },
}

/// A frame message longer than this (bytes) is dropped. wubble change (27).
pub const MAX_FRAME_MESSAGE_BYTES: usize = 256 * 1024;
/// At most this many frame messages wait in a view's queue; later ones are
/// dropped until the host drains it. A count, not a rate. wubble change (27).
pub const MAX_PENDING_FRAME_MESSAGES: usize = 64;

/// The strings of a list CEF passed to a callback, which still owns it.
/// wubble change, see `WUBBLE_CHANGES.md`.
#[cfg(feature = "cef-runtime")]
pub(crate) fn borrowed_string_list(list: &mut cef::CefStringList) -> Vec<String> {
    // `CefStringList::clone` copies a borrowed list into an empty one, so
    // iterate a second borrowed view of the same pointer instead; neither
    // view frees it.
    let raw: *mut cef::sys::_cef_string_list_t = list.into();
    cef::CefStringList::from(raw).into_iter().collect()
}

/// Shared callback queue used by every platform producer.
#[derive(Debug, Default)]
pub(crate) struct WebEventQueue(Mutex<VecDeque<CefSurfaceEvent>>);

impl WebEventQueue {
    pub(crate) fn push(&self, event: CefSurfaceEvent) {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push_back(event);
    }

    /// Queues a frame message unless it is too long or too many wait
    /// already; whether it was queued. wubble change (27).
    pub(crate) fn push_frame_message(&self, event: CefSurfaceEvent) -> bool {
        let CefSurfaceEvent::FrameMessage { text, .. } = &event else {
            return false;
        };
        if text.len() > MAX_FRAME_MESSAGE_BYTES {
            return false;
        }
        let mut queue = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let pending = queue
            .iter()
            .filter(|e| matches!(e, CefSurfaceEvent::FrameMessage { .. }))
            .count();
        if pending >= MAX_PENDING_FRAME_MESSAGES {
            return false;
        }
        queue.push_back(event);
        true
    }

    pub(crate) fn poll(&self) -> Option<CefSurfaceEvent> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .pop_front()
    }
}

#[cfg(test)]
mod web_event_tests {
    use super::*;

    #[test]
    fn mixed_callbacks_keep_observation_order() {
        let events = WebEventQueue::default();
        let id = WebRequestId::new(u64::MAX);
        events.push(CefSurfaceEvent::WebMessage("first".into()));
        events.push(CefSurfaceEvent::ScriptCompleted {
            id,
            result: Ok("4".into()),
        });
        events.push(CefSurfaceEvent::Navigation(NavigationEvent::TitleChanged {
            title: "last".into(),
        }));
        events.push(CefSurfaceEvent::Navigation(NavigationEvent::LoadingStateChanged {
            loading: false,
            can_go_back: true,
            can_go_forward: false,
        }));
        events.push(CefSurfaceEvent::Navigation(NavigationEvent::FaviconUrlsChanged {
            urls: vec!["https://example.com/favicon.ico".into()],
        }));
        events.push(CefSurfaceEvent::Navigation(NavigationEvent::JsDialogRequested {
            id: 1,
            kind: JsDialogKind::Confirm,
            origin_url: "https://example.com/".into(),
            message: "sure?".into(),
            default_text: String::new(),
        }));
        events.push(CefSurfaceEvent::Navigation(NavigationEvent::FullscreenChanged {
            fullscreen: true,
        }));
        events.push(CefSurfaceEvent::Navigation(NavigationEvent::RequestsDismissed { ids: vec![2] }));
        events.push(CefSurfaceEvent::BrowserClosed);
        events.push(CefSurfaceEvent::KeyUnhandled {
            windows_key_code: 0x52,
            modifiers: 1 << 2,
            focus_on_editable_field: true,
        });
        assert!(events.push_frame_message(frame_message("f1", "after")));

        assert!(matches!(
            events.poll(),
            Some(CefSurfaceEvent::WebMessage(message)) if message == "first"
        ));
        assert!(matches!(
            events.poll(),
            Some(CefSurfaceEvent::ScriptCompleted { id: seen, result: Ok(value) })
                if seen == id && value == "4"
        ));
        assert!(matches!(
            events.poll(),
            Some(CefSurfaceEvent::Navigation(NavigationEvent::TitleChanged { title }))
                if title == "last"
        ));
        assert!(matches!(
            events.poll(),
            Some(CefSurfaceEvent::Navigation(NavigationEvent::LoadingStateChanged {
                loading: false,
                can_go_back: true,
                can_go_forward: false,
            }))
        ));
        assert!(matches!(
            events.poll(),
            Some(CefSurfaceEvent::Navigation(NavigationEvent::FaviconUrlsChanged { urls }))
                if urls == ["https://example.com/favicon.ico"]
        ));
        assert!(matches!(
            events.poll(),
            Some(CefSurfaceEvent::Navigation(NavigationEvent::JsDialogRequested {
                id: 1,
                kind: JsDialogKind::Confirm,
                ..
            }))
        ));
        assert!(matches!(
            events.poll(),
            Some(CefSurfaceEvent::Navigation(NavigationEvent::FullscreenChanged {
                fullscreen: true
            }))
        ));
        assert!(matches!(
            events.poll(),
            Some(CefSurfaceEvent::Navigation(NavigationEvent::RequestsDismissed { ids }))
                if ids == [2]
        ));
        assert!(matches!(events.poll(), Some(CefSurfaceEvent::BrowserClosed)));
        assert!(matches!(
            events.poll(),
            Some(CefSurfaceEvent::KeyUnhandled {
                windows_key_code: 0x52,
                modifiers: 4,
                focus_on_editable_field: true,
            })
        ));
        assert!(matches!(
            events.poll(),
            Some(CefSurfaceEvent::FrameMessage { frame_id, text, pid, .. })
                if frame_id == "f1" && text == "after" && pid == 4242
        ));
        assert!(events.poll().is_none());
    }

    fn frame_message(frame_id: &str, text: &str) -> CefSurfaceEvent {
        CefSurfaceEvent::FrameMessage {
            frame_id: frame_id.into(),
            frame_url: "https://a.test/".into(),
            is_main: true,
            pid: 4242,
            text: text.into(),
        }
    }

    #[test]
    fn frame_messages_keep_order_and_respect_the_caps() {
        let events = WebEventQueue::default();
        assert!(events.push_frame_message(frame_message("f", "1")));
        events.push(CefSurfaceEvent::BrowserClosed);
        assert!(events.push_frame_message(frame_message("f", "2")));
        // Too long: dropped, the queue unchanged.
        let long = "x".repeat(MAX_FRAME_MESSAGE_BYTES + 1);
        assert!(!events.push_frame_message(frame_message("f", &long)));
        // Not a frame message: refused.
        assert!(!events.push_frame_message(CefSurfaceEvent::BrowserClosed));
        for i in 2..MAX_PENDING_FRAME_MESSAGES {
            assert!(events.push_frame_message(frame_message("f", &i.to_string())));
        }
        assert!(!events.push_frame_message(frame_message("f", "over")));
        let texts: Vec<String> = std::iter::from_fn(|| events.poll())
            .filter_map(|e| match e {
                CefSurfaceEvent::FrameMessage { text, .. } => Some(text),
                _ => None,
            })
            .collect();
        assert_eq!(texts.len(), MAX_PENDING_FRAME_MESSAGES);
        assert_eq!(texts[..2], ["1", "2"]);
        // Drained: there is room again.
        assert!(events.push_frame_message(frame_message("f", "again")));
    }

    #[test]
    fn request_ids_preserve_the_full_u64_space() {
        let id = WebRequestId::new(u64::MAX);
        assert_eq!(id.get(), u64::MAX);
        assert_eq!(id.to_string(), u64::MAX.to_string());
    }
}

/// How CEF can participate in a host compositor on the current platform.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum CefSurfaceMode {
    /// CEF can produce GPU-importable frames via `OnAcceleratedPaint`.
    /// Platform adapters copy or retain callback-scoped resources as needed
    /// before exposing host-owned textures to wgpu.
    AcceleratedPaint,
    /// CPU-bitmap fallback via `OnPaint` (`feature = "cpu-paint-fallback"`).
    /// Available regardless of GPU support; requires a texture upload per frame.
    #[cfg(feature = "cpu-paint-fallback")]
    CpuPaint,
    /// No viable surface path (missing CEF GPU support or unsupported OS).
    Unsupported,
}

/// Capability probe result for the current platform + CEF configuration.
#[derive(Debug)]
pub struct CefSurfaceCapabilities {
    pub preferred_mode: CefSurfaceMode,
    pub accelerated_paint_available: bool,
    pub cpu_paint_available: bool,
    pub cookies: BrowserFeatureStatus,
    pub cookie_change_events: BrowserFeatureStatus,
    pub script_execution: BrowserFeatureStatus,
    pub script_result: BrowserFeatureStatus,
    pub devtools: BrowserFeatureStatus,
    pub downloads: BrowserFeatureStatus,
    pub auth_challenges: BrowserFeatureStatus,
    pub permission_requests: BrowserFeatureStatus,
    pub devtools_protocol: BrowserFeatureStatus,
    pub popups: BrowserFeatureStatus,
    pub context_menus: BrowserFeatureStatus,
    pub console_messages: BrowserFeatureStatus,
    /// Native system-print dialog (`CefBrowserHost::Print`).
    pub printer: BrowserFeatureStatus,
    /// Host-to-page and page-to-host drag/drop forwarding.
    pub drag_drop: BrowserFeatureStatus,
    /// Direct multi-touch input (`CefBrowserHost::SendTouchEvent`).
    pub touch: BrowserFeatureStatus,
    /// One-shot PNG capture via Chromium's `Page.captureScreenshot` method.
    pub png_snapshot: BrowserFeatureStatus,
    /// A distinct CEF request context for every producer.
    pub profile_isolation: BrowserFeatureStatus,
}

pub(crate) const CEF_OSR_DEVTOOLS_REASON: &str = "DevTools windows are disabled for windowless CEF: CEF 151 crashes Windows/Linux; macOS remains refused after its CEF 148 crash";

impl CefSurfaceCapabilities {
    /// Probe capabilities for the current platform.
    ///
    /// `accelerated_paint_available` reflects whether CEF was built with GPU
    /// support and `windowless_rendering_enabled` is set. This can only be
    /// definitively confirmed after creating a browser and observing whether
    /// `OnAcceleratedPaint` fires; this probe returns the best static estimate.
    pub fn probe() -> Self {
        // Without cef-runtime, welding exposes public types but no CEF-backed
        // runtime or producer can run. Keep the probe as conservative as the
        // constructors instead of reporting platform capabilities for code
        // that is not compiled in.
        let runtime_feature_enabled = cfg!(feature = "cef-runtime");
        let supported_platform = cfg!(any(
            target_os = "windows",
            target_os = "linux",
            target_os = "macos"
        ));
        let accelerated_paint_available = runtime_feature_enabled && supported_platform;
        let cpu_paint_available =
            runtime_feature_enabled && supported_platform && cfg!(feature = "cpu-paint-fallback");
        let preferred_mode = if accelerated_paint_available {
            CefSurfaceMode::AcceleratedPaint
        } else {
            #[cfg(feature = "cpu-paint-fallback")]
            {
                if cpu_paint_available {
                    CefSurfaceMode::CpuPaint
                } else {
                    CefSurfaceMode::Unsupported
                }
            }
            #[cfg(not(feature = "cpu-paint-fallback"))]
            {
                CefSurfaceMode::Unsupported
            }
        };

        if !runtime_feature_enabled || !supported_platform {
            let unavailable = BrowserFeatureStatus::Unsupported(if !runtime_feature_enabled {
                "requires the cef-runtime feature"
            } else {
                "unsupported target platform"
            });
            return Self {
                preferred_mode,
                accelerated_paint_available,
                cpu_paint_available,
                cookies: unavailable,
                cookie_change_events: unavailable,
                script_execution: unavailable,
                script_result: unavailable,
                devtools: unavailable,
                downloads: unavailable,
                auth_challenges: unavailable,
                permission_requests: unavailable,
                devtools_protocol: unavailable,
                popups: unavailable,
                context_menus: unavailable,
                console_messages: unavailable,
                printer: unavailable,
                drag_drop: unavailable,
                touch: unavailable,
                png_snapshot: unavailable,
                profile_isolation: unavailable,
            };
        }

        // Windows, Linux and macOS have each been verified end to end against
        // a real CEF build (see the accelerated-OSR plan's phase table). The
        // remaining honesty gap is the one the doc comment describes: whether
        // *this* CEF distribution was built with GPU support can only be
        // confirmed once `OnAcceleratedPaint` actually fires.
        Self {
            preferred_mode,
            accelerated_paint_available,
            cpu_paint_available,
            cookies: BrowserFeatureStatus::Supported,
            cookie_change_events: BrowserFeatureStatus::Unsupported(
                "CEF cookie observers are not wired yet",
            ),
            script_execution: BrowserFeatureStatus::Supported,
            script_result: BrowserFeatureStatus::Supported,
            // CEF's native DevTools window is unsafe for accelerated
            // windowless browsers. The CDP path below is separate, supported,
            // and is the safe basis for a host-owned inspector.
            devtools: BrowserFeatureStatus::Unsupported(CEF_OSR_DEVTOOLS_REASON),
            // A CefDownloadHandler is registered on every producer. Whether a
            // download can actually land is a per-producer question --
            // `download_dir` decides that -- so this reports the wiring, and
            // `cancel_download` says so plainly when the directory is unset.
            downloads: BrowserFeatureStatus::Supported,
            // The handler is registered and the answer path is implemented,
            // but CEF has never been seen to call it. A probe inside
            // GetAuthCredentials counted zero invocations against a top-level
            // 401 that Chromium itself failed with ERR_INVALID_AUTH_CREDENTIALS,
            // with and without CEF_RUNTIME_STYLE_ALLOY, while other methods on
            // that same handler fire normally. Proxy authentication is
            // untested.
            // Both CEF permission callbacks are registered and answerable;
            // a page's own geolocation success callback has been seen to fire
            // after grant_permission on Windows.
            permission_requests: BrowserFeatureStatus::Supported,
            // CDP passes through in both directions, opt-in via
            // CefSurfaceConfig::devtools_protocol. Browser.getVersion returns a
            // real protocol result and Page.enable subscribes.
            devtools_protocol: BrowserFeatureStatus::Supported,
            auth_challenges: BrowserFeatureStatus::Partial(
                "GetAuthCredentials is wired and answerable, but CEF 147 was not                  observed to call it for server auth; proxy auth untested",
            ),
            // Two unrelated things share this name. Widget surfaces (select
            // dropdowns, autocomplete) are rendered: see `acquire_popup`.
            // Popup *browsers* (window.open) are denied and reported as
            // NewWindowRequested, because welding renders one surface per
            // producer and has nowhere to put a second browser.
            popups: BrowserFeatureStatus::Partial(
                "widget surfaces are rendered via acquire_popup; popup browsers \
                 (window.open) are denied and surfaced as NewWindowRequested",
            ),
            // A CefContextMenuHandler is registered on every producer: CEF's
            // own menu is suppressed (it has nowhere to draw itself under
            // windowless rendering) and the host is handed the hit-test
            // details to draw its own.
            context_menus: BrowserFeatureStatus::Supported,
            console_messages: BrowserFeatureStatus::Supported,
            // Windows and macOS have a Chromium-owned native print dialog.
            // Linux CEF instead requires the embedder to implement a complete
            // CefPrintHandler, including a system printer UI and spooler.
            // welding does not silently substitute `lp` or choose a printer.
            #[cfg(not(target_os = "linux"))]
            printer: BrowserFeatureStatus::Supported,
            #[cfg(target_os = "linux")]
            printer: BrowserFeatureStatus::Unsupported(
                "Linux CEF requires an embedder-owned print handler and printer UI",
            ),
            // The destination path maps DragEnter/Over/Leave/Drop directly to
            // CEF. A page-originated drag is handed to the host as an event so
            // its windowing toolkit can run the native system drag loop.
            drag_drop: BrowserFeatureStatus::Supported,
            touch: BrowserFeatureStatus::Supported,
            // Screenshot capture is asynchronous, like PDF printing: the
            // browser answers through a DevTools observer after the next
            // compositor frame rather than on the caller's stack.
            png_snapshot: BrowserFeatureStatus::Supported,
            profile_isolation: BrowserFeatureStatus::Supported,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BrowserFeatureStatus {
    Supported,
    Unsupported(&'static str),
    Partial(&'static str),
}

#[cfg(test)]
mod capability_tests {
    use super::*;

    /// A capability report that overstates is worse than a missing feature,
    /// so every `Supported` here is pinned to a handler that exists. When a
    /// feature lands, flip the status *and* this test in the same change.
    #[test]
    fn probe_only_claims_what_is_wired() {
        let caps = CefSurfaceCapabilities::probe();

        if !cfg!(feature = "cef-runtime") {
            let all_browser_features = [
                caps.cookies,
                caps.cookie_change_events,
                caps.script_execution,
                caps.script_result,
                caps.devtools,
                caps.downloads,
                caps.auth_challenges,
                caps.permission_requests,
                caps.devtools_protocol,
                caps.popups,
                caps.context_menus,
                caps.console_messages,
                caps.printer,
                caps.drag_drop,
                caps.touch,
                caps.png_snapshot,
                caps.profile_isolation,
            ];
            assert_eq!(caps.preferred_mode, CefSurfaceMode::Unsupported);
            assert!(!caps.accelerated_paint_available);
            assert!(!caps.cpu_paint_available);
            assert!(all_browser_features.iter().all(|status| {
                matches!(
                    status,
                    BrowserFeatureStatus::Unsupported("requires the cef-runtime feature")
                )
            }));
            return;
        }

        if !cfg!(any(
            target_os = "windows",
            target_os = "linux",
            target_os = "macos"
        )) {
            assert_eq!(caps.preferred_mode, CefSurfaceMode::Unsupported);
            assert!(!caps.accelerated_paint_available);
            assert!(!caps.cpu_paint_available);
            assert!(matches!(
                caps.cookies,
                BrowserFeatureStatus::Unsupported("unsupported target platform")
            ));
            return;
        }

        // Script execution, console messages (the display handler's
        // on_console_message), and accelerated paint are wired on all three
        // verified platforms. The native DevTools window is deliberately
        // refused because it crashes accelerated windowless browsers.
        assert_eq!(caps.script_execution, BrowserFeatureStatus::Supported);
        assert!(matches!(
            caps.devtools,
            BrowserFeatureStatus::Unsupported(CEF_OSR_DEVTOOLS_REASON)
        ));
        assert_eq!(caps.console_messages, BrowserFeatureStatus::Supported);
        // Cookies went from a trait default that errored to real producer
        // implementations over CEF's global cookie manager.
        assert_eq!(caps.cookies, BrowserFeatureStatus::Supported);
        // Script results went from impossible to wired when welding gained a
        // CefApp with a render-process handler: evaluation happens in the
        // renderer, so nothing in the browser process could ever answer it.
        assert_eq!(caps.script_result, BrowserFeatureStatus::Supported);
        // A CefDownloadHandler is registered on every producer as of W7a, with
        // the destination decided by `CefSurfaceConfig::download_dir`.
        assert_eq!(caps.downloads, BrowserFeatureStatus::Supported);
        assert!(caps.accelerated_paint_available);
        assert_eq!(caps.preferred_mode, CefSurfaceMode::AcceleratedPaint);

        // Not wired. These carry a reason string rather than a bare status so
        // a host can report *why* to its own user.
        assert_eq!(caps.context_menus, BrowserFeatureStatus::Supported);
        #[cfg(not(target_os = "linux"))]
        assert_eq!(caps.printer, BrowserFeatureStatus::Supported);
        #[cfg(target_os = "linux")]
        assert!(matches!(caps.printer, BrowserFeatureStatus::Unsupported(_)));
        assert_eq!(caps.drag_drop, BrowserFeatureStatus::Supported);
        assert_eq!(caps.touch, BrowserFeatureStatus::Supported);
        assert_eq!(caps.png_snapshot, BrowserFeatureStatus::Supported);
        assert_eq!(caps.profile_isolation, BrowserFeatureStatus::Supported);

        let status = caps.cookie_change_events;
        assert!(
            matches!(status, BrowserFeatureStatus::Unsupported(reason) if !reason.is_empty()),
            "expected an explained Unsupported, got {status:?}"
        );

        assert_eq!(caps.permission_requests, BrowserFeatureStatus::Supported);
        assert_eq!(caps.devtools_protocol, BrowserFeatureStatus::Supported);

        // Auth is the other split case: implemented and registered, never
        // seen to fire.
        assert!(matches!(
            caps.auth_challenges,
            BrowserFeatureStatus::Partial(_)
        ));

        // Popups are the split case: creation is handled, rendering is not.
        assert!(matches!(caps.popups, BrowserFeatureStatus::Partial(_)));
    }

    /// CEF's `cef_color_t` is ARGB, and windowless transparency is enabled by
    /// alpha 0 — there is no partial option. Pin the mapping: `Some` is fully
    /// opaque, `None` is all-zero, and the default is opaque white (an unset
    /// background used to silently render every CSS-background-less page
    /// transparent).
    #[test]
    fn background_color_maps_to_cef_argb() {
        let mut config = CefSurfaceConfig::default();
        assert_eq!(config.background_color, Some([255, 255, 255]));
        assert_eq!(config.cef_background_color(), 0xFFFF_FFFF);

        config.background_color = Some([0x12, 0x34, 0x56]);
        assert_eq!(config.cef_background_color(), 0xFF12_3456);

        config.background_color = None;
        assert_eq!(config.cef_background_color(), 0);
    }

    /// wubble change: the defaults keep welding 0.15's hardcoded behaviour.
    #[test]
    fn frame_timing_defaults_match_upstream() {
        let config = CefSurfaceConfig::default();
        assert_eq!(config.windowless_frame_rate, 60);
        assert!(!config.external_begin_frame);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SameSite {
    Strict,
    Lax,
    None,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Cookie {
    pub name: String,
    pub value: String,
    pub domain: String,
    pub path: String,
    pub secure: bool,
    pub http_only: bool,
    pub same_site: Option<SameSite>,
    pub expires: Option<f64>,
    pub partitioned: bool,
}

// ── Popup widget surfaces ────────────────────────────────────────────────────

/// Where CEF wants the popup widget drawn, relative to the view's top-left.
///
/// In **physical** pixels, like everything else a host hands to or takes from
/// this API. CEF reports popup geometry in DIP; welding scales it up so the
/// rect can be used directly as a viewport.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PopupRect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

/// A popup widget surface: `<select>` dropdowns, autocomplete lists, date
/// pickers.
///
/// Under windowless rendering CEF paints these as a **separate** OSR element
/// rather than compositing them into the view, so a host that only ever draws
/// [`CefSurfaceProducer::acquire_frame`] shows a page whose dropdowns silently
/// do nothing. Draw this over the view at [`PopupRect`] while
/// [`CefSurfaceProducer::popup_rect`] keeps returning `Some`.
///
/// Note this is unrelated to [`NavigationEvent::NewWindowRequested`], which is
/// a request for a whole new *browser* (`window.open`, `target="_blank"`).
pub struct PopupSurface {
    pub texture: ImportedTexture,
    pub rect: PopupRect,
}

// ── Cursor and IME ───────────────────────────────────────────────────────────

/// A cursor shape the page is asking the host to show.
///
/// Vocabulary shared with `scrying`'s `WebSurfaceProducer`, deliberately: a
/// host that abstracts over both lanes should not need two mappings. Note the
/// crossed names against CEF's own: `Pointer` here is the CSS `pointer`, the
/// link hand, which CEF calls `HAND`; CEF's `POINTER` is the plain arrow and
/// maps to `Default`.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum CursorShape {
    Default,
    Pointer,
    Text,
    Wait,
    Crosshair,
    Move,
    NotAllowed,
    Help,
    Progress,
    ResizeNs,
    ResizeEw,
    ResizeNeSw,
    ResizeNwSe,
    ResizeAll,
    Grab,
    Grabbing,
    ZoomIn,
    ZoomOut,
    /// wubble change (60): the rest of the CSS cursors Chromium tells apart
    /// (CEF `CT_COLUMNRESIZE`, `CT_ROWRESIZE`, `CT_NODROP`, `CT_COPY`,
    /// `CT_ALIAS`, `CT_CELL`, `CT_CONTEXTMENU`, `CT_VERTICALTEXT`) and the
    /// edge resizes (`CT_EASTRESIZE` and the other seven).
    ColResize,
    RowResize,
    NoDrop,
    Copy,
    Alias,
    Cell,
    ContextMenu,
    VerticalText,
    ResizeN,
    ResizeE,
    ResizeS,
    ResizeW,
    ResizeNe,
    ResizeNw,
    ResizeSe,
    ResizeSw,
    /// wubble change (60): a page's own `cursor: url(...)` image.
    Image(CursorImage),
    /// wubble change (32): middle-click autoscroll's cursors (CEF `CT_*PANNING`).
    Panning(Panning),
    /// Anything the shared vocabulary has no name for. `"none"` means CEF
    /// asked for a hidden cursor, which the vocabulary cannot express.
    Custom(String),
}

/// wubble change (60): a page's cursor image (CEF's `CefCursorInfo`), as the
/// straight-alpha RGBA pixels windowing toolkits take (CEF's are premultiplied
/// BGRA). `width`/`height` and the hotspot are in device pixels, so the image
/// is already scaled for the display CEF was told about.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CursorImage {
    pub width: u16,
    pub height: u16,
    pub hotspot_x: u16,
    pub hotspot_y: u16,
    pub rgba: std::sync::Arc<[u8]>,
}

/// wubble change (32): which autoscroll cursor Chromium asked for: still (both
/// axes, or one), or scrolling toward a compass point.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Panning {
    Middle,
    MiddleVertical,
    MiddleHorizontal,
    North,
    NorthEast,
    East,
    SouthEast,
    South,
    SouthWest,
    West,
    NorthWest,
}

/// Where the text currently being composed sits, so a host can place its IME
/// candidate window under it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ImeComposition {
    /// Union of the per-character bounds CEF reported, in **physical** pixels.
    pub bounds: PopupRect,
    /// Selected range within the composition, in characters.
    pub selection_start: u32,
    pub selection_end: u32,
}

/// wubble change, see `WUBBLE_CHANGES.md`: one network request CEF is about
/// to send, as [`RequestFilter`] sees it. Built on CEF's IO thread.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ResourceRequest {
    pub url: String,
    /// The top-level page the request belongs to: the request's own URL for
    /// a main-frame navigation, else the main frame's URL, else the
    /// initiator. Empty when CEF names none of them.
    pub top_url: String,
    /// The origin that initiated the request, as CEF reports it (may be empty).
    pub initiator: String,
    /// CEF's raw `cef_resource_type_t`, mapped by the host.
    pub resource_type: u32,
    pub is_navigation: bool,
    /// CEF's browser identifier, 0 when the request has no browser
    /// (service workers).
    pub browser_id: i32,
}

/// wubble change: what to do with a [`ResourceRequest`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RequestAction {
    Allow,
    Cancel,
    /// Load this URL instead (CEF treats a changed URL as a redirect).
    Redirect(String),
    /// wubble change 35: answer with the content of this base64 `data:` URL
    /// (status 200) instead of sending the request; cancelled when it does
    /// not parse. Served through a CEF resource handler, because Chromium
    /// refuses a `data:` URL as the redirect target of a web request.
    Serve(String),
}

/// wubble change: decides every request before it is sent. Called on CEF's
/// IO thread, so it must not wait on the host's UI thread. It should not
/// panic: see [`RequestFilter::check`].
#[derive(Clone)]
pub struct RequestFilter(pub(crate) Arc<dyn Fn(&ResourceRequest) -> RequestAction + Send + Sync>);

impl RequestFilter {
    pub fn new(filter: impl Fn(&ResourceRequest) -> RequestAction + Send + Sync + 'static) -> Self {
        Self(Arc::new(filter))
    }

    /// Runs the filter. With unwinding, a panic is caught and allows the
    /// request, so it never unwinds into CEF. Under `panic = "abort"`
    /// (wubble's release profile) nothing is caught and a panic aborts the
    /// process, so the filter itself has to be panic-free.
    pub fn check(&self, request: &ResourceRequest) -> RequestAction {
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| (self.0)(request)))
            .unwrap_or(RequestAction::Allow)
    }
}

/// wubble change 53: decides each top-level navigation (every hop of a
/// redirect included) before its request is sent; `false` cancels it and
/// [`NavigationEvent::NavigationRefused`] says so. Called on CEF's UI thread
/// from `OnBeforeBrowse`; it should not panic (a panic allows).
#[derive(Clone)]
pub struct NavigationGuard(pub(crate) Arc<dyn Fn(&str) -> bool + Send + Sync>);

impl NavigationGuard {
    pub fn new(guard: impl Fn(&str) -> bool + Send + Sync + 'static) -> Self {
        Self(Arc::new(guard))
    }

    /// Whether the main frame may go to `url`. A guard that panics refuses (wubble change:
    /// fail closed, the guard holds an agent session's tab to its sites).
    pub fn allows(&self, url: &str) -> bool {
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| (self.0)(url))).unwrap_or(false)
    }
}

impl std::fmt::Debug for NavigationGuard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("NavigationGuard")
    }
}

impl std::fmt::Debug for RequestFilter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RequestFilter")
    }
}

/// wubble change 36, see `WUBBLE_CHANGES.md`: the headers of one response
/// CEF received, as [`ResponseObserver`] sees it. Built on CEF's IO thread.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ResponseInfo {
    pub url: String,
    /// CEF's browser identifier, 0 when the request has no browser.
    pub browser_id: i32,
    /// CEF's raw `cef_resource_type_t` of the request.
    pub resource_type: u32,
    pub status: i32,
    /// The MIME type without parameters, lower case (may be empty).
    pub mime: String,
    /// `Content-Length`, when present and a number.
    pub content_length: Option<u64>,
    /// The raw `Content-Range` header (empty when absent).
    pub content_range: String,
    /// The raw `Content-Disposition` header (empty when absent).
    pub content_disposition: String,
    /// `Accept-Ranges: bytes`.
    pub accepts_ranges: bool,
}

impl ResponseInfo {
    /// The header values as CEF hands them over: the MIME type cut at its
    /// parameters and lower-cased, the length parsed.
    pub(crate) fn from_headers(
        url: String,
        browser_id: i32,
        resource_type: u32,
        status: i32,
        mime: &str,
        header: impl Fn(&str) -> String,
    ) -> Self {
        let mime = mime.split(';').next().unwrap_or_default().trim().to_ascii_lowercase();
        Self {
            url,
            browser_id,
            resource_type,
            status,
            mime,
            content_length: header("Content-Length").trim().parse().ok(),
            content_range: header("Content-Range").trim().to_owned(),
            content_disposition: header("Content-Disposition").trim().to_owned(),
            accepts_ranges: header("Accept-Ranges").trim().eq_ignore_ascii_case("bytes"),
        }
    }
}

/// wubble change 36: sees the headers of every response a view's requests
/// get (CEF `OnResourceResponse`). Called on CEF's IO thread: it must not
/// wait on the host's UI thread, and should be cheap and panic-free (a
/// panic is caught where unwinding is on, see [`RequestFilter::check`]).
#[derive(Clone)]
pub struct ResponseObserver(pub(crate) Arc<dyn Fn(&ResponseInfo) + Send + Sync>);

impl ResponseObserver {
    pub fn new(observer: impl Fn(&ResponseInfo) + Send + Sync + 'static) -> Self {
        Self(Arc::new(observer))
    }

    pub fn seen(&self, response: &ResponseInfo) {
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| (self.0)(response)));
    }
}

impl std::fmt::Debug for ResponseObserver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ResponseObserver")
    }
}

/// Configuration for a single CEF browser surface.
pub struct CefSurfaceConfig {
    pub initial_url: String,
    pub initial_size: PhysicalSize<u32>,
    /// What CEF paints where the page itself paints nothing (no CSS
    /// `background` on `<body>`/`<html>`, or during load).
    ///
    /// `Some([r, g, b])` paints those regions that opaque colour. `None`
    /// renders them transparent: the imported texture carries premultiplied
    /// alpha 0 there, which a readback sees as `[0, 0, 0, 0]`. CEF has no
    /// partially-transparent option — alpha is either 0 or 255.
    ///
    /// The default is opaque white, what a normal browser window shows.
    /// Before 0.5.0 this knob did not exist (a `transparent: bool` was
    /// declared but wired to nothing) and every page without its own CSS
    /// background silently rendered transparent.
    pub background_color: Option<[u8; 3]>,
    /// Prefer `OnAcceleratedPaint` over `OnPaint`. If accelerated paint is
    /// unavailable and `cpu-paint-fallback` is enabled, falls back automatically.
    pub prefer_accelerated: bool,
    /// Persistent per-producer user-data directory for cookies, storage,
    /// permissions, cache, and service workers.
    ///
    /// Each producer gets a separate CEF `RequestContext`, even when this is
    /// `None`; the `None` case is an isolated in-memory profile rather than
    /// CEF's process-global context. Set an absolute path to retain that one
    /// producer's profile across runs. This requires an explicit
    /// [`CefRuntimeConfig::cache_path`](crate::runtime::CefRuntimeConfig::cache_path)
    /// (CEF otherwise chooses its shared platform-default root cache
    /// directory), and the path must be a direct child of it (wubble change
    /// 46: CEF's Chrome runtime makes it a Chrome profile).
    pub user_data_dir: Option<std::path::PathBuf>,
    /// The request context this producer shares with others
    /// ([`SharedProfile`](crate::SharedProfile)): same cookies, storage,
    /// cache, permissions and service workers. When set, `user_data_dir` is
    /// ignored (the shared profile chose its directory). `None`, the default,
    /// gives the producer its own context as described above.
    /// wubble change 46, see `WUBBLE_CHANGES.md`.
    #[cfg(feature = "cef-runtime")]
    pub profile: Option<crate::SharedProfile>,
    /// Subscribe to the Chrome DevTools Protocol.
    ///
    /// Off by default because CDP is chatty and the subscription is not free:
    /// a single `Page.enable` produces a steady stream of events whether or not
    /// anything reads them.
    pub devtools_protocol: bool,
    /// Whether the host answers permission requests itself.
    ///
    /// Off by default, and for the same reason as `handle_auth_challenges`: an
    /// unanswered request leaves the page waiting forever. Left off, `welding`
    /// reports the request and immediately denies it, which is what a browser
    /// does when a user dismisses the prompt.
    pub handle_permission_requests: bool,
    /// Whether the host answers HTTP auth challenges itself.
    ///
    /// Off by default, and deliberately: CEF's auth callback can be answered
    /// later, but an unanswered one holds its request open forever. A host
    /// that has not wired [`NavigationEvent::AuthChallenged`] up would hang
    /// every authenticated request rather than fail it. Left off, `welding`
    /// reports the challenge and immediately declines it, which the page sees
    /// as an ordinary authentication failure.
    pub handle_auth_challenges: bool,
    /// Where downloads are written. `None`, the default, refuses them.
    ///
    /// This is policy rather than a per-download question because CEF asks
    /// where to put the file inside a callback it will cancel the download
    /// without an answer to, and on Linux and macOS the thread it asks on is
    /// the one that would have to carry the host's reply. The host still
    /// steers each transfer afterwards with
    /// [`CefSurfaceProducer::cancel_download`] and friends.
    ///
    /// The server's suggested filename is reduced to its final component, so
    /// it cannot place a file outside this directory.
    pub download_dir: Option<std::path::PathBuf>,
    /// Ask the host where each download goes instead of using
    /// [`Self::download_dir`]: each one arrives as
    /// [`NavigationEvent::DownloadRequested`] and starts once the host calls
    /// [`CefSurfaceProducer::answer_download_path`]. Off by default.
    /// wubble change, see `WUBBLE_CHANGES.md`.
    pub ask_download_path: bool,
    /// Display scale factor, e.g. `2.0` for a 2x HiDPI screen.
    ///
    /// `initial_size` and every other size and coordinate in this API stay in
    /// **physical** pixels; this tells CEF how many of those go to one CSS
    /// pixel. Leave it at 1.0 and a HiDPI host gets a page laid out at twice
    /// the CSS width, which renders everything at half its intended size.
    ///
    /// Follow the window at runtime with
    /// [`CefSurfaceProducer::set_scale_factor`].
    pub scale_factor: f32,
    /// CEF `windowless_frame_rate`: how often CEF paints on its own timer.
    /// Ignored by CEF while [`Self::external_begin_frame`] is on.
    /// wubble change, see `WUBBLE_CHANGES.md`.
    pub windowless_frame_rate: u32,
    /// CEF `external_begin_frame_enabled`: when true CEF paints only when the
    /// host calls `SendExternalBeginFrame` (e.g. once per host vsync).
    /// wubble change, see `WUBBLE_CHANGES.md`.
    pub external_begin_frame: bool,
    /// Checks every sub-resource and navigation request before CEF sends
    /// it. `None`, the default, installs no resource-request handler.
    /// wubble change, see `WUBBLE_CHANGES.md`.
    pub request_filter: Option<RequestFilter>,
    /// Sees the headers of every response. `None`, the default, observes
    /// nothing. wubble change 36, see `WUBBLE_CHANGES.md`.
    pub response_observer: Option<ResponseObserver>,
    /// Decides each top-level navigation before it starts. `None`, the
    /// default, allows all. wubble change 53, see `WUBBLE_CHANGES.md`.
    pub navigation_guard: Option<NavigationGuard>,
    /// Which windows the page opens become CEF browsers of their own, with
    /// what config. `None`, the default, cancels every one and reports it as
    /// [`NavigationEvent::NewWindowRequested`]. wubble change 57, see
    /// `WUBBLE_CHANGES.md`.
    pub popup_factory: Option<crate::opened::PopupFactory>,
}

impl Default for CefSurfaceConfig {
    fn default() -> Self {
        CefSurfaceConfig {
            initial_url: "about:blank".into(),
            initial_size: PhysicalSize::new(800, 600),
            background_color: Some([255, 255, 255]),
            prefer_accelerated: true,
            user_data_dir: None,
            #[cfg(feature = "cef-runtime")]
            profile: None,
            devtools_protocol: false,
            handle_permission_requests: false,
            handle_auth_challenges: false,
            download_dir: None,
            ask_download_path: false,
            scale_factor: 1.0,
            windowless_frame_rate: 60,
            external_begin_frame: false,
            request_filter: None,
            response_observer: None,
            navigation_guard: None,
            popup_factory: None,
        }
    }
}

impl CefSurfaceConfig {
    /// [`Self::background_color`] as CEF's ARGB `cef_color_t`: opaque for
    /// `Some`, all-zero (which is what enables transparent windowless
    /// painting) for `None`.
    ///
    /// Only the producers consume this and they only exist under the feature,
    /// but the unit tests below cover it in every configuration, so this is an
    /// allow rather than a cfg gate.
    #[cfg_attr(not(feature = "cef-runtime"), allow(dead_code))]
    pub(crate) fn cef_background_color(&self) -> u32 {
        match self.background_color {
            Some([r, g, b]) => {
                0xFF00_0000 | (u32::from(r) << 16) | (u32::from(g) << 8) | u32::from(b)
            },
            None => 0,
        }
    }

    /// [`Self::windowless_frame_rate`] as CEF's `int` (wubble change).
    #[cfg_attr(not(feature = "cef-runtime"), allow(dead_code))]
    pub(crate) fn cef_windowless_frame_rate(&self) -> i32 {
        i32::try_from(self.windowless_frame_rate).unwrap_or(i32::MAX)
    }
}

/// The main abstraction for a weld-managed CEF browser surface.
///
/// Platform implementations — [`crate::windows_cef::WindowsCefProducer`],
/// [`crate::macos_cef::MacosCefProducer`], [`crate::linux_cef::LinuxCefProducer`]
/// — all implement this trait.
///
/// For single-platform code, use the [`crate::PlatformCefProducer`] alias.
pub trait CefSurfaceProducer {
    fn surface_mode(&self) -> CefSurfaceMode;

    fn capabilities(&self) -> CefSurfaceCapabilities {
        CefSurfaceCapabilities::probe()
    }

    /// Take the newest application-owned native frame without importing it.
    ///
    /// This and [`Self::acquire_frame`] are alternative consumers of the same
    /// latest-frame mailbox. The returned value keeps platform handle, file
    /// descriptor, or retain custody until the host moves it into Weld's
    /// importer or another ownership-preserving boundary.
    fn acquire_native_frame(&mut self) -> Option<crate::NativeFrame>;

    /// Acquire the most recently painted frame as a wgpu texture.
    /// Returns `Ok(None)` if no new frame is available since the last call.
    fn acquire_frame(
        &mut self,
        ctx: &HostWgpuContext,
    ) -> Result<Option<ImportedTexture>, WeldError>;

    /// Acquire the most recently painted **popup widget** surface, if one is
    /// showing and has repainted since the last call.
    ///
    /// Hosts should cache the returned surface and keep drawing it over the
    /// view for as long as [`popup_rect`](Self::popup_rect) returns `Some`;
    /// like [`acquire_frame`](Self::acquire_frame), this only yields on a new
    /// paint. Drop the cached surface when `popup_rect` goes to `None`.
    fn acquire_popup(&mut self, _ctx: &HostWgpuContext) -> Result<Option<PopupSurface>, WeldError> {
        Ok(None)
    }

    /// Where the popup widget is showing, or `None` when no popup is open.
    ///
    /// This is the visibility signal: CEF hides a popup without painting
    /// anything, so a host that waits for a frame would leave a stale dropdown
    /// on screen.
    fn popup_rect(&self) -> Option<PopupRect> {
        None
    }

    /// Resize the view. `size` is in **physical** pixels.
    /// Tell CEF whether the surface is visible.
    ///
    /// A windowless browser that CEF believes is hidden throttles or drops
    /// work, so a host that never says otherwise can end up with a browser
    /// that paints but ignores input.
    fn set_visible(&mut self, _visible: bool) -> Result<(), WeldError> {
        Err(WeldError::PlatformUnsupported(
            "visibility is not wired for this producer",
        ))
    }

    /// CPU `OnPaint` frames CEF delivered instead of a shared texture, since
    /// the producer was created. `None` when this producer does not count them
    /// (never a made-up 0). wubble change, see `WUBBLE_CHANGES.md`.
    fn cpu_paint_count(&self) -> Option<u64> {
        None
    }

    /// The cursor shape the page is asking for, if it changed since the last
    /// call. The host owns the pointer under windowless rendering, so nothing
    /// happens unless the host applies it.
    fn poll_cursor_shape(&mut self) -> Option<CursorShape> {
        None
    }

    /// Where the text being composed sits, if it moved since the last call.
    /// Use it to place an IME candidate window.
    fn poll_ime_composition(&mut self) -> Option<ImeComposition> {
        None
    }

    /// Set or update the in-progress IME composition (the "preedit" text).
    ///
    /// `selection` is a character range within `text`. CEF draws the
    /// composition itself as part of the page.
    fn ime_set_composition(
        &mut self,
        _text: &str,
        _selection: (u32, u32),
    ) -> Result<(), WeldError> {
        Err(WeldError::PlatformUnsupported(
            "IME is not wired for this producer",
        ))
    }

    /// Commit the composition, or insert `text` directly when there is none.
    fn ime_commit_text(&mut self, _text: &str) -> Result<(), WeldError> {
        Err(WeldError::PlatformUnsupported(
            "IME is not wired for this producer",
        ))
    }

    /// Finish composing and keep what is there.
    fn ime_finish_composing(&mut self, _keep_selection: bool) -> Result<(), WeldError> {
        Err(WeldError::PlatformUnsupported(
            "IME is not wired for this producer",
        ))
    }

    /// Abandon the composition.
    fn ime_cancel_composition(&mut self) -> Result<(), WeldError> {
        Err(WeldError::PlatformUnsupported(
            "IME is not wired for this producer",
        ))
    }

    fn resize(&mut self, size: PhysicalSize<u32>) -> Result<(), WeldError>;

    /// The producer of a pop-up browser this one's page opened, announced by
    /// [`NavigationEvent::PopupBrowserCreated`]; `None` once taken. The
    /// caller owns it from then on, as one it created. wubble change 57.
    fn take_popup_browser(&mut self, _popup_id: i32) -> Option<Self>
    where
        Self: Sized,
    {
        None
    }

    /// Follow the window's display scale, e.g. after it moves between a 1x and
    /// a 2x screen. Live rather than construction-time, because a window can
    /// cross displays without ever being recreated.
    fn set_scale_factor(&mut self, _scale: f32) -> Result<(), WeldError> {
        Err(WeldError::PlatformUnsupported(
            "scale factor is not wired for this producer",
        ))
    }

    /// Tell CEF where the view sits in the host window, and treat the window
    /// as the screen, so popups (`<select>` lists) can extend outside the
    /// view. A no-op when the area is unchanged. wubble change.
    fn set_screen_area(&mut self, _area: crate::ScreenArea) -> Result<(), WeldError> {
        Err(WeldError::PlatformUnsupported(
            "screen area is not wired for this producer",
        ))
    }

    /// The scale factor currently reported to CEF.
    fn scale_factor(&self) -> f32 {
        1.0
    }

    fn navigate_to_url(&mut self, url: &str) -> Result<(), WeldError>;
    fn navigate_to_string(&mut self, content: &str, mime_type: &str) -> Result<(), WeldError>;
    fn reload(&mut self) -> Result<(), WeldError>;

    /// Send one Chrome DevTools Protocol message, as JSON.
    ///
    /// The wire format, unwrapped: `{"id":1,"method":"Page.enable"}`. Replies
    /// and events come back from [`Self::poll_devtools_message`] in the same
    /// form, so an existing CDP client can drive this directly.
    ///
    /// Requires `CefSurfaceConfig::devtools_protocol`.
    fn send_devtools_message(&mut self, _json: &str) -> Result<(), WeldError> {
        Err(WeldError::PlatformUnsupported(
            "the DevTools protocol is not wired for this producer",
        ))
    }

    /// Take the next protocol message, if any. Poll every tick: the queue is
    /// bounded, and a host that falls behind loses the oldest messages.
    fn poll_devtools_message(&mut self) -> Option<String> {
        None
    }

    /// How many protocol messages were dropped because the host was not
    /// polling. Non-zero means its view of the protocol has gaps.
    fn devtools_dropped(&self) -> u64 {
        0
    }

    /// Grant a permission request. Media requests are granted exactly what
    /// they asked for; anything else is accepted.
    fn grant_permission(&mut self, _id: PermissionId) -> Result<(), WeldError> {
        Err(WeldError::PlatformUnsupported(
            "permission requests are not wired for this producer",
        ))
    }

    /// Deny a permission request.
    fn deny_permission(&mut self, _id: PermissionId) -> Result<(), WeldError> {
        Err(WeldError::PlatformUnsupported(
            "permission requests are not wired for this producer",
        ))
    }

    /// Answer a permission request kind by kind: everything in `granted` is
    /// allowed, the rest denied. A prompt request is accepted only when every
    /// kind it asked for is granted (CEF takes one answer for the set); a
    /// media request gets the granted subset. wubble change.
    fn answer_permission(
        &mut self,
        _id: PermissionId,
        _granted: &[PermissionKind],
    ) -> Result<(), WeldError> {
        Err(WeldError::PlatformUnsupported(
            "permission requests are not wired for this producer",
        ))
    }

    /// Answer a JS dialog ([`NavigationEvent::JsDialogRequested`]): `accept`
    /// is OK / leave page, `text` the prompt's input. wubble change.
    fn answer_dialog(&mut self, _id: RequestId, _accept: bool, _text: &str) -> Result<(), WeldError> {
        Err(WeldError::PlatformUnsupported(
            "JS dialogs are not wired for this producer",
        ))
    }

    /// Answer a file chooser ([`NavigationEvent::FileChooserRequested`]);
    /// `None` cancels it. wubble change.
    fn answer_file_chooser(
        &mut self,
        _id: RequestId,
        _paths: Option<&[std::path::PathBuf]>,
    ) -> Result<(), WeldError> {
        Err(WeldError::PlatformUnsupported(
            "file choosers are not wired for this producer",
        ))
    }

    /// Proceed past or cancel a certificate error
    /// ([`NavigationEvent::CertificateError`]). wubble change.
    fn answer_certificate(&mut self, _id: RequestId, _proceed: bool) -> Result<(), WeldError> {
        Err(WeldError::PlatformUnsupported(
            "certificate errors are not wired for this producer",
        ))
    }

    /// Run one of the menu's commands, or dismiss the menu with `None`
    /// ([`NavigationEvent::ContextMenuRequested`]). CEF runs the standard
    /// commands itself. wubble change.
    fn answer_context_menu(&mut self, _id: RequestId, _command: Option<i32>) -> Result<(), WeldError> {
        Err(WeldError::PlatformUnsupported(
            "context menus are not wired for this producer",
        ))
    }

    /// Where a download goes ([`NavigationEvent::DownloadRequested`]); `None`
    /// cancels it. The path is used as given. wubble change.
    fn answer_download_path(
        &mut self,
        _id: RequestId,
        _path: Option<&std::path::Path>,
    ) -> Result<(), WeldError> {
        Err(WeldError::PlatformUnsupported(
            "asking for download paths is not wired for this producer",
        ))
    }

    /// Whether a page may start a drag out of the view. Off by default: CEF
    /// waits for the host to finish every drag it starts, so a host without
    /// a native drag loop must refuse them (`StartDragging` returns false).
    /// wubble change.
    fn set_drag_source(&mut self, _enabled: bool) -> Result<(), WeldError> {
        Err(WeldError::PlatformUnsupported(
            "drag and drop is not wired for this producer",
        ))
    }

    /// The newest popup-widget frame without importing it, with its placement
    /// (physical pixels, relative to the view; it may extend outside it).
    /// `None` while no popup is showing or nothing new was painted. Linux and
    /// macOS; Windows imports popups inside the paint callback. wubble change.
    fn acquire_native_popup(&mut self) -> Option<(NativeFrame, PopupRect)> {
        None
    }

    /// Answer an auth challenge.
    ///
    /// Neither the username nor the password is logged or kept; both go
    /// straight to CEF. Answering an id twice, or one that was already
    /// declined, is an error rather than a silent no-op.
    fn answer_auth(
        &mut self,
        _id: AuthId,
        _username: &str,
        _password: &str,
    ) -> Result<(), WeldError> {
        Err(WeldError::PlatformUnsupported(
            "auth challenges are not wired for this producer",
        ))
    }

    /// Decline an auth challenge. The request fails as if the user cancelled.
    fn cancel_auth(&mut self, _id: AuthId) -> Result<(), WeldError> {
        Err(WeldError::PlatformUnsupported(
            "auth challenges are not wired for this producer",
        ))
    }

    /// Ask CEF to cancel, pause or resume a download.
    ///
    /// wubble change: applied at once through the newest callback CEF handed
    /// over for it; an error for a download welding is not steering (not
    /// started, finished, or cancelled).
    fn cancel_download(&mut self, _id: DownloadId) -> Result<(), WeldError> {
        Err(WeldError::PlatformUnsupported(
            "downloads are not wired for this producer",
        ))
    }

    fn pause_download(&mut self, _id: DownloadId) -> Result<(), WeldError> {
        Err(WeldError::PlatformUnsupported(
            "downloads are not wired for this producer",
        ))
    }

    fn resume_download(&mut self, _id: DownloadId) -> Result<(), WeldError> {
        Err(WeldError::PlatformUnsupported(
            "downloads are not wired for this producer",
        ))
    }

    /// Ask CEF to repaint the whole view now.
    ///
    /// Painting is change-driven, so a view nothing has changed does not paint
    /// on its own. That matters after a render process dies: the host can
    /// navigate again and the page will load, but the replacement renderer has
    /// nothing queued for the surface, so the host keeps presenting its last
    /// pre-crash frame. This is the nudge that gets a first frame out of it.
    fn request_repaint(&mut self) -> Result<(), WeldError> {
        Err(WeldError::PlatformUnsupported(
            "repaint is not wired for this producer",
        ))
    }
    fn stop(&mut self) -> Result<(), WeldError>;
    /// Whether there is anything to go back to. A host with a back button
    /// needs this to know when to grey it out; `go_back` on an empty history
    /// is simply ignored, which tells the host nothing.
    fn can_go_back(&self) -> bool {
        false
    }

    fn can_go_forward(&self) -> bool {
        false
    }

    /// Step the zoom one notch, or return it to the default.
    fn zoom(&mut self, _command: ZoomCommand) -> Result<(), WeldError> {
        Err(WeldError::PlatformUnsupported(
            "zoom is not wired for this producer",
        ))
    }

    /// Set the zoom to an absolute level instead of stepping it.
    ///
    /// The argument is a CEF zoom *level*, not a scale factor: 0.0 is the
    /// default and the page scale is `1.2^level`, so 120% is level 1.0. A host
    /// holding a factor converts with `factor.ln() / 1.2_f64.ln()`.
    ///
    /// Unlike [`Self::zoom`], which walks Chromium's preset ladder, any level
    /// is accepted. Works everywhere: CEF applies the change immediately when
    /// called on its own UI thread and asynchronously otherwise, which is the
    /// Windows case.
    fn set_zoom_level(&mut self, _level: f64) -> Result<(), WeldError> {
        Err(WeldError::PlatformUnsupported(
            "zoom is not wired for this producer",
        ))
    }

    /// The current zoom level. 0.0 is the default and each notch steps
    /// Chromium's ladder — two steps in is 125%, not 120%.
    ///
    /// **Only meaningful where the host thread is CEF's UI thread**, which is
    /// Linux and macOS here. CEF documents `GetZoomLevel` as UI-thread-only,
    /// and Windows runs CEF's UI thread separately, so this reads 0.0 there
    /// however the page is actually zoomed. [`Self::zoom`] itself works
    /// everywhere; CEF applies it asynchronously when called off-thread.
    fn zoom_level(&self) -> f64 {
        0.0
    }

    /// Render the page to a PDF at `path`.
    ///
    /// Asynchronous: completion arrives as
    /// [`NavigationEvent::PdfPrintFinished`]. Chromium prints what the page
    /// looks like to a printer, not a screenshot — a page that styles itself
    /// for `@media print` gets that.
    ///
    /// wubble change 47: `options` carries the print sheet's settings
    /// (CEF's `cef_pdf_print_settings_t`).
    fn print_to_pdf(
        &mut self,
        _path: &std::path::Path,
        _options: &PdfOptions,
    ) -> Result<(), WeldError> {
        Err(WeldError::PlatformUnsupported(
            "print to PDF is not wired for this producer",
        ))
    }

    /// Open Chromium's native print dialog for the current page.
    ///
    /// This is deliberately separate from [`Self::print_to_pdf`]: PDF export
    /// is a file-producing, callback-backed operation; printing hands the
    /// final printer choice and cancellation to the platform dialog. `Ok(())`
    /// therefore means CEF accepted the request, not that the user printed.
    /// Linux deliberately returns [`WeldError::PlatformUnsupported`]: CEF
    /// delegates its full print UI and spooler to an embedder-owned handler
    /// there, and welding has no neutral system-printer abstraction yet.
    fn print(&mut self) -> Result<(), WeldError> {
        Err(WeldError::PlatformUnsupported(
            "printing is not wired for this producer",
        ))
    }

    /// Start a one-shot PNG capture of the currently composited page.
    ///
    /// The result arrives later from [`Self::poll_snapshot_png`]. It is a
    /// thumbnail/preview/diagnostic helper, not the live frame transport: it
    /// encodes and copies pixels through Chromium's DevTools screenshot path.
    ///
    /// The returned ID is retained in the later completion. Welding admits at
    /// most 16 captures awaiting completion or polling; it rejects another
    /// request once that bound is full instead of dropping an admitted result.
    fn request_snapshot_png(&mut self) -> Result<SnapshotRequestId, WeldError> {
        Err(WeldError::PlatformUnsupported(
            "PNG snapshots are not wired for this producer",
        ))
    }

    /// Take the next completed PNG capture. The completion retains the ID from
    /// [`Self::request_snapshot_png`]; `completion.result` is `Err(_)` when
    /// Chromium answered but could not capture or encode the page.
    fn poll_snapshot_png(&mut self) -> Option<SnapshotPngCompletion> {
        None
    }

    /// Search the page. Results arrive as [`NavigationEvent::FindResult`].
    ///
    /// `find_next` steps through matches for the same text; passing `false`
    /// starts a new search. Stop with [`Self::stop_finding`], which also clears
    /// the highlight.
    fn find(
        &mut self,
        _text: &str,
        _forward: bool,
        _match_case: bool,
        _find_next: bool,
    ) -> Result<(), WeldError> {
        Err(WeldError::PlatformUnsupported(
            "find is not wired for this producer",
        ))
    }

    fn stop_finding(&mut self, _clear_selection: bool) -> Result<(), WeldError> {
        Err(WeldError::PlatformUnsupported(
            "find is not wired for this producer",
        ))
    }

    fn go_back(&mut self) -> Result<(), WeldError>;
    fn go_forward(&mut self) -> Result<(), WeldError>;

    fn send_mouse_input(&mut self, event: MouseEvent) -> Result<(), WeldError>;

    /// Forward one physical-pixel touch contact to Chromium.
    ///
    /// Touch ids distinguish simultaneous contacts. A host must finish every
    /// started id with [`TouchPhase::Ended`] or [`TouchPhase::Cancelled`]; CEF
    /// otherwise retains a pressed contact just like a window system would.
    fn send_touch_input(&mut self, _event: TouchInput) -> Result<(), WeldError> {
        Err(WeldError::PlatformUnsupported(
            "touch input is not wired for this producer",
        ))
    }

    /// Forward an OS drag/drop operation over the webview.
    ///
    /// A `DragInput::Enter` must carry a payload. Subsequent `Over`, `Leave`,
    /// and `Drop` calls only need the operation, pointer position, modifiers,
    /// and allowed effects. Page-originated drags surface separately as
    /// [`NavigationEvent::DragStarted`].
    fn send_drag_input(&mut self, _event: DragInput) -> Result<(), WeldError> {
        Err(WeldError::PlatformUnsupported(
            "drag and drop is not wired for this producer",
        ))
    }

    /// wubble change 54: this producer's page drag, held since its
    /// `StartDragging`, for another producer's [`DragInput::from`].
    fn held_drag(&self) -> Option<HeldDrag> {
        None
    }

    /// Tell CEF that the native system drag started by
    /// [`NavigationEvent::DragStarted`] finished.
    ///
    /// `x` and `y` are physical pixels relative to the webview, matching every
    /// other coordinate in this API. Call this after the host toolkit's drag
    /// loop returns, including for cancellation (`DragOperations::NONE`).
    fn finish_drag_source(
        &mut self,
        _x: i32,
        _y: i32,
        _operation: DragOperations,
    ) -> Result<(), WeldError> {
        Err(WeldError::PlatformUnsupported(
            "drag and drop is not wired for this producer",
        ))
    }

    fn send_keyboard_input(&mut self, event: KeyEvent) -> Result<(), WeldError>;
    fn move_focus(&mut self, direction: FocusDirection) -> Result<(), WeldError>;

    fn post_web_message(&mut self, message: &str) -> Result<(), WeldError>;

    /// Take the next browser callback in producer-observed order.
    fn poll_web_event(&mut self) -> Option<CefSurfaceEvent>;

    /// Execute a JavaScript expression in the browser's main frame. CEF
    /// provides this natively (`cef_frame_t::execute_java_script`) without
    /// requiring a CDP round-trip.
    fn execute_script(&mut self, script: &str, source_url: &str) -> Result<(), WeldError>;

    /// Runs `script` in the frame `frame_id` (from
    /// [`NavigationEvent::ContextMenuRequested`]), or in the main frame when
    /// the id is empty or that frame is gone. wubble change 43.
    fn execute_script_in_frame(
        &mut self,
        frame_id: &str,
        script: &str,
        source_url: &str,
    ) -> Result<(), WeldError> {
        let _ = frame_id;
        self.execute_script(script, source_url)
    }

    /// Hands `text` to the context script's registered function in the frame
    /// `frame_id` (from a [`CefSurfaceEvent::FrameMessage`]). `Err` when the
    /// browser or that frame is gone. wubble change, see `WUBBLE_CHANGES.md`
    /// (27).
    fn send_frame_message(&mut self, _frame_id: &str, _text: &str) -> Result<(), WeldError> {
        Err(WeldError::PlatformUnsupported(
            "frame messages are not wired for this producer",
        ))
    }

    /// Evaluate `script` and return its value later under caller-minted `id`.
    ///
    /// The answer arrives through
    /// [`poll_web_event`](Self::poll_web_event), because the value has
    /// to be fetched from the renderer process over CEF's process-message
    /// channel. A blocking call would wait on the very loop that carries the
    /// reply.
    ///
    /// Results come back as JSON: `2+2` yields `4`, `document.title` yields a
    /// quoted string, an object yields an object. A script that throws yields
    /// `Err` with the exception message.
    fn request_script_result(&mut self, _id: WebRequestId, _script: &str) -> Result<(), WeldError> {
        Err(WeldError::BrowserOp(
            "script results are not wired for this producer".into(),
        ))
    }

    /// Write a cookie, as if `url` had sent it in a `Set-Cookie` header.
    ///
    /// CEF validates the cookie's domain and path against `url` and rejects
    /// mismatches, so pass the URL the cookie belongs to rather than any URL.
    fn set_cookie(&mut self, _url: &str, _cookie: &Cookie) -> Result<(), WeldError> {
        Err(WeldError::BrowserOp(
            "cookies are not wired for this producer".into(),
        ))
    }

    /// Start reading cookies. `url` of `None` reads the whole store.
    ///
    /// The answer arrives later through [`poll_web_event`](Self::poll_web_event);
    /// CEF delivers cookies through a visitor, one at a time, and on Linux and
    /// macOS the calling thread is CEF's own UI thread, so a blocking getter
    /// would wait on the loop that produces the answer.
    fn request_cookies(&mut self, _id: WebRequestId, _url: Option<&str>) -> Result<(), WeldError> {
        Err(WeldError::BrowserOp(
            "cookies are not wired for this producer".into(),
        ))
    }

    /// Delete cookies. `url` of `None` means every URL, `name` of `None` means
    /// every name, so both `None` clears the store.
    fn delete_cookies(&mut self, _url: Option<&str>, _name: Option<&str>) -> Result<(), WeldError> {
        Err(WeldError::BrowserOp(
            "cookies are not wired for this producer".into(),
        ))
    }

    fn open_devtools(&self) -> Result<(), WeldError>;

    /// CEF-internal browser identifier. Useful for routing multi-browser
    /// callback events in the `CefClient` vtable.
    fn browser_id(&self) -> i32;

    fn close(&mut self) -> Result<(), WeldError>;
}

// ── Input event types ────────────────────────────────────────────────────────

#[derive(Clone, Debug)]
pub struct MouseEvent {
    pub x: i32,
    pub y: i32,
    pub button: MouseButton,
    pub action: MouseAction,
    pub modifiers: EventModifiers,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseAction {
    Pressed,
    Released,
    Moved,
    WheelScrolled { delta_x: i32, delta_y: i32 },
}

#[derive(Clone, Debug)]
pub struct KeyEvent {
    pub kind: KeyEventKind,
    pub windows_key_code: i32,
    pub native_key_code: i32,
    pub character: Option<char>,
    pub modifiers: EventModifiers,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyEventKind {
    RawKeyDown,
    KeyDown,
    KeyUp,
    Char,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct EventModifiers {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub meta: bool,
    /// Buttons currently held while this input event is delivered. CEF needs
    /// these on move events to distinguish a page drag from a hover.
    pub left_mouse_button: bool,
    pub middle_mouse_button: bool,
    pub right_mouse_button: bool,
}

/// One direct-touch contact. Coordinates and radii are **physical** pixels
/// relative to the webview's top-left; welding converts them to the DIP units
/// CEF expects at the boundary.
#[derive(Clone, Copy, Debug)]
pub struct TouchInput {
    /// A non-negative contact id. Distinct live contacts need distinct ids.
    pub id: i32,
    /// CEF routes touch and pen contacts through the same event API but needs
    /// the Pointer Events device kind to preserve DOM `pointerType`.
    pub device: ContactDevice,
    pub x: f32,
    pub y: f32,
    pub radius_x: f32,
    pub radius_y: f32,
    /// Rotation clockwise in degrees, as reported by platform touch APIs.
    pub rotation_angle: f32,
    /// Contact pressure in `0.0..=1.0`. Hosts without pressure data use 1.0
    /// for pressed/moved contacts and 0.0 for ended/cancelled contacts.
    pub pressure: f32,
    pub phase: TouchPhase,
    pub modifiers: EventModifiers,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContactDevice {
    Touch,
    Pen,
}

/// The stage of a direct touch contact.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TouchPhase {
    Started,
    Moved,
    Ended,
    Cancelled,
}

/// A set of drag effects. CEF accepts more than one on entry and reports the
/// final choice on source completion; unknown bits are preserved so a newer
/// window system cannot silently turn into "no operation".
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DragOperations(pub u32);

impl DragOperations {
    pub const NONE: Self = Self(0);
    pub const COPY: Self = Self(1 << 0);
    pub const LINK: Self = Self(1 << 1);
    pub const GENERIC: Self = Self(1 << 2);
    pub const PRIVATE: Self = Self(1 << 3);
    pub const MOVE: Self = Self(1 << 4);
    pub const DELETE: Self = Self(1 << 5);
}

impl std::ops::BitOr for DragOperations {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        Self(self.0 | rhs.0)
    }
}

/// File data made available to the page during a host-originated drag.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DragFile {
    pub path: std::path::PathBuf,
    /// Optional human-facing filename. `None` lets Chromium derive it from
    /// [`Self::path`].
    pub display_name: Option<String>,
}

/// The portable part of a drag data transfer.
///
/// The host can offer files, a link, fragment text, or fragment HTML. It does
/// not expose a toolkit-specific drag object, which would pin a producer API
/// to winit, GTK, or AppKit and still fail on the other two.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DragPayload {
    pub files: Vec<DragFile>,
    pub link_url: Option<String>,
    pub link_title: Option<String>,
    pub fragment_text: Option<String>,
    pub fragment_html: Option<String>,
    pub fragment_base_url: Option<String>,
    /// wubble change 51: a dragged picture's own encoded bytes (CEF's
    /// `GetFileContents`) and Chromium's file name for them. Read only from a
    /// page drag; a host payload's are ignored (CEF has no setter).
    pub file_contents: Option<DragFileContents>,
    /// wubble change 51: the drag image CEF shows under the pointer, as PNG.
    /// Read only from a page drag.
    pub image: Option<DragImage>,
}

/// wubble change 51: a dragged picture's bytes as the page loaded them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DragFileContents {
    /// Chromium's safe file name for the bytes (`GetSafeFilenameForImageFileContents`);
    /// `None` when it found no image extension.
    pub name: Option<String>,
    pub bytes: Arc<[u8]>,
}

/// wubble change 51: the drag's thumbnail.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DragImage {
    /// PNG of CEF's representation nearest the view's device scale.
    pub png: Arc<[u8]>,
    /// The image's size in DIP: the PNG's pixels over this are its scale.
    pub size: (i32, i32),
    /// The pointer's point in the image, in DIP.
    pub hotspot: (i32, i32),
}

/// wubble change 47: how a page is printed to PDF, CEF's
/// `cef_pdf_print_settings_t` without the header and footer templates.
/// The default is Chromium's own: Letter, default margins, 100 %.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PdfOptions {
    pub landscape: bool,
    /// Print background colours and images.
    pub background: bool,
    /// 1.0 is 100 %; 0 or less is CEF's default.
    pub scale: f64,
    /// Paper width and height in inches; 0 or less is Letter.
    pub paper_inches: (f64, f64),
    pub margins: PdfMargins,
    /// Pages as "1-5, 8, 11-13"; empty is every page. CEF fails the print
    /// when a range starts after it ends or lies past the last page.
    pub page_ranges: String,
}

/// wubble change 47: the page margins of a [`PdfOptions`].
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum PdfMargins {
    /// Chromium's default, about 0.4 inch.
    #[default]
    Default,
    None,
    /// Top, right, bottom, left, in inches.
    Custom([f64; 4]),
}

/// One stage of an OS drag/drop operation over the webview.
#[derive(Clone, Debug)]
pub struct DragInput {
    pub kind: DragEventKind,
    /// The data of a [`DragEventKind::Enter`], ignored otherwise.
    /// wubble change 48: `None` on an enter re-enters with the data of the
    /// page's own drag, held since its `StartDragging`, so the types the
    /// page set itself (`dataTransfer.setData("application/x-...")`, which
    /// [`DragPayload`] cannot carry) survive a drag within the window.
    pub payload: Option<DragPayload>,
    /// wubble change 54: on an enter without a payload, another producer's
    /// page drag ([`CefSurfaceProducer::held_drag`]) instead of this one's,
    /// so a drag from one view drops into another with its custom types and
    /// picture bytes.
    pub from: Option<HeldDrag>,
    pub x: i32,
    pub y: i32,
    pub modifiers: EventModifiers,
    pub allowed_operations: DragOperations,
}

/// wubble change 54: a page drag's data as CEF holds it, from its
/// `StartDragging` until `finish_drag_source`; opaque to the host.
#[derive(Clone)]
pub struct HeldDrag {
    #[cfg(feature = "cef-runtime")]
    pub(crate) data: cef::DragData,
}

impl std::fmt::Debug for HeldDrag {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("HeldDrag")
    }
}

/// Two handles are equal only as the same handle: CEF data has no equality.
impl PartialEq for HeldDrag {
    fn eq(&self, other: &Self) -> bool {
        std::ptr::eq(self, other)
    }
}

impl Eq for HeldDrag {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DragEventKind {
    Enter,
    Over,
    Leave,
    Drop,
}

/// Map CEF's termination status onto the platform-neutral one.
///
/// Compared against the associated constants rather than matched: the status
/// is a newtype over a C enum whose repr differs by platform, so `==` is the
/// portable read. An unrecognised value is carried through rather than
/// flattened into `Abnormal`, so a newer CEF cannot silently look ordinary.
#[cfg(feature = "cef-runtime")]
pub(crate) fn termination_status(status: cef::TerminationStatus) -> ProcessTerminationStatus {
    use cef::TerminationStatus as Ts;
    if status == Ts::PROCESS_WAS_KILLED {
        ProcessTerminationStatus::Killed
    } else if status == Ts::PROCESS_CRASHED {
        ProcessTerminationStatus::Crashed
    } else if status == Ts::PROCESS_OOM {
        ProcessTerminationStatus::OutOfMemory
    } else if status == Ts::LAUNCH_FAILED {
        ProcessTerminationStatus::LaunchFailed
    } else if status == Ts::INTEGRITY_FAILURE {
        ProcessTerminationStatus::IntegrityFailure
    } else if status == Ts::ABNORMAL_TERMINATION {
        ProcessTerminationStatus::Abnormal
    } else {
        ProcessTerminationStatus::Unknown(*status.as_ref() as i32)
    }
}

/// A zoom step. Chromium zooms in notches rather than by percentage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ZoomCommand {
    In,
    Out,
    /// Back to the default level.
    Reset,
}

/// What a right-click landed on. Several apply at once: a right-click on a
/// link inside a page reports both.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ContextMenuTarget {
    Page,
    Frame,
    Link,
    Media,
    Selection,
    Editable,
    /// A flag this build does not name, kept rather than dropped.
    Other(u32),
}

/// Split CEF's `cef_context_menu_type_flags_t` into named targets.
// Consumed by the cef_backed handlers alone; unit-tested either way.
#[cfg_attr(not(feature = "cef-runtime"), allow(dead_code))]
pub(crate) fn context_menu_targets(flags: u32) -> Vec<ContextMenuTarget> {
    let mut out = Vec::new();
    let mut seen = 0u32;
    for (bit, target) in [
        (1 << 0, ContextMenuTarget::Page),
        (1 << 1, ContextMenuTarget::Frame),
        (1 << 2, ContextMenuTarget::Link),
        (1 << 3, ContextMenuTarget::Media),
        (1 << 4, ContextMenuTarget::Selection),
        (1 << 5, ContextMenuTarget::Editable),
    ] {
        if flags & bit != 0 {
            out.push(target);
            seen |= bit;
        }
    }
    let mut rest = flags & !seen;
    while rest != 0 {
        let bit = rest & rest.wrapping_neg();
        out.push(ContextMenuTarget::Other(bit));
        rest &= !bit;
    }
    out
}

/// Why a render process died, as CEF reports it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProcessTerminationStatus {
    /// Ended abnormally, with no more specific reason.
    Abnormal,
    /// Killed from outside, e.g. by a task manager or a signal.
    Killed,
    /// Crashed.
    Crashed,
    /// Ran out of memory. Reloading on a loop will only repeat this.
    OutOfMemory,
    /// The process never started.
    LaunchFailed,
    /// Failed an integrity check.
    IntegrityFailure,
    /// A status this build of `welding` does not recognise; the raw value is
    /// carried so a host can still log something useful.
    Unknown(i32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FocusDirection {
    Forward,
    Backward,
}

// ── Navigation event ─────────────────────────────────────────────────────────

/// wubble change 33: what a page asked for when it opened a window (CEF's
/// `cef_window_open_disposition_t`, reduced to what a host can act on).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NewWindowKind {
    ForegroundTab,
    BackgroundTab,
    /// `window.open` with window features: Chromium's `NEW_POPUP`.
    Popup,
    /// A new window without features (Shift+click, `NEW_WINDOW`).
    Window,
    /// Anything else (picture-in-picture, save to disk, ...).
    Other,
}

#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum NavigationEvent {
    LoadStart {
        url: String,
    },
    LoadEnd {
        url: String,
        http_status: i32,
    },
    LoadError {
        url: String,
        error_code: i32,
        error_text: String,
    },
    TitleChanged {
        title: String,
    },
    AddressChanged {
        url: String,
    },
    /// CEF's `OnLoadingStateChange`: the browser started or stopped loading
    /// (any frame), with the history state at that moment. The only source
    /// of `can_go_back` / `can_go_forward` changes. wubble change, see
    /// `WUBBLE_CHANGES.md`.
    LoadingStateChanged {
        loading: bool,
        can_go_back: bool,
        can_go_forward: bool,
    },
    /// CEF's `OnFaviconURLChange`: the page's icon URLs, possibly empty.
    /// wubble change, see `WUBBLE_CHANGES.md`.
    FaviconUrlsChanged {
        urls: Vec<String>,
    },
    /// The render process backing this browser died. The browser object
    /// itself survives, and `reload()` brings the page back: Chromium spawns a
    /// fresh renderer for the next navigation. `status` says which kind of
    /// death it was, which is what decides whether reloading is sensible --
    /// retrying an `OutOfMemory` in a loop just kills the machine slower.
    ContentProcessTerminated {
        status: ProcessTerminationStatus,
        /// CEF's error code, 0 when it supplies none.
        error_code: i32,
        /// CEF's description, empty when it supplies none.
        error_string: String,
    },
    NewWindowRequested {
        url: String,
        user_gesture: bool,
        // wubble change 33: what the page asked for and the size it gave.
        kind: NewWindowKind,
        /// The `width`/`height` window features, when both were given.
        size: Option<(u32, u32)>,
        /// wubble change (59): the `left`/`top` window features, when both
        /// were given (screen DIP).
        position: Option<(i32, i32)>,
    },
    /// wubble change 57: CEF created the window the page opened as a
    /// browser of its own ([`CefSurfaceConfig::popup_factory`] gave it a
    /// config); take it with [`CefSurfaceProducer::take_popup_browser`].
    PopupBrowserCreated(crate::opened::OpenedWindow),
    /// wubble change 57: a pop-up the factory gave a config to was not
    /// created after all (CEF `OnBeforePopupAborted`).
    PopupAborted {
        popup_id: i32,
    },
    /// A `print_to_pdf` finished. `ok` is false when Chromium could not write
    /// the file; the path is the one that was asked for either way.
    PdfPrintFinished {
        path: std::path::PathBuf,
        ok: bool,
    },
    /// The page began a drag that must be driven by the host's native drag
    /// loop. CEF cannot manufacture an OS drag manager for a windowless
    /// surface, so the host receives a portable copy of the payload, starts
    /// its toolkit drag, and then calls [`CefSurfaceProducer::finish_drag_source`].
    ///
    /// `x` and `y` are physical pixels relative to the view. `allowed_operations`
    /// is a set, because the destination chooses the final effect.
    DragStarted {
        payload: DragPayload,
        allowed_operations: DragOperations,
        x: i32,
        y: i32,
    },
    /// wubble change 48: CEF's `UpdateDragCursor`. After each drag enter or
    /// move the page names the operation a drop there would do (none, copy,
    /// move, link), which the host shows as the cursor and reports back when
    /// the drag ends.
    DragCursor { operation: DragOperations },
    /// wubble change 49: CEF's `OnScrollOffsetChanged`, the page's root
    /// scroll offset in DIP, after it changed.
    ScrollOffset { x: f64, y: f64 },
    /// wubble change (61): CEF's `OnVirtualKeyboardRequested`, after the
    /// focused node's text-input state changed: `false` when the mode is NONE
    /// (no text field has focus), so the host switches its input method on
    /// for a field and off elsewhere.
    TextInput { wanted: bool },
    /// wubble change 50: an accessibility tree update CEF's
    /// `OnAccessibilityTreeChange` handed over (only once the host enabled
    /// accessibility on the browser host), as `cef_write_json` wrote the value.
    AccessibilityTree { json: String },
    /// wubble change 50: `OnAccessibilityLocationChange`, likewise.
    AccessibilityLocation { json: String },
    /// How a page search is going. Chromium reports progressively: several of
    /// these arrive for one search as more of the page is scanned, and
    /// `final_update` marks the last.
    FindResult {
        /// How many matches so far.
        count: i32,
        /// Which match is highlighted, 1-based. 0 when there is none.
        active_match: i32,
        /// The last update for this search.
        final_update: bool,
    },
    /// The user right-clicked, and the host should draw its own menu.
    ///
    /// CEF's own menu is inert under windowless rendering — there is no window
    /// to put it in — so `welding` suppresses it and reports this instead. A
    /// host that ignores the event gets what it got before: nothing on
    /// right-click.
    ///
    /// `x` and `y` are **physical** pixels, like every other coordinate in this
    /// API. CEF reports them in DIP; they are converted on the way out.
    ///
    /// wubble change: CEF's own menu model is kept and the menu is held
    /// until the host answers with [`CefSurfaceProducer::answer_context_menu`]
    /// quoting `id` (a command from `items`, or `None` to dismiss). While it
    /// waits, CEF ignores further right-clicks; it is dismissed when the
    /// browser closes.
    ContextMenuRequested {
        /// wubble change: the menu waiting on the host.
        id: RequestId,
        x: i32,
        y: i32,
        /// What was under the cursor: page, frame, link, media, selection,
        /// editable. More than one at once is normal.
        targets: Vec<ContextMenuTarget>,
        /// The link's href, when the click was on a link.
        link_url: String,
        /// The media element's source, when the click was on one.
        source_url: String,
        /// The page's own URL.
        page_url: String,
        /// Any selected text under the cursor.
        selection_text: String,
        /// wubble change: the URL of the frame clicked in.
        frame_url: String,
        /// wubble change 42: CEF's identifier of the frame clicked in (for
        /// [`CefSurfaceProducer::execute_script_in_frame`]); empty when
        /// CEF gave no frame.
        frame_id: String,
        /// wubble change: the media element clicked on.
        media: ContextMenuMedia,
        /// wubble change: whether the click was on an image that has loaded.
        has_image_contents: bool,
        /// wubble change: whether the click was in an editable field.
        editable: bool,
        /// wubble change: CEF's `cef_context_menu_edit_state_flags_t`
        /// (can undo, redo, cut, copy, paste, delete, select all, ...).
        edit_flags: u32,
        /// wubble change (30): CEF's `cef_context_menu_media_state_flags_t` for the
        /// media element clicked on (paused, muted, loop, controls, ...).
        media_flags: u32,
        /// wubble change: the misspelled word under the cursor, if any, and
        /// the spellchecker's suggestions for it.
        misspelled_word: String,
        suggestions: Vec<String>,
        /// wubble change: CEF's default menu for this click, invisible items
        /// left out, submenus at most [`MAX_MENU_DEPTH`] deep.
        items: Vec<MenuItem>,
    },
    /// A page asked for something Chromium gates behind a prompt: the
    /// camera, the microphone, the user's location, notifications.
    ///
    /// Answer with [`CefSurfaceProducer::grant_permission`] or
    /// [`CefSurfaceProducer::deny_permission`], quoting `id`. If
    /// `CefSurfaceConfig::handle_permission_requests` is off, `welding` has
    /// already denied it and this is a notification rather than a question.
    PermissionRequested {
        id: PermissionId,
        /// The origin asking, e.g. `https://example.com/`.
        origin: String,
        /// What was asked for. Bits this build does not name still arrive, as
        /// `PermissionKind::Other`.
        permissions: Vec<PermissionKind>,
        /// CEF's raw bitmask, for hosts that want to be exact.
        raw: u32,
    },
    /// A server or proxy demanded credentials.
    ///
    /// Carries no credentials, only the challenge. Answer with
    /// [`CefSurfaceProducer::answer_auth`] or decline with
    /// [`CefSurfaceProducer::cancel_auth`], quoting `id`. If
    /// `CefSurfaceConfig::handle_auth_challenges` is left off, `welding` has
    /// already declined it by the time this arrives and the id is spent — the
    /// event is then a notification, not a question.
    ///
    /// CEF has one challenge channel and reports whether it came from a proxy;
    /// it does not say whether a page load or a download provoked it, so
    /// unlike `scrying` there is no page/download split to report.
    AuthChallenged {
        id: AuthId,
        /// The URL whose load triggered the challenge.
        origin_url: String,
        /// Host the credentials are for.
        host: String,
        port: u16,
        realm: String,
        /// e.g. `basic`, `digest`, `negotiate`.
        scheme: String,
        /// True when a proxy is asking rather than the origin server.
        is_proxy: bool,
    },
    /// A download began. `welding` has already chosen `destination_path`
    /// under the configured download directory and accepted the transfer; the
    /// host owns any UI. The `id` ties the later events to this one.
    DownloadStarted {
        id: DownloadId,
        url: String,
        suggested_filename: String,
        destination_path: std::path::PathBuf,
        /// What the server announced, when it announced anything.
        total_bytes_expected: Option<u64>,
    },
    /// Throttled progress, at most ten a second per download, plus a final one
    /// when it finishes. `bytes_received` is cumulative.
    DownloadProgress {
        id: DownloadId,
        bytes_received: u64,
        total_bytes_expected: Option<u64>,
    },
    /// A download ended. `error` is `Some` when it failed, in which case the
    /// file may be partial or absent. Host-driven cancellation arrives as
    /// `DownloadCancelled` instead.
    DownloadFinished {
        id: DownloadId,
        destination_path: std::path::PathBuf,
        error: Option<String>,
    },
    /// A download was cancelled, either by `cancel_download` or by CEF.
    ///
    /// There is no resume blob: CEF exposes live pause and resume on a running
    /// download but nothing that survives the process, so a cancelled download
    /// starts over.
    DownloadCancelled {
        id: DownloadId,
        destination_path: std::path::PathBuf,
    },
    ConsoleMessage {
        level: i32,
        message: String,
        source: String,
        line: i32,
    },
    /// A download wants a destination (only with
    /// [`CefSurfaceConfig::ask_download_path`]). Answer with
    /// [`CefSurfaceProducer::answer_download_path`] quoting `id`;
    /// `DownloadStarted` follows an accepted one. wubble change.
    DownloadRequested {
        id: RequestId,
        download: DownloadId,
        url: String,
        /// wubble change 41: the URL the download was started for, before
        /// redirects (what `start_download` was given).
        original_url: String,
        suggested_filename: String,
        total_bytes_expected: Option<u64>,
    },
    /// `alert`, `confirm`, `prompt` or a `beforeunload` confirmation. The
    /// page waits until [`CefSurfaceProducer::answer_dialog`] quotes `id`.
    /// wubble change.
    JsDialogRequested {
        id: RequestId,
        kind: JsDialogKind,
        origin_url: String,
        message: String,
        /// The prompt's default input; empty for the other kinds.
        default_text: String,
    },
    /// CEF closed a context menu the host had not answered yet (navigation,
    /// or CEF cancelled it); the host should close what it shows. wubble
    /// change.
    ContextMenuDismissed,
    /// CEF dropped any JS dialog still showing (navigation, or a dialog
    /// closed); the host should close what it shows. wubble change.
    JsDialogsReset,
    /// `<input type=file>` and `showOpenFilePicker`: answer with
    /// [`CefSurfaceProducer::answer_file_chooser`]. wubble change.
    FileChooserRequested {
        id: RequestId,
        mode: FileChooserMode,
        title: String,
        default_path: std::path::PathBuf,
        accept: Vec<AcceptFilter>,
    },
    /// A certificate error on `url`. The load waits until
    /// [`CefSurfaceProducer::answer_certificate`]; the default is cancel.
    /// `error_code` is Chromium's net error (`ERR_CERT_*`, -200 to -219).
    /// wubble change.
    CertificateError {
        id: RequestId,
        url: String,
        error_code: i32,
    },
    /// The page entered or left fullscreen (`requestFullscreen`). Under
    /// windowless rendering the host grows the view itself. wubble change.
    FullscreenChanged { fullscreen: bool },
    /// A download was interrupted (network, disk or server error;
    /// Chromium's `interrupt_reason`, `cef_download_interrupt_reason_t`).
    /// Not an end: [`CefSurfaceProducer::resume_download`] may retry it,
    /// CEF may resume it itself (progress follows), and
    /// [`CefSurfaceProducer::cancel_download`] ends it. wubble change.
    DownloadInterrupted { id: DownloadId, reason: i32 },
    /// CEF gave up on these held requests without an answer (the page they
    /// came from was left, or a permission prompt was dismissed): close
    /// whatever shows them; answering them now fails. wubble change.
    RequestsDismissed { ids: Vec<RequestId> },
    /// CEF's `OnStatusMessage`: the text a browser shows in its status
    /// bubble, which is the hovered link's URL; empty hides it. wubble
    /// change 39.
    StatusChanged { text: String },
    /// CEF's `OnTooltip`: a `title` tooltip to draw near the pointer (under
    /// windowless rendering the host draws it); empty hides it. wubble
    /// change 39.
    Tooltip { text: String },
    /// A navigation or request to a scheme Chromium has no handler for
    /// (`mailto:`, `tel:`, an app's own scheme). welding never lets CEF
    /// hand it to the OS; the host decides and opens it itself.
    /// `user_gesture` is what `OnBeforeBrowse` said for the same URL, when
    /// it ran for it. wubble change 40.
    ProtocolExecution { url: String, user_gesture: Option<bool> },
    /// The [`NavigationGuard`] cancelled a top-level navigation to `url`
    /// (`redirect`: a hop of a redirect) before its request was sent.
    /// wubble change 53.
    NavigationRefused { url: String, redirect: bool },
}

/// Which JS dialog a page opened. wubble change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JsDialogKind {
    Alert,
    Confirm,
    Prompt,
    /// "Leave site?" before a navigation, or before a reload.
    BeforeUnload { is_reload: bool },
}

impl JsDialogKind {
    /// The answer a dismissed or dropped dialog of this kind gets: cancel,
    /// except a `beforeunload`, which leaves the page so a host that dropped
    /// it never traps the user there.
    pub fn default_accept(self) -> bool {
        matches!(self, Self::BeforeUnload { .. })
    }
}

/// Map CEF's `cef_jsdialog_type_t`. An unknown value is treated as an alert,
/// the one kind whose answer carries nothing. wubble change.
#[cfg_attr(not(feature = "cef-runtime"), allow(dead_code))]
pub(crate) fn js_dialog_kind(raw: i64) -> JsDialogKind {
    match raw {
        1 => JsDialogKind::Confirm,
        2 => JsDialogKind::Prompt,
        _ => JsDialogKind::Alert,
    }
}

/// What a file chooser picks. wubble change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileChooserMode {
    Open,
    OpenMultiple,
    OpenFolder,
    Save,
}

/// Map CEF's `cef_file_dialog_mode_t`. wubble change.
#[cfg_attr(not(feature = "cef-runtime"), allow(dead_code))]
pub(crate) fn file_chooser_mode(raw: i64) -> FileChooserMode {
    match raw {
        1 => FileChooserMode::OpenMultiple,
        2 => FileChooserMode::OpenFolder,
        3 => FileChooserMode::Save,
        _ => FileChooserMode::Open,
    }
}

/// One entry of a file chooser's `accept`: a MIME type (`image/*`) or an
/// extension (`.png`), the extensions it stands for, and a description
/// (both possibly empty). wubble change.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AcceptFilter {
    pub filter: String,
    pub extensions: Vec<String>,
    pub description: String,
}

/// Zip CEF's three parallel lists. CEF documents them as equal length; a
/// shorter one is padded with empty values rather than dropping filters.
/// Extensions arrive `;`-separated. wubble change.
#[cfg_attr(not(feature = "cef-runtime"), allow(dead_code))]
pub(crate) fn accept_filters(
    filters: Vec<String>,
    extensions: Vec<String>,
    descriptions: Vec<String>,
) -> Vec<AcceptFilter> {
    let mut extensions = extensions.into_iter();
    let mut descriptions = descriptions.into_iter();
    filters
        .into_iter()
        .map(|filter| AcceptFilter {
            filter,
            extensions: extensions
                .next()
                .unwrap_or_default()
                .split(';')
                .filter(|e| !e.is_empty())
                .map(str::to_owned)
                .collect(),
            description: descriptions.next().unwrap_or_default(),
        })
        .collect()
}

/// The media element a right-click landed on. wubble change.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ContextMenuMedia {
    #[default]
    None,
    Image,
    Video,
    Audio,
    Canvas,
    File,
    Plugin,
}

/// Map CEF's `cef_context_menu_media_type_t`. wubble change.
#[cfg_attr(not(feature = "cef-runtime"), allow(dead_code))]
pub(crate) fn context_menu_media(raw: i64) -> ContextMenuMedia {
    match raw {
        1 => ContextMenuMedia::Image,
        2 => ContextMenuMedia::Video,
        3 => ContextMenuMedia::Audio,
        4 => ContextMenuMedia::Canvas,
        5 => ContextMenuMedia::File,
        6 => ContextMenuMedia::Plugin,
        _ => ContextMenuMedia::None,
    }
}

/// One entry of CEF's context menu. wubble change.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuItem {
    /// What [`CefSurfaceProducer::answer_context_menu`] takes. 0 for a
    /// separator.
    pub command_id: i32,
    pub label: String,
    pub kind: MenuItemKind,
    pub enabled: bool,
    pub checked: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MenuItemKind {
    Command,
    Check,
    Radio,
    Separator,
    SubMenu(Vec<MenuItem>),
}

/// Submenus deeper than this are left out of
/// [`NavigationEvent::ContextMenuRequested`]. wubble change.
pub const MAX_MENU_DEPTH: usize = 2;

/// Read access to a menu model; CEF's `MenuModel`, or a fake in tests.
/// wubble change.
#[cfg_attr(not(feature = "cef-runtime"), allow(dead_code))]
pub(crate) trait MenuSource: Sized {
    fn count(&self) -> usize;
    /// `cef_menu_item_type_t` as an integer.
    fn kind_at(&self, index: usize) -> i64;
    fn command_id_at(&self, index: usize) -> i32;
    fn label_at(&self, index: usize) -> String;
    fn visible_at(&self, index: usize) -> bool;
    fn enabled_at(&self, index: usize) -> bool;
    fn checked_at(&self, index: usize) -> bool;
    fn sub_menu_at(&self, index: usize) -> Option<Self>;
}

/// The visible items of `menu`, submenus nested at most `MAX_MENU_DEPTH`
/// deep (`depth` is the current level, 1 for the top), with separators that
/// hiding left at either end or next to each other removed. wubble change.
#[cfg_attr(not(feature = "cef-runtime"), allow(dead_code))]
pub(crate) fn menu_items<M: MenuSource>(menu: &M, depth: usize) -> Vec<MenuItem> {
    let mut items: Vec<MenuItem> = Vec::new();
    for index in 0..menu.count() {
        if !menu.visible_at(index) {
            continue;
        }
        let kind = match menu.kind_at(index) {
            1 => MenuItemKind::Command,
            2 => MenuItemKind::Check,
            3 => MenuItemKind::Radio,
            4 => MenuItemKind::Separator,
            5 if depth < MAX_MENU_DEPTH => match menu.sub_menu_at(index) {
                Some(sub) => MenuItemKind::SubMenu(menu_items(&sub, depth + 1)),
                None => continue,
            },
            _ => continue,
        };
        let separator = kind == MenuItemKind::Separator;
        if separator
            && items
                .last()
                .is_none_or(|last| last.kind == MenuItemKind::Separator)
        {
            continue;
        }
        items.push(MenuItem {
            command_id: if separator { 0 } else { menu.command_id_at(index) },
            label: if separator { String::new() } else { menu.label_at(index) },
            kind,
            enabled: menu.enabled_at(index),
            checked: menu.checked_at(index),
        });
    }
    if items.last().is_some_and(|last| last.kind == MenuItemKind::Separator) {
        items.pop();
    }
    items
}

#[cfg(test)]
mod request_vocabulary_tests {
    use super::*;

    #[derive(Clone)]
    struct Fake(Vec<(i64, i32, &'static str, bool, Option<Fake>)>);

    impl MenuSource for Fake {
        fn count(&self) -> usize {
            self.0.len()
        }
        fn kind_at(&self, i: usize) -> i64 {
            self.0[i].0
        }
        fn command_id_at(&self, i: usize) -> i32 {
            self.0[i].1
        }
        fn label_at(&self, i: usize) -> String {
            self.0[i].2.into()
        }
        fn visible_at(&self, i: usize) -> bool {
            self.0[i].3
        }
        fn enabled_at(&self, _: usize) -> bool {
            true
        }
        fn checked_at(&self, _: usize) -> bool {
            false
        }
        fn sub_menu_at(&self, i: usize) -> Option<Self> {
            self.0[i].4.clone()
        }
    }

    const SEP: (i64, i32, &str, bool, Option<Fake>) = (4, 0, "", true, None);

    fn labels(items: &[MenuItem]) -> Vec<&str> {
        items.iter().map(|i| i.label.as_str()).collect()
    }

    #[test]
    fn js_dialog_types_decode() {
        assert_eq!(js_dialog_kind(0), JsDialogKind::Alert);
        assert_eq!(js_dialog_kind(1), JsDialogKind::Confirm);
        assert_eq!(js_dialog_kind(2), JsDialogKind::Prompt);
        assert_eq!(js_dialog_kind(77), JsDialogKind::Alert);
    }

    #[test]
    fn only_beforeunload_defaults_to_accept() {
        for kind in [JsDialogKind::Alert, JsDialogKind::Confirm, JsDialogKind::Prompt] {
            assert!(!kind.default_accept(), "{kind:?}");
        }
        assert!(JsDialogKind::BeforeUnload { is_reload: false }.default_accept());
        assert!(JsDialogKind::BeforeUnload { is_reload: true }.default_accept());
    }

    #[test]
    fn file_chooser_modes_decode() {
        let got: Vec<_> = (0..5).map(file_chooser_mode).collect();
        use FileChooserMode::*;
        assert_eq!(got, vec![Open, OpenMultiple, OpenFolder, Save, Open]);
    }

    #[test]
    fn accept_filters_zip_and_pad() {
        let got = accept_filters(
            vec!["image/*".into(), ".txt".into()],
            vec![".png;.jpg".into()],
            vec!["Images".into(), "Text".into(), "extra".into()],
        );
        assert_eq!(
            got,
            vec![
                AcceptFilter {
                    filter: "image/*".into(),
                    extensions: vec![".png".into(), ".jpg".into()],
                    description: "Images".into(),
                },
                AcceptFilter {
                    filter: ".txt".into(),
                    extensions: vec![],
                    description: "Text".into(),
                },
            ]
        );
    }

    #[test]
    fn hidden_items_and_dangling_separators_are_left_out() {
        let menu = Fake(vec![
            SEP,
            (1, 100, "Back", true, None),
            (1, 101, "Forward", false, None),
            SEP,
            SEP,
            (1, 102, "Reload", true, None),
            (1, 132, "View source", false, None),
            SEP,
        ]);
        let items = menu_items(&menu, 1);
        assert_eq!(labels(&items), vec!["Back", "", "Reload"]);
        assert_eq!(items[1].kind, MenuItemKind::Separator);
        assert_eq!(items[1].command_id, 0);
        assert_eq!(items[2].command_id, 102);
    }

    #[test]
    fn submenus_nest_up_to_the_depth_cap() {
        let deepest = Fake(vec![(1, 3, "three", true, None)]);
        let deeper = Fake(vec![(1, 2, "two", true, None), (5, 0, "more", true, Some(deepest))]);
        let menu = Fake(vec![(1, 1, "one", true, None), (5, 0, "sub", true, Some(deeper))]);
        let items = menu_items(&menu, 1);
        assert_eq!(labels(&items), vec!["one", "sub"]);
        let MenuItemKind::SubMenu(sub) = &items[1].kind else { panic!("{items:?}") };
        // Level 2 is shown, its own submenu (level 3) is not.
        assert_eq!(labels(sub), vec!["two"]);
    }

    #[test]
    fn media_types_decode() {
        assert_eq!(context_menu_media(0), ContextMenuMedia::None);
        assert_eq!(context_menu_media(1), ContextMenuMedia::Image);
        assert_eq!(context_menu_media(6), ContextMenuMedia::Plugin);
        assert_eq!(context_menu_media(99), ContextMenuMedia::None);
    }

    // wubble change: the request filter.
    #[test]
    fn no_request_filter_by_default() {
        assert!(CefSurfaceConfig::default().request_filter.is_none());
        assert!(CefSurfaceConfig::default().response_observer.is_none());
        assert_eq!(format!("{:?}", RequestFilter::new(|_| RequestAction::Allow)), "RequestFilter");
    }

    #[test]
    // wubble change 36.
    fn response_info_reads_the_headers() {
        let headers = |name: &str| {
            match name {
                "Content-Length" => " 1234 ",
                "Content-Range" => "bytes 0-99/5000",
                "Accept-Ranges" => "Bytes",
                _ => "",
            }
            .to_owned()
        };
        let info = ResponseInfo::from_headers("https://v.test/a".into(), 3, 8, 206, "Video/MP4; codecs=avc1", headers);
        assert_eq!(info.mime, "video/mp4");
        assert_eq!(info.content_length, Some(1234));
        assert_eq!(info.content_range, "bytes 0-99/5000");
        assert!(info.accepts_ranges);
        assert_eq!((info.browser_id, info.resource_type, info.status), (3, 8, 206));
        let bare = ResponseInfo::from_headers(String::new(), 0, 0, 200, "", |_| "x".into());
        assert_eq!((bare.mime.as_str(), bare.content_length, bare.accepts_ranges), ("", None, false));
        let panics = ResponseObserver::new(|_| panic!("observer bug"));
        panics.seen(&bare);
    }

    #[test]
    fn a_panicking_request_filter_allows() {
        let request = ResourceRequest { url: "https://a.test/".into(), ..Default::default() };
        let cancel = RequestFilter::new(|r| {
            if r.url.contains("a.test") { RequestAction::Cancel } else { RequestAction::Allow }
        });
        assert_eq!(cancel.check(&request), RequestAction::Cancel);
        let panics = RequestFilter::new(|_| panic!("filter bug"));
        assert_eq!(panics.check(&request), RequestAction::Allow);
    }
}

#[cfg(test)]
mod context_menu_tests {
    use super::*;

    #[test]
    fn the_flags_seen_from_a_real_right_click_decode() {
        // Observed: a right-click on plain page text reported 3.
        assert_eq!(
            context_menu_targets(3),
            vec![ContextMenuTarget::Page, ContextMenuTarget::Frame]
        );
    }

    #[test]
    fn a_link_inside_a_page_reports_both() {
        let got = context_menu_targets(1 | 1 << 2);
        assert!(got.contains(&ContextMenuTarget::Page));
        assert!(got.contains(&ContextMenuTarget::Link));
    }

    #[test]
    fn an_unnamed_flag_is_reported_not_dropped() {
        let odd = 1 << 20;
        assert_eq!(
            context_menu_targets(odd),
            vec![ContextMenuTarget::Other(odd)]
        );
    }

    #[test]
    fn nothing_under_the_cursor_decodes_to_nothing() {
        assert!(context_menu_targets(0).is_empty());
    }

    #[test]
    fn drag_effects_are_a_set_not_a_single_choice() {
        let effects = DragOperations::COPY | DragOperations::MOVE;
        assert_eq!(effects.0, 0b1_0001);
        assert_eq!(DragOperations::NONE.0, 0);
    }
}

// wubble change: the navigation guard fails closed.
#[cfg(test)]
mod navigation_guard_tests {
    use super::*;

    #[test]
    fn a_panicking_guard_refuses() {
        let guard = NavigationGuard::new(|url| {
            assert!(!url.contains("boom"), "the rule broke");
            true
        });
        assert!(guard.allows("https://a.test/"));
        assert!(!guard.allows("https://boom.test/"));
    }
}
