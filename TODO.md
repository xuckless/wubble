# wubble roadmap

A Chromium-based browser with a Slint shell. Web content is rendered off-screen by
Chromium and composited by Slint as GPU textures, so the whole UI (sidebar, tabs,
popups, split views) is one scene we control.

Legend: `[ ]` todo · `[~]` in progress · `[x]` done · **(?)** blocked on an open decision.

## Decided

| Topic | Decision |
|---|---|
| Theme | Dark only, as the design repo decides ("Dark only; light mode comes later"). The tokens are the port's `tokens.slint` verbatim (Palette, Spaces, Radius, Spacing, Chrome, Glass, Fonts, Type, Motion); the light values invented earlier are deleted. Values the port has no token for come from the design repo's CSS (`browser.css`) and live in `app-tokens.slint` |
| Slint ids | Slint has only `int` (i32), so `TabItem.id` / `SpaceItem.id` are `int`; the wiring item converts from core `u64`/`u32` with `try_from`, never truncating silently |
| Key routing | Each default binding has a `Route` column: `Shell` (shell always takes it: NewTab, CloseTab, ReopenClosedTab, NewWindow, Next/PrevTab, Quit, Fullscreen, spaces, sidebar; the Chromium `kReservedCommandIds` / Firefox `reserved` set plus wubble-only actions) or `PageFirst` (page gets the key, the shell acts only if unhandled, via CEF `OnKeyEvent`; everything else, incl. Zen's Mod+Alt+S compact mode). Overrides take the action's default route. In page fullscreen only Fullscreen stays `Shell` |
| Licence | GPL-3.0-only (matches Slint's GPL option; Chromium/CEF are BSD and compatible) |
| Platforms | Daily-drive targets: Linux (lead, Wayland) and macOS, both built locally. Windows is built and tested on CI runners only and may trail by a milestone |
| Engine, phase 1 | Prebuilt CEF 154 via the `cef` crate (stock Chromium, **not** ungoogled) |
| Engine, phase 2 | Our own build: ungoogled-chromium patches + CEF (M6) |
| UI | Slint 1.19, winit backend, wgpu 30 |
| Frame path | CEF accelerated OSR → native handle → `wgpu::Texture` → `slint::Image`, modelled on `welding` / `grafting` |
| Extensions | Not before our own Chromium build (M6). Built-in content blocker until then |
| DRM | Not in the shipped binary. Widevine is bolted on later (M6): detect a CDM in an existing Chrome/Brave/Firefox install, or opt-in download from Google's CDN on demand |
| Texture import | `welding` 0.15.0, forked: vendored in `vendor/welding` (see "welding fork") |
| Daily-drive features | Vertical tabs, Spaces, command bar, compact mode. Everything else in M2 comes after |
| Sync | Deferred, no design work yet |
| Import | Zen and Arc first; Helium where useful. `wubble-import` reads `places.sqlite` with `rusqlite` (bundled, `serialize`; header bytes 18/19 patched WAL to rollback since SQLite cannot deserialize a WAL image) and mozLz4 with `lz4_flex` (block only); Zen spaces/essentials/folders come from `zen-sessions.jsonlz4`, not SQLite |
| UX source | The owner's design repo `xuckless/the-wallpaper-project`, directory `src/wubble` (private; clone to `~/Dev/wubble-wt/ref/the-wallpaper-project`). It defines the UI feel, UX and interactions, and wubble's UX is built to match it. Its `slint/` directory is a ready Slint + Rust + WGSL port (tokens, springs, seam, split view, tabs, scroller, components, wgpu glass/fluid renderer) meant to be copied in. It does not cover the whole feature set: features it lacks follow its style. Where it differs from an earlier decision in this table, raise it. The older canvas snapshot in `ux/pack/` is superseded |
| Glass | Wubble draws its own glass (blur + saturate + tint + specular + rim) so it looks the same on every OS; the window itself is also transparent where the OS allows |
| Frame budget | 120 Hz, 8.33 ms, no dropped frames during animation; springs are physics-stepped per frame and retarget on interruption |
| Layout | No top toolbar. The sidebar follows the design repo's `chrome.tsx` / `browser.css`: address pill, favourites (pinned tabs, 4 columns), the Space name, tabs, new-tab row, Space switcher at the bottom; padding 6, 12 between groups, every icon and label on one grid 24 px from the window edge. Window lights are in the top row (30 px); back/forward/reload are not built yet |
| New tab | Ctrl+T opens the command bar over the current page; a tab is created only when something is chosen. The design repo's bubble start page is deferred (confirmed 2026-11-18); the repo is a kit of what we might need, not a list of what must ship |
| Frameless window | `no-frame: true` with a transparent background; the shell draws the window (Abyss ground, lights, 7 px edge and 16 px corner resize zones, as the design's `desktop.tsx`). Move is Slint's `WindowMoveArea` (winit `drag_window`), resize is `drag_resize_window` from the edge zones: both hand the grab to the compositor, which is the only way on Wayland. macOS has no `drag_resize_window`, so resize there needs a client-side path (open) |
| Window platform flags | `wubble_ui::window::Platform` (`CLIENT_GEOMETRY`, `TRANSPARENT_CORNERS`) per windowing system, chosen from the raw window handle: Wayland (false, true), X11 (true, false), macOS (true, true), Windows (true, false). Corners are rounded only where transparent corners are true and the window is not full screen or maximised; `WUBBLE_SQUARE_CORNERS=1` forces square. The window spring runs only where `CLIENT_GEOMETRY` (not built yet) |
| Minimise | Through winit `set_minimized(true)` directly, never Slint's `minimized` property: on Wayland winit cannot report the state back, so Slint's flag would stay set and the button would work once |
| Command bar state | One `CommandBarState` enum written only by a pure `next()` via `dispatch` in Slint; closing to closed is a Slint Timer equal to the out duration (no animation-finished callback) |
| Passwords | Built-in manager (encrypted store, OS keyring) before extensions; extension password managers work once the extension pass (M6) lands |
| Glass under load | Glass degrades automatically (lower-res blur → cached backdrop → flat tint); animation never drops frames for it |
| Glass backdrop | Slint 1.19 has a native `backdrop-blur` on `Rectangle`, femtovg-wgpu included (i-slint-compiler `typeregister.rs` `RESERVED_BACKDROP_BLUR_PROPERTIES`, i-slint-renderer-femtovg `itemrenderer.rs` `blur_backdrop`), but no saturate, no per-tier sharing, no intermediate quality, and each panel ends the render pass and copies the screen. So `wubble_gfx::glass` renders it: one shared dual-Kawase down chain (ported from KWin) from the web frame, one up chain per tier in use with saturate folded into its last pass, half-res `slint::Image` outputs; blur and saturate run on gamma-encoded sRGB (`Rgba8Unorm` views), as Chromium applies CSS `blur()`/`saturate()` and KWin blurs. Slint layers tint, specular, rim, film rim (`@conic-gradient` border, no image) and shadow; the shadow is drawn only under a backdrop, from an opaque caster, because femtovg takes shadow alpha from the element's fill and does not cut it out of the panel (Flat glass has no shadow, unverified on screen). Quality is one enum `Full/HalfRes/Cached/Flat` moved by a pure `Governor::next(cost, budget)`; a level that fails soon after a step up is retried only after a calm run that doubles each time, so steady load cannot flip it between two levels. Only web content is blurred: glass not wholly over the web frame shows the flat fill. Native `backdrop-blur` stays a hand-measured alternative |
| Storage | wubble's own Turso database (the Rust SQLite rewrite, `turso` crate, pinned `=0.9.0-pre.5`, default features off because `mimalloc` would install a global allocator; no async runtime needed) with a declared schema and `PRAGMA user_version` migrations: Spaces, tabs, session, history, bookmarks, settings; passwords get their own encrypted store later. Chromium keeps only cookies/cache per container |
| Session storage | `wubble-store` loads and saves `wubble_core::model::Model` directly (`load_session`/`save_session`, one transaction, schema v2). Restored tabs are Discarded, idle, no opener; scroll, busy, folder collapse and id counters are not stored (counters derived as max id + 1 by `Model::from_parts`, so a freed highest id can be reused after restart). `Tab.container` is the effective container, so it covers the per-tab override. Recently-closed list bounded to 25. Settings are typed through one `settings!` table |
| Containers | Each Space has a default container; a tab can be reopened in another, and Spaces may share one |
| Deleting a Space | Closes its tabs (as Zen does); the UI confirms first. The last Space cannot be deleted |
| Ctrl+T in the model | `Command::OpenCommandBar` returns only `Effect::ShowCommandBar`; the tab exists once `NewTab` is sent after a choice |
| Content blocker | Brave's `adblock-rust` (network + cosmetic, uBlock/EasyList lists), on by default, per-site toggle |
| Blocker build | `adblock` with `default-features = false` (+ `embedded-domain-resolver`, `full-regex-handling`): the default `single-thread` feature makes `Engine` not `Send`/`Sync`, and CEF calls us from several threads |
| No-blur desktops | Opaque Abyss ground with Space tint (GNOME etc.); in-window glass unaffected |
| Search | First-run choice between Google and DuckDuckGo. Suggestions are local (tabs/history/bookmarks); live engine suggestions exist behind a flag, off by default, and never send typed addresses |
| Network on its own | No telemetry. Crash reports are opt-in and open a GitHub issue; designed at ship time (M7) |
| Internal surfaces | Native Slint for everything wubble owns: settings, history, bookmarks, downloads, task manager, menus, dialogs, prompts. Web pages only for: DevTools, error pages (network, certificate, crashed tab), PDF viewer, view-source, Chromium's diagnostic pages, extension pages |
| NVIDIA Linux frames (interim) | Prebuilt CEF 154 delivers empty dmabufs on NVIDIA (LINEAR buffers the driver cannot render into; Chromium commit a531c83a). Owner approved a temporary CPU fallback (2026-11-18): software paint uploaded to a GPU texture, NVIDIA-Linux only, logged and counted, removed once zero-copy works there. Zero-copy stays the rule everywhere else |
| CEF version | Move to CEF 156.0.30+ (`cef` crate 156.6) once the welding fork lands. Probe on this machine (RTX 5060, driver 615, Wayland): 156.0.30 delivers real frames with an NVIDIA block-linear modifier at default settings (CEF fix #4237 / #4313, not in the 154 branch). Do NOT pass `enable-features=Vulkan`: it yields empty frames even on 156. NVIDIA dmabufs carry no implicit fence. If 156 works through wubble's own import path, the CPU fallback is not built |
| Tab sleeping | Freeze after 1 min hidden, discard after 15 min, discard immediately under critical memory pressure. Pinned tabs freeze but never discard. Busy tabs (audio, capture) are left alone |
| Shell animation | The design repo's Slint port is the single implementation: pure physics in `wubble_core::{motion, seam, tabs, scroller}` (f64, copied 1:1, numeric parity with the TS checked against a stored bun run), and `wubble_ui::driver` (the port's frame body) pushes the values into Slint `in` properties. The per-frame hook is wubble's one vsynced rendering notifier, not the port's 16 ms `slint::Timer`; it requests a redraw only while something moves. Tweens (0.97 press, hover fades, film rim, overlap) stay Slint `animate` with the token easings. The sidebar has the port's three modes, extended (200–340, resizable), rail (56) and hidden, on one width spring owned by `Seam` (rubber band, snap to the rail under 128, hidden under 22 from the rail); the toggle is hidden ↔ extended, anything else hides (`chrome.tsx` `cycleSidebar`); double-clicking the seam flips rail/extended. Peek is presentation-only: one Slint bool set by two Slint Timers (60 ms in, 260 ms out). The driver never edits the Slint models: it re-reads the ids on `shell-model-changed`, a click is `tab-activated`, a drag `tab-moved(id, to)` (the host does not handle `tab-moved` yet, so a dropped row springs back to the model order). Selection shows by fill and the touch sheen; no focus borders |
| CEF sandbox | Off on Linux for now: welding `CefSandboxMode::UnsandboxedTrustedContent` (`no_sandbox=1`) in both `wubble-engine` (`SANDBOX`) and `wubble-helper`. The constant is duplicated on purpose (the helper must stay tiny and cannot depend on `wubble-engine`, which pulls in wgpu); the sandbox item must change both. Turned on by the M1 item "Sandbox enabled on all platforms" |
| CEF cache directory | Always wubble's own `root_cache_path`, never CEF's shared default `~/.config/cef_user_data` (its process-singleton lock made concurrent runs fail or crash). Browser: `$XDG_DATA_HOME/wubble/cef`, else `~/.local/share/wubble/cef` (Linux, via the `Platform` trait; macOS/Windows not yet). `--smoke boot`: a fresh per-process temp directory, removed on exit, so concurrent smokes never share the lock. Second-instance handling (handing a relaunch to the running browser) is not built yet |
| CEF message pump | Schedule-driven, with CEF's `external_message_pump` on: `OnScheduleMessagePumpWork` (welding fork's `PumpScheduler`, any thread) → `slint::invoke_from_event_loop` → `Engine::schedule_pump`, which runs `wubble_core::pump` (a port of cefclient's `MainMessageLoopExternalPump`: delay replaces the pending timer, at most 33 ms between runs, re-entrant requests re-run) and asks the host for one single-shot `slint::Timer`. The host picks up frames after each run of work. `Engine::tick` (a synchronous run) remains only for `--smoke boot` and the shutdown drain, which have no event loop. Windows never calls the hook (CEF's own loop thread) |
| Shared GPU device | `wubble-gfx::GpuContext` creates the one wgpu device (Vulkan, high-performance non-CPU adapter, `welding::build_dmabuf_capable_device` with Slint's own device descriptor) and hands it to Slint with `BackendSelector::require_wgpu_30(WGPUConfiguration::Manual)`, renderer `femtovg-wgpu`. Not the rendering notifier: Slint's own device lacks the dmabuf extensions. One handle is leaked at exit because Slint drops its copy during thread-local teardown, after wgpu-core's thread-locals (debug builds abort otherwise) |
| Frame copy | welding imports CEF's dmabuf as `Bgra8UnormSrgb` with `TEXTURE_BINDING \| COPY_SRC`; Slint accepts only `Rgba8Unorm(Srgb)` with `TEXTURE_BINDING \| RENDER_ATTACHMENT`. So each frame gets one GPU blit (ported wgpu fullscreen-triangle shader) into a ring of 2 `Rgba8UnormSrgb` targets, each wrapped once as a `slint::Image`; alternating keeps `web-frame` changing. GPU to GPU, no CPU readback; one extra full-frame copy to account for in the M0 measurement. The welding fork cannot remove it: CEF's buffer is BGRA (importing it as RGBA swaps red and blue, and Slint takes no swizzled view), and render-attachment usage on the imported DRM-modifier image needs a grafting fork and is driver/modifier dependent. Revisit if Slint accepts `Bgra8` or a swizzled view |
| Chromium GPU on Linux | `enable-features=Vulkan` for every wubble process (`Platform::CHROMIUM_SWITCHES`). Without it NVIDIA 615 (Wayland and X11) produces no accelerated paint: Chromium's GL path cannot draw into the dmabuf shared images ("Unable to initialize SkSurface"). **With it, on NVIDIA 615 / Wayland, the frames arrive but are empty** (every byte 0, alpha 0; the buffers are `DRM_FORMAT_MOD_LINEAR`, which NVIDIA cannot render into, cf. Chromium bug 428022619), so the content area shows the window ground. No CEF switch tried fixes it (X11, headless, ANGLE-Vulkan, Graphite, native GMBs). Open; not yet run on Mesa (Intel/AMD) |
| Pixel check | `--smoke frames=N` on the bundled page snapshots the window after the last frame (`Window::take_snapshot`, which re-renders through femtovg-wgpu, imported textures included) and fails unless a 16x16 grid over the content rectangle has at least 8 distinct colours and mean luma at least 40. A correctness check, not timing; skipped for `--url` pages, whose colours are unknown |
| CEF frame rate (interim) | The browser is created with `windowless_frame_rate` = the display refresh (current monitor, else the fastest connected one, since Wayland reports the monitor late), capped at 240, through the welding fork's `CefSurfaceConfig::windowless_frame_rate`. The view opens once winit has created the window. The fork also exposes `CefSurfaceConfig::external_begin_frame`, still off: turning it on (and calling `SendExternalBeginFrame` through `cef::browser_host_get_browser_by_identifier`) is the "Frame pacing" item |
| CPU fallback count | Measured on Linux: the welding fork's `on_paint` counts every CPU frame (never read or shown) and `View::cpu_fallback_frames` reports it; the host logs new ones and `--smoke frames` prints the real `cpu_fallback`. Other platforms print `n/a`. The smoke still passes on `accelerated > 0`; whether `cpu_fallback > 0` should fail it is open |
| View close | Event-driven: the welding fork pushes `CefSurfaceEvent::BrowserClosed` from `OnBeforeClose`; the engine drains every view's event queue after each run of CEF work and reports `BrowserClosed` to the lifecycle. A page that closes itself is reported the same way |
| Web events | One enum, `wubble_engine::nav::WebEvent` (URL, title, loading + can-go-back/forward, favicon URL, cursor, popup request, renderer gone), collected from every view after each run of CEF work into `PumpOutcome::events`. Navigation per view is the pure `nav::transition` over `NavState` (a `Page` plus a `Phase`: Blank/Loading/Loaded/Crashed): events only on change, Back/Forward only when possible, commands after a crash also repaint. Loading state and favicon URLs come from two welding fork callbacks (`OnLoadingStateChange`, `OnFaviconURLChange`), which welding lacked. Popups: welding's `OnBeforePopup` already cancels CEF's own window; the host opens `PopupRequested` as a new tab next to its opener. A dead renderer is reloaded once by the transition itself; a second crash before that reload has finished loading leaves the view crashed |
| Web input | Keyboard from raw winit events (`slint::winit_030` `on_winit_window_event`), because Slint's `KeyEvent` keeps only text: no physical key, scancode or location, nothing for keys without text, Ctrl/Cmd swapped on Apple. Mouse, wheel and hover from a Slint `TouchArea` over the web frame (hit-testing against the shell, view-relative logical px = CEF DIP, grab and leave for free); click count from a ported cefclient tracker; sub-pixel wheel motion is carried per view, not rounded away; a release of one button releases every held button, because Slint ends the `TouchArea` grab on any release. Keys go to the page only while its `FocusScope` has focus; modifier keys also reach Slint; `PRIMARY`+T/L/W stay with the shell. The engine sends input straight to CEF's `BrowserHost` (welding's input path lacks click count, leave, wheel flags, system keys and a key table). Key codes: cefclient GTK's policy (layout character first, Chromium's US-position table for the rest, digit row and keypad positional). Lost: Caps/Num Lock state, AltGr flag, layout remaps of text-less keys, IME |
| welding fork | `welding` 0.15.0 (MPL-2.0) vendored in `vendor/welding`, excluded from the workspace and from `cargo fmt` by its own `rustfmt.toml` (upstream style, `cargo test -p welding`). Changes, each marked `wubble change` and listed in `vendor/welding/WUBBLE_CHANGES.md`: browser-process `PumpScheduler`, `BrowserClosed` event, Linux `on_paint` count, `windowless_frame_rate` / `external_begin_frame` config, `LoadingStateChanged` / `FaviconUrlsChanged` events. `grafting` is not forked. When moving CEF or welding, start from the new upstream release and re-apply the change list |
| Tabs in the host | The binary owns one `wubble_core::model::Model`; every UI callback and engine event becomes a `Command` for `model::apply`, and each returned effect runs through one pure table (`host::call`) into one `execute`. `apply` also settles tab lifecycles: whatever the command, the visible tab (active tab of the active Space) is Active and every other tab hidden, with the `lifecycle::transition` effects appended as `Effect::Lifecycle`; a private model clock, set by `Command::Tick`, stamps hides. One engine view per live (not discarded) tab; only the visible tab's view takes frames, layout and input, the rest are told `was_hidden(true)` (and repaint on show). Discard and ReleaseTexture close the view and keep the tab; Restore reopens a view at the model's URL (a live view after a crash reloads itself). **Freeze has no CEF mapping**: CEF 154 has no freeze call (no `freez`/`frozen` in the cef 154.5.0 bindings), so `Freeze`/`Unfreeze` are no-ops and `was_hidden` (Chromium throttles timers and stops compositing) is the only lever; a real freeze would be CDP `Page.setWebLifecycleState` through `BrowserHost::execute_dev_tools_method` (unverified on Chromium 154, follow-up). Snapshots are no-ops until there is UI for them. The lifecycle tick is a 5 s `slint::Timer` with `Pressure::None` (no pressure source yet); smokes never start it. The sidebar is re-derived after every change by `shell::view_model` and replaced wholesale. Popups open a new tab in the opener's Space right after it, shown only when the opener is the visible tab (a hidden tab's popup opens hidden). `LoadingChanged` marks the tab busy while it loads, so a loading tab is never frozen or discarded (busy has no other source yet). Switching the visible tab blanks the web frame until the new tab paints, so another tab's frame never shows while input goes to the new one. A page that closes itself closes its tab |
| Helper binary | `wubble-helper` is a separate package, so `cargo build --workspace` must run before `cargo run -p wubble`; the engine refuses to start with a message saying so if the helper is missing |

## Layout

```
crates/wubble          main binary: wires engine + gfx + ui
crates/wubble-helper   CEF subprocess entry point (renderer/GPU/utility), no Slint
crates/wubble-core     tabs, spaces, sleep policy, session — pure logic, unit-tested
crates/wubble-engine   the only crate that names CEF types
crates/wubble-gfx      the only crate that touches native texture handles
crates/wubble-ui       Slint shell
ux/                    UX pack (source of truth for wubble-ui)
docs/adr/              one short record per architectural decision
```

Rule: the shell never sees a CEF type. That boundary is what makes the M6 engine swap cheap.

Working rules and code principles: see [AGENTS.md](AGENTS.md).

## M0 — Prove the frame path (spikes, throwaway code allowed)

Exit: one live page inside the Slint window at display refresh rate, with zero CPU
copies, on Linux/Wayland/NVIDIA. Nothing else starts until this passes.

- [x] Workspace skeleton; `cargo check --workspace` passes against prebuilt CEF
- [x] CEF init + external message pump integrated with Slint's winit event loop (`wubble --smoke boot` checks it)
- [x] `wubble-helper` runs `execute_process`; subprocesses start from it
- [x] Linux: `OnAcceleratedPaint` dmabuf → Vulkan external memory → `wgpu::Texture` on Slint's device (`wubble --smoke frames=300` checks it, including pixels)
- [x] Texture → `slint::Image` → `AppWindow.web-frame` (through one GPU blit, see "Frame copy")
- [~] Confirm which Slint renderer accepts wgpu textures at full speed (femtovg-wgpu vs vello vs skia-vulkan): femtovg-wgpu works with a shared Manual device; speed not yet measured
- [ ] **Glass spike**: backdrop blur + saturation of live web content behind a Slint panel at 120 Hz. If Slint can't do it natively, `wubble-gfx` renders the blurred backdrop as its own wgpu pass and hands Slint a second texture
- [ ] **Spring spike**: one per-frame physics-driven property (follow spring k 300 / c 15) with no missed frames while a page scrolls
- [x] Frameless window: lights, drag to move, resize zones, rounded corners where the platform has transparency (not run on a display yet; see Decided)
- [ ] Transparent window + compositor blur per platform (macOS vibrancy, KDE blur; GNOME has no blur protocol, so fall back to opaque Abyss ground)
- [ ] Frame pacing: Chromium begin-frames driven by the compositor frame callback, not a timer. The welding fork exposes `external_begin_frame` (off) and the host is reachable; wiring it is this item. Interim: CEF frame rate = display refresh, pump scheduled by CEF
- [ ] Resize without waiting for CEF: with the 8 ms poll gone the host applies a new content size on the next run of CEF work (at most 33 ms later); a Slint `changed` hook on the content size could schedule a run at once
- [ ] Measure: frame time, latency input→photon, count of fallback `OnPaint` (CPU) frames — must be 0 (counted on Linux through the welding fork; `--smoke frames` prints it)
- [~] NVIDIA specifics: dmabuf modifiers, explicit sync. RTX 5060 / 615.71.09 / Wayland imports frames with `enable-features=Vulkan`, but Chromium leaves them empty (linear dmabufs, see "Chromium GPU on Linux"); `--smoke frames` now fails on this. Explicit sync unverified
- [ ] macOS port of the spike: IOSurface → Metal wgpu texture; helper `.app` bundle layout
- [ ] Windows CI runner builds the workspace; D3D11 shared-handle port follows once Linux + macOS pass
- [x] ADR: keep `welding` as a dependency, or vendor/fork the import code: forked into `vendor/welding` (see "welding fork")

## M1 — Engine layer (a correct windowless browser)

Each item is done when it works on Linux and macOS; Windows is verified on CI.

- [~] View lifecycle: create, resize, HiDPI + fractional scale, visibility, close: visibility (`View::set_visible`, `was_hidden` + repaint on show) and closing one view (`Engine::close_view`) done; per-platform check open
- [~] Mouse, wheel (smooth + kinetic), touch, touchpad gestures (pinch-zoom, swipe-back): mouse (move, leave, buttons with click count) and pixel wheel on both axes done (see "Web input"); kinetic phase, touch and gestures open
- [~] Keyboard: layouts, dead keys, shortcuts routed shell-first vs page-first: key down/up/char with modifiers from raw winit, non-US layouts by character then position, shell keeps Ctrl/Cmd+T/L; dead keys unverified (hand check), Caps/Num Lock state and AltGr flag not available from winit
- [~] Cursor shapes and custom cursors: CEF's shape applied to the window through one table in `app.slint` indexed by the engine's `Cursor`, and shown at once by replaying the pointer position (Slint reads a cursor only on a mouse event); custom (image) cursors and zoom-in/out open
- [x] Navigation API on a view (load, back, forward, reload, stop) and one host event enum (`WebEvent`); `wubble [URL]`; `--smoke nav`
- [ ] IME: composition, candidate window positioning (text-input-v3 / TSF / NSTextInputClient)
- [ ] OSR popups: `<select>`, autofill, date/colour pickers — drawn as Slint layers that may leave the view rect
- [ ] JS dialogs, context menus, permission prompts, file choosers, auth prompts — all Slint-native
- [ ] `window.open` / popup windows → wubble's own policy (new tab, glance, real window). Reported as `WebEvent::PopupRequested` (CEF opens nothing); interim: a new tab next to its opener; the policy is still open
- [ ] Multi-window; invisible/hidden windows for background work
- [ ] Clipboard, drag-and-drop (into, out of, and between views)
- [ ] Downloads; printing / save as PDF
- [ ] Fullscreen (element and window), picture-in-picture
- [ ] Find in page, zoom, text selection handles
- [ ] Audio indicators, mute, media session
- [ ] Accessibility tree bridged into Slint's AccessKit
- [~] Crash handling: renderer/GPU process death recovers the view. Renderer death: `WebEvent::RendererGone`, the host reloads once per URL with a repaint; GPU process death open
- [ ] Sandbox enabled on all platforms

## M2 — Shell (styled after the UX pack)

The pack sets colour, glass, type and motion feel. Layouts on its Browser board are illustrative; exact sizes quoted below are starting points.

Foundations
- [x] Tokens as Slint globals: ground (Abyss/Deep/Shell/Rim), ink (Ink/Mist/Haze), film (Sky/Lilac/Blush/Mint), Coral; radius 6/10/14/20/28/pill; 4 pt spacing; light theme fills
- [x] Fonts embedded: Fredoka (display, 32 px+), Geist (interface), Geist Mono (addresses)
- [x] Glass component, three tiers: Thin (blur 16), Regular (blur 30 + Space tint 10%), Thick (blur 44 + film rim) (`wubble-ui` `GlassPanel`, the port's Slint-only fallback, + `GlassBackdrops`, backdrop pass in `wubble_gfx::glass`; host wiring into the present loop and the on-screen look are the next batch)
- [x] Glass quality governor: measures frame time and steps blur quality down/up; exposed as explicit levels, not ad-hoc flags (`glass::Quality` + pure `Governor::next` with retry backoff; the host supplies the measured costs, next batch)
- [ ] Film rim (conic soap-film ring) on hover, press, drag and focus only
- [x] Motion library in Rust (springs, rubber band, stretch, easing constants in wubble-core::motion; press/squash/wobble/Overlap compose these): follow spring, stretch spring (volume-preserving), rubber band, press 0.97, squash, wobble, Overlap in/out with stagger
- [ ] App icons exported per platform from the Icons board

Daily-drive set
- [ ] Vertical sidebar on Regular glass: tab rows 36 px, pinned section, "Today" group, new-tab row (rows, favourites, new-tab row, scrolling list and drag-reorder in the UI done; no "Today" group; glass waits for the wgpu renderer)
- [x] Spaces, containers and tab-tree model with one pure `apply(model, command)` (`wubble_core::model`) with tests
- [x] Tabs for real: one `Model` in the binary drives the engine (a view per live tab, only the visible one shown) and the sidebar (`view_model`); Ctrl+T/L/W, tab click/close, Space switch, popups as tabs; `--smoke tabs` (see "Tabs in the host")
- [ ] Spaces: switcher with stretching indicator, per-Space tint, per-Space container (switcher with the drawn icons and the stretching capsule done; the model has no icon per Space yet, so the three drawn icons repeat)
- [ ] Command bar on Thick glass: address, search, switch-to-tab, actions, shortcuts (UI component done on flat Glass; results wired: a go/search row first, then switch-to-tab over open tabs, through one minimal `bar::resolve` classifier; glass, history/bookmarks and actions remain)
- [x] Suggestion planning + live-suggestions flag (`wubble_core::suggest`), with tests
- [x] Command bar brain (`wubble_core::omnibox`): `classify` (Chromium Parse rules), one search-engine table, `rank` over tabs/history/bookmarks/actions; wiring to Slint results remains
- [x] Keybindings and actions as data (`wubble_core::action`, `keys`): one `Action` enum shared with the command bar, `DEFAULTS` chord table (Zen first, then Firefox/Chromium) with per-platform rows, chord parser/formatter, user overrides + conflict detection; the key-event router and editor UI remain
- [ ] Suggestion fetch effect (run `Source::Remote` requests, debounce, cancel stale) and the settings toggle for the flag
- [x] Compact mode: the design repo's three sidebar modes (extended, rail, hidden with peek) on one width spring; the seam is the rubber string, bowing while held and twanging on release (`wubble_core::seam`)
- [x] URL pill (Thin glass): host in Ink, rest in Haze (the port's Slint-only `GlassPanel` until the wgpu glass lands)
- [x] Web view inset: radius 14, 8 px gap from glass
- [x] Turso schema + migrations (`wubble-store`): session, history, bookmarks table, settings
- [ ] Session restore and history recording wired into the app
- [ ] First-run: search engine choice (Google / DuckDuckGo)

After daily-drive
- [ ] Component sheet: elastic slider, toggle, pin, download button, toolbar dock + gliding tooltip, menus, checkboxes
- [ ] Split view (2–4 panes); glance / peek
- [ ] Tab drag: reorder, between Spaces, into split, out to a window (reorder: the UI emits `tab-moved(id, to)`; the host's `on_tab_moved` → `Command::MoveTab` is not wired)
- [ ] Bookmarks, settings UI, keybinding editor
- [ ] Touch sizing (44 px rows)

## M3 — Memory and sleeping

- [x] Sleep policy as a pure function (`wubble_core::sleep`) with tests
- [x] Tab lifecycle state machine (`wubble_core::lifecycle`) with tests
- [~] Apply liveness effects in the engine: hide → freeze → discard → restore, with snapshot texture for discarded tabs: hide, discard and restore done; freeze is a no-op (no CEF API, see "Tabs in the host"); snapshot texture open
- [ ] "Busy" detection: audio, capture, WebRTC, downloads, unsaved form input
- [ ] Memory pressure source per OS (Linux PSI, Windows memory notifications, macOS dispatch source)
- [ ] Per-tab memory/CPU accounting and a task manager view
- [ ] Process model tuning: site isolation level, renderer process limit, spare renderer
- [ ] Release GPU textures of hidden views; cap decoded-image and back/forward caches
- [ ] Benchmark vs Zen and Chrome: 10 / 50 / 200 tabs, cold and after 1 h idle
- [ ] Per-site and per-space overrides ("never sleep")

## M4 — DevTools and built-ins

- [ ] DevTools docked as a wubble view, or detached; inspect element
- [ ] Remote debugging endpoint (off by default)
- [~] Content blocker on `adblock-rust`: list fetch/refresh, network hook, cosmetic injection, per-site toggle
  - [x] `wubble-blocker` crate: decisions, cosmetic data, compiled cache, per-site shields, pure refresh policy
  - [ ] List fetch behind an off-by-default flag, CEF network hook, cosmetic injection, per-site toggle UI
- [ ] Built-in password manager: encrypted store keyed from the OS keyring, form detection and fill, save prompt, generator, import from Zen/Arc
- [ ] Reader mode, screenshots, PDF viewer

## M5 — Arc-style features

- [ ] Auto-archive of stale tabs; archive browser
- [ ] Link routing rules (which space/profile opens which site)
- [ ] Little window for links opened from other apps
- [ ] Boosts: per-site CSS/JS/colour tweaks
- [ ] Hover previews, tab tidy/auto-group
- [ ] Picture-in-picture on tab switch
- [ ] Sync (deferred)

## M6 — Own engine: ungoogled Chromium + CEF

Needs ~150 GB free and several hours per full build per platform.

- [ ] Reproducible build recipe: Chromium checkout at the CEF branch + ungoogled patch set + CEF patches
- [ ] Rebase conflicts between ungoogled and CEF patches; keep our delta as a patch queue
- [ ] Build with `enable_widevine` and proprietary codecs on; ship no CDM
- [ ] Widevine CDM loader: find an installed CDM (Chrome, Brave, Firefox profiles), else opt-in download from Google's CDN; verify version/ABI against our Chromium
- [ ] Document the limits: Linux gets software DRM (L3, typically capped around 720p). macOS and Windows services that require a Google-signed (VMP) browser will refuse us regardless of the CDM
- [ ] Build infrastructure: Linux and macOS local; Windows on a CI runner large enough for a Chromium build
- [ ] Extensions: patch the browser layer so `chrome.tabs`/`windows`/actions map to wubble tabs, spaces and toolbar
- [ ] Extension UI: action buttons, popups (OSR), options pages, install flow, Web Store access
- [ ] Swap in behind `wubble-engine`; shell unchanged

## M7 — Ship

- [ ] Security update cadence: track Chromium stable (weekly-ish); a browser that lags is unsafe
- [ ] Packaging: Flatpak/RPM, MSI/winget, signed + notarised DMG
- [ ] Auto-update
- [ ] Opt-in crash reports that open a GitHub issue (scrub URLs and personal data before anything leaves the machine)
- [ ] Default-browser registration, URL handlers
- [~] Import from Zen and Arc (Spaces, pinned tabs, history, bookmarks); Helium/Chromium profiles
  - [x] `wubble-import` readers: Zen/Firefox (profiles.ini discovery, `places.sqlite`, session, `zen-sessions.jsonlz4`, containers) and Arc `StorableSidebar.json`; pure parsers, plain `Imported` output
  - [ ] App wiring: pick a profile, map `Imported` onto the model and store (batch history)
  - [ ] Zen legacy `zen_workspaces`/`zen_pins` tables, Arc history, Little Arc, Helium/Chromium profiles
  - [ ] Verify Zen macOS and Arc Windows paths on real hardware

## Risks

1. **Slint ↔ wgpu texture import is an `unstable-` feature.** It can change on any Slint minor. Pin versions; M0 decides if it holds up.
2. **Own-drawn glass over live web content is the hardest rendering requirement.** Blurring a full-window texture at three radii every frame at 120 Hz must fit in 8.33 ms alongside Chromium's own GPU work. M0 measures it.
8. **Windows is CI-only.** Input, IME and windowing bugs there will be found late.
3. **Prebuilt CEF is not ungoogled and has no proprietary codecs.** Until M6, wubble talks to Google services like stock Chromium and H.264/AAC video may not play.
4. **`welding` pins CEF** (0.15 → CEF 154; newest `cef` crate is 156). Forked into `vendor/welding`, so a CEF upgrade is ours to do: re-apply `WUBBLE_CHANGES.md` on the newer welding, or bump `cef` in the fork.
5. **Extensions under windowless CEF are partial**, hence deferred to M6 — where it is a real Chromium patching project, not an integration task.
6. **OSR costs one extra composite per frame** and bypasses direct scanout for video. Measure in M0; a native-subsurface fast path for fullscreen video is the escape hatch.
9. **Turso is pre-release (0.9.0-pre).** History and passwords will sit on a pre-1.0 database; keep the file format SQLite-compatible and back up before migrations.
7. **Maintenance load**: an ungoogled+CEF patch queue rebased for every Chromium milestone is permanent work for one person.

## Open decisions

- Fallback if Turso (pre-1.0) proves unstable: the schema stays SQLite-compatible so `rusqlite` can replace it
