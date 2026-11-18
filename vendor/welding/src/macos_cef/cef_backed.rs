// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! CEF handler vtables for the macOS producer: render handler (accelerated
//! OSR paint), life-span, client, load, and display handlers.
//!
//! Split out of `macos_cef/mod.rs`; the producer itself stays there.

use super::*;
use cef::*;
use std::ffi::c_void;

fn push_navigation(events: &WebEventQueue, event: crate::surface::NavigationEvent) {
    events.push(CefSurfaceEvent::Navigation(event));
}

cef::wrap_render_handler! {
    pub(super) struct WeldRenderHandler {
        handler: WeldRenderHandlerInner,
    }

    impl RenderHandler {
        fn view_rect(
            &self,
            _browser: Option<&mut cef::Browser>,
            rect: Option<&mut cef::Rect>,
        ) {
            if let Some(rect) = rect {
                // GetViewRect is answered in DIP, not physical pixels.
                // wubble change: x, y carry the view's offset in the window.
                let (x, y, w, h) = self.handler.metrics.lock().unwrap().view_rect_dip();
                log::trace!("view_rect -> {x},{y} {w}x{h} dip");
                rect.x = x;
                rect.y = y;
                rect.width = w;
                rect.height = h;
            }
        }

        fn screen_info(
            &self,
            _browser: Option<&mut cef::Browser>,
            screen_info: Option<&mut cef::ScreenInfo>,
        ) -> ::std::os::raw::c_int {
            if let Some(info) = screen_info {
                let metrics = self.handler.metrics.lock().unwrap();
                info.device_scale_factor = metrics.scale();
                // wubble change: the window is the screen, so Blink can place
                // popups outside the view (cefclient `real_screen_bounds`).
                if let Some((x, y, w, h)) = metrics.screen_rect_dip() {
                    let rect = cef::Rect { x, y, width: w, height: h };
                    info.rect = rect.clone();
                    info.available_rect = rect;
                }
                return 1;
            }
            0
        }

        fn root_screen_rect(
            &self,
            _browser: Option<&mut cef::Browser>,
            rect: Option<&mut cef::Rect>,
        ) -> ::std::os::raw::c_int {
            // wubble change: see `screen_info`.
            let area = self.handler.metrics.lock().unwrap().screen_rect_dip();
            match (rect, area) {
                (Some(rect), Some((x, y, w, h))) => {
                    *rect = cef::Rect { x, y, width: w, height: h };
                    1
                }
                _ => 0,
            }
        }

        fn screen_point(
            &self,
            _browser: Option<&mut cef::Browser>,
            view_x: ::std::os::raw::c_int,
            view_y: ::std::os::raw::c_int,
            screen_x: Option<&mut ::std::os::raw::c_int>,
            screen_y: Option<&mut ::std::os::raw::c_int>,
        ) -> ::std::os::raw::c_int {
            // wubble change: view point to screen point (device px; DIP on
            // macOS), as cefclient's `GetScreenPoint` does.
            let point = self.handler.metrics.lock().unwrap().screen_point_dip(view_x, view_y);
            match (screen_x, screen_y, point) {
                (Some(sx), Some(sy), Some((x, y))) => {
                    *sx = x;
                    *sy = y;
                    1
                }
                _ => 0,
            }
        }

        // wubble change: with `shared_texture_enabled` CEF calls OnPaint only
        // when it cannot hand over a shared texture, so every call is a CPU
        // fallback frame (popups included). Counted, never read or shown.
        fn on_paint(
            &self,
            _browser: Option<&mut cef::Browser>,
            type_: cef::PaintElementType,
            _dirty_rects: Option<&[cef::Rect]>,
            _buffer: *const u8,
            width: ::std::os::raw::c_int,
            height: ::std::os::raw::c_int,
        ) {
            let n = self.handler.cpu_paints.fetch_add(1, Ordering::Relaxed) + 1;
            log::debug!("on_paint (CPU fallback #{n}): {type_:?} {width}x{height}");
        }

        #[cfg(feature = "cef-runtime")]
        fn on_accelerated_paint(
            &self,
            _browser: Option<&mut cef::Browser>,
            type_: cef::PaintElementType,
            _dirty_rects: Option<&[cef::Rect]>,
            info: Option<&cef::AcceleratedPaintInfo>,
        ) {
            // CEF paints the popup widget (select dropdowns, autocomplete) as
            // its own element rather than compositing it into the view, so it
            // lands here with PET_POPUP and goes to a separate slot.
            let is_popup = type_ == cef::PaintElementType::POPUP;
            // wubble change: stamped for `CopiedFrame::arrived`.
            let arrived = std::time::Instant::now();
            let Some(info) = info else { return };
            // macOS names this field for what it is: the paint info carries an
            // IOSurfaceRef, not the Windows shared HANDLE.
            let io_surface = info.shared_texture_io_surface;
            if io_surface.is_null() {
                return;
            }

            // Retain the IOSurface for the import; `MetalTextureRef` releases
            // it when dropped. IOSurfaceRef is a CFTypeRef; use CFRetain.
            unsafe extern "C" {
                fn CFRetain(cf: *const c_void) -> *const c_void;
            }
            unsafe { CFRetain(io_surface) };

            let width = info.extra.coded_size.width as u32;
            let height = info.extra.coded_size.height as u32;
            let format = match *info.format.as_ref() {
                cef::sys::cef_color_type_t::CEF_COLOR_TYPE_RGBA_8888 => {
                    wgpu::TextureFormat::Rgba8Unorm
                }
                _ => wgpu::TextureFormat::Bgra8Unorm,
            };

            let mut slot = if is_popup {
                self.handler.popup_slot.lock().unwrap()
            } else {
                self.handler.frame_slot.lock().unwrap()
            };
            let generation = slot.next_generation();
            let frame = unsafe {
                crate::native_frame::MetalTextureRef::from_retained_raw_io_surface(
                    io_surface as *mut c_void,
                    PhysicalSize::new(width, height),
                    format,
                    generation,
                )
            };
            // wubble change: copy before returning, see `copy_metal_to_owned`.
            let copied = frame
                .map_err(|error| format!("failed to wrap retained CEF IOSurface: {error}"))
                .and_then(|frame| {
                    crate::native_frame::copy_metal_to_owned(frame, &self.handler.gpu, arrived)
                        .map_err(|error| format!("failed to copy CEF IOSurface: {error}"))
                });
            match copied {
                Ok(copy) => slot.store(crate::native_frame::NativeFrame::Copied(copy)),
                Err(error) => log::error!("{error}"),
            }
        }

        fn on_ime_composition_range_changed(
            &self,
            _browser: Option<&mut cef::Browser>,
            selected_range: Option<&cef::Range>,
            character_bounds: Option<&[cef::Rect]>,
        ) {
            let rects: Vec<crate::surface::PopupRect> = character_bounds
                .unwrap_or(&[])
                .iter()
                .map(|r| crate::surface::PopupRect {
                    x: r.x,
                    y: r.y,
                    width: r.width.max(0) as u32,
                    height: r.height.max(0) as u32,
                })
                .collect();
            let Some(bounds) = crate::ime::bounds_union(&rects) else { return };
            // CEF reports these in DIP, like popup geometry.
            let bounds = self.handler.metrics.lock().unwrap().rect_to_physical(bounds);
            let (start, end) = selected_range
                .map(|r| (r.from, r.to))
                .unwrap_or((0, 0));
            self.handler.ime.set(crate::surface::ImeComposition {
                bounds,
                selection_start: start,
                selection_end: end,
            });
        }

        fn on_popup_show(&self, _browser: Option<&mut cef::Browser>, show: ::std::os::raw::c_int) {
            let showing = show != 0;
            log::debug!("on_popup_show({showing})");
            self.handler.popup.set_visible(showing);
            if !showing {
                // A hidden popup never paints again; drop the stale surface so
                // acquire_popup cannot hand back a dropdown that is gone.
                let _ = self.handler.popup_slot.lock().unwrap().take();
            }
        }

        fn on_popup_size(&self, _browser: Option<&mut cef::Browser>, rect: Option<&cef::Rect>) {
            let Some(rect) = rect else { return };
            log::debug!(
                "on_popup_size({}x{} at {},{})",
                rect.width, rect.height, rect.x, rect.y
            );
            self.handler.popup.set_rect(crate::surface::PopupRect {
                x: rect.x,
                y: rect.y,
                width: rect.width.max(0) as u32,
                height: rect.height.max(0) as u32,
            });
        }

        fn start_dragging(
            &self,
            _browser: Option<&mut cef::Browser>,
            drag_data: Option<&mut cef::DragData>,
            allowed_ops: cef::DragOperationsMask,
            x: ::std::os::raw::c_int,
            y: ::std::os::raw::c_int,
        ) -> ::std::os::raw::c_int {
            let Some(drag_data) = drag_data else { return 0 };
            // wubble change: refuse the drag unless the host can run one;
            // CEF waits for DragSourceEndedAt after a true here.
            if !self.handler.requests.drag_source() { return 0; }
            let scale = self.handler.metrics.lock().unwrap().scale();
            push_navigation(
                &self.handler.events,
                crate::surface::NavigationEvent::DragStarted {
                    payload: crate::drag::payload_from_cef(drag_data),
                    allowed_operations: crate::DragOperations(allowed_ops.as_ref().0 as u32),
                    x: (x as f32 * scale).round() as i32,
                    y: (y as f32 * scale).round() as i32,
                }
            );
            1
        }
    }
}

impl WeldRenderHandler {
    pub fn build(inner: WeldRenderHandlerInner) -> cef::RenderHandler {
        Self::new(inner)
    }
}

cef::wrap_client! {
    pub(super) struct WeldClient {
        render_handler: cef::RenderHandler,
        load_handler: cef::LoadHandler,
        display_handler: cef::DisplayHandler,
        life_span_handler: cef::LifeSpanHandler,
        // wubble change: the request, download, permission, context-menu,
        // JS dialog and file dialog handlers (`crate::handlers`).
        shared: crate::handlers::SharedHandlers,
        find_handler: cef::FindHandler,
        scripts: Arc<crate::app::PendingScripts>,
        events: Arc<WebEventQueue>,
    }

    impl Client {
        fn render_handler(&self) -> Option<cef::RenderHandler> {
            Some(self.render_handler.clone())
        }

        fn load_handler(&self) -> Option<cef::LoadHandler> {
            Some(self.load_handler.clone())
        }

        fn life_span_handler(&self) -> Option<cef::LifeSpanHandler> {
            Some(self.life_span_handler.clone())
        }

        fn request_handler(&self) -> Option<cef::RequestHandler> {
            Some(self.shared.request.clone())
        }

        fn download_handler(&self) -> Option<cef::DownloadHandler> {
            Some(self.shared.download.clone())
        }

        fn permission_handler(&self) -> Option<cef::PermissionHandler> {
            Some(self.shared.permission.clone())
        }

        fn context_menu_handler(&self) -> Option<cef::ContextMenuHandler> {
            Some(self.shared.context_menu.clone())
        }

        fn jsdialog_handler(&self) -> Option<cef::JsdialogHandler> {
            Some(self.shared.jsdialog.clone())
        }

        fn dialog_handler(&self) -> Option<cef::DialogHandler> {
            Some(self.shared.dialog.clone())
        }

        fn find_handler(&self) -> Option<cef::FindHandler> {
            Some(self.find_handler.clone())
        }

        fn display_handler(&self) -> Option<cef::DisplayHandler> {
            Some(self.display_handler.clone())
        }

        // wubble change: unhandled keys for the host's page-first shortcuts.
        fn keyboard_handler(&self) -> Option<cef::KeyboardHandler> {
            Some(crate::keyboard::WeldKeyboardHandler::build(self.events.clone()))
        }

        fn on_process_message_received(
            &self,
            _browser: Option<&mut cef::Browser>,
            frame: Option<&mut cef::Frame>,
            source_process: cef::ProcessId,
            message: Option<&mut cef::ProcessMessage>,
        ) -> ::std::os::raw::c_int {
            if source_process != cef::ProcessId::RENDERER { return 0; }
            let Some(msg) = message else { return 0 };
            // wubble change: the context script's frame messages.
            if crate::handlers::frame_message_received(&self.events, frame, msg) { return 1; }
            let name = cef::CefString::from(&msg.name()).to_string();
            if name == crate::app::EVAL_RESULT {
                // The renderer answering a request_script_result.
                if let Some(args) = msg.argument_list() {
                    let id = cef::CefString::from(&args.string(0)).to_string();
                    let Ok(id) = id.parse::<u64>() else {
                        log::error!("CEF returned an invalid Weld request id: {id}");
                        return 1;
                    };
                    let ok = args.int(1) != 0;
                    let payload = cef::CefString::from(&args.string(2)).to_string();
                    self.scripts.complete(
                        WebRequestId::new(id),
                        if ok { Ok(payload) } else { Err(payload) },
                    );
                }
                return 1;
            }
            if name != "weld.message" { return 0; }
            if let Some(args) = msg.argument_list() {
                let text = cef::CefString::from(&args.string(0)).to_string();
                self.events.push(CefSurfaceEvent::WebMessage(text));
            }
            1
        }
    }
}

impl WeldClient {
    pub fn build(
        render_handler: cef::RenderHandler,
        load_handler: cef::LoadHandler,
        display_handler: cef::DisplayHandler,
        life_span_handler: cef::LifeSpanHandler,
        shared: crate::handlers::SharedHandlers,
        find_handler: cef::FindHandler,
        scripts: Arc<crate::app::PendingScripts>,
        events: Arc<WebEventQueue>,
    ) -> cef::Client {
        Self::new(
            render_handler,
            load_handler,
            display_handler,
            life_span_handler,
            shared,
            find_handler,
            scripts,
            events,
        )
    }
}

// ── Life-span handler: popup policy ───────────────────────────────────────

cef::wrap_life_span_handler! {
    pub(super) struct WeldLifeSpanHandler {
        events: Arc<WebEventQueue>,
        scripts: Arc<crate::app::PendingScripts>,
        cookies: Arc<crate::cookies::CookieJar>,
        requests: Arc<crate::handlers::Requests>,
    }

    impl LifeSpanHandler {
        // Popup browsers are denied and reported. welding renders one surface
        // per producer, so a second browser here would be invisible to the
        // host. The host decides what to do with the URL.
        #[allow(clippy::too_many_arguments)]
        fn on_before_popup(
            &self,
            _browser: Option<&mut cef::Browser>,
            _frame: Option<&mut cef::Frame>,
            _popup_id: ::std::os::raw::c_int,
            target_url: Option<&cef::CefString>,
            _target_frame_name: Option<&cef::CefString>,
            target_disposition: cef::WindowOpenDisposition,
            user_gesture: ::std::os::raw::c_int,
            popup_features: Option<&cef::PopupFeatures>,
            _window_info: Option<&mut cef::WindowInfo>,
            _client: Option<&mut Option<cef::Client>>,
            _settings: Option<&mut cef::BrowserSettings>,
            _extra_info: Option<&mut Option<cef::DictionaryValue>>,
            _no_javascript_access: Option<&mut ::std::os::raw::c_int>,
        ) -> ::std::os::raw::c_int {
            let url = target_url.map(|u| u.to_string()).unwrap_or_default();
            push_navigation(
                &self.events,
                // wubble change 29: the disposition and size go with it.
                crate::handlers::new_window_requested(
                    url,
                    target_disposition,
                    user_gesture != 0,
                    popup_features,
                )
            );
            1 // cancel popup creation
        }

        fn on_before_close(&self, _browser: Option<&mut cef::Browser>) {
            log::debug!("CEF on_before_close");
            self.scripts.fail_all("browser closed");
            self.cookies.fail_active("browser closed");
            // wubble change: nothing waits on a host answer past the close.
            self.requests.drain_default();
            // wubble change: tell the host, so it need not poll for the close.
            self.events.push(CefSurfaceEvent::BrowserClosed);
        }
    }
}

impl WeldLifeSpanHandler {
    pub fn build(
        events: Arc<WebEventQueue>,
        scripts: Arc<crate::app::PendingScripts>,
        cookies: Arc<crate::cookies::CookieJar>,
        requests: Arc<crate::handlers::Requests>,
    ) -> cef::LifeSpanHandler {
        Self::new(events, scripts, cookies, requests)
    }
}

// ── Load handler ─────────────────────────────────────────────────────────

cef::wrap_load_handler! {
    pub(super) struct WeldLoadHandler {
        inner: WeldRenderHandlerInner,
    }

    impl LoadHandler {
        // wubble change: loading state and history flags for the host.
        fn on_loading_state_change(
            &self,
            _browser: Option<&mut cef::Browser>,
            is_loading: ::std::os::raw::c_int,
            can_go_back: ::std::os::raw::c_int,
            can_go_forward: ::std::os::raw::c_int,
        ) {
            self.inner.events.push(CefSurfaceEvent::Navigation(
                crate::surface::NavigationEvent::LoadingStateChanged {
                    loading: is_loading != 0,
                    can_go_back: can_go_back != 0,
                    can_go_forward: can_go_forward != 0,
                },
            ));
        }

        fn on_load_start(
            &self,
            _browser: Option<&mut cef::Browser>,
            frame: Option<&mut cef::Frame>,
            _transition_type: cef::TransitionType,
        ) {
            let is_main = frame.as_ref().map(|f| f.is_main() != 0).unwrap_or(false);
            if !is_main { return; }
            // wubble change: requests of the page being left are stale.
            self.inner.requests.page_left();
            let url = frame.map(|f| cef::CefString::from(&f.url()).to_string()).unwrap_or_default();
            push_navigation(
                &self.inner.events,
                crate::surface::NavigationEvent::LoadStart { url }
            );
        }

        fn on_load_end(
            &self,
            _browser: Option<&mut cef::Browser>,
            frame: Option<&mut cef::Frame>,
            http_status_code: ::std::os::raw::c_int,
        ) {
            let is_main = frame.as_ref().map(|f| f.is_main() != 0).unwrap_or(false);
            if !is_main { return; }
            let url = frame.map(|f| cef::CefString::from(&f.url()).to_string()).unwrap_or_default();
            push_navigation(
                &self.inner.events,
                crate::surface::NavigationEvent::LoadEnd {
                    url,
                    http_status: http_status_code,
                }
            );
        }

        fn on_load_error(
            &self,
            _browser: Option<&mut cef::Browser>,
            frame: Option<&mut cef::Frame>,
            _error_code: cef::Errorcode,
            error_text: Option<&cef::CefString>,
            failed_url: Option<&cef::CefString>,
        ) {
            let is_main = frame.as_ref().map(|f| f.is_main() != 0).unwrap_or(false);
            if !is_main { return; }
            let url = failed_url.map(|u| u.to_string()).unwrap_or_default();
            let text = error_text.map(|t| t.to_string()).unwrap_or_default();
            // Safety: Errorcode wraps cef_errorcode_t which wraps c_int;
            // both are #[repr(transparent)] around a 4-byte integer.
            let code: i32 = unsafe { std::mem::transmute(_error_code) };
            push_navigation(
                &self.inner.events,
                crate::surface::NavigationEvent::LoadError {
                    url,
                    error_code: code,
                    error_text: text,
                }
            );
        }
    }
}

impl WeldLoadHandler {
    pub fn build(inner: WeldRenderHandlerInner) -> cef::LoadHandler {
        Self::new(inner)
    }
}

// ── Display handler ───────────────────────────────────────────────────────

cef::wrap_display_handler! {
    pub(super) struct WeldDisplayHandler {
        inner: WeldRenderHandlerInner,
    }

    impl DisplayHandler {
        // wubble change: favicon URLs for the host.
        fn on_favicon_urlchange(
            &self,
            _browser: Option<&mut cef::Browser>,
            icon_urls: Option<&mut cef::CefStringList>,
        ) {
            let urls = icon_urls.map(crate::surface::borrowed_string_list).unwrap_or_default();
            self.inner.events.push(CefSurfaceEvent::Navigation(
                crate::surface::NavigationEvent::FaviconUrlsChanged { urls },
            ));
        }

        fn on_address_change(
            &self,
            _browser: Option<&mut cef::Browser>,
            frame: Option<&mut cef::Frame>,
            url: Option<&cef::CefString>,
        ) {
            let is_main = frame.as_ref().map(|f| f.is_main() != 0).unwrap_or(false);
            if !is_main { return; }
            let url = url.map(|u| u.to_string()).unwrap_or_default();
            push_navigation(
                &self.inner.events,
                crate::surface::NavigationEvent::AddressChanged { url }
            );
        }

        // CEF puts OnCursorChange on the display handler, not the render
        // handler, and types the handle differently per platform (*mut u8 here).
        fn on_cursor_change(
            &self,
            _browser: Option<&mut cef::Browser>,
            _cursor: *mut u8,
            type_: cef::CursorType,
            _custom_cursor_info: Option<&cef::CursorInfo>,
        ) -> ::std::os::raw::c_int {
            self.inner.cursor.set(crate::cursor::from_cef(type_));
            1 // handled: under OSR the host owns the pointer
        }

        fn on_console_message(
            &self,
            _browser: Option<&mut cef::Browser>,
            level: cef::LogSeverity,
            message: Option<&cef::CefString>,
            source: Option<&cef::CefString>,
            line: ::std::os::raw::c_int,
        ) -> ::std::os::raw::c_int {
            push_navigation(
                &self.inner.events,
                crate::surface::NavigationEvent::ConsoleMessage {
                    // cef_log_severity_t is repr(u32) here and repr(i32) on
                    // Windows, so go through the reference rather than From.
                    level: *level.as_ref() as i32,
                    message: message.map(|m| m.to_string()).unwrap_or_default(),
                    source: source.map(|s| s.to_string()).unwrap_or_default(),
                    line,
                }
            );
            0 // let CEF log it as well
        }

        fn on_title_change(
            &self,
            _browser: Option<&mut cef::Browser>,
            title: Option<&cef::CefString>,
        ) {
            let title = title.map(|t| t.to_string()).unwrap_or_default();
            push_navigation(
                &self.inner.events,
                crate::surface::NavigationEvent::TitleChanged { title }
            );
        }

        // wubble change: the page entered or left fullscreen.
        fn on_fullscreen_mode_change(
            &self,
            _browser: Option<&mut cef::Browser>,
            fullscreen: ::std::os::raw::c_int,
        ) {
            push_navigation(
                &self.inner.events,
                crate::surface::NavigationEvent::FullscreenChanged { fullscreen: fullscreen != 0 }
            );
        }
    }
}

impl WeldDisplayHandler {
    pub fn build(inner: WeldRenderHandlerInner) -> cef::DisplayHandler {
        Self::new(inner)
    }
}

// ── DevTools protocol observer ────────────────────────────────────────────

cef::wrap_dev_tools_message_observer! {
    pub(super) struct WeldDevToolsObserver {
        channel: Arc<crate::devtools::DevToolsChannel>,
        snapshots: Arc<crate::snapshot::SnapshotChannel>,
    }

    impl DevToolsMessageObserver {
        fn on_dev_tools_message(
            &self,
            _browser: Option<&mut cef::Browser>,
            message: Option<&[u8]>,
        ) -> ::std::os::raw::c_int {
            // The raw wire format, results and events alike. Taking it here
            // rather than from the parsed callbacks keeps what a host sees
            // identical to what the protocol documents.
            if self.channel.is_enabled()
                && let Some(bytes) = message
            {
                self.channel.push(String::from_utf8_lossy(bytes).into_owned());
            }
            // 0: not consumed, so CEF still runs its own parsed callbacks for
            // anything else that wants them.
            0
        }

        fn on_dev_tools_method_result(
            &self,
            _browser: Option<&mut cef::Browser>,
            message_id: ::std::os::raw::c_int,
            success: ::std::os::raw::c_int,
            result: Option<&[u8]>,
        ) {
            self.snapshots.complete(message_id, success != 0, result);
        }
    }
}

impl WeldDevToolsObserver {
    pub fn build(
        channel: Arc<crate::devtools::DevToolsChannel>,
        snapshots: Arc<crate::snapshot::SnapshotChannel>,
    ) -> cef::DevToolsMessageObserver {
        Self::new(channel, snapshots)
    }
}

// ── Find handler ──────────────────────────────────────────────────────────

cef::wrap_find_handler! {
    pub(super) struct WeldFindHandler {
        events: Arc<WebEventQueue>,
    }

    impl FindHandler {
        fn on_find_result(
            &self,
            _browser: Option<&mut cef::Browser>,
            _identifier: ::std::os::raw::c_int,
            count: ::std::os::raw::c_int,
            _selection_rect: Option<&cef::Rect>,
            active_match_ordinal: ::std::os::raw::c_int,
            final_update: ::std::os::raw::c_int,
        ) {
            push_navigation(
                &self.events,
                crate::surface::NavigationEvent::FindResult {
                    count,
                    active_match: active_match_ordinal,
                    final_update: final_update != 0,
                }
            );
        }
    }
}

impl WeldFindHandler {
    pub fn build(events: Arc<WebEventQueue>) -> cef::FindHandler {
        Self::new(events)
    }
}

// ── PDF print callback ────────────────────────────────────────────────────

cef::wrap_pdf_print_callback! {
    pub(super) struct WeldPdfCallback {
        events: Arc<WebEventQueue>,
    }

    impl PdfPrintCallback {
        fn on_pdf_print_finished(
            &self,
            path: Option<&cef::CefString>,
            ok: ::std::os::raw::c_int,
        ) {
            push_navigation(
                &self.events,
                crate::surface::NavigationEvent::PdfPrintFinished {
                    path: path.map(|p| p.to_string()).unwrap_or_default().into(),
                    ok: ok != 0,
                }
            );
        }
    }
}

impl WeldPdfCallback {
    pub fn build(events: Arc<WebEventQueue>) -> cef::PdfPrintCallback {
        Self::new(events)
    }
}
