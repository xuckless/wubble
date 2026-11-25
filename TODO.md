# wubble: now

What is being worked on and researched now. The whole roadmap is [todo/](todo/README.md) (how items move, how features
are built, the one engine build). Decisions: [docs/decisions.md](docs/decisions.md) · numbers: [docs/measurements.md](docs/measurements.md).

## Next, in this order (2026-11-24, after 0.6.2)
The goal stays the main process under 5 % CPU in steady states; 0.6.2 brought windowed video on the Mac from 28 to 37 % to 10 to 15 %.
1. **Measure what the remaining 10 to 15 % is** on the 0.6.2 build with the page on its layer (`scripts/bench/profile.sh`, `breakdown.py`; owner's Mac): frame receipt, the pool copy, the layer commit, the begin-frame timer, the accessibility mirror (estimated 3 to 8 %, unverified). Every later step is judged against this ([01](todo/upcoming/01-optimization.md))
2. **Road 2, the part with no engine patch:** CEF's own begin-frame source while a page is on its layer (no shell timer), and present inside a Core Animation transaction only when a route or placement changes. Touches two suspects of the shelved dropped frames ([video-dropped-frames.md](todo/shelved/video-dropped-frames.md))
3. **The engine patch queue** (owner: "do all the things that require engine repatch first and then build engine once"): write unbuilt in `patches/chromium/` all 21 items of [`todo/upcoming/engine-build.md`](todo/upcoming/engine-build.md) (road 2's zero-copy, road 3's `CALayerHost`, the extension queue and the rest); then the one engine build; then road 3's three experiments on it
4. In parallel when the build lock is free: compile and test the blind overlays of `untested/store-step2` ([surface-manager.md](todo/in-progress/surface-manager.md))

Owner, by hand on the Mac: whether your 0.6.1 install updated itself to 0.6.2. (The page layer's 18 hand checks passed on 2026-11-25.)

## Working on
- [~] Fix round from the owner's 0.6.2 hand checks: the pointer after a drop, picture-in-picture from a split pane, built-in pages in split view ([fix-round.md](todo/in-progress/fix-round.md))
- [~] Surface manager: tested parts shipped in 0.6.2; blind overlays, command bar and focus left ([surface-manager.md](todo/in-progress/surface-manager.md))
- Open owner decisions: [decisions.md](todo/in-progress/decisions.md)

## Research
- Road 3, the CEF patch presenting the page through a CAContext id / `CALayerHost` (est. 2 to 5 % main): three experiments ([docs/research/page-layer.md](docs/research/page-layer.md))

## Upcoming, in order
1. [Optimization](todo/upcoming/01-optimization.md): main process under 5 % CPU, then memory, battery and heat, the Linux page layer
2. [Tabs and windows](todo/upcoming/02-tabs-windows.md) · 3. [Navigation](todo/upcoming/03-navigation.md) · 4. [Page interaction](todo/upcoming/04-page-interaction.md)
5. [Media and downloads](todo/upcoming/05-media-downloads.md) · 6. [Privacy and security](todo/upcoming/06-privacy-security.md) · 7. [Web platform UI](todo/upcoming/07-web-platform-ui.md)
8. [Passwords, autofill and import](todo/upcoming/08-passwords-autofill-import.md) · 9. [Extensions](todo/upcoming/09-extensions.md) · 10. [MCP server](todo/upcoming/10-mcp-server.md)
11. [Translation and OCR](todo/upcoming/11-translate-ocr.md) · 12. [Small tools](todo/upcoming/12-small-tools.md) · 13. [Release, updater and website](todo/upcoming/13-release-updater-site.md)

Everything that waits for the one engine build: [engine-build.md](todo/upcoming/engine-build.md) (21 items; shelved ones: [todo/shelved/engine-build.md](todo/shelved/engine-build.md))

Shelved: [todo/shelved/](todo/shelved/) · done: [todo/done/](todo/done/) · risks: [todo/risks.md](todo/risks.md)
