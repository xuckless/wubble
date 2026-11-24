# Private windows, containers, pop-ups and window state

Part of the roadmap ([TODO.md](../../TODO.md)). Legend: `[ ]` open · `[~]` in progress. Done items are removed: `git log` has them.

## Private windows and containers (hardening pass, open)
- [ ] Private windows: a gallery scene and shots for the owner's eye (the Private chip and tint, the close question, the extension private row and prompt box; `derived-review.md`); `--smoke private --shot-dir` shoots the window
- [ ] Private windows: Settings shows stored site choices only (the private ones are invisible there); reopen closed private tabs from memory, as Chrome; the install prompt's private box can keep the last tick when a second pick replaces an open prompt; Chrome's keep-a-normal-window-open-when-emptied question (owner)
- [ ] Private windows, unchecked: docked DevTools reopening in the right container between a normal and a private tab; reopen from a private window, `open_routed`, ext `open_command`, Open as Tab; whether CEF's off-the-record profile inherits the primary's stored content settings or zoom
- [ ] Extensions P5: "Allow in private windows" reaches only the action button and its popup; needed: `SetIsIncognitoEnabled` per extension through `cef_wubble_ext_v1`, registry controllers for off-the-record profiles, the projection carrying private windows, `ExtRequest` naming its extension (unknown whether Chromium runs an extension in a non-primary off-the-record profile)
- [ ] No way to create, rename or delete a container (only agent sessions make one): the routing rules' Container column can name only Default or a live agent container
- [ ] Site permission decisions are per origin, not per container (a camera allowed in one container is allowed in all)
- [ ] Routing rules: renaming or deleting a Space (or container) a rule names silently turns the rule off, with no mark in the list; Space names are not unique; a Space named with `,` or `->` cannot be named by a rule; the row label does not say typed addresses and page links are not routed
- [ ] Pill and counter: no check drives the shell by pointer for the blocker menu, the Sites list revoke or the Spaces list in a running window (only the engine side is driven, `notes/hp/privtools` of the pass)

## Pop-ups and window state
- [ ] Pop-ups: an allowed site's pop-up opened without a click has no `window.opener` (the factory cannot see site permissions; pass it the decision if sign-ins need it); after a click a cross-origin frame can navigate the top page (Chrome's rule with a gesture)
- [ ] A pop-up window minimum-size design token (approved by the owner, not started): pop-up windows inherit `Chrome.windowMin` (560x380) today
- [ ] Window state restore, by hand on X11, Wayland and macOS: settling; `window.open(u,'x','width=500,height=600')` gives inner 500x600; drag by the gutters while clicks in the page's top 28 px reach the page; Cmd+W and the peeked sidebar close a pop-up; restore maximized, fullscreen on a second display, half off-screen; a no-gesture pop-up on a loading page shows the pill and Allow opens the next without a click; after a restart the last focused window is in front
- [ ] macOS: a window restored fullscreen lands on the monitor its saved position gives, not the named one (winit treats borderless-to-borderless as a no-op); a Dock Quit in the background shows the downloads question in a window that is not brought forward
