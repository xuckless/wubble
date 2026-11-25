# wubble: now

What is being worked on and researched now. The whole roadmap is [todo/](todo/README.md) (how items move, how features
are built, the one engine build). Decisions: [docs/decisions.md](docs/decisions.md) · numbers: [docs/measurements.md](docs/measurements.md).

## Working on
- [~] Surface manager (the surface store; its tested parts shipped in 0.6.2): compile and test the blind overlays of `untested/store-step2`, then the command bar and focus/key ownership through the store ([surface-manager.md](todo/in-progress/surface-manager.md))
- [~] Page layer (window mode on by default on macOS since 0.6.2): the owner's hand-check list, the frame rate under glass; dropped video frames shelved ([page-layer.md](todo/in-progress/page-layer.md), [video-dropped-frames.md](todo/shelved/video-dropped-frames.md))
- [~] `main` moves to the 0.6.2 release commit ([merge-main.md](todo/in-progress/merge-main.md))
- Next optimization roads: zero-copy and CEF's begin frames, then the CEF patch (below; [01](todo/upcoming/01-optimization.md))
- Open owner decisions: [decisions.md](todo/in-progress/decisions.md)

## Research
- Zero-copy of CEF's IOSurface and CEF's own begin-frame source, the next CPU road (est. 6 to 10 %) ([01](todo/upcoming/01-optimization.md))
- The CEF patch presenting the page through a CAContext id / `CALayerHost`: three experiments (`docs/research/page-layer.md`)
- Whether the accessibility mirror costs 3 to 8 % of the main process (unverified)

## Upcoming, in order
1. [Optimization](todo/upcoming/01-optimization.md): main process under 5 % CPU, then memory, battery and heat, the Linux page layer
2. [Tabs and windows](todo/upcoming/02-tabs-windows.md)
3. [Navigation](todo/upcoming/03-navigation.md): address bar, bookmarks, history
4. [Page interaction](todo/upcoming/04-page-interaction.md)
5. [Media and downloads](todo/upcoming/05-media-downloads.md)
6. [Privacy and security](todo/upcoming/06-privacy-security.md)
7. [Web platform UI](todo/upcoming/07-web-platform-ui.md): site permissions and web features
8. [Passwords, autofill and import](todo/upcoming/08-passwords-autofill-import.md)
9. [Extensions](todo/upcoming/09-extensions.md)
10. [MCP server](todo/upcoming/10-mcp-server.md)
11. [Translation and OCR](todo/upcoming/11-translate-ocr.md)
12. [Small tools](todo/upcoming/12-small-tools.md): read aloud, reader options, task manager
13. [Release, updater and website](todo/upcoming/13-release-updater-site.md)

Shelved: [todo/shelved/](todo/shelved/) · done: [todo/done/](todo/done/) · risks: [todo/risks.md](todo/risks.md)
