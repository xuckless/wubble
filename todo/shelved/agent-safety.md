# Agent safety

Shelved by the owner (2026-11-23): "shelf the safety optimizations on agent, that would be on demand after testing".

Legend: `[ ]` open. Picked up only when the owner says so; how items move: [todo/README.md](../README.md).

Plan: [docs/research/mcp-server.md](../../docs/research/mcp-server.md) sections 2 to 7. The rest of the MCP work: [10-mcp-server.md](../upcoming/10-mcp-server.md).

## From the old roadmap
- [ ] Guardrails, after use shows the need (plan sections 2 to 7): the airlocks
- [ ] Agent tabs ask the human (the airlocks' prompt) for a permission, a download or an external link, instead of refusing them outright as now
- [ ] Clipboard isolation for agent tabs on every platform (the Wayland bridge cannot tell an agent's copy or paste from the human's; elsewhere Chromium writes itself)

## Build-out features (2026-11-22)

## Agent airlocks: ask the human for permissions, downloads, external links, popups and off-list navigation (`agent-airlock-prompts`)

Key: `agent-airlock-prompts` · engine patch: no · state: not written.

TODO lines 86-87, docs/research/mcp-server.md section 5 "Airlocks" and decision "Agent tabs fail closed". Today an agent-driven tab denies every permission request with no prompt, refuses downloads (also the media downloader's job) and external-protocol links, and blocks off-list navigation, each logged to the session. Build the prompt instead. Add a pure `wubble_core::agent::airlock`: `Airlock` (Idle, Asking{kind, origin, detail}, Answered) per tab with `transition` and effects (Allow, Deny, AllowForOrigin, AddSite). Kinds: Permission, Download{name,size,type}, External{scheme}, Popup{target}, Navigation{from,to,link}. Per-origin pre-approvals live in the session's `Grant` so a repeat does not ask again. The prompt is drawn outside the page's pixels from the kit's prompt family (`kit/overlay/a` permission prompt, `a-prompt`), never focused (no keyjacking), labelled "during an agent session". The call that triggered it waits (`Wait` in crates/wubble/src/host/agent.rs) with the session's `request_human` semantics, and answers allowed or refused. Hook it into the existing deny sites (permission handler, `can_download`/`on_before_download`, the external handler `wubble_core::external`, the popup path, the site guard). Log every answer to the activity log. Unit tests in core.

## Agent guardrails: hold non-GET requests after an action (write window) and flag hidden content (`agent-write-window`)

Key: `agent-write-window` · engine patch: no · state: not written.

TODO line 86 (plan section 5: "This click submits a form" and the hidden-content flags). (1) Write window: after each agent action, hold every non-GET request from the tab's frames for N seconds and ask through the airlock prompt (`agent-airlock-prompts`). Per-origin grants are on by default, and a "review all writes" mode is chosen in the session sheet. This needs the request method and a body summary on the resource-request path: a welding change in `vendor/welding/src/handlers.rs`/`surface.rs`, marked `wubble change` and listed in vendor/welding/WUBBLE_CHANGES.md, adding `method` and the first KB of the upload data to `ResourceRequest` and an asynchronous answer through CEF's `CefCallback` in `on_before_resource_load`. In agent containers, turn service workers off (Playwright's block-service-workers) so requests stay attributable, and keep WebRTC's non-proxied UDP off. (2) Hidden-content flags: `read_page` marks items with opacity 0, font-size 0, off-screen, clipped away, colour on its own background or aria-hidden but visible. Read computed style through a `Runtime.evaluate` in an isolated world (`Page.createIsolatedWorld`) for the listed refs, and add a `hidden` note per line (crates/wubble-core/src/agent/refs.rs, `line()` in host/agent.rs). Flags are a signal, not a defence; say so in the tool text.

## Clipboard isolation for agent tabs on every platform (`agent-clipboard-isolation`)

Key: `agent-clipboard-isolation` · engine patch: no · state: not written.

TODO line 88, plan's clipboard airlock and open question 6. On Wayland the page clipboard goes through the helper's `clipboard.js` bridge (`crates/wubble-helper/src/clipboard.js`, `PageEvent::Clipboard`, crates/wubble/src/clipboard.rs). It cannot tell an agent's copy from the human's. On X11, macOS and Windows, Chromium writes the system clipboard itself. Build: (1) Agent containers get a private clipboard. Force the bridge on in agent containers' renderers on every platform (the helper decides per renderer from a switch or the frame's container, sent by the browser), and suppress Chromium's own writes there by denying the `clipboard-write` and `clipboard-read` permissions to agent containers and handling `execCommand('copy')` through the bridge. The host keeps one clipboard per session (an in-memory buffer). The agent's paste reads only it. (2) A copy out to the human's clipboard goes through the airlock prompt (`agent-airlock-prompts`), showing the truncated text and its origin. (3) A paste from the human's clipboard into an agent tab happens only through an explicit human action. Model the per-session clipboard as data in `wubble_core::agent` with a pure transition.

