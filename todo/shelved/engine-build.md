# Shelved items that need the one engine build

Shelved: each item keeps the reason of the file it came from. If the owner picks one up, it joins [../upcoming/engine-build.md](../upcoming/engine-build.md).

Legend: `[ ]` open. Picked up only when the owner says so.

## HDR (from [ideas.md](ideas.md))

### HDR through the OS layer
The owner (2026-11-23): "it doesn't cost too much and it can be implemented with a Chromium rebuild".

HDR would only ever be the page, shown by the OS layer, never the Slint shell. It comes nearly free on macOS with the
CEF patch that lets the GPU process present the page (Core Animation carries the HDR headroom; road 3 in
[01-optimization.md](../upcoming/01-optimization.md)), so the 16-bit shell path in the two specs below is not the road;
it rides in the one engine build.
- [~] HDR output: researched and spiked; needs the own build (F16 capture) and a Slint renderer patch (old M6 line)
- [ ] Undecided, to be decided before the CEF patch is built: the page never leaves the OS layer; when glass opens over it the shell takes a copy of the page only to draw the blur inside the glass area, which removes the switch between the two paths (and the standard-range dimming of an HDR video)

### HDR: CEF patch for F16 shared-texture capture (`hdr-engine-f16`)

Key: `hdr-engine-f16` · engine patch: yes: waits for the one engine build · state: not written.

Chrome draws HDR video and HDR images in HDR on capable displays. wubble cannot: CEF's windowless accelerated paint (OnAcceleratedPaint) hands out 8-bit BGRA/RGBA shared textures in sRGB, so HDR content is tone-mapped to SDR before wubble sees it (decisions.md "Colour and HDR", TODO M6 "HDR output: researched and spiked; needs the own build (F16 capture) and a Slint renderer patch"). The surface spike is crates/wubble-gfx/examples/hdr_surface.rs.

What to build: (1) A patch, patches/chromium/5x-cef-osr-f16.patch, in the queue's style. In CEF's OSR video consumer (libcef/browser/osr/video_consumer_osr.cc), when the browser is created with a new windowless setting (or a command-line switch read by an exported marker `cef_wubble_hdr_v1`), request the frame sink video capturer in RGBA_F16 with an extended-linear-sRGB (or the content's) gfx::ColorSpace. Pass the colour space with the shared-texture info, extending cef_accelerated_paint_info_t through the marker table rather than changing the public struct. Cover Linux dmabuf, macOS IOSurface and Windows D3D11 handles. (2) List it in patches/README.md with its marker. (3) welding: read the marker. When present, mark NativeFrame with the pixel format (NativeFramePixelFormat gains Rgba16Float) and the colour space (`wubble change`). The patch cannot be built now; write it so that it applies to CEF 156.0.30. The shell side is a separate feature.

## Pointer lock and macOS momentum scrolling (from [web-view-parity.md](web-view-parity.md))

Shelved as on demand by the owner (2026-11-23): "Hardening and review are on demand. And close them as required. Hopefully the redux system would eliminate state errors".

- [ ] Fling (macOS): momentum phases and elastic overscroll need a CEF patch (split from the hold-gesture line of [web-view-parity.md](web-view-parity.md))
- [ ] Pointer lock: the permission prompt exists, nothing confines the pointer or sends relative motion (a CEF patch, likely)

## The own engine recipe (from [own-engine.md](own-engine.md))

Shelved by the owner (2026-11-23): "not sure what we do with our own engine build, but I'd prefer if we arranged our work with least amount of engine builds as possible".

- [ ] Reproducible recipe: Chromium at the CEF branch + ungoogled patches + CEF patches, kept as a patch queue
