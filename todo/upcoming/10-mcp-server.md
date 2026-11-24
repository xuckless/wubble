# 10 MCP server and agent sessions

Legend: `[ ]` open · `[~]` in progress. How items move: [todo/README.md](../README.md).

Plan: [docs/research/mcp-server.md](../../docs/research/mcp-server.md) (section 0: build order; section 0.1: after the first tests). Released in `0.1.0-preview.2`: 20 tools (22 since, with `select` and `upload_file`), sessions in their own Space and container held to their sites, the approval sheet, live hosting and agent-app registration from Settings, off by default.
Tested on 2026-11-20 by two agents on real sites (GitHub, Excalidraw, DuckDuckGo, Wikipedia).
Elsewhere: agent views rendering while hidden is in [01-optimization.md](01-optimization.md); the guardrails, airlocks,
asking the human and clipboard isolation are shelved ([agent-safety.md](../shelved/agent-safety.md)), and so are the
agent widgets ([agent-widgets.md](../shelved/agent-widgets.md)).

## Tools that do not work fully yet
- [ ] An element inside a frame is still placed from the accessibility mirror's boxes, which a scrolled page leaves behind (the top document uses the live layout); the mirror's coordinates are wrong after a scroll for the screen-reader pane too (Chromium sends no root scroll offset)
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

## MCP tools inside frames: flattened DevTools sessions for out-of-process frames, live layout for same-process frames (`agent-frame-sessions`)

Key: `agent-frame-sessions` · engine patch: no · state: not written.

TODO lines 62, 71 and 72. In `crates/wubble/src/host/agent.rs`, `agent_act` uses DOM (`DOM.scrollIntoViewIfNeeded`, `DOM.getContentQuads` by backendNodeId) only for the top document (`top`). A frame's element is placed from the accessibility mirror's boxes, which a scroll leaves stale. `type`'s focus check (`focus_script`) runs in the top document only. `upload_file` cannot reach a cross-site frame's input or chooser. Fix all three on one shared base. (1) In `crates/wubble-engine/src/cdp.rs` and `view.rs`, add `Target.setAutoAttach {autoAttach:true, waitForDebuggerOnStart:false, flatten:true}` on agent-driven views. Track `Target.attachedToTarget` and `detachedFromTarget` (pure bookkeeping: frame id or AX tree id to sessionId), and add `View::cdp_in(session, method, params)` that adds `sessionId` to the message. Map an AX tree (wubble_ax `TreeId`) to its frame's session. (2) A same-process frame's AX id is its backend node id: use the DOM path through the top session. An OOPIF uses its own session, and its quads are offset by the frame's box in the parent (`DOM.getBoxModel` of the iframe element). (3) Run the `type` focus check through `Runtime.evaluate` in the frame's session (or its execution context id from `Runtime.executionContextCreated`), and walk `document.activeElement` through open shadow roots; a closed shadow host no longer counts as editable. (4) For `upload_file`, enable `Page.setInterceptFileChooserDialog` and `DOM.setFileInputFiles` in the frame's session. Update the tool texts in `crates/wubble-mcp/src/tools.rs`.

## Accessibility mirror: correct boxes after the page scrolls (screen-reader pane and MCP) (`ax-mirror-scroll-offset`)

Key: `ax-mirror-scroll-offset` · engine patch: no · state: not written.

TODO line 62, second half. `wubble_ax` (crates/wubble-ax/src/lib.rs ~357) offsets a node's bounds by each container's `scrollX`/`scrollY`. Chromium sends no root scroll offset in the location-change events CEF passes, so after a page scroll the mirror's boxes are wrong for the screen-reader pane (`crates/wubble/src/host/ax.rs`, `render_ax`, `ax_action`) and for anything placed from them. Fix: keep the root document's (and each frame document's) scroll offset in the `PageTree`, from a cheap source. Use DevTools `Page.getLayoutMetrics` (`cssVisualViewport.pageX/pageY`) asked through `View::cdp` after a wheel, a scroll action or a `scroll` event. Or have the helper's probe script report `scrollX/scrollY` on a passive scroll listener, throttled to a frame, as a frame message. Apply it in `PageTree::flatten`/`viewport` (crates/wubble-ax/src/flat.rs) so `reach` and the item views use live positions. Keep wubble-ax pure: the offset comes in as data (`PageTree::set_scroll(tree, x, y)`). Add unit tests in crates/wubble-ax/src/tests.rs for pure logic only.

## Wheel over a nested scroller scrolls the pane, not the document (agent `scroll` and the user's wheel) (`agent-nested-scroll`)

Key: `agent-nested-scroll` · engine patch: no · state: not written.

TODO line 67. `scroll` with a direction over an `overflow:auto` pane sends `View::wheel` at the pane's point (`agent_wheel` in `crates/wubble/src/host/agent.rs`, 5 ticks of 20 ms). The event reaches the pane, but in most runs the document scrolls. The likely cause is Chromium's wheel scroll latching and hit-test: wubble sends no mouse move to the point first, so the latched scroll target and hover state come from the last pointer position, and the phase (`wheel_deltas` in `crates/wubble-engine/src/input.rs`) may start a new latch sequence on the document. Fix: before the first tick, send a mouse move to the point and fence it (`Fence::Pointer`). Then send the ticks as one gesture with Chromium's phase flags (began, changed, ended) where CEF exposes them through `MouseEvent` modifiers (`EVENTFLAG_PRECISION_SCROLLING_DELTA`), or move the agent path to DevTools `Input.dispatchMouseEvent {type:'mouseWheel'}` at the point. That event is hit-tested at its own coordinates and scrolls the innermost scroller as a user's would. Check that `View::wheel` for the user's own wheel moves the pointer first too (`crates/wubble/src/input.rs`). Update the `scroll` tool text in `crates/wubble-mcp/src/tools.rs`.

## `wait_for load` follows in-page navigations' fetches: network idle through DevTools events (`agent-wait-network-idle`)

Key: `agent-wait-network-idle` · engine patch: no · state: not written.

TODO line 68. `Wait::Loaded` (crates/wubble/src/host/agent.rs) waits for no load running and a tree. A history-API navigation's fetches are not followed. Add Playwright's `networkidle`: in `crates/wubble-engine/src/cdp.rs`, a pure `NetIdle` reader (like `NavRequests`) that counts in-flight requests from `Network.requestWillBeSent`, `Network.loadingFinished`, `Network.loadingFailed` and `Network.webSocketCreated/Closed`, excluding data: URLs and long-polling event streams after 5 s, and records the time it last went to zero. Enable the Network domain only on agent-driven views (`Network.enable` with small buffer sizes, sent with the session's other setup) so ordinary tabs pay nothing. Expose `View::net_idle_since() -> Option<Instant>`. Add `Wait::Idle { tab }` (0 in flight for 500 ms) and a `network` (or `idle`) value for `wait_for` in `crates/wubble-mcp/src/tools.rs`. Make `load` wait for idle after a same-document navigation when the URL changed during the call. Update the tool description that documents the gap.

## `read_page` lists options, list boxes and multiple selects; `select` can add a choice (`agent-listbox-options`)

Key: `agent-listbox-options` · engine patch: no · state: not written.

TODO lines 69-70. `read_page` gives no ref for an `<option>`, a list box or a `<select multiple>`/`size>1`. A collapsed select's options are AX-invisible, and `listBox`/`listBoxOption` are no `wubble_ax` item kind (crates/wubble-ax/src/flat.rs). `select` (crates/wubble/src/host/agent.rs `agent_select`/`agent_selected`, wubble-core `agent/forms.rs`) sets one choice and clears the others. (1) In wubble-ax, add item kinds for listBox and listBoxOption (role, name, selected state) and include them in `flatten`/`line`. For a collapsed `<select>`, show its options as a comma list on the select's line (from the select's own DevTools resolution in forms.rs, `DOM.describeNode` with depth 1), so the agent sees choices without opening it. (2) `select` gains an `add: bool` argument (crates/wubble-mcp/src/tools.rs schema and `AgentCall` parsing). On a multiple select, `add:true` keeps the other selected options and toggles on the named one; `values: [..]` sets exactly that set. It dispatches `input` and `change` once, through the same per-call object group (`SELECT_GROUP`).

## `upload_file`: one path-check worker instead of a thread per call (`upload-path-check-worker`)

Key: `upload-path-check-worker` · engine patch: no · state: not written.

TODO line 73. `validate_upload_paths` (crates/wubble/src/host/agent.rs ~721) runs on a fresh thread per call with a 5 s answer limit (`UPLOAD_PATHS_WAIT`). A path on a hung mount fails the call at 5 s, but that thread stays blocked until the file system answers, and each further call leaks another. Replace it with one long-lived checker thread (lazily started and owned by the host's agent state) fed by a channel of `(nonce, paths)`, answering through the existing `agent_paths(nonce, result)` on the Slint thread. Track whether the worker is busy. While a check is outstanding past its limit, later calls fail at once with "a previous file check is still waiting on the file system (path X)", without spawning anything. When the worker answers late, the late answer is dropped (the nonce is gone) and the worker takes new work. Keep the pure parts (`validate_upload_paths` semantics, `upload_log`) unchanged.

## `evaluate`: a DevTools object group per call, released on timeout and on session end (`evaluate-group-per-call`)

Key: `evaluate-group-per-call` · engine patch: no · state: not written.

TODO line 74. `agent_evaluate` (crates/wubble/src/host/agent.rs ~3374) puts every call's remote objects in one shared group `EVAL_GROUP` ("wubble-agent"). The group is not released when a call times out (`EVAL_GRACE`) or its session goes, so a long session's page holds every value an agent ever made. `select` already does it right (`SELECT_GROUP` per call, released via `agent_release_group` and `held_group(&Wait)`). Do the same for `evaluate`: name the group per call (for example `wubble-agent-eval-<id>`), carry it in `Wait::Eval`, and release it through `Runtime.releaseObjectGroup` when the answer is sent, when the wait times out (`agent_let_go`), and for every pending call of a session in `agent_forget`. Drop `agent_release_values`' reliance on the shared name. Extend the existing `held_group` unit test pattern if one already covers `select`.

## Agent `drag` carries an HTML draggable drag through the page's drag and drop (`agent-html-drag`)

Key: `agent-html-drag` · engine patch: no · state: not written.

TODO line 75. `agent_drag` (crates/wubble/src/host/agent.rs ~2844) presses, glides and releases through pointer events only. A page's HTML `draggable` drag starts CEF's `StartDragging` into the window's own drag loop (`crates/wubble/src/host/drag.rs`, `drag_started`), which the agent cannot drive, so `agent_dragged` cancels it and says so. Fix: when an agent-driven tab starts a page drag (`Routed::Drag` in host/surfaces.rs) while an agent drag is in flight, keep the `DragData` in the agent's pending wait instead of handing it to the OS drag. Then drive the drop inside the same view with CEF's drag-target calls along the glide: `drag_target_drag_enter(data, point, ops)`, then `drag_target_drag_over` per step, then `drag_target_drop` at the end, then `drag_source_ended_at` and `drag_source_system_drag_ended` (welding `drag.rs`; existing `View` methods used by the dnd path). Fence with `Fence::Pointer`, then answer "dragged (HTML drag and drop)". A drop target in another tab or app stays unsupported. Update the `drag` tool text in crates/wubble-mcp/src/tools.rs.

## Agent sign-ins
- [ ] Agent sign-ins: copy a site's localStorage/IndexedDB too (DevTools DOMStorage needs a live page per container; MSAL, Firebase and SPA tokens live there)
- [ ] Agent sign-ins: CHIPS-partitioned cookies arrive unpartitioned (CEF's cookie struct has no partition key)
- [ ] Agent sign-ins: `--smoke mcp` under load failed once in 87 loaded runs with the session's first request to the shared site carrying no cookie ("sent: none") after the copy answered; not reproduced or traced yet (the copy's report was not printed)

## Agent sign-ins: copy localStorage/IndexedDB, keep CHIPS partition keys, print the copy's report (`agent-signin-storage-copy`)

Key: `agent-signin-storage-copy` · engine patch: no · state: not written.

TODO lines 81-83 (docs/research/mcp-server.md 0.3). Signing a session in copies the owner's cookies for a site into the session's container (`agent_copy_cookies`/`agent_cookies_copied` in crates/wubble/src/host/agent.rs, `crates/wubble-engine/src/storage.rs`). Missing: (1) Site storage. MSAL, Firebase and SPA tokens live in localStorage and IndexedDB. Open a hidden view on the site's origin in the source container (or reuse a live tab of it). Read `DOMStorage.getDOMStorageItems` for the origin's storage id. Read IndexedDB through `IndexedDB.requestDatabaseNames`, `requestData` and a page script for structured values. Write them in a hidden view of the session's container (`DOMStorage.setDOMStorageItem`; an IndexedDB restore script). Close both views. (2) CHIPS. CEF's cookie struct has no partition key, so partitioned cookies arrive unpartitioned. Read with DevTools `Storage.getCookies` (it carries `partitionKey`) and write with `Network.setCookie {partitionKey}` in the session container's view, instead of the CEF cookie manager for those cookies. (3) The one-in-87 "sent: none" failure: print the copy's report (counts per domain, partitioned or not, timings) to stderr and to the session log every time, so the next failure can be traced. Keep it all inside `COPY_BOUND`.

## Setup and distribution
- [ ] publish wubble-mcp to npm (`packages/wubble-mcp`, built, unpublished; owner go, owner's npm account; steps in docs/release.md)
- [ ] agent apps wubble does not write yet (VS Code, Copilot CLI, Goose; Windsurf on macOS): verify each file's name and entry shape, then make them `Format`s in `agent::harness`
- [ ] Human-like simulated pointer motion for agent pointer actions (owner, 2026-11-23)
- [ ] A SKILL.md that wubble offers and broadcasts, telling an agent how to use wubble (owner, 2026-11-23)

## Agent apps: VS Code, Copilot CLI, Goose and Windsurf on macOS as written formats; Windows paths (`harness-apps-and-windows`)

Key: `harness-apps-and-windows` · engine patch: no · state: not written.

TODO lines 65-66. `crates/wubble-core/src/agent/harness.rs` lists VS Code, Copilot CLI and Goose with `format: None` (found, never written), Windsurf on Linux only, and paths for Linux and macOS only (`files`/`detect` keyed by `Os`). Add from each app's documentation: VS Code user `mcp.json` with `{"servers": {name: {"type":"stdio","command","args"}}}` (path `["servers"]`, `Entry::Stdio`); Copilot CLI `~/.copilot/mcp-config.json` `mcpServers` with `{"type":"local","command","args","tools":["*"]}` (a new `Entry` variant); Goose `~/.config/goose/config.yaml` `extensions.<name>: {type: stdio, cmd, args, enabled: true, name, timeout}`. Goose needs a new `Format::GooseYaml`, edited keeping the rest of the file byte for byte (a yaml-edit-preserving crate, or a narrow line-based edit of the `extensions:` block that refuses odd shapes); Windsurf on macOS is `~/.codeium/windsurf/mcp_config.json`. Windows: add `(Os::Windows, ...)` entries for every app (Claude desktop under `AppData/Roaming/Claude`, VS Code `AppData/Roaming/Code/User/mcp.json`, home-relative dotfiles as on Linux). The manifest's runtime dir (`crates/wubble-core/src/agent/manifest.rs`, `crates/wubble/src/instance.rs` `private_root`) uses `%LOCALAPPDATA%\wubble\mcp`. Extend the existing harness unit tests for the new formats (core logic).

## Agent Spaces in the shell
- [ ] a tab moved into an agent session's Space keeps its back-forward cache and any prerender made before: activating either runs no navigation throttle, so the guard is not asked and only the commit backstop takes the tab back (after the page showed); drop those for a tab that joins a session (or disable the cache and prerender for session tabs) and add a smoke probe
- [ ] Later (owner, 2026-11-20): "Move tab to" in a tab's right-click menu, a two-level menu: hovering the row (with its ">") opens the list of user-created Spaces; moving a tab out of an agent Space makes it an ordinary tab of that Space
- [ ] Later (owner, 2026-11-20): clearing an agent Space that still has tabs asks first: "Close anyway", or "Move tabs", which opens the list of user-created Spaces downward and moves them before the Space is cleared (until then the dustbin clears at once)

## Agent session tabs: drop back/forward cache and prerenders when a tab joins a session (`agent-tab-bfcache-prerender`)

Key: `agent-tab-bfcache-prerender` · engine patch: no · state: not written.

TODO line 63. A tab moved into an agent session's Space keeps its back/forward cache entries and any prerender made before. Activating either runs no navigation throttle, so the session's site guard is not asked, and only the commit backstop (`agent_backstop`/`agent_take_back` in `crates/wubble/src/host/agent.rs`) takes the tab back, after the page showed. Fix: when a tab joins a session (the Effect that binds a tab to a session in `wubble_core::agent`, executed in `agent_effect`), and for every tab the session opens, (1) send DevTools `Page.resetNavigationHistory`, which evicts the tab's bfcache entries along with its history, and (2) `Page.setPrerenderingAllowed {isAllowed:false}`, which cancels and blocks prerenders for that page. Both go through `View::cdp`; re-send after a renderer swap. Consider also turning speculation rules off in agent containers through the container's request-context preferences if CEF exposes them. Log each drop to the session's activity log. Add a probe step to the existing `--smoke mcp` scenario code only if it is a one-liner; no new test files.

## Tab menu: two-level "Move tab to" listing user Spaces; moving out of an agent Space makes an ordinary tab (`move-tab-to-space-submenu`)

Key: `move-tab-to-space-submenu` · engine patch: no · state: not written.

TODO line 77 (owner, 2026-11-20). Today `wubble_core::tabmenu` (`Row::MoveToSpace`, ~line 179) adds one flat "Move to <Space>" row per other Space. Replace it with one row "Move tab to" with a ">" that opens a submenu on hover, the way Chrome's "Move tab to another window" does. The submenu lists user-created Spaces only (not agent Spaces, `Space::agent`), in sidebar order, the current one left out. Use the kit menu's submenu if it has one (`kit/controls/menu.slint`); if not, the submenu is a derived piece (menu rows in a second Menu positioned at the row's right edge) and goes on docs/fidelity/derived-review.md. Model it in the menu data (`Run::Submenu(Vec<Entry>)` or similar) in tabmenu.rs and its renderer in `crates/wubble/src/menu.rs`. Moving a tab out of an agent Space through it releases it from its session: run the agent effect that unbinds the tab (wubble_core::agent) so it becomes an ordinary tab of the target Space, with its container kept or switched to the Space's container as Chrome would (ask the core model; record the choice in the final report). Add unit tests in tabmenu.rs and model tests for core logic.

## Clearing an agent Space with tabs asks first: Close anyway or Move tabs (`clear-agent-space-ask`)

Key: `clear-agent-space-ask` · engine patch: no · state: not written.

TODO line 78 (owner, 2026-11-20). The dustbin on an agent Space's row in the Spaces popup (`wubble_core::agent` ~line 982, `spacemenu.rs` `Row` with the dustbin) clears the Space at once. When it still has tabs, ask first. Show a confirm in the popup (kit confirm/sheet parts, derived if needed, on docs/fidelity/derived-review.md) with "Close anyway" and "Move tabs". "Move tabs" opens the list of user-created Spaces downward, in the popup. Picking one moves every tab there as ordinary tabs (the same unbind as `move-tab-to-space-submenu`), then clears the Space. Model it as one enum in core (for example `DustbinAsk { Idle, Asking(SpaceId), Picking(SpaceId) }` with a pure transition returning effects: StopSession, MoveTabs(to), DeleteSpace), executed in the host (`crates/wubble/src/host/agent.rs` where the dustbin is handled). A Space with no tabs still clears at once. Unit tests in wubble-core.

## Agent sessions: no icon-table entries for their pages; the activity log can be cleared (`agent-pages-icons-log-clear`)

Key: `agent-pages-icons-log-clear` · engine patch: no · state: not written.

TODO line 97. Agent session pages are not history, but their origin and icon are filed in the profile's icon table (`crates/wubble-store/src/favicon.rs`, through the host's favicon path `crates/wubble/src/host` and `wubble_core::favicon`). (1) Skip the favicon store write for tabs an agent session drives and for agent Spaces' tabs (the same test as `Model::closes_into_recent`). Keep icons in memory for the session's own rows. (2) Clearing the activity log: the log is a memory ring (`wubble_core::agent::log::ActivityLog`) plus a store tail (`agent_log`, `STORE_MAX`). Add `ActivityLog::clear` (and per session), a store delete, and a "Clear activity" control on the agent activity page (kit button). Also clear it with "Clear browsing data" when its new checkbox "Agent activity" is ticked (`crates/wubble/src/host/data.rs`). Unit tests for the log's clear.

## Passwords and autofill
- [ ] Autofill: offer the login form a page shows right after a submit (the save check consumes that report today)
- [ ] Autofill: re-probe Reddit (bot wall) and Wikipedia (redirects to auth.wikimedia.org) logins by hand
- [ ] Passwords: the owner's "saved by hand, then gone" still unexplained (a keyring answering "no key" made a new one over the rows until now: guarded); run `--smoke passwords` with the Keychain on the Mac

## Autofill: offer the login form a page shows right after a submit (`autofill-offer-after-submit`)

Key: `autofill-offer-after-submit` · engine patch: no · state: not written.

TODO line 79. In `crates/wubble-core/src/autofill.rs`, `settled_after_submit` handles the report that follows a submit. Unless it is the same origin's sign-in form back (a failed sign-in), it goes to `check(p)` (`Phase::Checking`), so a login form on the next page (a second step, an SSO page, a different origin) gets no key chip or menu. The cause is that one `Phase` holds both the save flow and the offer. Fix: split the machine into two parts that run side by side, inside the one transition: `save: SavePhase` (Idle, Submitted, Checking, Prompt) and `offer: OfferPhase` (Idle, Offer, Menu). Each event updates both. The report after a submit both starts the save check and, through `offer(...)`, offers any login form it carries. `View` (`chip`, `menu`, `prompt`) reads both. When the prompt and the chip would show at once, Chrome shows both: keep both. Update the host (`crates/wubble/src/host/autofill.rs`) to the new state shape. Update the existing unit tests and add cases for a second-step login form and a cross-origin SSO form.

