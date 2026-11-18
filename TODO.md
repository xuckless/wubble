# wubble roadmap

A Chromium-based browser with a Slint shell. Web content is rendered off-screen by
Chromium and composited by Slint as GPU textures, so the whole UI (sidebar, tabs,
popups, split views) is one scene we control.

Legend: `[ ]` todo · `[~]` in progress · `[x]` done · **(?)** blocked on an open decision.

## Decided

| Topic | Decision |
|---|---|
| Licence | GPL-3.0-only (matches Slint's GPL option; Chromium/CEF are BSD and compatible) |
| Platforms | Daily-drive targets: Linux (lead, Wayland) and macOS, both built locally. Windows is built and tested on CI runners only and may trail by a milestone |
| Engine, phase 1 | Prebuilt CEF 154 via the `cef` crate (stock Chromium, **not** ungoogled) |
| Engine, phase 2 | Our own build: ungoogled-chromium patches + CEF (M6) |
| UI | Slint 1.19, winit backend, wgpu 30 |
| Frame path | CEF accelerated OSR → native handle → `wgpu::Texture` → `slint::Image`, modelled on `welding` / `grafting` |
| Extensions | Not before our own Chromium build (M6). Built-in content blocker until then |
| DRM | Not in the shipped binary. Widevine is bolted on later (M6): detect a CDM in an existing Chrome/Brave/Firefox install, or opt-in download from Google's CDN on demand |
| Texture import | Start on `welding`; fork it when it lags or doesn't fit |
| Daily-drive features | Vertical tabs, Spaces, command bar, compact mode. Everything else in M2 comes after |
| Sync | Deferred, no design work yet |
| Import | Zen and Arc first; Helium where useful |
| UX source | Design canvas "Wubble", snapshot in `ux/pack/`. It is a **style guideline** (colour, glass, type, motion feel), not a layout spec: the designer did not know the full feature set, so layout and feature placement are ours to decide |
| Glass | Wubble draws its own glass (blur + saturate + tint + specular + rim) so it looks the same on every OS; the window itself is also transparent where the OS allows |
| Frame budget | 120 Hz, 8.33 ms, no dropped frames during animation; springs are physics-stepped per frame and retarget on interruption |
| Layout | No top toolbar. Address pill at the top of the sidebar, then pinned, tabs, new-tab row, Space switcher |
| New tab | Ctrl+T opens the command bar over the current page; a tab is created only when something is chosen. No start page |
| Passwords | Built-in manager (encrypted store, OS keyring) before extensions; extension password managers work once the extension pass (M6) lands |
| Glass under load | Glass degrades automatically (lower-res blur → cached backdrop → flat tint); animation never drops frames for it |
| Storage | wubble's own Turso database (the Rust SQLite rewrite, `turso` crate) with a declared schema and migrations: Spaces, tabs, session, history, bookmarks, passwords. Chromium keeps only cookies/cache per container |
| Containers | Each Space has a default container; a tab can be reopened in another, and Spaces may share one |
| Content blocker | Brave's `adblock-rust` (network + cosmetic, uBlock/EasyList lists), on by default, per-site toggle |
| No-blur desktops | Opaque Abyss ground with Space tint (GNOME etc.); in-window glass unaffected |
| Search | First-run choice between Google and DuckDuckGo. Suggestions are local (tabs/history/bookmarks); live engine suggestions exist behind a flag, off by default, and never send typed addresses |
| Network on its own | No telemetry. Crash reports are opt-in and open a GitHub issue; designed at ship time (M7) |
| Tab sleeping | Freeze after 1 min hidden, discard after 15 min, discard immediately under critical memory pressure. Pinned tabs freeze but never discard. Busy tabs (audio, capture) are left alone |
| CEF sandbox | Off on Linux for now: welding `CefSandboxMode::UnsandboxedTrustedContent` (`no_sandbox=1`) in both `wubble-engine` (`SANDBOX`) and `wubble-helper`. The constant is duplicated on purpose (the helper must stay tiny and cannot depend on `wubble-engine`, which pulls in wgpu); the sandbox item must change both. Turned on by the M1 item "Sandbox enabled on all platforms" |
| CEF cache directory | Always wubble's own `root_cache_path`, never CEF's shared default `~/.config/cef_user_data` (its process-singleton lock made concurrent runs fail or crash). Browser: `$XDG_DATA_HOME/wubble/cef`, else `~/.local/share/wubble/cef` (Linux, via the `Platform` trait; macOS/Windows not yet). `--smoke boot`: a fresh per-process temp directory, removed on exit, so concurrent smokes never share the lock. Second-instance handling (handing a relaunch to the running browser) is not built yet |
| CEF message pump (interim) | A repeating 8 ms `slint::Timer` calls `Engine::tick` (`do_message_loop_work`), with CEF's `external_message_pump` on. Not schedule-driven (`OnScheduleMessagePumpWork`) because welding's `CefRuntime` owns the CEF `App` and exposes no browser-process handler, and the welding producers need that runtime. The same polled pump is what welding's README and the cef-rs `osr` example use. Replaced by compositor-driven ticks in "Frame pacing" |
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
- [ ] Linux: `OnAcceleratedPaint` dmabuf → Vulkan external memory → `wgpu::Texture` on Slint's device
- [ ] Texture → `slint::Image` → `AppWindow.web-frame`
- [ ] Confirm which Slint renderer accepts wgpu textures at full speed (femtovg-wgpu vs vello vs skia-vulkan)
- [ ] **Glass spike**: backdrop blur + saturation of live web content behind a Slint panel at 120 Hz. If Slint can't do it natively, `wubble-gfx` renders the blurred backdrop as its own wgpu pass and hands Slint a second texture
- [ ] **Spring spike**: one per-frame physics-driven property (follow spring k 300 / c 15) with no missed frames while a page scrolls
- [ ] Transparent window + compositor blur per platform (macOS vibrancy, KDE blur; GNOME has no blur protocol, so fall back to opaque Abyss ground)
- [ ] Frame pacing: Chromium begin-frames driven by the compositor frame callback, not a timer
- [ ] Measure: frame time, latency input→photon, count of fallback `OnPaint` (CPU) frames — must be 0
- [ ] NVIDIA specifics: dmabuf modifiers, explicit sync
- [ ] macOS port of the spike: IOSurface → Metal wgpu texture; helper `.app` bundle layout
- [ ] Windows CI runner builds the workspace; D3D11 shared-handle port follows once Linux + macOS pass
- [ ] ADR: keep `welding` as a dependency, or vendor/fork the import code

## M1 — Engine layer (a correct windowless browser)

Each item is done when it works on Linux and macOS; Windows is verified on CI.

- [ ] View lifecycle: create, resize, HiDPI + fractional scale, visibility, close
- [ ] Mouse, wheel (smooth + kinetic), touch, touchpad gestures (pinch-zoom, swipe-back)
- [ ] Keyboard: layouts, dead keys, shortcuts routed shell-first vs page-first
- [ ] Cursor shapes and custom cursors
- [ ] IME: composition, candidate window positioning (text-input-v3 / TSF / NSTextInputClient)
- [ ] OSR popups: `<select>`, autofill, date/colour pickers — drawn as Slint layers that may leave the view rect
- [ ] JS dialogs, context menus, permission prompts, file choosers, auth prompts — all Slint-native
- [ ] `window.open` / popup windows → wubble's own policy (new tab, glance, real window)
- [ ] Multi-window; invisible/hidden windows for background work
- [ ] Clipboard, drag-and-drop (into, out of, and between views)
- [ ] Downloads; printing / save as PDF
- [ ] Fullscreen (element and window), picture-in-picture
- [ ] Find in page, zoom, text selection handles
- [ ] Audio indicators, mute, media session
- [ ] Accessibility tree bridged into Slint's AccessKit
- [ ] Crash handling: renderer/GPU process death recovers the view
- [ ] Sandbox enabled on all platforms

## M2 — Shell (styled after the UX pack)

The pack sets colour, glass, type and motion feel. Layouts on its Browser board are illustrative; exact sizes quoted below are starting points.

Foundations
- [ ] Tokens as Slint globals: ground (Abyss/Deep/Shell/Rim), ink (Ink/Mist/Haze), film (Sky/Lilac/Blush/Mint), Coral; radius 6/10/14/20/28/pill; 4 pt spacing; light theme fills
- [ ] Fonts embedded: Fredoka (display, 32 px+), Geist (interface), Geist Mono (addresses)
- [ ] Glass component, three tiers: Thin (blur 16), Regular (blur 30 + Space tint 10%), Thick (blur 44 + film rim)
- [ ] Glass quality governor: measures frame time and steps blur quality down/up; exposed as explicit levels, not ad-hoc flags
- [ ] Film rim (conic soap-film ring) on hover, press, drag and focus only
- [ ] Motion library in Rust: follow spring, stretch spring (volume-preserving), rubber band, press 0.97, squash, wobble, Overlap in/out with stagger
- [ ] App icons exported per platform from the Icons board

Daily-drive set
- [ ] Vertical sidebar on Regular glass: tab rows 36 px, pinned section, "Today" group, new-tab row
- [ ] Spaces: switcher with stretching indicator, per-Space tint, per-Space container
- [ ] Command bar on Thick glass: address, search, switch-to-tab, actions, shortcuts
- [x] Suggestion planning + live-suggestions flag (`wubble_core::suggest`), with tests
- [ ] Suggestion fetch effect (run `Source::Remote` requests, debounce, cancel stale) and the settings toggle for the flag
- [ ] Compact mode: sidebar/toolbar hide and reveal; the stretchy seam ("Room to play") for sidebar width
- [ ] URL pill (Thin glass): host in Ink, rest in Haze
- [ ] Web view inset: radius 14, 8 px gap from glass
- [ ] Turso schema + migrations; session restore, history
- [ ] First-run: search engine choice (Google / DuckDuckGo)

After daily-drive
- [ ] Component sheet: elastic slider, toggle, pin, download button, toolbar dock + gliding tooltip, menus, checkboxes
- [ ] Split view (2–4 panes); glance / peek
- [ ] Tab drag: reorder, between Spaces, into split, out to a window
- [ ] Bookmarks, settings UI, keybinding editor
- [ ] Touch sizing (44 px rows)

## M3 — Memory and sleeping

- [x] Sleep policy as a pure function (`wubble_core::sleep`) with tests
- [ ] Apply liveness: hide → freeze → discard → restore, with snapshot texture for discarded tabs
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
- [ ] Content blocker on `adblock-rust`: list fetch/refresh, network hook, cosmetic injection, per-site toggle
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
- [ ] Import from Zen and Arc (Spaces, pinned tabs, history, bookmarks); Helium/Chromium profiles

## Risks

1. **Slint ↔ wgpu texture import is an `unstable-` feature.** It can change on any Slint minor. Pin versions; M0 decides if it holds up.
2. **Own-drawn glass over live web content is the hardest rendering requirement.** Blurring a full-window texture at three radii every frame at 120 Hz must fit in 8.33 ms alongside Chromium's own GPU work. M0 measures it.
8. **Windows is CI-only.** Input, IME and windowing bugs there will be found late.
3. **Prebuilt CEF is not ungoogled and has no proprietary codecs.** Until M6, wubble talks to Google services like stock Chromium and H.264/AAC video may not play.
4. **`welding` pins CEF** (0.15 → CEF 154; newest `cef` crate is 156). We upgrade CEF only when it does, unless we fork.
5. **Extensions under windowless CEF are partial**, hence deferred to M6 — where it is a real Chromium patching project, not an integration task.
6. **OSR costs one extra composite per frame** and bypasses direct scanout for video. Measure in M0; a native-subsurface fast path for fullscreen video is the escape hatch.
9. **Turso is pre-release (0.9.0-pre).** History and passwords will sit on a pre-1.0 database; keep the file format SQLite-compatible and back up before migrations.
7. **Maintenance load**: an ungoogled+CEF patch queue rebased for every Chromium milestone is permanent work for one person.

## Open decisions

- Fallback if Turso (pre-1.0) proves unstable: the schema stays SQLite-compatible so `rusqlite` can replace it
