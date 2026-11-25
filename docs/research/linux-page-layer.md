# Linux page layer: the page as a Wayland subsurface

Research agent, 2026-11-25, read only on main (f6a0b43): nothing was built or run. [v] means checked in sources (the
cargo registry, `vendor/`, the CEF/Chromium checkout in `.local/engine/cef-build/chromium_git`). [u] means it has to
be tested on a machine. [m] is from memory of upstream projects and has to be checked against their sources before
anything is copied. Read [page-layer.md](page-layer.md) first: this is its Linux half, and it reuses that design.

## Verdict

**Feasible on Wayland, with conditions.** The design is Layer: windowed as on macOS: one `wl_subsurface` per pane,
placed below the shell's `wl_surface`, showing a dmabuf `wl_buffer`. The shell clears a rounded hole with the same
`PageHole`, and the same `page_layer` machine, `MODES` rows and overlay snapshots apply. Nothing above the gfx trait
changes. The conditions are:

1. **One GPU copy stays.** CEF hands its buffer back to the capturer's pool when the callback returns. We copy into
   a pool of our own exportable dmabufs, as macOS does with IOSurfaces. Removing the copy needs a CEF patch (below).
2. **The shell surface must really blend.** It needs a pre-multiplied alpha swapchain on NVIDIA's Vulkan WSI under
   mutter. Rounded corners already assume this, but nobody has looked at it on a display [u]. This is the experiment's
   first question.
3. **Only Wayland.** X11 and XWayland keep today's composited path (`Route::Unsupported`).

Expected CPU per page frame (RTX 5060, from today's ~1.8 ms):

| | main | GPU process | renderer | total |
|---|---|---|---|---|
| today | ~1.0 | ~0.5 | ~0.3 | ~1.8 ms |
| subsurface, one copy (this plan) | ~0.35 to 0.45 | ~0.5 | ~0.3 | **~1.15 to 1.25 ms** |
| + CEF keep-buffer patch (no copy, imports cached) | ~0.25 to 0.3 | ~0.5 | ~0.3 | ~1.05 to 1.1 ms |
| Firefox (today's bench) | | | | 0.75 to 1.1 ms |

How the main-thread number is worked out: about 55 to 60 % of the main thread during video is the Slint render of the
window (measurements.md, "step 1" table: walk, Skia flush, recording, present). Page frames no longer cause that
render. The overlay snapshots add about 4 renders a second, which is nothing at 60 to 300 fps. What stays: the frame
arrival (import and copy, ~15 %), the CEF message pump (~18 %), and a few tens of µs of Wayland requests per frame.
This is a guess from profile shares, not a measurement. The remaining gap to Firefox is the GPU process's capture
copy (off-screen rendering's `CopyOutputRequest` into the capturer's pool). No client-side change removes that.
Compositor CPU (gnome-shell) is not in either number. The experiment has to read it, because a subsurface commit
moves some work there [u].

## 1. winit 0.30 / Slint 1.19: handles, subsurface, transparency

- **Handles** [v]. `crates/wubble/src/wl/wayland.rs:165-176` already takes winit's `wl_display` from
  `RawDisplayHandle::Wayland` and wraps it with `Backend::from_foreign_display`. `:274-286` turns the window's
  `wl_surface` pointer into a proxy with `ObjectId::from_ptr(WlSurface::interface(), ..)`. A layer does the same on
  its own `EventQueue` on that connection, as page-layer.md planned.
- **Subsurface** [v]. winit 0.30.13 creates no subsurfaces of its own once the window has no decorations
  (`no-frame` on Linux, app.slint:92; the only mention is the CSD frame, `wayland/window/state.rs:158`). We bind
  `wl_subcompositor`, `wp_viewporter` and `zwp_linux_dmabuf_v1` (v4, with feedback) from the registry on our queue.
  Then `get_subsurface(child, shell_surface)` and `place_below(shell_surface)`: the protocol allows the parent as the
  reference. The protocol modules are in the locked crates: wayland-client 0.31.15, wayland-protocols 0.32.13 (stable
  `viewporter`, `linux-dmabuf`; staging `linux-drm-syncobj`, `single-pixel-buffer`, `fractional-scale`), sctk 0.20,
  gbm 0.18, ash 0.38.
- **Input** [v, Chromium]. Give the subsurface an empty input region, as Chromium does
  (`ui/ozone/platform/wayland/host/wayland_subsurface.cc:104-108`). All input then stays on winit's surface, which is
  on top anyway. Cursors, IME, drag and drop and accessibility stay as they are, for the reasons given in
  page-layer.md.
- **Transparency** [v code, u display]. The window is already transparent on Wayland. app.slint:105 has
  `background: transparent`, and `wubble-ui/src/window.rs:43-49` `transparent_corners` is true for Wayland. winit then
  sets no opaque region (`wayland/window/state.rs:661-662`). The vendored surface asks for `PreMultiplied` or
  `PostMultiplied` (`vendor/i-slint-renderer-skia/wgpu_30_surface.rs:63-74`), and wgpu-hal maps that to Vulkan's
  composite alpha (`wgpu-hal-30.0.2/src/vulkan/conv.rs:533-539`). The window.rs comment calls it a belief: Mesa and
  NVIDIA expose a pre-multiplied mode on Wayland. If NVIDIA offers only `OPAQUE`, the alpha mode falls back and the
  hole shows black. That is the first kill condition.
- **The hole** [v]. `vendor/i-slint-renderer-skia/hole.rs` and the `Clear` draw in `itemrenderer.rs` are not
  cfg-gated, and Linux uses the same vendored Skia-on-wgpu renderer. `PageHoleSource` is set in
  `crates/wubble/src/main.rs:1168`. Nothing new is needed on the shell side.
- **Corners** [v]. Wayland has no clip or corner radius for a subsurface, and it needs none. The shell sits above and
  paints opaque pixels everywhere outside the hole's rounded, anti-aliased shape, so the subsurface's square corners
  are covered. `PageLayer::place`'s `radius` is ignored on Wayland.
- **Mapping the trait** (`wubble-gfx/src/layer.rs:17-40`):

| `PageLayer` | Wayland |
|---|---|
| `root()` | (window height in logical px, top-left origin) |
| `place(frame, contents, scale, _)` | `wl_subsurface.set_position` (applied at the shell's next commit in either mode), `wp_viewport.set_destination` (logical size), `set_source` (the `contents` rect) |
| `show(frame)` | `attach(slot's wl_buffer)`, `damage_buffer(CEF's update rect)`, `frame()`, `commit`, flush |
| `hide()` | `attach(null)`, commit (Chromium releases the last buffer by hand here: mutter may never send `release` when a role goes, `wayland_frame_manager.cc:361-370`) |
| `present_in_transaction(on)` | `on`: `set_sync` (the child's state waits for the shell's commit, so a hole and its layer land together); off: `set_desync` (page frames commit on their own, with no shell commit) |
| new: `dispatch()` | `dispatch_pending` plus a non-blocking `prepare_read().read()` on the layer's queue, called before a slot is chosen and in `show_frame` |

- **Fractional scale** [u]. `set_position` and `set_destination` take integer logical pixels. At 1.25 the pane's
  physical edges can fall between them. The hole is drawn one physical pixel smaller on each side and the layer is
  rounded outwards, so shell pixels cover any seam (already in page-layer.md's Wayland notes).

## 2. CEF 156's dmabuf: lifetime, format, a `wl_buffer` from it

- **Lifetime** [v]. Off-screen frames come from Viz's video capturer.
  `cef/libcef/browser/osr/video_consumer_osr.cc:17-27,102` calls `Done()` when `OnFrameCaptured` returns, and that
  returns the buffer to the pool. `cef_types_linux.h:176-180` says the same ("released to the underlying pool for
  reuse when the callback returns"). The pool has `kFramePoolCapacity` = 11 frames, and the capturer throttles once
  more than `kDesignLimitMaxFrames` (10) × `kTargetPipelineUtilization` (0.6) are in flight
  (`frame_sink_video_capturer_impl.h:175-180`). So **without a patch, CEF's buffer cannot be given to the compositor**.
  It would be overwritten while mutter samples it.
- **What the buffer is** [v]. `media/gfx/renderable_mappable_shared_image_video_frame_pool.cc:209-237`: a native
  pixmap (GBM) with `BufferUsage::SCANOUT` on Linux (CEF asks for `kPreferSharedImageWithNativeHandle`, so there is
  no CPU access and no forced LINEAR), plus `SHARED_IMAGE_USAGE_SCANOUT` when the GPU supports it.
  `ARGB`/`ABGR` maps to CEF's BGRA or RGBA (`video_consumer_osr.cc:137-139`). It has 1 plane (up to 4), and fds, offset,
  stride and modifier are passed through (`:193-213`). On NVIDIA the modifier arrives as `DRM_FORMAT_MOD_INVALID`,
  which welding imports as LINEAR (`vulkan_dmabuf.rs:131-147`).
- **A `wl_buffer` from it** [v protocol, u NVIDIA]. In principle yes:
  `zwp_linux_buffer_params_v1.add(fd, plane, offset, stride, mod_hi, mod_lo)` and then `create_immed`. Two catches:
  (a) the INVALID modifier is accepted only if the compositor lists the format with INVALID (an implicit modifier) in
  its feedback table; (b) there is no fence. Welding notes that NVIDIA dmabufs carry no implicit fence
  (`vulkan_dmabuf.rs:185-193`), so the compositor could read before the GPU process finished writing.
  Explicit sync (`wp_linux_drm_syncobj_v1`) would need a fence from CEF, which it does not give.
- **So: copy into our own pool**, the macOS shape (`OsBufferRequest`, `OsBuffer`, `POOL_SLOTS` = 5;
  `native_frame/mod.rs:560-599`):
  - Allocate N slots once per view size. Either use GBM (`gbm_bo_create_with_modifiers2` on the render node that
    matches the Vulkan device, with the modifiers from `zwp_linux_dmabuf_feedback_v1`'s main-device tranche that
    Vulkan also supports for the format), or use Vulkan exportable images (`VK_EXT_image_drm_format_modifier` +
    `vkGetMemoryFdKHR`). Format: CEF's own (BGRA → `DRM_FORMAT_XRGB8888`, RGBA → `XBGR8888`), so
    `copy_texture_to_texture` stays a plain copy. X (no alpha) plus a full opaque region lets the compositor skip
    blending.
  - Import each slot into wgpu **once**, as a copy destination. Grafting 0.6.2's import is read-only today: it uses
    `SAMPLED | TRANSFER_SRC` and adds `TRANSFER_DST` only for tracking, and the wgpu usage is
    `TEXTURE_BINDING | COPY_SRC` (`grafting-0.6.2/src/vulkan_dmabuf.rs:359-365,465`). A small welding addition (ash)
    or a grafting change is needed for `COPY_DST`, plus a release barrier to `VK_QUEUE_FAMILY_FOREIGN_EXT` after the
    copy [u: NVIDIA may not need it, Mesa does].
  - Create one `wl_buffer` per slot, once. A slot is in use from `attach` until `wl_buffer.release`. That feeds the
    existing `choose(&[bool], Option<usize>)`. Keep 5 slots, as the Mac does (bursts).
  - The copy is waited for before `commit`, as it is today (`copy_to_owned`'s `device.poll(Wait)`). Later, export a
    sync file and use `wp_linux_drm_syncobj_v1` so the main thread does not block.
  - Side win, also for today's path: `copy_to_owned` creates a **new texture for every frame**
    (`native_frame/mod.rs:718`), and `import_vulkan` makes a new Vulkan image and memory import for CEF's buffer every
    frame. A pool removes the first. A cache of CEF's 11 buffers by fd identity (`fd_identity`, already in
    `vulkan_dmabuf.rs:231`) would remove most of the second. Both cut driver CPU with or without a layer.
- **Zero copy (later, an engine patch)** [v code, m Electron]. Keep `callbacks` (the
  `FrameSinkVideoConsumerFrameCallbacks` remote) alive past the callback, and call `Done()` when the client says so:
  a new `CefRenderHandler` "release" token, or a ref-counted object in `cef_accelerated_paint_info_t`. Electron's
  off-screen shared texture does this (the JS `texture.release()` of `useSharedTexture`, MIT) [m]. Even then the
  fence problem remains. Either the capturer's completion is already waited for before `OnFrameCaptured` [u], or
  welding keeps a cheap GPU wait. With 2 or 3 frames held, the 0.6 × 10 throttle is not reached. Saves the copy and
  the per-frame import, roughly 0.1 ms of main thread. Waits for the one engine build (AGENTS.md).

## 3. Prior art to copy (all GPL-3.0-compatible)

| Project | Where | Licence | What to take |
|---|---|---|---|
| Chromium Ozone/Wayland | `ui/ozone/platform/wayland/host/wayland_subsurface.cc`, `wayland_frame_manager.cc`, `wayland_buffer_manager_host.cc`, `wayland_zwp_linux_dmabuf.cc`, `wayland_syncobj_timeline.cc`, `overlay_prioritizer.cc` (all in the local checkout) [v] | BSD-3 | empty input region, sync by default, place above or below, the buffer-release bookkeeping and mutter workarounds (manual release on hide, "never attach an unreleased buffer"), dmabuf format and modifier tables, syncobj timelines |
| GTK 4 graphics offload | `gdk/wayland/gdksubsurface-wayland.c`, `gsk/gskoffload.c` (GTK 4.14+; "below" with a punched hole from 4.16) [m] | LGPL-2.1+ | the closest match: a subsurface **below** the main surface, with the main surface cleared to transparent over it, `wp_viewport` source and destination, single-pixel-buffer backgrounds, falling back when a transform cannot be offloaded. Fedora 44 ships gtk4 4.22.4 |
| Firefox native layers | `widget/gtk/NativeLayerWayland.cpp`, `widget/gtk/WaylandSurface.cpp`, `widget/gtk/DMABufSurface.cpp` [m] | MPL-2.0 | one subsurface per compositor layer, a surface pool, viewporter, frame callbacks per layer, dmabuf buffer pools with release tracking |
| mpv `vo_dmabuf_wayland` | `video/out/vo_dmabuf_wayland.c`, `video/out/wayland_common.c` [m] | LGPL-2.1+ | video in a subsurface under a solid-colour parent, a buffer pool keyed by dmabuf with `wl_buffer.release`, viewporter scaling, frame-callback pacing |
| Weston clients | `clients/simple-dmabuf-feedback.c`, `simple-dmabuf-egl.c` [m] | MIT | the smallest complete dmabuf-feedback + GBM allocation + `create_immed` loop; a good start for the experiment |
| Electron OSR | `shell/browser/osr/osr_video_consumer.cc`, the shared-texture `release()` [m] | MIT | holding the capturer's frame until the embedder releases it (for the CEF patch) |

Each copy needs a source comment and a `THIRD_PARTY_NOTICES.md` entry (AGENTS.md).

## 4. X11, and which compositors

- **X11: no equivalent.** X11 has no subsurfaces with their own buffers. Child windows are always above their parent,
  and a compositing manager composites a top-level as one picture, so a child window cannot show through an
  alpha hole in its parent. The closest thing is a child window *above* the shell, with DRI3 `PixmapFromBuffers` plus
  `Present` of our pool. That is "Layer: fullscreen" only (a fullscreen page, picture in picture), with no shell
  pixels over it. Not worth it while X11 is untested. X11 and XWayland keep today's path: `Route::Unsupported`, one
  copy, a shell render per page frame.
- **What is needed, and where it exists** [m; check on each with `wayland-info` before claiming it]:

| Need | GNOME (mutter) | KDE (KWin) | wlroots (sway, etc.) |
|---|---|---|---|
| `wl_subcompositor`, `wp_viewporter`, linux-dmabuf v3 | every supported release | every supported release | every supported release |
| linux-dmabuf v4 feedback | 42+ | 5.24+ | 0.15+ |
| `wp_fractional_scale_v1` | 43+ (experimental setting earlier) | 5.27+ | 0.17+ |
| `wp_linux_drm_syncobj_v1` (explicit sync; later) | 48+ | 6.1+ | 0.18+ |
| subsurface **scanout** on an overlay plane (a bonus) | no for windowed (direct scanout only for a fullscreen surface) | overlay planes from Plasma 6.2/6.3, hardware-dependent | experimental (libliftoff), 0.18+ |

  The headless bench display is mutter 50.4 (Fedora 44, `scripts/headless.sh`) with a virtual monitor: no KMS
  planes, so it always composites. The saving there is client CPU only, which is the goal. NVIDIA has needed
  explicit sync on Wayland since driver 555. wubble is tested on 615.71.09.

## 5. Overlays over the page (find bar, menus, glass)

The same as macOS, unchanged: page-layer.md "Overlays over a layered page" (`Coverage::Glass`, `Hold::Frost`,
`Effect::Snapshot`, `SNAPSHOT_MS` = 250, per-pane `overlay_over`). The Linux pool slots are wgpu textures (imported
once, as above), so `FramePresenter::refresh` blits the latest slot into the pane's ring target as it does on macOS,
and the page blur `P` stays low resolution through its existing ¼ to 1/16 levels. Non-glass overlays (popups,
tooltips, the status bubble, CEF's `<select>` popup, which is its own `PET_POPUP` frame drawn by the shell) are shell
pixels over the hole. The page's live pixels still go back to the shell for Entrance, SeamBow, resizes
(`Hold::Brief`, ending after `BRIEF_MS`) and pixel readers. The one Linux difference is reading a slot while mutter
samples it, which is concurrent reads only.

## 6. Effort, unknowns, the first experiment

**Effort** (sprint mode, an agent writing and a review per part). Everything platform-neutral is done: the machine,
the modes, the hole, the snapshots, the host executor.
- welding: Linux `OsBufferPool` (GBM or exported Vulkan images, imported once as copy destinations, the release
  barrier, `OsBuffer` with fds, planes, modifier and a release flag), `CopiedFrame.os` on Linux. About 2 to 3 days.
- wubble-gfx `layer.rs`: `wayland::Subsurface` (globals and feedback, own queue, `wl_buffer` per slot, the trait
  above, sync and desync, hide with manual release). About 2 to 3 days.
- host: the `PLATFORM_MODES` row for Wayland only (X11 off), `dispatch` in `show_frame`, the Settings label ("(macOS)"
  goes), measurements. About 1 to 2 days.
- Hand checks on GNOME, then one KWin or wlroots box. About 2 days.
- In all, **about 1.5 to 2 weeks**. The CEF keep-buffer patch and syncobj are separate, later items (engine build).

**Risky unknowns, by how likely they are to kill it:**
1. NVIDIA's Wayland Vulkan WSI offers no pre-multiplied alpha, or mutter blends the shell surface as opaque, so the
   hole is black.
2. mutter rejects our `wl_buffer`s: no common format and modifier between NVIDIA's GBM or Vulkan and mutter's
   feedback, or `create_immed` fails.
3. Sync artefacts: tearing or old frames if the compositor reads before the copy lands (we wait on the CPU, so this
   is unlikely), or a missing foreign-queue release on NVIDIA.
4. Atomicity at hole changes. The child's cached state must land with the shell's next commit, but NVIDIA's WSI
   presents with `wp_fifo_v1` and `wp_commit_timer_v1` on mutter 50 (headless.sh notes) and may commit from its own
   thread or late. That would show one frame of the hole over nothing, or the old page.
5. The cost moves to the compositor: a desync subsurface commit at 300 Hz may cost gnome-shell more than the client
   saves. Measure gnome-shell's CPU alongside wubble's.
6. Buffer release cadence: mutter holds released buffers late, so the pool runs dry and frames are dropped
   (`pool_busy`).

**The first experiment (one day, kills or proves 1, 2, 4, 5):** `crates/wubble-engine/examples/layer_probe.rs`, Linux
only. It is an engine example because it names CEF. Dev-dependencies: wayland-client, wayland-protocols, gbm (all
already locked).
- Morning, no CEF. A winit window with `with_transparent(true)` and a wgpu surface in `PreMultiplied` mode (log the
  advertised alpha modes) that clears to opaque grey with a transparent rounded rect in the middle. Below it, a
  subsurface (`place_below`, empty input region, `set_desync`, viewport) showing 4 GBM buffers. A wgpu
  `clear`/copy fills each with a moving colour bar at the display's rate, with no shell present per frame.
  Pass: the bar shows through the hole, sharp, at 300 Hz; `wl_buffer.release` comes back; the shell surface commits
  0 times a second while the bar moves.
- Afternoon, CEF. Replace the bar with one CEF view (the bench's video page and a scroll page). In
  `OnAcceleratedPaint`, copy into the slot (welding's `copy_to_owned` with the pool texture as the destination, a
  local hack), then attach and commit. Toggle once a second between `set_sync` with a moving hole (the shell redraws)
  and desync.
  Pass: no garbage and no stale frame across 60 s of video; during a toggle, no frame with the hole over nothing.
- Read for 20 s each under `scripts/headless.sh` (300 Hz virtual monitor), against today's wubble on the same page:
  wubble main, GPU process and renderer CPU (the bench's own grouping), and **gnome-shell's** CPU.
  Go: main ≤ ~0.5 ms per frame and the total, gnome-shell included, below today's. Kill: risk 1 or 2 with no
  workaround, or the compositor eats the saving.

## 7. The experiment (run 2026-11-25)

`crates/wubble-engine/examples/layer_probe.rs` (Linux only), with `layer_probe/shot.py` (the headless
display's own pixels through mutter's ScreenCast and PipeWire), `layer_probe/measure.py` (CPU per process
group, mutter's and the headless ticker's) and `layer_probe/anim.html` (a bright page that paints every
frame). Branch `research/linux-layer`. RTX 5060, NVIDIA 615.71.09, mutter 50.4 headless
(`scripts/headless.sh`, `WUBBLE_HEADLESS_SIZE=1360x880@60|120|300`). Shots and raw logs:
`~/Dev/wubble/.local/tmp/layer-probe/` (`bar.png`, `cef-video.png`, `toggle-mailbox-worst.png`,
`toggle-control-worst.png`, `runs/`, `runs2/`).

What the probe is: a plain wgpu shell (not Slint), a 1360x880 transparent window drawn 0.35 grey with a
rounded 1100x760 hole; below it a `wl_subsurface` (`place_below`, empty input region, `wp_viewport`)
showing 5 Vulkan-exported dmabufs (`VK_EXT_image_drm_format_modifier`, modifiers intersected with
mutter's `zwp_linux_dmabuf_v1` v3 list), one `wl_buffer` each from `create_immed`. CEF frames come
through the engine's pump as in wubble (`PumpScheduler`, `page_painted`), are taken as welding's
`Copied` frame and copied **again** into a free slot (submit and wait), then attach, damage, commit.
The second copy is the probe's only shortcut; the build copies CEF's buffer straight into the slot.
Two small enablers: wubble-gfx `NativeFrame::copied_texture`, welding change 70 (`COPY_SRC` on the copy).

**Results, by the unknowns of section 6:**

1. **Transparency: works.** NVIDIA's Wayland Vulkan surface offers `[Opaque, PreMultiplied]`; with
   `PreMultiplied` the hole is truly transparent and the corners anti-aliased (`bar.png`,
   `cef-video.png`). No black, no garbage.
2. **Formats: accepted.** mutter lists 322 format/modifier pairs, 14 for `XRGB8888`; the driver picked
   NVIDIA's block-linear `0x0300000000606014`, not LINEAR. `create_immed` never failed. No
   foreign-queue release barrier was needed on NVIDIA.
3. **Sync artefacts: none seen.** The CPU wait before commit is enough: no stale or torn frame in the
   shots or by eye over the runs.
4. **Atomicity: holds, but sync deadlocks under FIFO.** With the shell presenting `Fifo` (what Slint's
   `AutoVsync` resolves to here), the first `set_sync` second with CEF frames arriving stalls the
   window: every shell acquire waits about 1 s, `wl_buffer.release` stops, the pool runs dry and the
   page freezes, also after `set_desync` (5 of 6 runs at 60 and 300 Hz; one of them recovered after
   about 3 s, the others never; the bar mode, whose commits follow the display, did not stall). Most likely NVIDIA's `wp_fifo_v1` and
   `wp_commit_timer_v1` presents and a sync child's cached commits waiting on each other in mutter
   [u: not checked on a real display]. With the shell on `Mailbox` it never stalls, and the toggle
   (sync with the hole and layer swinging 40 px each display frame, then desync, each 1 s) shows
   **0 frames of hole over nothing in 1951 composited frames at 300 Hz**. The check is real: the
   negative control (`--control`, the hole moves and the layer does not) shows a dark strip in 969 of
   1922 frames, up to 23,400 pixels (`toggle-control-worst.png`).
5. **The compositor does not eat the saving.** mutter costs less with the layer than with today's
   wubble (table). The bar mode at 300 Hz (300 desync commits a second, the shell idle): mutter
   60 ms/s, 0.2 ms per composited frame; the probe's main thread 24 ms/s, 0.08 ms per commit
   including its copies and waits.
6. **Release cadence: fine.** Releases match commits every second (300/300 in the bar mode, 60/60 in
   CEF mode); `pool dry` 0 outside the FIFO stall.

**CPU, the bench's video page** (`video-page.sh`: 1080p60 VP9 at 854x480, 60 page frames a second in
both; probe in its 1100x760 hole, wubble = `.local/wt/all/target/dist/wubble` in its 1280x800
window). 20 s after 10 s of warm-up, two rounds; ms of CPU per page frame (ms/s ÷ 60), round 2
(round 1 within ±0.1 ms on main):

| display | | main thread | main process | GPU process | renderer | mutter | total without renderer | total |
|---|---|---|---|---|---|---|---|---|
| 60 Hz | probe | 0.64 | 0.71 | 0.96 | 3.17 | 0.23 | **1.89** | 5.06 |
| | wubble | 1.23 | 1.35 | 1.02 | 3.16 | 0.28 | **2.64** | 5.80 |
| 120 Hz | probe | 0.63 | 0.70 | 0.84 | 2.80 | 0.19 | **1.73** | 4.53 |
| | wubble | 1.43 | 1.59 | 0.98 | 3.08 | 0.41 | **2.98** | 6.07 |
| 300 Hz | probe | 0.68 | 0.74 | 0.93 | 3.02 | 0.22 | **1.89** | 4.91 |
| | wubble | 1.41 | 1.56 | 1.00 | 3.12 | 0.44 | **3.00** | 6.12 |

The renderer is mostly VP9's software decode and noisy (±0.3 ms between rounds); it is the same work
in both, so "total without renderer" is the comparison. The headless ticker (not counted) costs
0.1 to 0.35 ms per frame under wubble's smaller window and nothing under the probe's full-size one.
Today's main thread measured 1.2 to 1.4 ms here, not the ~1.0 the plan assumed.

**Verdict: go, with two conditions.** The total, compositor included, is 0.75 to 1.25 ms per frame
below today's at every rate, and the main thread halves (1.23-1.43 → 0.63-0.68 ms). The main thread
misses the ~0.5 ms line by about 0.15 ms, but it carries the probe's second copy and wait; the build
copies once, and the keep-buffer patch removes that copy too. The conditions:
- **The shell must not present FIFO while a layer is synced** (risk 4 as found): `Mailbox` with the
  shell's own pacing while the layer exists, or sync only for one shell commit with the child idle,
  and a check on a real GNOME display first.
- The main-thread target becomes "about half of today's", checked again on the build.

**What the build needs** (on top of section 6's list): the welding Linux `OsBufferPool` as the probe's
`export_image` (exported Vulkan images with the compositor's modifiers, imported once, `COPY_DST`),
copying CEF's buffer into the slot inside `OnAcceleratedPaint` instead of welding's own texture; the
`wayland::Subsurface` layer as the probe's `Layer` (v3 modifiers were enough; v4 feedback only for the
main-device tranche); the vendored Skia surface's present mode settable to `Mailbox` with the shell
paced by the existing frame clock; the stall reproduced on the desktop session before relying on sync.

## Lines for TODO.md / decisions.md (not edited here)
- decisions.md: "Linux page layer: on Wayland, Layer: windowed as on macOS (a `wl_subsurface` below the shell
  through `PageHole`, sync only while a hole or place changes); frames copied into a per-view pool of dmabufs (CEF
  returns its buffer when the callback returns); X11 and XWayland keep the composited path; zero copy waits for a
  CEF keep-buffer patch."
- todo/in-progress/next.md, the Linux page layer item: research done (this file); next the one-day `layer_probe`.

### 7.x The desktop check (2026-11-25, once, the owner's go)
On the real GNOME desktop (300 Hz, 2560x1440, RTX 5060, driver 615.71.09), the probe with CEF frames and sync toggling once a second:
- FIFO shell: no stall in 30 s. Every second released what it committed (frames 144 to 223 a second, released the same, pool dry 0 except one second with 2), slowest present 4 to 24 ms in Sync seconds. So the headless display's stall comes from its timer-driven virtual monitor, not from NVIDIA or mutter on a real output.
- Mailbox shell: clean too in 15 s (pool dry 0 throughout), but the shell presents unpaced (up to 6153 a second in Sync seconds): Mailbox needs the shell's own pacing.
Decision for the build: Mailbox with the shell's own pacing while a layer exists works on both the real display and the headless one (smokes, the bench); FIFO is fine on a real display. Logs: `.local/tmp/desk-fifo.log`, `.local/tmp/desk-mailbox.log`.

## 8. The build (written blind, 2026-11-25, branch `sprint/linux-layer`)

Written without compiling or running anything (the owner's "sprint build, test end"); one compile-and-test round
follows as its own step. Base: main at 0.6.3. Nothing above the gfx trait changed shape: the same
`page_layer::Route` machine, `MODES` rows, `PageHole`, snapshots for glass and route lines.

- **welding change 71** (`native_frame/vulkan_pool.rs`, new): a Linux pool slot is a Vulkan image made with a
  modifier the compositor listed (`OsBufferRequest::formats`, the layer's `zwp_linux_dmabuf_v1` v3 events) and the
  driver can copy into and sample, exported as one dmabuf plane, imported into wgpu once (`COPY_DST |
  TEXTURE_BINDING`, `B8G8R8A8_SRGB`/`R8G8B8A8_SRGB` as the copy, `XR24`/`XB24` to the compositor). CEF's frame is
  copied straight into the slot inside `OnAcceleratedPaint` (the probe's second copy is gone) and waited for before
  the frame is handed over. A slot is in use while a clone of its `OsBuffer` lives or the compositor holds it
  (`compositor_hold`, set by the layer on attach, cleared on `wl_buffer.release`). `OsBufferPool` and `Next` moved
  from `metal.rs` into `mod.rs`, shared by macOS and Linux; busy drops are counted and reported (`take_os_buffers_busy`,
  `take_os_buffers_lost`) as on macOS, so the host repaints.
- **wubble-gfx** `layer::Subsurface`: one `wl_subsurface` per pane, `place_below` the shell's `wl_surface` (winit's,
  from raw-window-handle), empty input region, `wp_viewport` destination (whole points, rounded outwards:
  `page_layer::covering`) and source (the pane's part of the buffer). One `Compositor` (queue, globals, formats) per
  process on winit's connection, shared by every layer; each slot's `wl_buffer` (`create_immed`) is made once and
  kept in the slot (`OsBuffer::host_data`). Page frames commit **desync** (no shell render); a hide and a viewport
  change are made **sync**, so they land with the shell's next commit; the position is the parent's state and lands
  with its commit either way. So the trait's `present_in_transaction` does nothing on Wayland (the host still calls
  it). `read_compositor` reads the compositor's events without blocking; the host calls it before each CEF pump.
  X11 and XWayland: `make_layer` fails once per window ("not a Wayland window") and the window keeps the shell's path.
- **Skia change 5** (`present_mode.rs`): the application's hook says, before each frame, `Vsync` or `Paced(period)`;
  paced is Mailbox where offered, and a frame less than a period after the last present sleeps out the rest, as
  FIFO's acquire would block (renders are already throttled to the compositor's frame callbacks by winit, since
  Slint calls `pre_present_notify`; the sleep is the floor 7.x asked for). The host answers `Paced(refresh)` for a
  window with any layer (`page_layer::shell_present`, Linux only; `refresh` is CEF's rate, capped at 240 Hz), else
  `Vsync`.
- **Host**: `PLATFORM_MODES` has `("linux", Window)`; the Settings row (`page.layer`, now "Show pages on their own
  layer (macOS, Wayland)") defaults to on on macOS and **off on Linux**, so on Linux the layer runs only with the row
  on or `--page-layer=window`; the owner decides the default after measuring. Retired layers are now dropped one
  present after their hide (a destroyed subsurface unmaps at once). `WUBBLE_TRACE=1` prints the same route lines and
  per-second summary as on macOS.
- **Left as TODOs**: the per-frame texture on the composited path (no pool there) and the per-frame Vulkan import of
  CEF's buffer (a cache by `fd_identity`), both in `native_frame/mod.rs`; the release barrier to
  `VK_QUEUE_FAMILY_FOREIGN_EXT` after the copy (NVIDIA needed none, Mesa may); fractional scale (`wp_fractional_scale_v1`:
  the layer is rounded outwards, so at 1.25 the page is scaled by a fraction of a pixel); explicit sync
  (`wp_linux_drm_syncobj_v1`) instead of the CPU wait; v4 feedback's main-device tranche.

### Test round plan (one compile-and-test round)
1. `cargo check --workspace`, `cargo test --workspace`, `cargo test -p welding` (modifier table), the Skia crate's
   `present_mode` tests, then `cargo build --workspace`.
2. Hand runs under `scripts/headless.sh` (300 Hz and 60 Hz) with `WUBBLE_TRACE=1 --page-layer=window`: the route
   lines reach `Entering -> Layer`; the summary has `layer_shows` near the page's frame rate, `slint_renders` near 0 on
   a still overlay-free page, `pool_busy 0`; a shot shows the page through the hole (layer_probe/shot.py); a resize, a
   sidebar spring, a tab switch and a toast as page-layer.md's macOS list, with no frame of a hole over nothing.
3. The Linux-vs-Firefox bench (`next/bench`) states, each with `--page-layer=off` and `--page-layer=window` and in
   Firefox: idle, a still page, 1080p60 video, video under glass (a toast or the video strip), scrolling, a CSS
   animation, typing. Per state: CPU per process group (main thread, main process, GPU process, renderer, mutter),
   memory, `cef_frames`, `slint_renders`, `pool_busy`. Go for the default: main thread about half of `off` on video
   and scroll (section 7: 1.2-1.4 → 0.6-0.7 ms per frame), the total with mutter below `off`, and no state worse than
   `off`. `--measure idle --seconds 4` with `window`: 0 to 2 `slint_renders`.
4. Once on the real GNOME desktop (the owner's go): the same video and scroll states, and a sidebar toggle and a split.
