# MCP server and agent sessions

Part of the roadmap ([TODO.md](../../TODO.md)). Legend: `[ ]` open · `[~]` in progress. Done items are removed: `git log` has them.

Plan: [docs/research/mcp-server.md](../research/mcp-server.md) (section 0: build order; section 0.1: after the first tests). Released in `0.1.0-preview.2`: 20 tools (22 since, with `select` and `upload_file`), sessions in their own Space and container held to their sites, the approval sheet, live hosting and agent-app registration from Settings, off by default.
Tested on 2026-11-20 by two agents on real sites (GitHub, Excalidraw, DuckDuckGo, Wikipedia). In order:
- [ ] An element inside a frame is still placed from the accessibility mirror's boxes, which a scrolled page leaves behind (the top document uses the live layout); the mirror's coordinates are wrong after a scroll for the screen-reader pane too (Chromium sends no root scroll offset)
- [ ] a tab moved into an agent session's Space keeps its back-forward cache and any prerender made before: activating either runs no navigation throttle, so the guard is not asked and only the commit backstop takes the tab back (after the page showed); drop those for a tab that joins a session (or disable the cache and prerender for session tabs) and add a smoke probe
- [ ] publish wubble-mcp to npm (`packages/wubble-mcp`, built, unpublished; owner go, owner's npm account; steps in docs/release.md)
- [ ] agent apps wubble does not write yet (VS Code, Copilot CLI, Goose; Windsurf on macOS): verify each file's name and entry shape, then make them `Format`s in `agent::harness`
- [ ] `scroll` with a direction over a nested scroller (an `overflow: auto` pane): the wheel event reaches the pane but in most `--smoke mcp` runs the document takes the scroll; a user's wheel goes the same path (`View::wheel`): check by hand, then fix (the smoke checks the page only)
- [ ] `wait_for load` does not follow an in-page (history API) navigation's fetches (documented in the tool; `url` and `navigation` cover the address change): network idle would need DevTools' network events through the engine
- [ ] `read_page` lists no `<option>`, list box or `<select multiple>`/`size` (a collapsed select's options are AX-invisible; `listBox` and its options are no `wubble_ax` item kind): `select` takes an option's ref and a multiple list, but no ref reaches one yet
- [ ] `select` on a `<select multiple>` sets one choice (the others cleared); adding a choice is not offered
- [ ] `type`'s focus check runs in the top document: focus moving between fields inside one frame is not seen (a frame counts as editable), and a custom element whose shadow root is closed counts as editable once focus moved to it
- [ ] `upload_file` into a frame of another site (its own process): the top target's DevTools session neither reaches its input (`DOM.setFileInputFiles`) nor intercepts its chooser, so the click's chooser is cancelled and the call fails; reaching it needs `Target.setAutoAttach` with flattened sessions through the engine
- [ ] `upload_file`'s path check runs on a thread of its own per call: a path on a hung mount fails the call at 5 s, but its thread stays blocked until the file system answers
- [ ] `evaluate`'s object group is shared by a tab's calls and not released when one times out or its session goes (`select`'s is per call and always released)
- [ ] `drag` drives pointer events only: an HTML `draggable` drag runs in the window's own drag loop (dropped with nothing, the reply says so)
- [ ] Not yet tested: a right click's menu; the draft-issue run on a signed-in private repository (waits for the owner's sign-in in the session's tab)
- [ ] Later (owner, 2026-11-20): "Move tab to" in a tab's right-click menu, a two-level menu: hovering the row (with its ">") opens the list of user-created Spaces; moving a tab out of an agent Space makes it an ordinary tab of that Space
- [ ] Later (owner, 2026-11-20): clearing an agent Space that still has tabs asks first: "Close anyway", or "Move tabs", which opens the list of user-created Spaces downward and moves them before the Space is cleared (until then the dustbin clears at once)
- [ ] Autofill: offer the login form a page shows right after a submit (the save check consumes that report today)
- [ ] Autofill: re-probe Reddit (bot wall) and Wikipedia (redirects to auth.wikimedia.org) logins by hand
- [ ] Agent sign-ins: copy a site's localStorage/IndexedDB too (DevTools DOMStorage needs a live page per container; MSAL, Firebase and SPA tokens live there)
- [ ] Agent sign-ins: CHIPS-partitioned cookies arrive unpartitioned (CEF's cookie struct has no partition key)
- [ ] Agent sign-ins: `--smoke mcp` under load failed once in 87 loaded runs with the session's first request to the shared site carrying no cookie ("sent: none") after the copy answered; not reproduced or traced yet (the copy's report was not printed)
- [ ] Passwords: the owner's "saved by hand, then gone" still unexplained (a keyring answering "no key" made a new one over the rows until now: guarded); run `--smoke passwords` with the Keychain on the Mac
- [ ] Agent views keep rendering while no window shows them (a hidden view has no accessibility tree and no screenshot): find a cheaper way
- [ ] Guardrails, after use shows the need (plan sections 2 to 7): the airlocks
- [ ] Agent tabs ask the human (the airlocks' prompt) for a permission, a download or an external link, instead of refusing them outright as now
- [ ] Clipboard isolation for agent tabs on every platform (the Wayland bridge cannot tell an agent's copy or paste from the human's; elsewhere Chromium writes itself)

## Agent widgets
- [ ] Wire the thinking orb (in the kit, gallery only so far): the agent mark while a call is in flight, the activity page header, the approval sheet; after the owner reads the proposal in `docs/research/agent-widgets.md` section 5
- [ ] The mascots are researched, not ported: their fabric and plastic shading is CPU-baked with per-frame readbacks in the web version, so a faithful port needs a GPU rewrite (`docs/research/agent-widgets.md` section 3.3); owner decides whether to do it
- [ ] Thinking orb: the `gravity` cursor warp is not ported; one moving 64 px orb costs a whole-window redraw per frame (about 8 to 9 % of a core at 60 Hz, hand run): tied to the shell's redraw work in [performance.md](performance.md)
- [ ] The agent glyph: the design's `sparkle` stands in (one alias, `Glyph.agent`) and reads as a crosshair at badge size; the owner's glyph, a mascot or the orb, then one swap
