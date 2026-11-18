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
6. Tests for the above: `runtime.rs` (`pump_scheduler_tests`), `surface.rs`
   (`frame_timing_defaults_match_upstream`; `BrowserClosed`,
   `LoadingStateChanged` and `FaviconUrlsChanged` added to
   `mixed_callbacks_keep_observation_order`).

## Not changed, on purpose

- **No `Browser` accessor.** wubble already reaches the browser host through
  `cef::browser_host_get_browser_by_identifier(producer.browser_id())`, and
  `CefSurfaceProducer` already has input, navigation and `set_visible`.
- **Input untouched.** wubble-engine sends mouse, wheel and key events to the
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
