# wubble changes to welding

Vendored from crates.io `welding` 0.15.0 (repository merely-made/wgpu-weld),
MPL-2.0, retargeted to `cef` 156.6 (change 7). Copied unmodified: `build.rs`, `Cargo.toml` (the registry's
normalised manifest), `README.md`, `src/`. Added: `LICENSE` (the MPL-2.0 text;
the published crate ships none), `rustfmt.toml` and this file. Every modified
file keeps its MPL-2.0 header and stays under MPL-2.0 (section 3.3).

The vendored copy is excluded from the wubble workspace, so
`cargo clippy --workspace` does not lint it. `cargo fmt --all` still formats
path dependencies, so `rustfmt.toml` here sets `disable_all_formatting = true`
to keep upstream formatting and a diff against 0.15.0 that holds only the
changes below. Test it with `cargo test -p welding`.

When moving to a newer welding (or CEF), start from the new upstream release
and re-apply each change below.

## Changes

Every change is marked `wubble change` in the source.

1. **Message-pump schedule hook.** `runtime.rs`: new `PumpScheduler`
   (an `Arc<dyn Fn(i64) + Send + Sync>` with a manual `Debug`, re-exported
   from `lib.rs`) and `CefRuntimeConfig::pump_scheduler` (default `None`).
   `app.rs`: new `WeldBrowserProcessHandler` whose
   `on_schedule_message_pump_work` calls the scheduler; `WeldApp` returns it
   from `browser_process_handler`; `WeldApp::build` takes the scheduler (the
   browser-process `initialize` paths pass the config's, subprocess entry
   points pass `None`). Lets the host schedule `do_message_loop_work` instead
   of polling it.
2. **Browser-closed event.** `surface.rs`: new `CefSurfaceEvent::BrowserClosed`.
   Pushed from `WeldLifeSpanHandler::on_before_close` on Linux, macOS and
   Windows (`*/cef_backed.rs`). Lets the host stop polling
   `browser_host_get_browser_by_identifier` for a close.
3. **CPU fallback count.** `linux_cef/cef_backed.rs`: `WeldRenderHandler`
   gains `on_paint`, which only counts (an `AtomicU64` in
   `WeldRenderHandlerInner`, `linux_cef/mod.rs`) and logs at debug level; the
   bitmap is never read. `surface.rs`: `CefSurfaceProducer::cpu_paint_count`
   (default `None`); `LinuxCefProducer` returns `Some(count)`. macOS and
   Windows still return `None`.
4. **Frame timing config.** `surface.rs`: `CefSurfaceConfig::windowless_frame_rate`
   (default 60) and `CefSurfaceConfig::external_begin_frame` (default false),
   the values welding hardcoded before; wired into `BrowserSettings` and
   `WindowInfo` in `linux_cef/mod.rs`, `macos_cef/mod.rs`, `windows_cef/mod.rs`.
5. **Loading state and favicon events.** `surface.rs`: new
   `NavigationEvent::LoadingStateChanged { loading, can_go_back,
   can_go_forward }` and `NavigationEvent::FaviconUrlsChanged { urls }`, plus
   `borrowed_string_list` (reads a `CefStringList` CEF still owns;
   `CefStringList::clone` would copy a borrowed list into an empty one).
   Pushed from new `WeldLoadHandler::on_loading_state_change` and
   `WeldDisplayHandler::on_favicon_urlchange` on Linux, macOS and Windows
   (`*/cef_backed.rs`). welding had no source for either: `can_go_back` /
   `can_go_forward` were only queryable, favicons not reported at all.
7. **`cef` 154.5.0 -> 156.6.0** (CEF 156.0.30, Chromium 156.0.8078.96).
   `Cargo.toml` `[dependencies.cef]` version only. CEF 156.0.30 is the first
   build whose shared-texture OSR hands over real, tiled dmabufs on NVIDIA
   and sends each frame only after its GPU work completes (CEF #4237,
   #4313). No source change: the 154.5 -> 156.6 binding diff is additive for
   welding (no `Impl*` trait signature changed; `Settings` and
   `BrowserSettings` are built with `..Default::default()`), and the import
   path already passes CEF's DRM modifier through. Upstream has no release
   for CEF 156 (2026-11-18: crates.io `welding` max 0.15.0 with `cef ^154.5`;
   wgpu-weld `main` at 9edccc08 still pins `cef` 154.5.0), so there was
   nothing to take.
8. **Frame copy inside `OnAcceleratedPaint` (Linux).** CEF returns each
   dmabuf to its pool when the callback returns, and NVIDIA dmabufs carry no
   implicit fence, so welding's import at present time could read a newer or
   half-written frame. `native_frame/vulkan_dmabuf.rs`: new
   `copy_dmabuf_to_owned` imports the frame (the existing grafting path) and
   copies it into a weld-owned texture, waiting on the copy's submission;
   ported from cef-rs `cef/src/osr_texture_import/common.rs`
   (`copy_to_owned`, `wait_for_copy`, cef-rs #498, MIT OR Apache-2.0).
   `native_frame/mod.rs`: new `NativeFrame::Copied(ImportedTexture)` /
   `NativeFrameKind::Copied`, whose import is a no-op; `ImportedTexture`
   derives `Debug`. `linux_cef/mod.rs`: `LinuxCefProducer::new` takes the
   host's `&HostWgpuContext` (as `WindowsCefProducer::new` already does),
   kept in `WeldRenderHandlerInner::gpu`. `linux_cef/cef_backed.rs`:
   `on_accelerated_paint` stores the copy, for the view and popups, instead
   of the dup'd dmabuf. macOS is unchanged.
Tests for the above: `runtime.rs` (`pump_scheduler_tests`), `surface.rs`
   (`frame_timing_defaults_match_upstream`; `BrowserClosed`,
   `LoadingStateChanged`, `FaviconUrlsChanged` and `KeyUnhandled` added to
   `mixed_callbacks_keep_observation_order`).
9. **One table for held requests.** New `held.rs`: `Held<C: Answer>` (ids
   from 1, never reused while waiting, `take` once, at most `MAX_HELD` = 64
   per kind, `drain` / `drain_default`, a `Drop` that answers what is left
   with its default) and `RequestId` (re-exported). `auth.rs` and
   `permissions.rs` hold their callbacks in it instead of an id counter and
   a `HashMap`; `permissions::Pending::Prompt` keeps the requested mask.
   `held::request_answers!` writes the producer methods that answer held
   requests once for the three producers. An opted-out auth or permission
   notice now carries id 0 (never waiting) and is pushed only when CEF gave
   a callback.
10. **Shared handlers.** New `handlers.rs` (cef-runtime): the request,
   download, permission and context-menu handlers move there from the three
   `*/cef_backed.rs` copies (their bodies were identical but for
   formatting), and `Requests` (auth, permissions, downloads, JS dialogs, file
   choosers, certificate errors, context menus, the drag-source switch)
   replaces each producer's `auth` / `permissions` / `downloads` fields.
   Each platform's `WeldClient` takes one `SharedHandlers` and returns its
   handlers, plus the three new ones below, from its accessors.
11. **Download control fixed.** `downloads.rs`: the latest
   `DownloadItemCallback` is kept per download (`DownloadControl`,
   `set_control`, `steer`) and `cancel/pause/resume_download` act at once.
   Upstream recorded the op and applied it on the next `OnDownloadUpdated`,
   which a paused download never sends, so resume after pause was never
   delivered. `CefDownloadItemCallbackImpl` holds only a weak manager and the
   id and posts to the UI thread, so keeping it is safe
   (`download_manager_delegate_impl.cc`). New
   `CefSurfaceConfig::ask_download_path` (default false, which keeps
   `download_dir` behaviour): `OnBeforeDownload`'s callback is held,
   `NavigationEvent::DownloadRequested` reports it and
   `CefSurfaceProducer::answer_download_path` answers (a path, or `None`,
   which drops the callback: CEF's cancel). An op sent for a started
   download before CEF handed over its first control is queued and applied
   when it arrives. Chromium's INTERRUPTED state (neither complete nor
   cancelled; upstream never reported it) is reported as
   `NavigationEvent::DownloadInterrupted { id, reason }`, keeping the control
   so `resume_download` can retry it.
12. **Context menu kept and held.** `OnBeforeContextMenu` no longer empties
   CEF's model (which stopped CEF from ever calling `RunContextMenu`,
   `menu_manager.cc`); `run_context_menu` reads params and model, holds the
   callback and returns 1. `ContextMenuRequested` moves there and gains
   `id`, `frame_url`, `media` (`ContextMenuMedia`), `has_image_contents`,
   `editable`, `edit_flags`, `misspelled_word`, `suggestions` and `items`
   (`MenuItem`, `MenuItemKind`; `surface::menu_items` over a `MenuSource`:
   invisible items and dangling separators dropped, submenus at most
   `MAX_MENU_DEPTH` = 2 deep). New `answer_context_menu(id, Option<i32>)`;
   CEF runs the standard command. `OnContextMenuDismissed` with a menu still
   held reports `ContextMenuDismissed`.
13. **JS dialogs and file choosers.** New `WeldJsDialogHandler` (`OnJSDialog`,
   `OnBeforeUnloadDialog`; `OnResetDialogState` answers what is held and
   reports `JsDialogsReset`) and `WeldDialogHandler` (`OnFileDialog`), both
   holding the callback and returning true (without them Alloy runs its own
   GTK dialog). Events `JsDialogRequested` (`JsDialogKind`, whose
   `default_accept` is the one default table: cancel, except beforeunload,
   which leaves) and `FileChooserRequested` (`FileChooserMode`,
   `AcceptFilter`); answers `answer_dialog`, `answer_file_chooser`.
14. **Certificate errors, fullscreen.** `WeldRequestHandler::
   on_certificate_error` holds the callback (`CertificateError` event,
   `answer_certificate`; default cancel). `WeldDisplayHandler::
   on_fullscreen_mode_change` on Linux, macOS and Windows reports
   `FullscreenChanged`. No audio handler: with one, CEF's capture silences
   the page's own playback (checked on Linux: the page's PulseAudio stream is
   corked while the capture runs, and plays without the handler).
15. **Drag source off by default.** `start_dragging` returns 0 and reports
   nothing unless the host called `CefSurfaceProducer::set_drag_source(true)`:
   CEF waits for `DragSourceEndedAt` after a true, and a host without a
   native drag loop would never send it.
16. **Native popup frame.** `CefSurfaceProducer::acquire_native_popup`
   (Linux, macOS): the popup slot's frame and its physical rect without an
   import, for hosts that present `NativeFrame`s. Windows keeps the trait
   default (`None`): it imports popups inside the paint callback.
17. **Stale requests dismissed.** `Held` ids come from one process-wide
   counter, so an id names one request across every kind and browser.
   Permission prompts keep CEF's `prompt_id`; `OnDismissPermissionPrompt`
   drops a still-held one. On a main-frame `OnLoadStart` (all three
   platforms) `Requests::page_left` answers held auth, certificate, file
   chooser and permission requests with their default (CEF abandons them on
   navigation without a callback). Both report
   `NavigationEvent::RequestsDismissed { ids }`. Permission answers are all
   or nothing for both callbacks (`permissions::all_granted`): CEF requires
   a getUserMedia answer to hand back exactly the requested media bits.
18. **Nothing waits past the close.** `OnBeforeClose` on all three
   platforms calls `Requests::drain_default`, answering every held request
   with its default; answers are given outside the table locks, because a
   CEF callback can call straight back into a handler.
19. Tests: `held.rs` (ids, unique across tables, take once, take by
   predicate, cap, drain, drop, fresh ids after a drain), `downloads.rs`
   (newest control used at once, ops queued before the first control, none
   after the end, asking keeps the directory policy), `permissions.rs`
   (prompt and media all or nothing), `surface.rs` (`request_vocabulary_tests`; new
   events in `mixed_callbacks_keep_observation_order`). Linux built and run
   (wubble-engine's `engine_smoke` example); Windows compile-checked with
   `cargo check -p welding --target x86_64-pc-windows-msvc` (build script
   overridden); macOS not compiled here.
20. **Unhandled-key event.** New `src/keyboard.rs` (registered in `lib.rs`):
   `WeldKeyboardHandler`, whose `OnKeyEvent` pushes
   `CefSurfaceEvent::KeyUnhandled { windows_key_code, modifiers }`
   (`surface.rs`) for each RAWKEYDOWN / KEYDOWN the page did not handle and
   returns 0, so CEF's default handling still runs. The `WeldClient` of
   Linux, macOS and Windows (`*/cef_backed.rs`) returns it from
   `keyboard_handler`. welding had no keyboard handler; the host needs it to
   run a page-first shortcut only when the page leaves the key alone.
21. **Request filter.** `surface.rs`: new `ResourceRequest` (URL, top-level
   page URL, initiator, CEF's raw `cef_resource_type_t`, navigation flag,
   browser id), `RequestAction { Allow, Cancel, Redirect(url) }` and
   `RequestFilter` (an `Arc<dyn Fn + Send + Sync>` with a manual `Debug`,
   `check` catches a panic and allows when panics unwind; under
   `panic = "abort"` a panic aborts, so filters must be panic-free), re-exported from `lib.rs`, and
   `CefSurfaceConfig::request_filter` (default `None`). `handlers.rs`:
   `WeldRequestHandler::resource_request_handler` returns a new
   `WeldResourceRequestHandler` only when a filter is set (otherwise `NULL`,
   CEF's default handling, as before); its `on_before_resource_load` (IO
   thread) builds the request (top page = the request URL for a main-frame
   request, else the main frame's URL unless the request's
   `first_party_for_cookies` names another site (the frame URL can be stale
   right after a commit), else that first party, else the initiator), runs the filter
   and maps `Cancel` to `RV_CANCEL` and `Redirect` to `set_url` + `RV_CONTINUE`.
   `SharedHandlers::build` takes the filter; the Linux, macOS and Windows
   producers pass `config.surface.request_filter`. Shape from cefclient
   `ClientHandler::GetResourceRequestHandler` (BSD-3-Clause), no code copied.
   Lets the host block or rewrite requests (wubble's content blocker).
   Tests: default `None`, `Debug`, panic allows (unwinding builds), the top-page rule
   including a stale main-frame URL.
22. **Frame timing on the copied frame.** `native_frame/mod.rs`: new
   `CopiedFrame { texture, arrived, copy, copy_wait }` (re-exported from
   `lib.rs`); `NativeFrame::Copied` holds it instead of a bare
   `ImportedTexture`. `linux_cef/cef_backed.rs` stamps `arrived` on entry to
   `on_accelerated_paint`; `vulkan_dmabuf.rs` `copy_dmabuf_to_owned` takes it
   and measures the whole copy and the `device.poll(Wait)` part. For
   wubble's hand-run `--measure`; costs three `Instant::now()` per frame.
23. **A dismissed context menu is retired, not dropped.** `held.rs`: new
   `Held::retire` (CEF answered every waiting request itself: ids go stale,
   nothing is answered, the handles are kept) and `Held::take_retired`;
   `handlers.rs` `hold` and `drain` release the retired handles outside the
   lock, and `OnContextMenuDismissed` retires instead of draining. CEF
   cancels its own menu (`CefMenuManager::CancelContextMenu`, from
   `SetFocus(false)`, `Hide`, a non-right mouse press, a wheel, browser
   destruction) by calling `Continue` through a raw pointer it holds no
   reference for, and `OnContextMenuDismissed` runs inside that `Continue`:
   dropping the held handle there freed the callback under it (SIGSEGV in
   `BindStateHolder::Reset`). Tests: retire answers nothing and keeps the
   handles; every request answered exactly once whoever closes it.
24. **Popups outside the view.** `view.rs`: new `ScreenArea { window, origin }`
   (physical; re-exported from `lib.rs`) in `ViewMetrics`, with pure
   `view_rect_dip`, `screen_rect_dip`, `screen_point` and `screen_point_dip`.
   All three render handlers: `view_rect` also sets x and y, `screen_info`
   sets `rect` and `available_rect`, new `root_screen_rect`, `screen_point`
   reports the window point (device px; DIP on macOS). New
   `CefSurfaceProducer::set_screen_area` (default `PlatformUnsupported`; the
   three producers store the area, then `notify_screen_info_changed`, only
   when it changed; no `was_resized`, the size is unchanged). Ported from cefclient
   `osr_window_win.cc` in its `real_screen_bounds` mode (BSD-3-Clause), with the
   host window standing in for the screen so no global window position is
   needed (Wayland has none). Without an area nothing changes. Tests: the
   rectangles at scale 1, 2 and 1.5, origin carried, default unchanged.
25. **Frame copy inside `OnAcceleratedPaint` (macOS), shared copy.** CEF
   renders into pooled IOSurfaces, so a retained surface imported at present
   time could hold a newer frame (cef-rs copies too: `iosurface.rs`
   `import_to_wgpu` then `copy_to_owned`). `native_frame/mod.rs`: the copy
   and wait of change 8 moved into one `copy_to_owned(source, ctx, arrived,
   started)`, whose target format is `source.format.add_srgb_suffix()` (a
   no-op for Linux's sRGB import; on macOS the `Bgra8Unorm` IOSurface holds
   display-encoded bytes, and the host samples the copy through an sRGB
   view). `vulkan_dmabuf.rs` `copy_dmabuf_to_owned` is now its import plus
   `copy_to_owned`; `metal.rs`: new `copy_metal_to_owned` (`import_metal`
   plus `copy_to_owned`). `macos_cef/mod.rs`: `MacosCefProducer::new` and
   `try_new_with_prepared_profile` take the host's `&HostWgpuContext`, kept in
   `WeldRenderHandlerInner::gpu`; a `cpu_paints` counter and
   `cpu_paint_count()` as on Linux. `macos_cef/cef_backed.rs`:
   `on_accelerated_paint` stamps `arrived`, retains and wraps the surface,
   copies it and stores `NativeFrame::Copied` (view and popup); new `on_paint`
   counts CPU fallback frames.
26. **Context script in every frame.** `app.rs`: `WeldRenderProcessHandler`
   takes `context_script: Option<&'static str>` (`WeldApp::build`'s third
   argument; `None` everywhere but the new
   `CefRuntime::{try_,}run_subprocess_with_script`, which wubble-helper calls
   with its `autofill.js`). `on_context_created` evaluates the script (a
   function expression) in the frame's new V8 context and calls it with two
   native functions from a `wrap_v8_handler!` (`ContextNatives`): `send(text)`
   posts a `FRAME_MESSAGE` (`"wubble.frame"`, at most
   `MAX_FRAME_MESSAGE_BYTES`) to the browser from that frame; `register(fn)`
   keeps `fn` and its context per frame id in a renderer thread-local.
   Neither is put on `window`: page code has no name for them (closure
   capture; CEF 156 has no isolated worlds and no `cef_register_extension`).
   `on_context_released` drops the frame's entry if it is that context's;
   a browser `FRAME_MESSAGE` in `on_process_message_received` calls the
   registered function with the text inside its context. Pattern: cefclient
   `client_renderer.cc` and cef-rs `wrapper/message_router.rs` (BSD-3-Clause;
   MIT OR Apache-2.0). Not unit-tested (V8 needs a renderer); `--smoke
   autofill` runs it.
27. **Frame messages, browser side.** `surface.rs`: new
   `CefSurfaceEvent::FrameMessage { frame_id, frame_url, is_main, text }`,
   `WebEventQueue::push_frame_message` (drops text over
   `MAX_FRAME_MESSAGE_BYTES` = 256 KiB and anything past
   `MAX_PENDING_FRAME_MESSAGES` = 64 queued, a count not a rate), and
   `CefSurfaceProducer::send_frame_message(frame_id, text)` (default
   `PlatformUnsupported`). `handlers.rs`: shared `frame_message_received`
   (the frame's id, URL and `is_main` read from CEF's `CefFrame`, never from
   the text) and `send_frame_message` (`Browser::frame_by_identifier`, `Err`
   when the frame is gone). The three platform clients call
   `frame_message_received` first in `on_process_message_received`; the three
   producers implement `send_frame_message` (Windows through its browser
   accessor). Tests: order with the other events, the size cap, the pending
   cap and room again after a drain. Not unit-tested: an unknown frame id
   (needs a live browser).
28. **macOS frame copy on its own Metal queue.** `metal.rs`:
   `copy_metal_to_owned` no longer goes through `copy_to_owned` (now
   Linux-only in `native_frame/mod.rs`): it creates the copy as a raw
   private `MTLTexture` (`add_srgb_suffix` of the frame's format; new
   `wgpu_format_to_mtl` arms for `Bgra8UnormSrgb`/`Rgba8UnormSrgb`), blits
   into it on a command queue of its own (`copy_queue`, one per device,
   thread-local: `OnAcceleratedPaint` runs on CEF's UI thread), waits for
   that command buffer only (`waitUntilCompleted`, status checked) and wraps
   the copy with `grafting::import_metal_texture_ref`, so wgpu counts it as
   initialised. On wgpu's queue the wait also covered the scene and Slint's
   frame queued before the copy (M2 Pro, `--measure scroll`: copy wait p95
   4.8 to 5.1 ms before, 2.9 to 3.2 ms after; the UI thread is blocked for
   it). `Cargo.toml`: objc2-metal features `MTLBlitCommandEncoder`,
   `MTLCommandBuffer`, `MTLCommandEncoder`, `MTLCommandQueue`. Not
   unit-tested (needs a live CEF frame); the macOS `--smoke frames` pixel and
   glass checks run it.
29. **Switches for child processes.** `runtime.rs`: new
   `CefRuntimeConfig::child_process_switches`, added to every child
   process's command line by the browser process handler's new
   `on_before_child_process_launch` (`app.rs`, `WeldApp::build` takes them).
   Chromium hands only switches it knows on to renderers, so a switch the
   helper reads (wubble's `--wubble-clipboard-bridge`, which turns on its
   clipboard content script on change 26's channel) cannot go in
   `command_line_switches`.
30. **Media state on the context menu.** `NavigationEvent::
   ContextMenuRequested` gains `media_flags` (`cef_context_menu_media_state_
   flags_t`: paused, muted, loop, controls, ...), read in `handlers.rs`
   `run_context_menu`.
31. **Links opened elsewhere reach the host.** `handlers.rs`: the request
   handler's new `on_open_urlfrom_tab` (a middle or Ctrl click on a link)
   reports `NavigationEvent::NewWindowRequested` and cancels CEF's in-place
   navigation; the disposition says whether it wants a background tab
   (change 33's `NewWindowKind`).
32. **Autoscroll cursors by name.** `surface.rs`: `CursorShape::Panning`
   with the new `Panning` (middle, middle vertical/horizontal, eight compass
   points; exported from `lib.rs`); `cursor.rs` `from_cef` maps CEF's
   `CT_*PANNING` types to it (`CT_MIDDLEPANNING` was `Move`).
33. **What a popup asked for.** `surface.rs`: new `NewWindowKind`
   (foreground tab, background tab, popup, window, other) and
   `NavigationEvent::NewWindowRequested` gains `kind` and `size` (the
   `width`/`height` window features when both are set). `handlers.rs`: shared
   `new_window_requested` maps `cef::WindowOpenDisposition` and
   `cef::PopupFeatures`; the three platform `on_before_popup` and change 31's `on_open_urlfrom_tab` call it; `kind` replaces change 31's `background` flag (a background tab is `NewWindowKind::BackgroundTab`). The popup
   is still cancelled (one surface per producer); wubble's popup policy
   (`wubble_core::popup`) decides what opens. Tests: the disposition mapping and
   the size rule.
34. **The renderer's pid on frame messages.** CEF 156 tells the browser
   no renderer pid, and `SystemInfo.getProcessInfo` is a browser-target CDP
   domain a page's `ExecuteDevToolsMethod` cannot reach. `app.rs`: the
   context script's `send` adds `std::process::id()` as argument 1 of the
   process message. `handlers.rs`: `frame_message_received` reads it
   (`sender_pid`: 0 when absent or negative). `surface.rs`:
   `CefSurfaceEvent::FrameMessage` gains `pid: u32`. The pid is the
   renderer's own report, so a compromised renderer could lie about it; it
   is used for accounting only. Tests: the queue test carries a pid through;
   `sender_pid` (with `cef-runtime`).
35. **Served responses.** `surface.rs`: `RequestAction::Serve(data_url)`.
   `handlers.rs`: `WeldResourceRequestHandler` gains a `served` slot;
   `on_before_resource_load` decodes the base64 `data:` URL into `Served`
   (MIME type, body; `RV_CANCEL` when it does not parse) and returns
   `RV_CONTINUE`, and its new `resource_handler` hands the slot to a new
   `WeldServedResource` (`open` handles at once, `response_headers` gives
   200, the MIME type, the length and `Access-Control-Allow-Origin: *`,
   `read` copies from memory). Contract from CEF `cef_resource_handler.h`
   and libcef_dll `CefStreamResourceHandler` (BSD-3-Clause), no code copied.
   Lets the content blocker serve uBO's `$redirect=` stubs, which Chromium
   would refuse as a `data:` redirect target. Test: the data URL parsing.
36. **Response observer.** `surface.rs`: new `ResponseInfo` (URL, browser
   id, raw resource type, status, MIME type without parameters,
   `Content-Length`, raw `Content-Range` and `Content-Disposition`,
   `Accept-Ranges: bytes`) and `ResponseObserver` (a shared closure, a
   panic caught where unwinding is on); `CefSurfaceConfig::response_observer`
   (default `None`). `handlers.rs`: `SharedHandlers::build` takes it next to
   the request filter; `resource_request_handler` returns the handler when
   either is set (the filter becomes optional inside it), and the handler's
   new `on_resource_response` reports the headers and returns false (the
   request is never changed). The three platform producers pass it on. Shape
   from cefclient `ClientHandler::OnResourceResponse` (BSD-3-Clause), no code
   copied. wubble's media downloader detects video, audio, HLS and DASH by
   the response's MIME type, which the request hook cannot see. Test: the
   header reading and a panicking observer.
37. **Resource types as `u32` on Windows too.** `handlers.rs`: the request
   filter's and the response observer's raw resource type is
   `get_raw() as u32` (clippy's `unnecessary_cast` allowed): CEF's enum raw
   type is `i32` on Windows and `u32` elsewhere, so the Windows CI build
   failed. No behaviour change.
38. **GLib's default context runs with the external pump (Linux).**
   `runtime.rs`: `CefRuntime::do_message_loop_work` also dispatches what is
   ready on GLib's default main context (`g_main_context_iteration(NULL,
   FALSE)`, at most 64 dispatches, resolved with `dlsym` from the GLib libcef
   loads). CEF's external pump never runs that context, and Chromium watches
   its own native windows' display connection there (Ozone's
   `WaylandEventWatcherGlib`): a Chrome-style window such as DevTools was
   created but never got its first `configure`, so it never appeared.
   cefclient's Linux external pump runs the same context as its loop.
39. **Status text and tooltips reach the host.** `surface.rs`:
   `NavigationEvent::StatusChanged { text }` and `Tooltip { text }`. The
   display handler of each platform (`linux_cef`, `macos_cef`,
   `windows_cef` `cef_backed.rs`) implements `on_status_message` and
   `on_tooltip` and queues them; under windowless rendering the host draws
   both (CEF's header says so for tooltips).
40. **Unknown schemes go to the host, never to the OS or an error page.**
   `handlers.rs`: `on_before_browse` cancels a navigation whose scheme
   Chromium does not load itself (`chromium_handles`: the web, local, data
   and Chromium page schemes) and queues `NavigationEvent::ProtocolExecution
   { url, user_gesture }` with its gesture bit; without the cancel, CEF
   fails it with `ERR_UNKNOWN_URL_SCHEME` and commits an error page in place
   of the page the link was on. `chromium_handles` is public
   (`welding::chromium_handles`) so a host can tell an external URL before
   it opens a tab for it. The resource-request handler (returned only with a
   filter or observer, as before) implements `on_protocol_execution` as a
   backstop for a scheme the list wrongly claims (IO thread): it answers
   `allow_os_execution = 0` and queues the same event with no gesture bit
   (CEF gives none there).
41. **A download request names its original URL.** `DownloadRequested` gains
   `original_url` (`CefDownloadItem::GetOriginalUrl`): the URL
   `start_download` was given, before redirects, so a host can tell its own
   Save As downloads apart.
42. **A context menu names its frame.** `ContextMenuRequested` gains
   `frame_id` (`CefFrame::GetIdentifier` of the frame clicked in; empty when
   CEF gave none).
43. **Scripts in one frame.** `CefSurfaceProducer::execute_script_in_frame`
   (default: the main frame), implemented by the three producers through
   one shared `handlers::run_in_frame`, which falls back to the main frame
   when the id is empty or the frame is gone.
44. **One sandbox rule for browser and helpers.** `runtime.rs`:
   `CefSandboxMode::NO_SANDBOX_SWITCH` (Chromium's `no-sandbox`) and
   `CefSandboxMode::for_direct_entry(no_sandbox)` (`Sandboxed` unless asked
   off, or on Windows, where the direct entry point refuses it). wubble-engine
   and wubble-helper both call it, so the two processes cannot disagree.
45. **macOS helpers load the framework after the seatbelt.**
   `runtime.rs`: `try_run_subprocess_with_script` initialises
   `libcef_sandbox.dylib` first, then loads `Contents/Frameworks/Chromium
   Embedded Framework.framework` itself (`helper_frameworks_dir`,
   `maybe_load_library`), then pins the API version (which calls into the
   framework). CEF's helper order (cefsimple `process_helper_mac.cc`); the
   helper no longer loads the framework, so its initialisers run sandboxed.
46. **Shared request contexts (`SharedProfile`).** `profile.rs`: a public
   `SharedProfile` owns one `cef::RequestContext` created with a readiness
   handler (`is_ready` once CEF calls `OnRequestContextInitialized`; the
   handler moved here from `macos_cef`), and `CefSurfaceConfig::profile`
   hands it to a producer: the Linux, macOS and Windows producers create
   their browser in that context (`profile::for_surface`) instead of a fresh
   one, so several producers share cookies, storage, cache, permissions and
   service workers. `MacosCefProducer::new` accepts a ready shared profile
   (no `prepare_profile` dance). `user_data_dir` must now be a direct child
   of the runtime's `cache_path`, not merely inside it: CEF 156's Chrome
   runtime makes a disk-backed context a Chrome profile and, for any deeper
   path, logs "Cannot create profile at path" and silently falls back to an
   in-memory profile. wubble gives each container one shared profile
   (`wubble-engine` `storage.rs`); before this every view had its own
   in-memory jar, so tabs never shared a login and nothing survived a
   restart. Test: `wubble-engine`'s `cookie_smoke` example.
47. **Print settings.** `CefSurfaceProducer::print_to_pdf(path, &PdfOptions)`:
   `PdfOptions` (landscape, background graphics, scale, paper size in
   inches, `PdfMargins` default / none / custom inches, page ranges) is
   filled into `cef::PdfPrintSettings` (`drag::pdf_settings`) by the Linux,
   macOS and Windows producers, which used CEF's defaults before. wubble's
   print sheet sets them.
48. **Drag operation and the page's own drag data.** The render handlers
   implement `UpdateDragCursor` as `NavigationEvent::DragCursor { operation }`,
   so the host learns what a drop would do (a text drag between two fields
   moves, not copies) and shows it. `StartDragging` keeps a CEF clone of the
   drag data in `Requests` (dropped by `finish_drag_source`); a
   `DragInput` enter with no payload re-enters with that clone, which keeps
   `DropData`'s custom types (`setData("application/x-...")`) that
   `DragPayload` cannot carry. Test: `drag::tests`.
49. **Scroll offset.** The render handlers implement `OnScrollOffsetChanged`
   as `NavigationEvent::ScrollOffset { x, y }` (Linux, macOS, Windows
   `*/cef_backed.rs`, `surface.rs`), so the host can tell whether a page took
   a sideways scroll before it turns a two-finger swipe into back or forward.
50. **Accessibility tree.** New `accessibility.rs` (cef-runtime): a
   `WeldAccessibilityHandler` whose `OnAccessibilityTreeChange` and
   `OnAccessibilityLocationChange` write CEF's value as JSON (`cef_write_json`)
   and queue `NavigationEvent::AccessibilityTree { json }` /
   `AccessibilityLocation { json }` (`surface.rs`); the render handlers of
   all three platforms return it from `GetAccessibilityHandler`
   (`*/cef_backed.rs`). Nothing turns accessibility on: CEF calls the handler
   only after the host's `BrowserHost::set_accessibility_state(ENABLED)`.

51. **A dragged picture's bytes and the drag image.** `DragPayload` gains
   `file_contents: Option<DragFileContents { name, bytes }>` and
   `image: Option<DragImage { png, size, hotspot }>` (`surface.rs`).
   `drag::payload_from_cef(data, scale)` copies them in `StartDragging` on
   the UI thread (the data is callback-scoped and not `Send`): the bytes
   through `GetFileContents` into a `wrap_write_handler!` buffer (32 MiB cap),
   the name from `GetFileName`, the image as `GetImage()->GetAsPNG(scale)` with
   its size and hotspot in DIP (`GetAsPNG` returns nothing unless its
   pixel-size out-params are given: CEF's C glue checks them). `to_cef_payload` ignores both (CEF has no
   setter; a drag back into the window re-enters with the held clone,
   change 48). The three platforms' render handlers pass their scale.
   Test: `drag::tests`.
52. **CEF's C API before `initialize`.** `runtime.rs`: new
   `CefRuntime::prepare_api(cef_path)` loads the framework (macOS) and pins
   the API version, so plain C functions such as `cef_zip_reader_create` can
   run before `initialize`; `maybe_load_library` remembers a macOS load in a
   static, since `cef_load_library` returns 0 on a second call
   (`libcef_dll_dylib.cc`) and `initialize` would then fail. The stub returns
   `FeatureRequired`. Used by wubble-engine's .crx unpacker.

53. **A navigation guard.** `surface.rs`: `NavigationGuard` (a `Fn(&str) -> bool`)
    and `CefSurfaceConfig::navigation_guard`; `handlers.rs`: `OnBeforeBrowse`
    asks it for each main-frame navigation, every redirect hop included, before
    the request is sent, and cancels a refused one (returns true), pushing
    `NavigationEvent::NavigationRefused { url, redirect }`. A panicking guard
    allows. wubble keeps an agent session's tab on the sites its grant allows.

## Not changed, on purpose

- **No `Browser` accessor.** wubble already reaches the browser host through
  `cef::browser_host_get_browser_by_identifier(producer.browser_id())`, and
  `CefSurfaceProducer` already has input, navigation and `set_visible`.
- **Input untouched** (apart from the unhandled-key report). wubble-engine sends mouse, wheel and key events to the
  `BrowserHost` itself: welding's input path has no click count, mouse
  leave, wheel precision flag, `is_system_key` or key table, and fixing it
  would mean three platform copies of the change.
- **Texture import format/usage (the per-frame blit in wubble-gfx stays).**
  Slint 1.19's `Image::try_from(wgpu::Texture)` takes only `Rgba8Unorm` /
  `Rgba8UnormSrgb` with `TEXTURE_BINDING | RENDER_ATTACHMENT`
  (i-slint-core 1.19.0 `graphics/wgpu_30.rs`). CEF's dmabuf is BGRA: importing
  it as RGBA swaps red and blue, and Slint allows no swizzled view. The
  `VkImage` is created by `grafting` with `SAMPLED | TRANSFER_SRC` only
  (grafting 0.6 `vulkan_dmabuf.rs`); adding colour-attachment usage to an
  imported DRM-modifier image needs a grafting fork and is valid only for
  modifiers whose format features include it, which is driver dependent.
  Revisit if Slint accepts `Bgra8` or a swizzled view.
- `grafting` is not vendored.
