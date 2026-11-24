# wubble roadmap

A Chromium-based browser with a Slint shell: Chromium renders off-screen, Slint composites it as GPU textures.
Decisions: [docs/decisions.md](docs/decisions.md) · numbers: [docs/measurements.md](docs/measurements.md) ·
fidelity: [docs/fidelity/](docs/fidelity/kit-plan.md) · research: [docs/agents/research.md](docs/agents/research.md) · commands: [docs/smokes.md](docs/smokes.md)
Legend: `[ ]` open · `[~]` in progress. One line per item; done items are removed: `git log` has them.
Open items live in the area files below; add a new item to the file of its area.

## Where we are (2026-11-23)
- wubble 0.6.1 is released (Linux x86_64, macOS arm64); versions are semantic from here; Windows is parked.
- `main` is at the released commit; `store/lane` is pushed nowhere; the owner allowed pushing `main` once the merged work is green on Linux and macOS.
- Surface store (`wubble_core::overlay`; the owner's "surface manager"): the table, hover, page-attached and drag parts, and the downloads and Spaces overlays, are on `store/lane`, tested on Linux and macOS; the other overlays are written blind on `untested/store-step2`.
- Page layer: Layer: fullscreen is on `store/lane` behind `--page-layer=fullscreen` (off by default), tested; on an M2 it took the main process from 26 % to 12 % during fullscreen 60 fps video.
- Layer: windowed (`--page-layer=window`) is on `untested/page-layer-2`: passes everything on Linux; macOS compile and first run pending.
- Goal (owner): the main process under 5 % CPU in steady states; measured today 26 to 31 % for windowed video.
- Fixed on `store/lane`, not released: the autofill offer race (a cross-origin iframe's report replaced the main frame's pending offer).
- Shelved by the owner: a scroll lag seen once on the macOS store test build ([performance](docs/todo/performance.md)).

## Now
- [ ] Surface store: compile and test the blind overlays from `untested/store-step2`; then the command bar and focus/key ownership through the store; drop the `Env` bridge ([surface-store.md](docs/todo/surface-store.md))
- [ ] Layer: windowed: the macOS compile and first run, then the owner's hand-check list ([performance.md](docs/todo/performance.md))
- [ ] Merge the tested work and push `main` once it is green on Linux and macOS
- [ ] After Layer: windowed, the next road to the under-5 % goal: zero-copy of CEF's IOSurface and CEF's own begin-frame source ([performance.md](docs/todo/performance.md))

## Open decisions (owner)
- Approved, not started: a pop-up window minimum-size design token ([private-popups.md](docs/todo/private-popups.md)). After the store work: one despawn path for keyed host state ([surface-store.md](docs/todo/surface-store.md))
- Mac heat (measure first with `scripts/bench/mac-energy.sh`): an opaque window on macOS, a lower rate for the start page's liquid
- PR #1's defaults as merged: `install.sh` installs on the sha256 alone without `minisign` (a warning), page Escape, sentence-case labels, agent-tab refusals, the shortcut reset
- PR #2's choices as merged: compact mode is the sidebar toggle; the autofill key chip at the page's top left
- Attach FFmpeg's source tarball to each release, or keep the link in the notes
- The inset-shadow edge under partial rendering: snap the shadow to the pixel grid (moves its edges up to 3 levels) or leave it
- Hardware video decode: keep the opt-in row (unsandboxed GPU process) or remove it
- Start page liquid: stirring per second or per frame
- Ship codecs by default or opt-in (AVC/AAC patent exposure)
- Delete the 38 GB Chromium checkout used for the codec build
- Apple Passwords / passkeys entitlement request (the owner files it)
- Video strip: does a collapse carry over to later pages; Pip: a cropped video or the page's own element fullscreen
- Sidebar day labels ("Today": the design draws none); whether `cpu_fallback > 0` fails `--smoke frames`
- If Turso (pre-1.0) proves unstable: the schema stays SQLite-compatible so `rusqlite` can replace it

## Areas (open items)
- [docs/todo/performance.md](docs/todo/performance.md) (40): the under-5 % goal and its roads, Layer: fullscreen and windowed, the Mac findings, perf pass 3
- [docs/todo/surface-store.md](docs/todo/surface-store.md) (9): the store's remaining overlays, command bar, focus, lock, despawn decision
- [docs/todo/web-view-parity.md](docs/todo/web-view-parity.md) (24): Chrome parity of the page view: hand checks, touch, clipboard, fling, cursors, drags
- [docs/todo/private-popups.md](docs/todo/private-popups.md) (12): private windows, containers, pop-ups, window state restore
- [docs/todo/mcp-server.md](docs/todo/mcp-server.md) (30): MCP tools, agent sessions and sign-ins, autofill, agent widgets
- [docs/todo/hardening.md](docs/todo/hardening.md) (9): built-in pages and their data (task manager, list pages, media downloader, `--smoke data`)
- [docs/todo/review.md](docs/todo/review.md) (43): perpetual review findings, small shell items, design fidelity, the feel pass
- [docs/todo/milestones.md](docs/todo/milestones.md) (54): what is left of M0 to M7, the built-in content blocker, the own engine and extensions
- [docs/todo/release.md](docs/todo/release.md) (9): releases, the updater, the website
- [docs/todo/later.md](docs/todo/later.md) (8): translation, OCR, unplanned features, shelved ideas
- [docs/todo/windows.md](docs/todo/windows.md) (4): Windows, parked
- [docs/todo/risks.md](docs/todo/risks.md): the standing risks
