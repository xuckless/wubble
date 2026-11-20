# Shell renderer: three roads (2026-11-18, spike/skia)

Question: Keep Slint on femtovg with our fork, move Slint to its Skia renderer, or draw the UI ourselves?
Status: Done 2026-11-18: Slint's Skia renderer on the shared wgpu device (road B); the femtovg fork is gone.

Question: keep Slint + femtovg with our fork (A), keep Slint and switch its renderer to
Skia (B), or drop Slint and draw the UI ourselves (C)? The owner preferred B with A as
the fallback and C off the table; the work below proves or disproves B against five
criteria, then switches the branch to it.

Marked **[ran]** = verified by building or running on this Linux box (RTX 5060, NVIDIA
615.71, Wayland, 1x scale, debug build); **[read]** = from the Slint 1.19 / femtovg 0.28 /
skia-safe 0.157 sources in the cargo registry; **[web]** = upstream lookups.

## Decision table

| | A: Slint + femtovg fork | B: Slint + Skia renderer | C: own UI on Skia/wgpu |
|---|---|---|---|
| Zero-copy page frames + scene texture | yes, today | **yes [ran]**: `skia-wgpu` on our `Manual` wgpu device, `frames=120 accelerated=121 cpu_fallback=0`, swatches and glass pass; macOS Metal path exists in the same code [read], runtime unverified | yes, ours to write |
| One frame = one scene (notifier hooks) | yes | **yes [ran]**: `RenderingSetup`/`BeforeRendering`/`AfterRendering` with `GraphicsAPI::WGPU30` unchanged; glass smoke and split masks pass | ours to write |
| Text quality at 1x | grayscale, swash hinted, lighter and softer; needs our fork for odd-sized centred text | **grayscale, Skia/FreeType (Chromium's own rasterizer)**: darker, crisper stems, consistent baselines; no fork needed [ran, crops below] | whatever stack we pick |
| LCD subpixel text | 5-8 days: fork femtovg too (swash `Format::Subpixel`, RGB atlas path, shader type, dual-source blending) | 1-2 days: `Edging::SubpixelAntiAlias` + `SurfaceProps(PixelGeometry::RGBH)` in a vendored `i-slint-renderer-skia`; drops to grayscale inside non-opaque layers | depends |
| HDR / 10-bit present | 5-10 days: femtovg pipelines + surface format + our own colour pass (femtovg has no colour management) | 2-4 days: surface format arms + Skia colour space (PQ/Rec.2020 or extended-linear F16; Skia converts sRGB content itself) + Slint core's 8-bit-only image check. Page HDR still blocked on our Chromium build (M6) | ours |
| Effort for this codebase | 0 now; every new capability is a femtovg feature | **switch done: ~0.5 day** (feature, renderer name, test port, docs); later patches small | 60-120+ person-days (rebuild 46 .slint files / 9.8k lines, 84 components, text input + IME, AccessKit, 17 UI tests) |
| Risk | maintaining a fork of a renderer nobody else drives for our needs; text quality ceiling | Skia submits on wgpu's raw queue outside wgpu's lock and tracker (Slint transitions explicitly; wubble submits on one thread); build downloads a 7 MB prebuilt; macOS runtime not yet seen | very high: scope, accessibility, IME |
| What we give up | nothing now | the femtovg fork's path-origin alignment (icons on fractional positions may soften); ~24 MB static lib in the binary | Slint's declarative UI, live tests, tooling |

**Recommendation: B, done on `spike/skia`.** It passes all five criteria on Linux (macOS
Metal by reading plus a build there, see below). The fork goes away, text matches
Chromium's rasterizer, and the two things wubble needs next (LCD, HDR) are small patches
to a renderer built on a colour-managed library, not features to invent in femtovg.

## 1. Road B, the hard gate

**The path exists in Slint 1.19 [read].** `i-slint-renderer-skia` 1.19.0 has a wgpu
surface (`wgpu_30_surface.rs`, renderer name `skia-wgpu`, selected in
`i-slint-backend-winit` `create_renderer`). With `require_wgpu_30(WGPUConfiguration::Manual
{instance, adapter, device, queue})` it takes our device (no sharing cache under Manual),
and `Backend::make_context` builds Skia's Ganesh `DirectContext` from wgpu-hal's raw
handles: Vulkan (`wgpu_30_surface/vulkan.rs`: VkInstance/VkPhysicalDevice/VkDevice/VkQueue,
the extensions wgpu enabled, API 1.3), Metal (`metal.rs`, the wgpu MTLDevice and command
queue), D3D12 (`dx12.rs`). `import_wgpu_texture` wraps the `wgpu::Texture`'s `VkImage`
(`SHADER_READ_ONLY_OPTIMAL`) or `MTLTexture` as a borrowed `SkImage`
(`Image::from_texture`, `AlphaType::Unpremul`): no copy. Each frame it transitions
imported textures through wgpu before `gr_context.submit`, renders into the swapchain
image wrapped as a Skia render target, hands the image back in `PRESENT_SRC_KHR` and
orders present after Skia's work with an extra wgpu submission. `take_snapshot` renders
into a Skia GPU surface and reads back (snapshot only). Formats: images
`Rgba8Unorm(Srgb)` (and `Bgra8Unorm` on Vulkan/Metal), surface `Rgba8Unorm`/`Bgra8Unorm`.

**It works in wubble [ran].** Both renderers compiled into one binary behind a run-time
switch, then the real switch (renderer `skia-wgpu`, femtovg removed).
`SLINT_DEBUG_PERFORMANCE=console,refresh_lazy` prints
`Backend: Skia renderer (skia backend wgpu; surface: 32 bpp)`.

| Smoke (Skia) | Result |
|---|---|
| boot | ok |
| frames=120 | `pixels ok distinct=98 mean_luma=171`, `glass ok quality=Full`, `accelerated=121 cpu_fallback=0` |
| tabs, nav, page, settings, pages, quit, session, autofill | ok |
| split | `panes=2 pixels ok`, resized, masks bowed and straight |
| windows | both windows `pixels ok` |
| video | ok |

Colour: UNORM textures are tagged sRGB and drawn to the UNORM surface without
conversion (`sampled_texture_color_space`/`attachment_color_space`), so the swatch check
(within 2 per channel) passes with the existing `Rgba8Unorm` blit target. The edge mask's
straight alpha still matches (`Unpremul`). Rounded clips: `clip_rrect`, antialiased, no
layer. Observation only (debug build, CEF at 240 Hz on a 300 Hz display): avg frame
interval 4.7 ms on Skia, 4.5 ms on femtovg.

**Build cost [ran/web].** `skia-bindings` 0.157 downloads a prebuilt static Skia at build
time; rust-skia publishes the exact feature sets Slint asks for: Linux x86_64
`ganesh-gl-jpegd-jpege-pdf-vulkan` (6.8 MB tarball, `libskia.a` 24 MB), macOS arm64
`ganesh-gl-jpegd-jpege-metal-pdf`, Windows x86_64 `d3d-ganesh-gl-jpegd-jpege-pdf`. Clean
build of `wubble` + helper with both renderers: 2 min 49 s here (14 jobs, nice 19); macOS
(the Mac, 10 cores) `cargo build --workspace` + bundle from scratch 5 min 48 s [ran].
Binary: stripped debug `wubble` 91.3 MB on Skia vs 88.4 MB for a femtovg build of a
sibling branch, so roughly +3 MB (rough: different branches; measure release). No
clang/ninja needed with the prebuilt. Offline/CI: `SKIA_BINARIES_URL` or cache the
download dir like CEF. Licence: Skia BSD-3-Clause, skia-safe MIT; fine under GPL-3.0.

**Pins [read].** `slint` 1.19 pins `i-slint-renderer-skia =1.19.0`, which pins
`skia-safe 0.157` and uses wgpu 30 (the same `wgpu-30` as femtovg-wgpu). welding and cef
are untouched. The pins rule holds.

**Risks to keep in view.** (1) Skia records and submits on the raw `VkQueue`, outside
wgpu-hal's queue lock; safe while every wgpu submit is on the UI thread, as today
(`import.rs`, `mask.rs`, scene, welding's copy all run there). A future off-thread wgpu
submit must not overlap a Slint render. (2) wgpu's tracker does not see Skia's layout
changes; Slint handles it with explicit `transition_resources` for imported textures and
the swapchain image. Our own textures that Slint samples are only read by Skia.

## 2. Text quality

Same binary, same Settings page (`--smoke settings --shot`, 1360x880 at 1x), femtovg
(with our fork) vs Skia. Crops at 400% (nearest), femtovg on top, Skia below:

- `crop-rows-femtovg-top-skia-bottom.png` (row labels, 14 px Geist)
- `crop-small-femtovg-top-skia-bottom.png` (13 px labels and the amber hint)
- `crop-segmented-femtovg-top-skia-bottom.png` (segmented control, the old 'Fuli' case)
- `crop-sidebar-femtovg-top-skia-bottom.png`, `crop-nav-...`, `crop-title-...`
- `pair-1x-femtovg-left-skia-right.png` (1:1, no zoom)
- `settings-femtovg-wgpu.png`, `settings-skia-wgpu.png`, `settings-after.png` (full shots)

in a scratch directory (not kept in the repo).

What they show: Skia's glyphs are darker with sharper vertical stems (FreeType hinting at
Skia's default `Normal`, plus Skia's gamma/contrast boost for grayscale text, the same as
Chromium on Linux draws page text), every baseline is level, and odd-sized centred labels
are whole without any fork (Skia aligns the anchor too but positions glyphs with
subpixel variants and baseline snapping, so a half-pixel origin no longer drops a column;
`crisp_text.rs` passes on Skia). femtovg's text is lighter and softer at the same sizes.
Both are grayscale: Slint's Skia renderer sets `font.set_subpixel(true)` and nothing else
(`itemrenderer.rs` `draw_glyph_run`), and wraps surfaces with no `SurfaceProps`, so LCD is
off. Icons (SVG images and Paths) look about the same; Paths lose the fork's origin
alignment.

## 3. What each road costs next

- **HDR / 10-bit.** Surface format is chosen in the renderer: femtovg
  `vendor/.../wgpu.rs` (now removed; it picked `Rgba8Unorm`/`Bgra8Unorm`) and Skia
  `wgpu_30_surface.rs` `init_with_parts` (same choice; `bits_per_pixel` errors on other
  formats; `vk_format_and_color_type` and `metal.rs` map only 8-bit). Under B the patch is
  the format choice (`Rgb10a2Unorm` or `Rgba16Float`), three match arms
  (`A2B10G10R10_UNORM_PACK32`/`RGBA1010102`, `R16G16B16A16_SFLOAT`/`RGBA_F16`) and the
  target `ColorSpace` (Skia ships PQ and Rec.2020; Skia then converts every sRGB colour,
  image and gradient itself), plus i-slint-core `graphics/wgpu_30.rs` accepting a
  `Rgba16Float` image. Under A femtovg has no colour management: everything is drawn in
  8-bit sRGB values and a final colour pass of our own would have to convert. Either way
  page HDR stays blocked by prebuilt CEF 156 (8-bit OSR frames) until the M6 Chromium.
- **120 Hz with many moving items.** Neither renderer does partial rendering on wgpu
  (Skia's wgpu surface passes buffer age 0: full redraw; femtovg always redraws). Skia
  caches paths, glyph atlases and images on the GPU and batches through Ganesh's op
  list; femtovg tessellates paths on the CPU each frame. Measure in release in the
  performance pass; nothing here asserts timings.
- **Blur/backdrop.** Still ours: the scene draws glass into its own wgpu texture and
  Slint shows it as an image (verified: `glass ok`). Skia's own backdrop blur would read
  the swapchain (`COPY_SRC` added for it) but we do not use it.
- **Paths/icons.** Skia: analytic AA, MSAA for complex paths, proper stroking; femtovg:
  stencil-then-cover with its own AA. Skia lacks the fork's Path origin snap.
- **Image scaling.** Both sample bilinearly without mipmaps for Slint images (the
  favicon area-average stays). Skia can do mipmapped/cubic sampling if Slint asked.
- **femtovg limits we hit** (Decided rows and the fork's `WUBBLE_CHANGES.md`): text
  origin on half pixels dropping/doubling glyph columns (fork change 1; not present in
  Skia), Path origin (fork change 2; not in Skia either: icons may soften), dark sRGB
  textures (we use UNORM; Skia handles both correctly), straight alpha for wgpu textures
  (Skia: same, `Unpremul`), the rounded-clip layer cache freezing the scene image (we set
  a new image each frame; harmless under Skia, which clips without a layer), 8-bit
  surface only (both, but a small patch under Skia).

## 4. Road A's real size (the fallback, not taken)

LCD text in femtovg means forking femtovg 0.28 as well as Slint's renderer:
`femtovg/src/text.rs` `render_glyph_swash` renders `Format::Alpha` (swash offers
`Format::Subpixel`); the atlas is already `Rgba8` and NEAREST-sampled (text.rs ~1030);
`renderer/wgpu/shader.wgsl` has `glyph_texture_type` 1 (alpha mask) and 2 (colour) and
would need a 3-channel coverage type; correct LCD blending needs dual-source blending
(`wgpu::Features::DUAL_SOURCE_BLENDING` and a second blend state in `renderer/wgpu.rs`
`blend_state`) or an opaque-background assumption. 5-8 days, plus the glyph cache key
and subpixel variants doubling atlas use. Upstreaming: Slint's CONTRIBUTING asks for a
click-through acceptance (MIT-0 contribution licence, contributor keeps copyright), small
focused PRs and prior discussion for features [web]. Related issues are closed: #5177
"Text blurry with femtovg renderer", #6215, #6298; #6455 (images pixel-aligned, which
our change 2 extended to Paths). Our change 1 (align the text origin, not the anchor) is
a small, upstreamable bug fix for the femtovg renderer; change 2 likewise, and would
apply to the Skia renderer's `draw_path` too.

## 5. Road C

Off the table (owner). For the record: wubble-ui has 46 `.slint` files, 9,771 lines, 84
components, 122 `in` properties and 123 callbacks on `app.slint`, 8 `TextInput`s, 17 UI
tests on Slint's testing backend, AccessKit via Slint's `accessibility`, and 59 Rust files
with 254 `slint::` references. Candidates (all MIT/Apache, none drop-in): Xilem/Masonry +
Vello + Parley (closest in spirit, young, wgpu); Freya (Skia, Dioxus-style, young); GPUI
(Zed's, Apache, not packaged for outside use); Iced (wgpu, Elm-style, mature, own look);
egui (immediate mode, wrong fit for a designed UI). 60-120+ person-days.

## Check round on the branch [ran]

Commit `a15aca7` on `spike/skia`: `cargo fmt --check`, `clippy --workspace --all-targets
-D warnings`, `test --workspace` (36 test binaries ok, `crisp_text` rendering through
Skia), `build --workspace`, and the smokes boot, frames=120, tabs, split, page, settings,
pages, windows, video, quit, nav, session, autofill all exit 0.

## macOS

Done [ran]: rsync to a checkout on the Mac, `cargo build --workspace && cargo run
-p wubble-bundle` succeeds (rust-skia's `aarch64-apple-darwin-ganesh-gl-jpegd-jpege-metal-pdf`
prebuilt downloaded and linked); `cargo test -p wubble-ui --test crisp_text` passes
windowless over ssh, not skipped: Skia's Metal context built from a wgpu Metal device
renders the centred text whole and `take_snapshot` reads it back. The console runner did
not answer a `--smoke boot` job (it ran it but the job hit its 120 s timeout with an empty
log, as an earlier `plat-boot` job did), so nothing windowed ran on macOS.

Still to verify at the Mac console:
- `--smoke boot`, `frames=120` (IOSurface frames imported into wgpu and shown through
  Skia's `import_metal_texture`, swatches, glass), `settings --shot` (text), `windows`,
  `split`, `tabs`, `page`.
- The transparent window: Metal offers only `PostMultiplied` alpha
  (`composite_alpha_mode`), and Slint sets the layer's contents gravity; check the
  rounded corners and the scene under them.
- Snapshot during a rendering notifier (`render_snapshot` uses `try_borrow_mut` on the
  context) and multi-window on one shared Metal device.
