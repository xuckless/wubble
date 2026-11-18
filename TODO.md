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
| Engine, phase 1 | Prebuilt CEF 156 via the `cef` crate (stock Chromium, **not** ungoogled) |
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
| Glass under load | Glass degrades before animation drops a frame. One enum `scene::Quality` `Full/HalfRes/Cached/Flat`, moved by a pure `Governor::next(cost, budget)` with retry backoff (steady load cannot flip it between two levels). Each level is a knob of the design scene: `HalfRes` halves the composite and blur pyramid and the Jacobi iterations; `Cached` also moves the backdrop only every 8th frame (by the time of all eight, so the liquid keeps its speed); `Flat` freezes the backdrop and draws the glass as a flat tint (one fill colour per panel, rim, sheen ring and beam; no refraction, dispersion, press waves, specular or pointer light) |
| Glass backdrop | The owner's design scene (the-wallpaper-project `src/wubble/slint` `render.rs` + `shaders/`), ported into `wubble_gfx::scene` on the shared `GpuContext` device: a stable-fluids liquid in the Space colours is the backdrop, glass refracts a blur pyramid of that backdrop (lens toward the rim, dispersion, pointer light, inner rim, sheen ring when touched, press waves, border beam), all drawn into one opaque `Rgba8Unorm` texture shown by a back-most Slint `Image`; Slint draws only the chrome's content on top (transparent window background and panels). Glass never blurs web content: over the page the design draws effects only (`material: 0`). Supersedes the earlier dual-Kawase blur of the web frame (2026-11-18, owner to confirm). The start page passes (bubble, pop, W field, voice glow) are left out until the start page is built. The window has one rendering notifier (Slint refuses a second), so the host calls `Renderer::before_rendering` from the `wubble-ui` driver's notifier |
| Storage | wubble's own Turso database (the Rust SQLite rewrite, `turso` crate, pinned `=0.9.0-pre.5`, default features off because `mimalloc` would install a global allocator; no async runtime needed) with a declared schema and `PRAGMA user_version` migrations: Spaces, tabs, session, history, bookmarks, settings; passwords get their own encrypted store later. Chromium keeps only cookies/cache per container |
| Persistence in the app | One store thread owns the `Store` (created on it, futures driven by `pollster::block_on`), requests over a channel, replies hop to the Slint thread with `invoke_from_event_loop`; only the newest queued save runs. Save policy `persist::should_save`: after 20 session changes, or one quiet lifecycle tick (5 s) after a change, and always at orderly shutdown. A database that fails to open or load is left untouched and the run has no persistence. Profile = `--profile` or the per-user data dir (`wubble_engine::app::user_data_dir`: XDG on Linux, `~/Library/Application Support/wubble`, `%APPDATA%\wubble`); DB `wubble.db`, CEF cache `cef/` inside it (the old cache path). Smokes use a temporary profile |
| Session storage | `wubble-store` loads and saves `wubble_core::model::Model` directly (`load_session`/`save_session`, one transaction, schema v2). Restored tabs are Discarded, idle, no opener; scroll, busy, folder collapse and id counters are not stored (counters derived as max id + 1 by `Model::from_parts`, so a freed highest id can be reused after restart). `Tab.container` is the effective container, so it covers the per-tab override. Recently-closed list bounded to 25. Settings are typed through one `settings!` table |
| Containers | Each Space has a default container; a tab can be reopened in another, and Spaces may share one |
| Deleting a Space | Closes its tabs (as Zen does); the UI confirms first. The last Space cannot be deleted |
| Ctrl+T in the model | `Command::OpenCommandBar` returns only `Effect::ShowCommandBar`; the tab exists once `NewTab` is sent after a choice |
| Content blocker | Brave's `adblock-rust` (network + cosmetic, uBlock/EasyList lists), on by default, per-site toggle |
| Blocker build | `adblock` with `default-features = false` (+ `embedded-domain-resolver`, `full-regex-handling`): the default `single-thread` feature makes `Engine` not `Send`/`Sync`, and CEF calls us from several threads |
| Blocker lists | Nothing is downloaded unless the typed setting `blocker.lists_auto_update` (default off) is on; then `wubble-blocker::refresh` fetches EasyList, EasyPrivacy, uBlock filters/unbreak and Brave's `resources.json` (`DEFAULT_LISTS`, ureq, HTTPS only) into the list directory when each is due by its file's age, checked at start and every 6 h. Off: lists are read from `<profile>/blocklists/*.txt` (+ `resources.json`) if present, else the blocker allows everything. No list ships; the built-in list is test-only. Compiled engine cached in `<profile>/blocker-engine.bin`, keyed (`.key` beside it) by each list's name, length and mtime; a mismatch or unreadable cache rebuilds on a background thread while requests pass |
| Blocker request path | CEF `GetResourceRequestHandler` → `OnBeforeResourceLoad` (IO thread, every navigation and sub-resource), through the welding fork's `CefSurfaceConfig::request_filter` (its change 21, raw CEF resource type) and wubble-engine's CEF-free `filter::ContentFilter` (`check` → Allow/Block/Redirect, `kind_of` the one CEF type table); the binary's `blocking.rs` implements it on `wubble-blocker`: blocker in an `RwLock<Arc<_>>` swapped on list update, site allowlist (`blocker.exceptions`) and `blocker.enabled` consulted per request, per-view blocked counters (reset by the next main-frame request). `$removeparam` rewrites are redirects; `$redirect` resources (data: URLs) are blocked until served through a resource handler. Cosmetic: on main-frame `LoadStart` the view runs `ContentFilter::page_script` (host hide selectors as a `<style>`, then scriptlets) with `execute_java_script`, so it lands after commit, not at document start |
| No-blur desktops | Opaque Abyss ground with Space tint (GNOME etc.); in-window glass unaffected |
| Search | First-run choice between Google and DuckDuckGo. Suggestions are local (tabs/history/bookmarks); live engine suggestions exist behind a flag, off by default, and never send typed addresses |
| Network on its own | No telemetry. Crash reports are opt-in and open a GitHub issue; designed at ship time (M7) |
| Internal surfaces | Native Slint for everything wubble owns: settings, history, bookmarks, downloads, task manager, menus, dialogs, prompts. Web pages only for: DevTools, error pages (network, certificate, crashed tab), PDF viewer, view-source, Chromium's diagnostic pages, extension pages |
| CEF version | CEF 156.0.30 (`cef` crate 156.6, Chromium 156.0.8078.96), moved in the welding fork itself (its change 7): upstream welding has no 156 release. 156.0.30 is the first build that delivers real frames on NVIDIA (RTX 5060, driver 615, Wayland) at default settings, as a tiled block-linear dmabuf (CEF #4237), and sends each frame only after its GPU work completes (CEF #4313: NVIDIA dmabufs carry no implicit fence, and the paint info has no fence field to wait on). 154 handed over empty LINEAR buffers there. `--smoke frames=120` passes on the accelerated path (GPU copies only, `cpu_fallback=0`); the owner-approved interim CPU fallback was not built. CEF returns the buffer to its pool when the callback returns, so welding (its change 8) imports each frame and GPU-copies it into an owned texture inside `OnAcceleratedPaint`, waiting on the copy, as the cef-rs OSR example does since cef-rs #498; wubble-gfx then blits that copy at present time. Linux only; macOS still imports at present time |
| Tab sleeping | Freeze after 1 min hidden, discard after 15 min, discard immediately under critical memory pressure. Pinned tabs freeze but never discard. Busy tabs (audio, capture) are left alone |
| Shell animation | The design repo's Slint port is the single implementation: pure physics in `wubble_core::{motion, seam, tabs, scroller}` (f64, copied 1:1, numeric parity with the TS checked against a stored bun run), and `wubble_ui::driver` (the port's frame body) pushes the values into Slint `in` properties. The per-frame hook is wubble's one vsynced rendering notifier, not the port's 16 ms `slint::Timer`; it requests a redraw only while something moves. Tweens (0.97 press, hover fades, film rim, overlap) stay Slint `animate` with the token easings. The sidebar has the port's three modes, extended (200–340, resizable), rail (56) and hidden, on one width spring owned by `Seam` (rubber band, snap to the rail under 128, hidden under 22 from the rail); the toggle is hidden ↔ extended, anything else hides (`chrome.tsx` `cycleSidebar`); double-clicking the seam flips rail/extended. Peek is presentation-only: one Slint bool set by two Slint Timers (60 ms in, 260 ms out). The driver never edits the Slint models: it re-reads the ids on `shell-model-changed`, a click is `tab-activated`, a drag `tab-moved(id, to)` (the host does not handle `tab-moved` yet, so a dropped row springs back to the model order). Selection shows by fill and the touch sheen; no focus borders |
| CEF sandbox | Off on Linux for now: welding `CefSandboxMode::UnsandboxedTrustedContent` (`no_sandbox=1`) in both `wubble-engine` (`SANDBOX`) and `wubble-helper`. The constant is duplicated on purpose (the helper must stay tiny and cannot depend on `wubble-engine`, which pulls in wgpu); the sandbox item must change both. Turned on by the M1 item "Sandbox enabled on all platforms" |
| CEF cache directory | Always wubble's own `root_cache_path`, never CEF's shared default `~/.config/cef_user_data` (its process-singleton lock made concurrent runs fail or crash). Browser: `$XDG_DATA_HOME/wubble/cef`, else `~/.local/share/wubble/cef` (Linux, via the `Platform` trait; macOS/Windows not yet). `--smoke boot`: a fresh per-process temp directory, removed on exit, so concurrent smokes never share the lock. Second-instance handling (handing a relaunch to the running browser) is not built yet |
| CEF message pump | Schedule-driven, with CEF's `external_message_pump` on: `OnScheduleMessagePumpWork` (welding fork's `PumpScheduler`, any thread) → `slint::invoke_from_event_loop` → `Engine::schedule_pump`, which runs `wubble_core::pump` (a port of cefclient's `MainMessageLoopExternalPump`: delay replaces the pending timer, at most 33 ms between runs, re-entrant requests re-run) and asks the host for one single-shot `slint::Timer`. The host picks up frames after each run of work. `Engine::tick` (a synchronous run) remains only for `--smoke boot` and the shutdown drain, which have no event loop. Windows never calls the hook (CEF's own loop thread) |
| Shared GPU device | `wubble-gfx::GpuContext` creates the one wgpu device (Vulkan, high-performance non-CPU adapter, `welding::build_dmabuf_capable_device` with Slint's own device descriptor) and hands it to Slint with `BackendSelector::require_wgpu_30(WGPUConfiguration::Manual)`, renderer `femtovg-wgpu`. Not the rendering notifier: Slint's own device lacks the dmabuf extensions. One handle is leaked at exit because Slint drops its copy during thread-local teardown, after wgpu-core's thread-locals (debug builds abort otherwise) |
| Frame copy | welding imports CEF's dmabuf as `Bgra8UnormSrgb` with `TEXTURE_BINDING \| COPY_SRC`; Slint accepts only `Rgba8Unorm(Srgb)` with `TEXTURE_BINDING \| RENDER_ATTACHMENT`. So each frame gets one GPU blit (ported wgpu fullscreen-triangle shader) into a ring of 2 `Rgba8UnormSrgb` targets, each wrapped once as a `slint::Image`; alternating keeps `web-frame` changing. GPU to GPU, no CPU readback; one extra full-frame copy to account for in the M0 measurement. The welding fork cannot remove it: CEF's buffer is BGRA (importing it as RGBA swaps red and blue, and Slint takes no swizzled view), and render-attachment usage on the imported DRM-modifier image needs a grafting fork and is driver/modifier dependent. Revisit if Slint accepts `Bgra8` or a swizzled view |
| Chromium GPU on Linux | No extra Chromium switch (`Platform::CHROMIUM_SWITCHES` is an empty table; any entry added gets a comment naming the bug it works around). CEF 156 defaults give real frames on NVIDIA 615 / Wayland. `enable-features=Vulkan`, which 154 needed for any accelerated paint there, makes every 156 frame empty, so it is gone. Chromium still logs that Wayland ozone is not compatible with Vulkan; harmless. Not yet run on Mesa (Intel/AMD) |
| Pixel check | `--smoke frames=N` on the bundled page snapshots the window after the last frame (`Window::take_snapshot`, which re-renders through femtovg-wgpu, imported textures included) and fails unless a 16x16 grid over the content rectangle has at least 8 distinct colours and mean luma at least 40. A correctness check, not timing; skipped for `--url` pages, whose colours are unknown |
| CEF frame rate (interim) | The browser is created with `windowless_frame_rate` = the display refresh (current monitor, else the fastest connected one, since Wayland reports the monitor late), capped at 240, through the welding fork's `CefSurfaceConfig::windowless_frame_rate`. The view opens once winit has created the window. The fork also exposes `CefSurfaceConfig::external_begin_frame`, still off: turning it on (and calling `SendExternalBeginFrame` through `cef::browser_host_get_browser_by_identifier`) is the "Frame pacing" item |
| CPU fallback count | Measured on Linux: the welding fork's `on_paint` counts every CPU frame (never read or shown) and `View::cpu_fallback_frames` reports it; the host logs new ones and `--smoke frames` prints the real `cpu_fallback`. Other platforms print `n/a`. The smoke still passes on `accelerated > 0`; whether `cpu_fallback > 0` should fail it is open |
| View close | Event-driven: the welding fork pushes `CefSurfaceEvent::BrowserClosed` from `OnBeforeClose`; the engine drains every view's event queue after each run of CEF work and reports `BrowserClosed` to the lifecycle. A page that closes itself is reported the same way |
| Web events | One enum, `wubble_engine::nav::WebEvent` (URL, title, loading + can-go-back/forward, favicon URL, cursor, popup request, renderer gone), collected from every view after each run of CEF work into `PumpOutcome::events`. Navigation per view is the pure `nav::transition` over `NavState` (a `Page` plus a `Phase`: Blank/Loading/Loaded/Crashed): events only on change, Back/Forward only when possible, commands after a crash also repaint. Loading state and favicon URLs come from two welding fork callbacks (`OnLoadingStateChange`, `OnFaviconURLChange`), which welding lacked. Popups: welding's `OnBeforePopup` already cancels CEF's own window; the host opens `PopupRequested` as a new tab next to its opener. A dead renderer is reloaded once by the transition itself; a second crash before that reload has finished loading leaves the view crashed. Everything else a page asks or tells (popup widgets, JS dialogs, context menus, downloads, permissions, file choosers, auth, certificate errors, find, zoom, fullscreen, mute, console, load errors, IME bounds, drags) is a second enum, `wubble_engine::page::PageEvent`, taken with `Engine::take_page_events` (bounded at 1024: only console output and intermediate progress, find, IME and popup-move notices are dropped, oldest first; requests and events that start or end something never are; progress/find/IME/popup-move bursts coalesce). It is separate only because the host matches `WebEvent`, builds `ViewConfig` and destructures `PumpOutcome` exhaustively; folding it into `WebEvent` is one line in the host batch |
| Engine requests | Every question a page asks the shell carries a `page::RequestId` and is answered through `View::answer_*` (dialog, context menu, permission, file chooser, auth, certificate, download path; false for a stale id). The CEF callback is held once, in the welding fork's `Held` table (cap 64 per kind and view; ids from one process-wide counter, so unique across kinds and views); the engine keeps no token table. Requests CEF abandons (permission prompt dismissed; auth, certificate, file chooser and permission requests of a page the main frame left) are answered with their default and reported as `PageEvent::RequestsDismissed`. Permissions are granted all or nothing per request (CEF requires getUserMedia answers to match the request). Defaults, given when the view closes (`OnBeforeClose` drains, `Held` drops answer too), when the cap is hit, or at once under `RequestMode::DefaultNow`: JS dialog cancel (beforeunload: leave), menu dismiss, permission deny, file chooser cancel, auth cancel, certificate cancel, download cancel. Nothing is auto-granted. `Engine::set_request_mode` defaults to `DefaultNow` (answer the default at once, report nothing) until the host draws prompts; the host batch switches to `AskHost`. State machines only where states gate commands: popup widget, download (Requested, Running, Paused, Interrupted: an interrupted download is `DownloadFailed` but resumable), find, fullscreen, zoom (percent = round(100 x 1.2^level), 25..500), drag source |
| Web input | Keyboard from raw winit events (`slint::winit_030` `on_winit_window_event`), because Slint's `KeyEvent` keeps only text: no physical key, scancode or location, nothing for keys without text, Ctrl/Cmd swapped on Apple. Mouse, wheel and hover from a Slint `TouchArea` over the web frame (hit-testing against the shell, view-relative logical px = CEF DIP, grab and leave for free); click count from a ported cefclient tracker; sub-pixel wheel motion is carried per view, not rounded away; a release of one button releases every held button, because Slint ends the `TouchArea` grab on any release. Keys go to the page only while its `FocusScope` has focus; modifier keys also reach Slint; `PRIMARY`+T/L/W stay with the shell. The engine sends input straight to CEF's `BrowserHost` (welding's input path lacks click count, leave, wheel flags, system keys and a key table). Key codes: cefclient GTK's policy (layout character first, Chromium's US-position table for the rest, digit row and keypad positional). Lost: Caps/Num Lock state, AltGr flag, layout remaps of text-less keys, IME |
| welding fork | `welding` 0.15.0 (MPL-2.0) vendored in `vendor/welding`, excluded from the workspace and from `cargo fmt` by its own `rustfmt.toml` (upstream style, `cargo test -p welding`). Changes, each marked `wubble change` and listed in `vendor/welding/WUBBLE_CHANGES.md`: browser-process `PumpScheduler`, `BrowserClosed` event, Linux `on_paint` count, `windowless_frame_rate` / `external_begin_frame` config, `LoadingStateChanged` / `FaviconUrlsChanged` events, `cef` 156.6 instead of 154.5, Linux frame copy inside `OnAcceleratedPaint`, one `Held` table for every held request, the platform-independent handlers in one `handlers.rs` (request, download, permission, context menu, new JS dialog, file dialog and audio handlers), download control through the latest item callback, a kept and held context menu, certificate-error and fullscreen hooks, drag source off by default, `acquire_native_popup`, drain on close. `grafting` is not forked. When moving CEF or welding, start from the new upstream release and re-apply the change list |
| Tabs in the host | The binary owns one `wubble_core::model::Model`; every UI callback and engine event becomes a `Command` for `model::apply`, and each returned effect runs through one pure table (`host::call`) into one `execute`. `apply` also settles tab lifecycles: whatever the command, the visible tab (active tab of the active Space) is Active and every other tab hidden, with the `lifecycle::transition` effects appended as `Effect::Lifecycle`; a private model clock, passed to `Model::new(now)` / `Model::from_parts(.., now)` and set by `Command::Tick`, stamps hides (core reads no clock). The model also keeps the last tick's pressure: when a hidden tab stops being busy or is unpinned, it is judged at once against the last tick's time and pressure instead of waiting for the next tick (a tab still loading at the tabs smoke's one tick stayed alive forever before). One engine view per live (not discarded) tab; only the visible tab's view takes frames, layout and input, the rest are told `was_hidden(true)` (and repaint on show). Discard and ReleaseTexture close the view and keep the tab; Restore reopens a view at the model's URL (a live view after a crash reloads itself). **Freeze has no CEF mapping**: CEF 156 has no freeze call (no `freez`/`frozen` in the cef 156.6.0 bindings), so `Freeze`/`Unfreeze` are no-ops and `was_hidden` (Chromium throttles timers and stops compositing) is the only lever; a real freeze would be CDP `Page.setWebLifecycleState` through `BrowserHost::execute_dev_tools_method` (unverified on Chromium 156, follow-up). Snapshots are no-ops until there is UI for them. The lifecycle tick is a 5 s `slint::Timer` with `Pressure::None` (no pressure source yet); smokes never start it. The sidebar is re-derived after every change by `shell::view_model` and replaced wholesale. Popups open a new tab in the opener's Space right after it, shown only when the opener is the visible tab (a hidden tab's popup opens hidden). `LoadingChanged` marks the tab busy while it loads, so a loading tab is never frozen or discarded (busy has no other source yet). Switching the visible tab blanks the web frame until the new tab paints, so another tab's frame never shows while input goes to the new one. A page that closes itself closes its tab |
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
- [~] **Glass spike**: superseded by the design scene (see "Glass backdrop"): `wubble_gfx::scene` renders the fluid backdrop and the glass as its own wgpu pass and hands Slint one texture; it runs on the shared device and headless (`cargo run -p wubble-gfx --example headless -- out.ppm`). Cost at 120 Hz alongside a page not yet measured by hand
- [ ] **Spring spike**: one per-frame physics-driven property (follow spring k 300 / c 15) with no missed frames while a page scrolls
- [x] Frameless window: lights, drag to move, resize zones, rounded corners where the platform has transparency (not run on a display yet; see Decided)
- [ ] Transparent window + compositor blur per platform (macOS vibrancy, KDE blur; GNOME has no blur protocol, so fall back to opaque Abyss ground)
- [ ] Frame pacing: Chromium begin-frames driven by the compositor frame callback, not a timer. The welding fork exposes `external_begin_frame` (off) and the host is reachable; wiring it is this item. Interim: CEF frame rate = display refresh, pump scheduled by CEF
- [ ] Resize without waiting for CEF: with the 8 ms poll gone the host applies a new content size on the next run of CEF work (at most 33 ms later); a Slint `changed` hook on the content size could schedule a run at once
- [ ] Measure: frame time, latency input→photon, count of fallback `OnPaint` (CPU) frames — must be 0 (counted on Linux through the welding fork; `--smoke frames` prints it)
- [x] NVIDIA specifics: dmabuf modifiers, explicit sync. On CEF 156.0.30, RTX 5060 / 615.71.09 / Wayland imports block-linear dmabufs through welding/grafting and `--smoke frames=120` passes its pixel check; no explicit sync needed, CEF sends each frame after its GPU work completes (see "CEF version"). Each frame is copied inside the paint callback, so a pooled buffer is never read after CEF takes it back
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
- [~] IME: composition, candidate window positioning (text-input-v3 / TSF / NSTextInputClient): engine side done (`View::ime_set_composition` / `ime_commit` / `ime_finish` / `ime_cancel`, `PageEvent::ImeBounds`); host wiring open. Gap: CEF reports character bounds only during a composition, so there is no caret rect before the first composed character, and `ime_set_composition` carries no underline styles or replacement range
- [~] OSR popups: `<select>`, autofill, date/colour pickers — drawn as Slint layers that may leave the view rect: engine side done (`PageEvent::PopupShown/Moved/Hidden`, `View::take_popup_frame` as a second frame stream; `<select>` checked by `engine_smoke`); Slint layer open. Gap: Windows has no native popup frame (welding imports it inside the paint callback); macOS popup frame not yet built or run
- [~] JS dialogs, context menus, permission prompts, file choosers, auth prompts — all Slint-native: engine side done (requests with ids and defaults, see "Engine requests"; alert/confirm/prompt, geolocation, download path and context menu checked by `cargo run -p wubble-engine --example engine_smoke`, plus `-- --default-now`); Slint UI and `RequestMode::AskHost` in the host open. Certificate errors are requests too (`PageEvent::CertificateError`, default cancel); load errors are `PageEvent::LoadFailed { url, code }` for a later error page; console messages are `PageEvent::Console`
- [ ] `window.open` / popup windows → wubble's own policy (new tab, glance, real window). Reported as `WebEvent::PopupRequested` (CEF opens nothing); interim: a new tab next to its opener; the policy is still open
- [ ] Multi-window; invisible/hidden windows for background work
- [~] Clipboard, drag-and-drop (into, out of, and between views): engine side wired where CEF's OSR API allows: edit commands on the focused frame (`View::edit`: undo, redo, cut, copy, paste, paste-and-match-style, delete, select all), drags into a view (`View::drag` Enter/Over/Leave/Drop with files, link, text, HTML) and out of it behind `View::set_drag_source(true)` (`PageEvent::DragStarted`, `View::drag_source_ended`, gated by a `DragSource` state). Missing: (a) CEF has no clipboard API; page copy/paste go through Chromium's own `ui::Clipboard`, and whether that reaches the Wayland clipboard from a windowless process is unverified; (b) Slint 1.19 has no OS drag source or drop target (only winit's `DroppedFile` / `HoveredFile`, paths only), so a host cannot yet feed drags in from other apps beyond files, nor run a drag out; page drags therefore stay refused (`StartDragging` returns false) until a host can end them
- [~] Downloads; printing / save as PDF: downloads engine side done (`PageEvent::DownloadRequested` -> `View::answer_download_path`, then Started / Progress / Paused / Resumed / Finished / Failed / Cancelled, `pause/resume/cancel_download` acting at once; one `download::Download` state machine); download shelf UI, a default download directory and printing open
- [~] Fullscreen (element and window), picture-in-picture: page fullscreen engine side done (`PageEvent::FullscreenChanged`, `View::exit_fullscreen`); the host must grow the view while fullscreen and map Esc to `exit_fullscreen` (Alloy style, windowless). Unverified end to end; window fullscreen and PiP open
- [~] Find in page, zoom, text selection handles: engine side done (`View::find` / `find_next` / `find_previous` / `stop_find` with `PageEvent::FindResult`; `zoom_in` / `zoom_out` / `zoom_reset` / `set_zoom_percent` with `PageEvent::ZoomChanged`, re-read after loads and address changes); find bar, zoom UI and selection handles open
- [~] Audio indicators, mute, media session: mute done (`View::set_muted` with `MutedChanged`). No audible signal: CEF 156 has no audible-state callback, and its `AudioHandler` capture silences the page while it runs (checked on Linux: with the handler the page's PulseAudio stream is corked, without it plays), so it was removed. Options: a CEF patch exposing `WebContentsObserver::OnAudioStateChanged`, or a JS probe on media elements and AudioContexts. Indicator UI and media session open
- [ ] Accessibility tree bridged into Slint's AccessKit
- [~] Crash handling: renderer/GPU process death recovers the view. Renderer death: `WebEvent::RendererGone`, the host reloads once per URL with a repaint; GPU process death open
- [ ] Sandbox enabled on all platforms

## M2 — Shell (styled after the UX pack)

The pack sets colour, glass, type and motion feel. Layouts on its Browser board are illustrative; exact sizes quoted below are starting points.

Foundations
- [x] Tokens as Slint globals: ground (Abyss/Deep/Shell/Rim), ink (Ink/Mist/Haze), film (Sky/Lilac/Blush/Mint), Coral; radius 6/10/14/20/28/pill; 4 pt spacing; light theme fills
- [x] Fonts embedded: Fredoka (display, 32 px+), Geist (interface), Geist Mono (addresses)
- [x] Glass component, three tiers: Thin, Regular, Thick (`wubble-ui` `Glass`; the material is the design scene's glass, `wubble_gfx::scene`, tier as `uTier`)
- [x] Living backdrop and real glass under the UI: the design's wgpu scene (fluid, blur pyramid, glass, distortion waves, border beam) in `wubble_gfx::scene`, with `set_glass` / `set_palette` / pointer / `tap` / `set_beam` + `lap_beam` and `Renderer::before_rendering`; naga-validated shaders and a headless GPU test
- [ ] Wire the scene into the host: scene `Image` at the back of `app.slint`, transparent window and glass panels, glass rects from the ui lane's layout, pointer and taps, Space palette, the governor fed with frame costs, pause while hidden
- [x] Glass quality governor: measures frame time and steps blur quality down/up; exposed as explicit levels, not ad-hoc flags (`scene::Quality` + pure `Governor::next` with retry backoff, mapped onto the scene's resolution, iteration count and refresh; the host supplies the measured costs, next batch)
- [ ] Film rim (conic soap-film ring) on hover, press, drag and focus only
- [x] Motion library in Rust (springs, rubber band, stretch, easing constants in wubble-core::motion; press/squash/wobble/Overlap compose these): follow spring, stretch spring (volume-preserving), rubber band, press 0.97, squash, wobble, Overlap in/out with stagger
- [ ] App icons exported per platform from the Icons board

Daily-drive set
- [ ] Vertical sidebar on Regular glass: tab rows 36 px, pinned section, "Today" group, new-tab row (rows, favourites, new-tab row, scrolling list and drag-reorder in the UI done; no "Today" group; glass waits for the wgpu renderer)
- [x] Spaces, containers and tab-tree model with one pure `apply(model, command)` (`wubble_core::model`) with tests
- [x] Tabs for real: one `Model` in the binary drives the engine (a view per live tab, only the visible one shown) and the sidebar (`view_model`); Ctrl+T/L/W, tab click/close, Space switch, popups as tabs; `--smoke tabs` (see "Tabs in the host")
- [ ] Spaces: switcher with stretching indicator, per-Space tint, per-Space container (switcher with the drawn icons and the stretching capsule done; the model has no icon per Space yet, so the three drawn icons repeat)
- [ ] Command bar on Thick glass: address, search, switch-to-tab, actions, shortcuts (UI component done on flat Glass; results wired through `omnibox::rank` over open tabs, store history and the action palette, with the keymap's shortcuts; glass and bookmarks remain)
- [x] Suggestion planning + live-suggestions flag (`wubble_core::suggest`), with tests
- [x] Command bar brain (`wubble_core::omnibox`): `classify` (Chromium Parse rules), one search-engine table, `rank` over tabs/history/bookmarks/actions; wiring to Slint results remains
- [x] Keybindings and actions as data (`wubble_core::action`, `keys`): one `Action` enum shared with the command bar, `DEFAULTS` chord table (Zen first, then Firefox/Chromium) with per-platform rows, chord parser/formatter, user overrides + conflict detection; the editor UI remains
- [x] Key router and actions in the app: winit keys become chords (`input::chord_of`), `Keymap::action_for` plus the route decide where they go (`input::key_route`), every action runs through `Host::perform` (`perform::plan`); not yet possible, listed in `perform::NOT_YET`: new window, new Space, compact-mode toggle (ToggleSidebar is the shell's toggle), find, zoom, DevTools, view source, fullscreen, copy URL
- [x] Exact page-first key routing: welding's keyboard handler reports keys the page left alone (CEF `OnKeyEvent`, `WebEvent::KeyUnhandled`); a page-first chord with the page focused goes to the page only and its action runs on that report (`input::PendingKeys`)
- [ ] Suggestion fetch effect (run `Source::Remote` requests, debounce, cancel stale) and the settings toggle for the flag
- [x] Compact mode: the design repo's three sidebar modes (extended, rail, hidden with peek) on one width spring; the seam is the rubber string, bowing while held and twanging on release (`wubble_core::seam`)
- [x] URL pill (Thin glass): host in Ink, rest in Haze (the port's Slint-only `GlassPanel` until the wgpu glass lands)
- [x] Web view inset: radius 14, 8 px gap from glass
- [x] Turso schema + migrations (`wubble-store`): session, history, bookmarks table, settings
- [x] Session restore and history recording wired into the app: the session loads at start (only the visible tab gets a view; the rest restore on activation), saves by `persist::should_save` and at shutdown; visits (typed when started from the bar) and titles are recorded for http(s) pages; closed tabs feed reopen-closed-tab; sidebar width and compact mode persist (`--smoke session`)
- [x] First-run: search engine choice (Google / DuckDuckGo), interim: the command bar offers both as its first rows until one is stored; a proper first-run screen comes later
- [x] Store: bookmarks tree API and command-bar search, import apply, `logins` table with byte-row CRUD for the vault, history upkeep (expire, clear range, forget URL/site, retention setting), `check_consistency` (schema v3)

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
  - [x] List fetch behind an off-by-default flag (`blocker.lists_auto_update`), lists from `<profile>/blocklists`, cached compiled engine
  - [x] CEF network hook (welding change 21, `wubble_engine::filter`, `wubble/src/blocking.rs`), per-site allowlist consulted, per-view blocked counters as data; engine-level `block_smoke` example (run by hand)
  - [x] Cosmetic injection of host hide selectors and scriptlets on main-frame commit
  - [ ] Per-site toggle UI and blocked-count badge (the shell reads `Blocking::blocked`); app-level `--smoke block`
  - [ ] Generic hide selectors (class/id collection in the page → `Blocker::generic_selectors`), procedural cosmetic filters, document-start injection
  - [ ] `$redirect` resources served through a CEF resource handler instead of blocked
- [~] Built-in password manager: encrypted store keyed from the OS keyring, form detection and fill, save prompt, generator, import from Zen/Arc
  - [x] `wubble-vault` crate: keyring-keyed AES-GCM store, origin matching, generator, Firefox/Chromium CSV parser
  - [ ] Form detection and fill, save prompt, store table in wubble-store, Zen/Arc import
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
  - [x] Store side (`wubble-store`): `Store::apply_import` merges Spaces/containers/tabs (appended as new Spaces, idempotent through `import_marks`), bookmarks (under `Imported from <source>`) and history (original times, deduped on url and time) in one transaction
  - [ ] App wiring: pick a profile, call `apply_import`, replace the live model with the returned one, show the summary
  - [ ] Zen legacy `zen_workspaces`/`zen_pins` tables, Arc history, Little Arc, Helium/Chromium profiles
  - [ ] Verify Zen macOS and Arc Windows paths on real hardware

## Risks

1. **Slint ↔ wgpu texture import is an `unstable-` feature.** It can change on any Slint minor. Pin versions; M0 decides if it holds up.
2. **Own-drawn glass and the living backdrop must fit the frame.** The scene runs about 40 small render passes per frame (fluid step, composite, blur pyramid, glass) at 120 Hz alongside Chromium's own GPU work, within 8.33 ms. Measured by hand; the quality ladder is the fallback.
8. **Windows is CI-only.** Input, IME and windowing bugs there will be found late.
3. **Prebuilt CEF is not ungoogled and has no proprietary codecs.** Until M6, wubble talks to Google services like stock Chromium and H.264/AAC video may not play.
4. **`welding` pins CEF** (upstream 0.15 → CEF 154). Forked into `vendor/welding`, which already bumps `cef` to 156.6 itself; each CEF upgrade is ours to do: re-apply `WUBBLE_CHANGES.md` on a newer welding, or bump `cef` in the fork.
5. **Extensions under windowless CEF are partial**, hence deferred to M6 — where it is a real Chromium patching project, not an integration task.
6. **OSR costs one extra composite per frame** and bypasses direct scanout for video. Measure in M0; a native-subsurface fast path for fullscreen video is the escape hatch.
9. **Turso is pre-release (0.9.0-pre).** History and passwords will sit on a pre-1.0 database; keep the file format SQLite-compatible and back up before migrations.
7. **Maintenance load**: an ungoogled+CEF patch queue rebased for every Chromium milestone is permanent work for one person.

## Open decisions

- Fallback if Turso (pre-1.0) proves unstable: the schema stays SQLite-compatible so `rusqlite` can replace it
