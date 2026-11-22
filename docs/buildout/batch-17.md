# Build-out batch 17

13 features, one branch each (`feat/<key>`, worktree `~/Dev/wubble-wt/feat/<key>/w`). Status is kept in [README.md](README.md).

## `spaces-button-menu-focus`: A Space button opened by keyboard gives the Spaces menu the keyboard focus

Area: todo-a. Needs an engine patch: no.

TODO line 137. Opening the Spaces popup from the Space button with the keyboard (Enter or Space on `kit/chrome/spaces-button.slint`) leaves the focus on the button. Chrome's menus take the keyboard on open, with the first or current item highlighted. When the popup opens by keyboard, move focus into the menu (`kit/chrome/spaces-menu.slint`), with the current Space's row focused. Up/Down move between rows, Enter opens, Esc closes and returns focus to the button. Record how it was opened (keyboard or pointer) as data in the overlay state (`wubble_core::overlay` or `spacemenu.rs`) so a pointer open does not show a focus ring. Kit rules apply.

## `popover-thumb-touch-area`: Popover scroll thumbs no longer cover the rows' right edge

Area: todo-a. Needs an engine patch: no.

TODO line 138. In popovers (downloads, extensions, menus), the scroll thumb's touch area (`crates/wubble-ui/ui/kit/controls/scroller.slint`) covers the rows' right edge, so clicks there grab the thumb instead of the row. Follow the design's scrollbar (the design repo's CSS for the popover scroll thumb: width, inset, hit area). Limit the thumb's TouchArea to the thumb's drawn rect plus the design's padding. Show the track's hit area only while the thumb is visible (hovered or scrolling), and let row clicks through elsewhere. Kit-only change, with the `// design:` header updated.

## `settings-select-fit`: Settings Select fits its longest label (no clipped value)

Area: todo-a. Needs an engine patch: no.

TODO line 139. The Settings Select (`crates/wubble-ui/ui/kit/controls/select.slint`) clips a long value at its fixed width: "Ask on first sea" for the search engine with none chosen. Follow the design's select (`pages/settings.css` / select source in the design repo). Either size the closed control to its widest option label (measure every option's text width in Slint through a hidden `Text` per option, or pass the max width from the model), capped at the row's available width, or elide with an ellipsis where the design elides. Prefer the design's behaviour. Keep it in the kit. The select's list (drawn by Slint, no blur) is a separate owner item; leave it.

## `popover-entrance-partial`: A popover's entrance dirties only its own region, so partial rendering helps

Area: todo-a. Needs an engine patch: no.

TODO line 140. During a popover's entrance, the glass's scene scale and alpha change each frame, and that marks the whole window dirty (`wubble-gfx` scene `state.rs`/`render.rs`, the partial renderer in `vendor/i-slint-renderer-skia`, the glass in `wubble_core::glass`). Make the glass layer's damage the union of its old and new bounds (scaled rect plus blur radius), not the window. Find where a change in a glass region's transform or opacity calls the full-window invalidate (probably the scene's backdrop or blur pass assuming a global change), and report the region instead. Check that the backdrop blur under the popover samples only the area it needs. After the change the owner measures (`--measure idle --seconds 4` must report 0 to 2 `slint_renders`, per AGENTS.md); do not run it yourself. No timing tests.

## `window-compositor-blur`: Transparent window with compositor blur: macOS vibrancy, KDE blur

Area: todo-a. Needs an engine patch: no.

TODO line 144. The window is frameless with a transparent background and transparent corners (decisions "Frameless window", "Window platform flags", `crates/wubble/src/host/window.rs`, `crates/wubble/src/backdrop.rs`). There is no compositor blur behind it. Add: (1) macOS, an `NSVisualEffectView` behind the content view (material `.underWindowBackground` or `.sidebar`, state active), through objc2 on the winit `NSView` from the raw window handle. The window-vibrancy crate (MIT/Apache) is a source to copy from, with a THIRD_PARTY_NOTICES entry. (2) KDE Plasma on Wayland: the `org_kde_kwin_blur_manager` protocol on the window's `wl_surface` (wubble's own Wayland worker `crates/wubble/src/wl/wayland.rs`, which already owns the display connection), with the region set to the sidebar and chrome areas. On X11 KDE, the `_KDE_NET_WM_BLUR_BEHIND_REGION` property. GNOME stays opaque. Then the shell's glass skips its own backdrop blur in the blurred regions (the scene's glass quality in `wubble-gfx/src/scene/quality.rs`). Detect support at run time, and add a setting to turn it off. One `WindowBlur` trait with per-platform implementations.

## `windows-ci-test-fixes`: Windows CI: make the four failing tests pass or gate them

Area: todo-a. Needs an engine patch: no.

TODO line 148 (run 44988591703). (1) `instance::tests::the_private_root_by_platform` (crates/wubble/src/instance.rs ~495): the test's temp dir `PathBuf::from("/tmp/me")` is not absolute on Windows, so `private_root_from` returns `/tmp`. Build the test paths so they are absolute on the host OS (for example from `std::env::temp_dir().join("me")`, with a drive prefix on Windows) and keep the per-OS expectations. (2) `updater::install::tests::linux_stages_switches_current_and_rolls_back` (crates/wubble/src/updater/install.rs) is Linux-only: gate it `#[cfg(target_os = "linux")]`. (3) `mcp::tests::a_connection_answers_discovery_and_hands_calls_to_the_host` and `a_stop_closes_open_connections_and_ids_never_repeat` (crates/wubble/src/mcp.rs): read the named-pipe transport path on Windows (interprocess local sockets under `\\.\pipe\`, `private_by_dir_impl`, socket naming from `runtime_dir`). Likely causes are a filesystem-path socket name that Windows does not accept, or a peer check (`peer_is_me`) unimplemented there. Fix the code so Windows uses a namespaced pipe name derived from the path. Do not run the tests here; CI runs them.

## `input-fling-touch`: Input: kinetic wheel fling on Linux and real touch input

Area: todo-a. Needs an engine patch: no.

TODO line 153 (first half). (1) Kinetic scroll. On Wayland and libinput touchpads, winit delivers wheel deltas with no momentum after the fingers lift. Chrome on Linux runs its own fling (`ui/events/gestures/fling_curve.cc`, BSD-3-Clause) from the scroll-stop event. Port the fling curve into a pure `wubble_core` module (state machine: Idle, Tracking(velocity), Flinging(curve, t0), with `transition` and unit tests). On winit's `TouchPhase::Ended` for pixel deltas, start the fling. Each frame, send its deltas through `View::wheel` (crates/wubble-engine/src/view.rs) until it decays or a new touch or click cancels it. (2) Touch. Forward winit `WindowEvent::Touch` (Slint's winit backend hook, `crates/wubble/src/slint_backend.rs`/`input.rs`) over a page to CEF `send_touch_event` (welding `drag::send_touch` already wraps it, `view.rs` ~737), with ids, pressure and DIP coordinates per pane. Touches over the shell stay with Slint. Turn on Chromium's touch events through the command line (`--touch-events=enabled`) where needed.

## `cursor-custom-altgr`: Cursors: CSS url() images and zoom cursors; the AltGr modifier flag

Area: todo-a. Needs an engine patch: no.

TODO line 153 (second half). (1) Cursors: `crates/wubble-engine/src/nav.rs` (`impl From<CursorShape> for Cursor`) maps ZoomIn/ZoomOut and every `Custom` shape (CSS `cursor:url(...)`) to Default, because Slint's MouseCursor has no such shapes. Add welding support for CEF's custom cursor (`on_cursor_change` with `CT_CUSTOM`'s `custom_cursor_info`: RGBA buffer, size, hotspot), as a `wubble change` in `vendor/welding/src/cursor.rs`/`surface.rs` (`CursorShape::Image{rgba,w,h,hot}`), listed in WUBBLE_CHANGES.md. In the host, set it on the winit window directly (`winit::window::CustomCursor::from_rgba`, and `CursorIcon::ZoomIn/ZoomOut` for zoom) through Slint's winit accessor while the pointer is over the page, and hand the cursor back to Slint when it leaves. Cache custom cursors by hash. (2) AltGr: `crates/wubble-engine/src/input.rs` notes `EVENTFLAG_ALTGR_DOWN` is lost. Track AltGraph from winit's logical key (`NamedKey::AltGraph`, seen in crates/wubble/src/input.rs ~97) as held state, and set the flag on key and mouse events. On Windows, Ctrl+Alt reported with text counts as AltGr (input.rs ~153).

## `ime-caret-underlines`: IME: caret rect before the first composed character, underline styles and replacement range

Area: todo-a. Needs an engine patch: no.

TODO line 154. `crates/wubble/src/ime.rs` maps winit `Ime` events to `View::ime_set_composition(text, selection)` (crates/wubble-engine/src/view.rs ~1643), which passes no underlines and no replacement range. The candidate window is placed only once CEF reports `OnImeCompositionRangeChanged`, which comes after the first composed character. (1) Caret rect before composition: when a page field gains focus (CEF `on_virtual_keyboard_requested` or the helper's focus report), get the caret rect from a page script: the focused element's selection range `getClientRects()`, or for input and textarea a mirror-div measurement. Then call winit `set_ime_cursor_area` with it. Update on selection change (`OnTextSelectionChanged`). (2) Underlines: winit's `Ime::Preedit` gives only a cursor range. Build CEF `CompositionUnderline`s: a thin underline over the whole preedit and a thick one over the selected clause (the cursor range when non-empty), as Chromium's GTK input method context does. (3) Replacement range: pass `Range{from,to}` as the invalid range (u32::MAX) unless winit reports a surrounding-text replacement. Keep the mapping pure in ime.rs with unit tests.

## `audible-signal-patch`: Engine patch: a real audible signal per tab (iframes, shadow DOM, WebAudio)

Area: todo-a. Needs an engine patch: yes.

TODO lines 122 and 162 (second half). The audible badge comes from a page probe that misses iframes, shadow DOM and WebAudio. CEF 156 has no audio-state callback: `CefAudioHandler` captures the stream and takes it from the output, so it cannot be used for a badge. Chrome uses `WebContentsObserver::OnAudioStateChanged` / `WebContents::IsCurrentlyAudible`. Write `patches/chromium/60-cef-audible.patch` (CEF side, `#if BUILDFLAG(ENABLE_CEF)`, wubble change marks, header per patches/README.md). Observe `OnAudioStateChanged` in CEF's browser host and call an exported callback registered through a new C table `cef_wubble_audible_v1` (set_callback(fn(browser_id, audible))), with its own marker symbol like patch 40. On the wubble side, look it up in `crates/wubble-engine/src/ext_api.rs` and turn it into `PageEvent::Audible(bool)`. Route it to the existing audible state (`wubble_core` tab facts, rail badge, media keys' first-sound trigger). Keep the probe as the fallback when the marker is absent. Do not build the engine (there are no Chromium rebuilds now).

## `mac-key-repeat-patch`: Engine patch: macOS key repeats reach the page with repeat true

Area: todo-a. Needs an engine patch: yes.

TODO line 135. On macOS, CEF 156 builds its NSEvent from a `CefKeyEvent` with `isARepeat:NO`, so a held key's repeats reach the page with `KeyboardEvent.repeat == false` (YouTube's Space tap versus hold-for-2x breaks). Write a one-concern patch `patches/chromium/70-cef-mac-key-repeat.patch` (CEF side, `libcef/browser/osr/render_widget_host_view_osr_mac` or the key event conversion in `browser_platform_delegate_native_mac.mm`). Carry a repeat bit, either a new `EVENTFLAG` bit in `CefKeyEvent::modifiers` that wubble sets for winit's `repeat: true` or the existing `is_system_key` path, into `isARepeat:`. Mark it `wubble change`, give it a What/Why/Applies-in header, and list it in patches/README.md. In wubble, set the bit on repeats in `crates/wubble-engine/src/input.rs` only when the patch's marker is found. When it lands, drop parity's `A.space-repeat` Mac gap (`crates/wubble/src/parity.rs`). Do not build the engine.

## `web-a11y-subtree`: Web content as a real accessibility subtree in the OS tree

Area: todo-a. Needs an engine patch: no.

TODO line 163. `wubble_ax` maps a page's CEF accessibility tree to AccessKit (`PageTree::apply`, `graft_node(tree)`), but the subtree reaches only wubble's own screen-reader pane: Slint exposes no API to graft foreign AccessKit nodes under one of its items, and its winit backend owns the AccessKit adapter. Take the backend-patch road: vendor Slint's `i-slint-backend-winit` accesskit module under `vendor/`, as `vendor/i-slint-renderer-skia` is vendored (same rules: a WUBBLE_CHANGES.md and `wubble change` marks). Add a hook so the embedder can supply extra `TreeUpdate`s whose nodes are children of a given Slint item's AccessKit node (the page view item), merged into the adapter's updates. Route AccessKit action requests on those nodes back to the embedder (`PageTree::action` → `PageAction`, executed in crates/wubble/src/host/ax.rs). Feed it from host/ax.rs on every tree update and location change, with frame offsets per pane. Keep the version pins together (slint, wgpu).

## `windows-sandbox-bootstrap`: Windows sandbox: wubble as a client DLL behind CEF's bootstrap.exe

Area: todo-a. Needs an engine patch: no.

TODO line 164. CEF 156 on Windows runs the sandbox only when the executable is CEF's `bootstrap.exe` (or `bootstrapc.exe` for console) loading the application as a client DLL that exports `RunWinMain`/`RunConsoleMain` with the sandbox info (CEF's `cef_sandbox_win` changes since M1xx; `tests/cefsimple` shows the layout). Restructure on Windows: build the `wubble` binary's entry as a `cdylib` (a new crate or a `[lib] crate-type=["cdylib"]` target, for example `crates/wubble-win-client`) exporting `RunWinMain(HINSTANCE, LPTSTR, int, void* sandbox_info, cef_version_info_t*)`, which calls the existing main with the sandbox info passed into `CefInitialize`/`CefExecuteProcess`. The helper (`crates/wubble-helper`) does the same for subprocesses. Packaging (`scripts/release/` Windows step, `crates/wubble-bundle`) renames bootstrap.exe to wubble.exe and puts `wubble.dll` beside it. Keep Linux and macOS untouched. Windows is CI-only: it must compile in CI.

