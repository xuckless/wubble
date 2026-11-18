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
23. **Popups outside the view.** `view.rs`: new `ScreenArea { window, origin }`
   (physical; re-exported from `lib.rs`) in `ViewMetrics`, with pure
   `view_rect_dip`, `screen_rect_dip`, `screen_point` and `screen_point_dip`.
   All three render handlers: `view_rect` also sets x and y, `screen_info`
   sets `rect` and `available_rect`, new `root_screen_rect`, `screen_point`
   reports the window point (device px; DIP on macOS). New
   `CefSurfaceProducer::set_screen_area` (default `PlatformUnsupported`; the
   three producers store the area, then `notify_screen_info_changed` and
   `was_resized`, only when it changed). Ported from cefclient
   `osr_window_win.cc` in its `real_screen_bounds` mode (BSD-3-Clause), with the
   host window standing in for the screen so no global window position is
   needed (Wayland has none). Without an area nothing changes. Tests: the
   rectangles at scale 1, 2 and 1.5, origin carried, default unchanged.

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
