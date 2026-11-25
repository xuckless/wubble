# Feel pass

Shelved by the owner (2026-11-23): "shell looks good as it is right now; if it doesn't regress it would be nice".

Legend: `[ ]` open. Picked up only when the owner says so; how items move: [todo/README.md](../README.md).

The thinking orb and its gravity warp are in [agent-widgets.md](agent-widgets.md).

## From the old roadmap
- [ ] Stretch and squash on every pressable and moving surface (menus, popovers, sheets, toasts, command bar, strip)
- [ ] Liquid glass under every floating surface, with the flat fallback kept
- [ ] The glowing ring and border beam wherever something is focused, loading or newly opened
- [ ] One audit: walk every gallery surface against the three effects
- [ ] Voice glow shader into the kit (no microphone)
- [ ] Film rim on the remaining controls (buttons, menu rows) (M2)
- [ ] Touch-kit pieces not yet ported: pin, download button, dock with gliding tooltip, the check box's mask grow (M2)
- [ ] Transparent window with compositor blur per platform (macOS vibrancy, KDE blur; GNOME stays opaque) (M0)

## Build-out features (2026-11-22)

## Feel pass: stretch and squash on every pressable and moving surface (`feel-stretch-squash`)

Key: `feel-stretch-squash` · engine patch: no · state: not written.

The design's touch kit gives pressables and moving surfaces a spring stretch and squash (`stretchOf`, `squashAt` and `spr` in pages/touch-kit.tsx:43-80, the `sq-a`/`sq-b` press classes in touch.tsx, and the `.wb-press` rules in core/wubble.css). wubble has this on some controls (`wubble_core::stretch`, `wubble_core::controls`, kit/motion). TODO "Feel pass" asks for it on every one: menus, popovers, sheets, toasts, the command bar and the tab strip.

To build: (1) Write down the design's exact squash and stretch curves as pure functions in wubble-core (extending stretch.rs or controls.rs), with their values from kit_gen, and unit tests. (2) Make one kit motion piece (for example `kit/motion/squash.slint`, headed with the design lines) that any surface wraps: a press squash, an entry overshoot, and a stretch along the direction of motion driven by velocity. Drive it from the one vsynced rendering notifier, redrawing only while something moves (decisions "Shell animation"). (3) Apply it to every kit overlay and chrome surface that the list names: kit/overlay/a/* and b/* popovers, the menu, the sheet, toasts, the command bar (chrome/command-bar.slint) and tab rows while dragged or reordered. Reduced motion (`ui.reduce_motion`) turns it off. (4) Add gallery shots at mid-motion for each. Never drop a frame at 120 Hz: the physics is O(1) per surface. All platforms.

## Feel pass: liquid glass under every floating surface (`feel-liquid-glass-floating`)

Key: `feel-liquid-glass-floating` · engine patch: no · state: not written.

TODO "Feel pass" asks for liquid glass (the scene's blurred and refracted backdrop) under every floating surface, with the flat fallback kept. In wubble, glass is data: every kit `GlassPanel` reports a `GlassItem` to one registry, which the scene draws (decisions "Glass in the shell"), and over-page glass lives in the scene's over layers (`OverLayer::Card/Peek/Top`, crates/wubble-gfx/src/scene/pages.rs). Some floating surfaces still draw a Slint-only fill rather than registering glass: for example tooltips, the status bubble, toasts, the drag ghost, the extension popup card, the autofill chip, hover cards and the swipe chip.

To build: walk every kit overlay and chrome surface (crates/wubble-ui/ui/kit/overlay/**, kit/chrome/drag-ghost.slint, kit/controls/tooltip.slint, the status bubble and hover card components) and list each one that floats over the page or the sidebar without a `GlassPanel`. For each, compose the kit `material/glass.slint` panel so it reports a `GlassItem` with the right layer (Card for in-card overlays, Top for window-wide ones). Check that the flat fallback path (`Glass::SlintOnly`, `WUBBLE_GLASS=slint`, `Quality::Flat`) still draws them correctly. Mind the cost: the governor (`scene::quality::Governor`) must hold 8.33 ms, so a surface that animates at 120 Hz marks only its own rect. Add gallery shots of every surface with scene glass and with the fallback. All platforms.

## Feel pass: glowing ring and border beam on focus, loading and new (`feel-ring-and-beam`)

Key: `feel-ring-and-beam` · engine patch: no · state: not written.

The design laps a glowing border beam around the address pill on tab change and page load (ported: `wubble_core::beam`, scene shader beam.wgsl, decisions "Border beam") and draws a glowing ring on focus. TODO "Feel pass" asks for the ring and beam wherever something is focused, loading or newly opened: a loading tab row, a newly opened tab or Space, a focused field or sheet, the command bar on open, a download that just finished.

To build: (1) Generalise `wubble_core::beam` from the single address beam to a small table of beams keyed by surface (a `BeamTarget` enum: Address, TabRow(id), Field, CommandBar, Sheet, DownloadButton). Each has its own lap state and a pure transition from events (Focused, LoadStarted/Finished, Opened), with unit tests. (2) Let the scene draw several beam shapes. `set_beam_shape` in crates/wubble/src/backdrop.rs currently takes one, so it becomes a short list in wubble-gfx scene state and render. For Slint-only glass, use the Slint beam (components/border-beam.slint, to be moved into the kit; see kit-lock-baseline-empty). (3) Add a kit focus-ring piece that follows the design's ring (components/focus-ring.slint into the kit) and use it on every focusable kit control. (4) Beams and rings wake the scene only while lapping (`run.wake_if_still`), and reduced motion turns them off. Add gallery shots. All platforms.

## Film rim on the remaining glass controls (`film-rim-remaining`)

Key: `film-rim-remaining` · engine patch: no · state: not written.

The design draws a 1.5 px soap-film ring (`.wb-film`, core/wubble.css:33-40 in the design clone, angle `--wb-film-a`) on every glass control while it is touched, hovered, pressed, focused or open. The kit has it on button.slint (glass kind), the menu and select trigger (menu.slint:595), the field, search capsule, toggle, favs, address pill and some overlays (`kit/motion/film.slint`). The design also puts `wb-film` on the settings confirm button (`settings.tsx:366` `.st-confirm`), the start page's site-tile icons (`start.tsx:48`), the settings search field (`settings.tsx:558`), the found-glass tiers and the touch page's buttons. Some of these kit parts lack it: confirm-button.slint, icon-button.slint, the start page's tiles (kit/pages/start.slint) and the menu rows the TODO names.

To build: `git pull` the design clone. Grep the web source for `wb-film` and list each use against the kit component that translates it. For each one missing, add the `film` binding through the existing `kit/motion/film.slint` piece, lit on the same states the CSS uses (`:hover`, `:active`, `:focus-visible`, `.wb-on`). Update each file's `// design:` header with the lines it now translates. Menu rows get it only if the web source has it; if it does not, say so in the report rather than inventing one. Add a gallery shot of each touched surface, and keep the kit lock baseline from growing (crates/wubble-ui/tests/kit_lock.rs). All platforms.

## Voice glow shader in the kit (no microphone) (`voice-glow-kit`)

Key: `voice-glow-kit` · engine patch: no · state: not written.

The design has a voice glow: low/mid/high bands with attack and release plus a flow phase (`core/voice.ts` `VOICE` constants and `VoiceBands` in the design clone), feeding uniforms of the glass shader and the listening bubble. In wubble the shader is ported, but `glass.wgsl` sends the voice uniforms as zero (crates/wubble-gfx/src/scene/mod.rs around lines 67-68), and nothing drives them. Decisions "No voice yet": the shader goes into the kit, with no microphone.

To build: (1) Port `VoiceBands` as a pure state machine in wubble-core (`voice.rs`, with a `// design: core/voice.ts:<lines> @<commit>` comment): `fn step(bands, input: Levels, dt) -> Bands` with the design's attack 18/s, release 4.5/s and flow 0.6 + 3.2·level rad/s. Generate the constants through `tools/fidelity/extract.mjs` into `wubble_core::kit_gen`, not by hand. Add the synthetic speech-like envelope the design uses for demos. Unit tests. (2) Add a scene input for the bands in wubble-gfx scene state (state.rs/render.rs) that feeds the existing voice uniforms in glass.wgsl, and the bubble if the design's listening bubble shares them. Effects run only while something sets non-zero bands, so idle cost stays zero (scene run state). (3) Add a kit-facing property, for example a `voice` struct on the glass layer, and a gallery scene driven by the synthetic envelope (crates/wubble/src/gallery_smoke.rs). No microphone, and no permission prompt. All platforms.

## Port the remaining touch-kit pieces: pin, download button, dock, checkbox mask (`touch-kit-port`)

Key: `touch-kit-port` · engine patch: no · state: not written.

The design's touch kit (`.local/the-wallpaper-project/src/wubble/pages/touch.tsx`: `Pin` at about line 257, `Download` at 316, `Dock` at 377 with its gliding tooltip, `Checks` at 555 with the check box's mask grow; styles in touch.css) has pieces that the kit does not have yet (crates/wubble-ui/ui/kit/controls has check-box.slint without the mask grow, and no pin, download button or dock).

To build: `git pull` the design clone first. Port each piece into crates/wubble-ui/ui/kit/controls/ as its own file headed `// design: pages/touch.tsx:<lines> @<commit>` (and touch.css lines). Take the values from the generated `ui/kit/gen/*.slint` and `wubble_core::kit_gen` by rerunning `node tools/fidelity/extract.mjs` if new tokens appear. Put each loop's physics in `wubble_core::controls` (the existing registry pattern, decisions "Component physics"), with unit tests. The pieces: Pin (the pin toggle's press and settle), Download (the progress ring, done check and bounce), Dock (an icon row with the tooltip gliding between hovered icons on a spring), and the check box's mask-grow fill. Then use them: Pin in the tab row's pin affordance (kit chrome/tab-row.slint), Download as the sidebar footer's downloads button with live progress from host/downloads.rs, the check box wherever check-box.slint is used. Dock goes in the gallery only unless a shell place is decided. Add gallery scenes (crates/wubble/src/gallery_smoke.rs) and keep the kit lock passing. All platforms.

## Transparent window with compositor blur: macOS vibrancy, KDE blur (`window-compositor-blur`)

Key: `window-compositor-blur` · engine patch: no · state: not written.

TODO line 144. The window is frameless with a transparent background and transparent corners (decisions "Frameless window", "Window platform flags", `crates/wubble/src/host/window.rs`, `crates/wubble/src/backdrop.rs`). There is no compositor blur behind it. Add: (1) macOS, an `NSVisualEffectView` behind the content view (material `.underWindowBackground` or `.sidebar`, state active), through objc2 on the winit `NSView` from the raw window handle. The window-vibrancy crate (MIT/Apache) is a source to copy from, with a THIRD_PARTY_NOTICES entry. (2) KDE Plasma on Wayland: the `org_kde_kwin_blur_manager` protocol on the window's `wl_surface` (wubble's own Wayland worker `crates/wubble/src/wl/wayland.rs`, which already owns the display connection), with the region set to the sidebar and chrome areas. On X11 KDE, the `_KDE_NET_WM_BLUR_BEHIND_REGION` property. GNOME stays opaque. Then the shell's glass skips its own backdrop blur in the blurred regions (the scene's glass quality in `wubble-gfx/src/scene/quality.rs`). Detect support at run time, and add a setting to turn it off. One `WindowBlur` trait with per-platform implementations.

