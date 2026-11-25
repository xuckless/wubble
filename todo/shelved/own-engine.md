# Own engine (the old M6), non-CPU items

Shelved by the owner (2026-11-23): "not sure what we do with our own engine build, but I'd prefer if we arranged our work with least amount of engine builds as possible".

Legend: `[ ]` open. Picked up only when the owner says so; how items move: [todo/README.md](../README.md).

The one engine build and the own engine's CPU items: [01-optimization.md](../upcoming/01-optimization.md); its extension
items: [09-extensions.md](../upcoming/09-extensions.md). The Widevine CDM loader was discarded by the owner on 2026-11-23;
HDR is a shelved idea ([ideas.md](ideas.md)).

## From the old roadmap (M6: ungoogled Chromium + CEF)
- [ ] macOS arm64 codec build (same recipe, `--arm64-build`)
- [ ] Build with `enable_widevine` and codecs; ship no CDM; a CDM loader (installed CDM, else opt-in download)
- [ ] Document DRM limits (Linux L3 around 720p; services that need a VMP-signed browser refuse us)
- [ ] Build infrastructure: Linux and macOS local; Windows on a runner large enough for Chromium
- [ ] Swap in behind `wubble-engine`, shell unchanged
