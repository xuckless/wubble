# Website and release

Part of the roadmap ([TODO.md](../../TODO.md)). Legend: `[ ]` open · `[~]` in progress. Done items are removed: `git log` has them.
How a release is made: [docs/release.md](../release.md). Shipping milestones (packaging, import, crash reports):
[milestones.md](milestones.md) "M7: Ship".

## Releases
`0.6.1` is released for Linux x86_64 and macOS arm64; versions are semantic from here; Windows is parked
([windows.md](windows.md)). Not released: extensions (they need the patched engine, which the release engine is not;
[milestones.md](milestones.md) "M6") and the npm launcher ([mcp-server.md](mcp-server.md)). The Mac build of
preview.5 was made by the owner on the Mac with `sh /Users/Shared/wubble-preview<N>-mac.sh`, then signed and uploaded from here.
- [ ] macOS: the quarantine fix for updates (`settle_quarantine`, the updater's clear before placing) is compiled but never run on a Mac: on the owner's Mac, an install updated by wubble itself must start from `/Applications` (Settings → Updates not "managed elsewhere") and offer the next update; preview.5 shipped it unchecked
- [ ] Never run on a Mac by hand (as of 2026-11-21): Cmd+Q, the socket check, Cmd+Z undo close tab (the macOS drags and the quarantine fix have their own lines)
- [ ] Updater: when is an old `versions/<V>` folder removed (preview.7's is still there after 0.6.1 is marked good)

## Website (wubble.cc)
`site/` is plain HTML, CSS and JS, served by a Cloudflare Worker (`site/README.md`); it went live as first built, with
README wording as placeholder copy.
- [ ] The copy and what the page says first: with the owner (the sentences not from README are listed in `site/README.md`'s history; nothing invented, no numbers without a measurement)
- [ ] Reshoot the app shots on a clean profile (the Space is named "Agent: draft the issue", one shot shows a test page), and put a real site in the window under the hero in place of a second start page
- [ ] The feature section reads as a list (bullets left, image right, four times; empty left columns on wide screens): recompose it from the design's pages
- [ ] The SVG diagrams' text is unreadable at 360 px; the install command is cut off in its capsule
- [ ] Owner: "Always Use HTTPS" for the zone (plain http serves the page without a redirect)
- [ ] Each release: bump `VERSION` in `site/assets/js/site.js` and `npx wrangler deploy` in `site/` (in docs/release.md)
