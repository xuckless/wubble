# Ideas

Shelved ideas: each section starts with the owner's word on it.

Legend: `[ ]` open. Picked up only when the owner says so; how items move: [todo/README.md](../README.md).

## HDR through the OS layer
The owner (2026-11-23): "it doesn't cost too much and it can be implemented with a Chromium rebuild".

HDR would only ever be the page, shown by the OS layer, never the Slint shell. It comes nearly free on macOS with the
CEF patch that lets the GPU process present the page (Core Animation carries the HDR headroom; road 3 in
[01-optimization.md](../upcoming/01-optimization.md)), so the 16-bit shell path in the two specs below is not the road;
it rides in the one engine build.
- [~] HDR output: researched and spiked; needs the own build (F16 capture) and a Slint renderer patch (old M6 line)
- [ ] Undecided, to be decided before the CEF patch is built: the page never leaves the OS layer; when glass opens over it the shell takes a copy of the page only to draw the blur inside the glass area, which removes the switch between the two paths (and the standard-range dimming of an HDR video)

## HDR: CEF patch for F16 shared-texture capture (`hdr-engine-f16`)

Key: `hdr-engine-f16` · engine patch: yes: waits for the one engine build · state: not written.

Chrome draws HDR video and HDR images in HDR on capable displays. wubble cannot: CEF's windowless accelerated paint (OnAcceleratedPaint) hands out 8-bit BGRA/RGBA shared textures in sRGB, so HDR content is tone-mapped to SDR before wubble sees it (decisions.md "Colour and HDR", TODO M6 "HDR output: researched and spiked; needs the own build (F16 capture) and a Slint renderer patch"). The surface spike is crates/wubble-gfx/examples/hdr_surface.rs.

What to build: (1) A patch, patches/chromium/5x-cef-osr-f16.patch, in the queue's style. In CEF's OSR video consumer (libcef/browser/osr/video_consumer_osr.cc), when the browser is created with a new windowless setting (or a command-line switch read by an exported marker `cef_wubble_hdr_v1`), request the frame sink video capturer in RGBA_F16 with an extended-linear-sRGB (or the content's) gfx::ColorSpace. Pass the colour space with the shared-texture info, extending cef_accelerated_paint_info_t through the marker table rather than changing the public struct. Cover Linux dmabuf, macOS IOSurface and Windows D3D11 handles. (2) List it in patches/README.md with its marker. (3) welding: read the marker. When present, mark NativeFrame with the pixel format (NativeFramePixelFormat gains Rgba16Float) and the colour space (`wubble change`). The patch cannot be built now; write it so that it applies to CEF 156.0.30. The shell side is a separate feature.

## HDR: shell import, Skia renderer and surface colour space (`hdr-shell-path`)

Key: `hdr-shell-path` · engine patch: no · state: not written.

This is the shell half of HDR output; the engine half (F16 capture) is a separate patch. Today crates/wubble-gfx/src/import.rs blits every page into an Rgba8Unorm target (TARGET_FORMAT), Slint's Skia renderer (vendor/i-slint-renderer-skia) draws to an 8-bit sRGB wgpu surface, and nothing asks the display for HDR. The spike (crates/wubble-gfx/examples/hdr_surface.rs) already picks the best (format, SurfaceColorSpace) the surface offers: HDR10, HLG, scRGB, extended sRGB, else SDR.

What to build: (1) wubble-gfx: an F16 import path. When a NativeFrame reports Rgba16Float with a colour space, import it into an Rgba16Float target with no 8-bit clamp, keeping the 8-bit path unchanged for SDR frames. Put the pure choice in a function with tests: frame format, surface mode → target format and the shader's encode step. (2) vendor/i-slint-renderer-skia, under its WUBBLE_CHANGES.md rules: create the surface with the HDR mode from the spike's `pick_mode`, render the Skia surface as RGBA_F16 with an SkColorSpace matching it (extended sRGB linear), and keep SDR UI colours at the SDR reference white (203 nits for PQ, 1.0 for scRGB). Extend the partial-render test. (3) A setting, `video.hdr` (default on where the display offers HDR, Section::Appearance), and a `--smoke codecs` report line saying which mode was chosen. (4) The glass and scene shaders (wubble-gfx/src/scene) stay correct in linear F16. Leave glass blur on 8-bit when HDR is off. Linux Wayland and macOS (EDR) first; Windows scRGB behind cfg.

## DRM through a WebKit tab (macOS)
The owner's idea (2026-11-23), not started.
- [ ] For a site that needs DRM the tab is rendered by the system's web view (WebKit, FairPlay) instead of Chromium, as a native view under the same hole in the shell that Layer: windowed uses, the shell staying on top; no agreement with Google, no signing scheme, no closed binary downloaded, no licence conflict with GPL-3.0; extensions and the content blocker could be simulated by injecting scripts into the web view, as the Orion browser does. Costs: that tab has separate cookies and sign-ins, needs its own thin input/find/zoom/downloads/menu handling, macOS only, and it is untested whether streaming services accept the system web view inside a third-party app (first check on two or three services)

## From the old roadmap ("Shelved ideas")
- [ ] Read aloud with an on-device voice (tiny-voca or the OS voice); a voice, not a screen reader ([docs/research/tts.md](../../docs/research/tts.md))
- [ ] OS password managers and passkey keychains (fingerprint, passkeys where the laptop has them), as a passthrough in a dedicated build after extensions ([docs/research/apple-passwords.md](../../docs/research/apple-passwords.md))
