# 13 Release, updater and website

Legend: `[ ]` open · `[~]` in progress. How items move: [todo/README.md](../README.md).

How a release is made: [docs/release.md](../../docs/release.md). `0.6.1` is released for Linux x86_64 and macOS arm64;
versions are semantic from here; Windows is parked ([windows.md](../shelved/windows.md)). Not released: extensions
(they need the patched engine; [09-extensions.md](09-extensions.md)) and the npm launcher
([10-mcp-server.md](10-mcp-server.md)). The Mac build of preview.5 was made by the owner on the Mac with
`sh /Users/Shared/wubble-preview<N>-mac.sh`, then signed and uploaded from here.

## Releases
- [ ] Serve releases from our own domain (owner, 2026-11-23: "For release we can use our own domain")
- [ ] Never run on a Mac by hand (as of 2026-11-21): Cmd+Q, the socket check, Cmd+Z undo close tab (the macOS drags and the quarantine fix have their own lines)
- [ ] macOS: the Mac archive was never in the update manifest before preview.2, so a Mac on preview.1 needs one install by hand; video playback (H.264, AAC) on the Mac build is not checked (it is built on the owner's codec CEF, `/Users/ali/cef-codecs`)
- [ ] Before every release: run `cargo test --workspace` on the Mac (it passes there in CI only; preview.2 shipped with MCP hosting broken on macOS; one test there would have caught it), and run the whole suite there once now (never done)
- [ ] Release gaps: Ubuntu's user-namespace limit vs the no-sudo sandbox; rollback misses a build that cannot load;
      shellcheck not run on `install.sh`
- [ ] macOS: CEF does not finish shutting down after a windowless run over ssh; find the commit and the cause (M0)
- [ ] Security update cadence: track Chromium stable
- [ ] Packaging: Flatpak/RPM, MSI/winget, signed and notarised DMG; auto-update
- [ ] Opt-in crash reports that open a GitHub issue (scrubbed)
- [ ] Default-browser registration, URL handlers

## A more seamless updater (owner, 2026-11-23: "Updater needs an upgrade for sure, need to be more seamless")
- [ ] A more seamless updater: the follow-ups below first
- [ ] macOS: the quarantine fix for updates (`settle_quarantine`, the updater's clear before placing) is compiled but never run on a Mac: on the owner's Mac, an install updated by wubble itself must start from `/Applications` (Settings → Updates not "managed elsewhere") and offer the next update; preview.5 shipped it unchecked
- [ ] Updater: when is an old `versions/<V>` folder removed (preview.7's is still there after 0.6.1 is marked good)

## Website (wubble.cc)
`site/` is plain HTML, CSS and JS, served by a Cloudflare Worker (`site/README.md`). The owner (2026-11-23): "it looks
good right now, the only work for website is to post a lot of articles and docs to increase searchability and seo".
- [ ] Articles and docs for search ranking
- [ ] Owner: "Always Use HTTPS" for the zone (plain http serves the page without a redirect)
- [ ] Each release: bump `VERSION` in `site/assets/js/site.js` and `npx wrangler deploy` in `site/` (in docs/release.md)

## Build-out features (2026-11-22)

## Default browser: status, a Settings row, and registration on Linux, macOS and Windows (`default-browser`)

Key: `default-browser` · engine patch: no · state: not written.

Chrome shows "Default browser" in Settings with "Make default" and the current status. wubble has no code for this (TODO.md M7 "Default-browser registration, URL handlers"). The Linux desktop entry (`assets/icons/linux/wubble.desktop`) lists only text/html, xhtml, http and https. To build: (1) `wubble_core::default_browser`: an enum `Status {Unknown, Checking, Default, NotDefault, Unsupported, Failed}` and a pure `transition(state, Event::{Check, MakeDefault, Answered(..)}) -> (state, Vec<Effect::{Probe, Register}>)`, with unit tests. (2) A trait `DefaultBrowser {fn is_default(&self) -> Result<bool>; fn make_default(&self) -> Result<()>}` in `crates/wubble/src/default_browser.rs`, run on a worker thread, with one impl per OS. Linux: `xdg-settings check/set default-web-browser wubble.desktop`, plus `xdg-mime default` for the scheme handlers. Unsupported inside Flatpak (Chrome's flatpak says so too). macOS: compare `NSWorkspace URLForApplicationToOpenURL:` for `https:` against bundle id `dev.wubble.browser`, and set with `setDefaultApplicationAtURL:toOpenURLsWithScheme:completionHandler:` for http and https (macOS shows its own confirmation). Windows: write the HKCU `Software\Clients\StartMenuInternet\wubble` Capabilities, `RegisteredApplications` and a `wubbleHTML`/`wubbleURL` ProgId, then open `ms-settings:defaultapps?registeredAppUser=wubble` (Windows 10+ forbids setting it silently). (3) Settings General row: status text and a "Make default" button, built from kit parts like the Updates group's "Check now" (`ui/components/settings.slint`:604, `UpdatesView`). (4) desktop entry: Chrome's MimeType set (image/*, svg, pdf, webp, xml) and `Actions=new-window;` with `[Desktop Action new-window] Exec=wubble --new-window`. No request leaves the machine.

## Opt-in crash reports that open a GitHub issue (`crash-report-issue`)

Key: `crash-report-issue` · engine patch: no · state: not written.

Decisions "Network on its own": no telemetry; crash reports are opt-in and open a GitHub issue (TODO M7). Chrome uses Crashpad with an upload consent. wubble must not upload anything itself.

To build: (1) Capture. Add a panic hook in crates/wubble/src/main.rs that writes a crash record: version, OS, CEF version, GPU adapter (`GpuContext::describe`), the panic message, a symbol-only backtrace and the last trace stage. It goes to `<profile>/crashes/<time>.json`. For CEF's own processes, turn on CEF's crash dump writing to the same directory (CEF settings `crash_reporter.cfg` with no `ServerURL`, so nothing uploads; minidumps stay local). The run-state record already tells an unclean exit (the updater's failed-start counter). (2) A pure scrubber in wubble-core (`crash.rs`): remove the home directory and user name from paths, URLs and query strings, e-mail-like strings and profile names. It returns the issue body as text, and is unit-tested. (3) A setting `crash.offer_reports` (off by default). With it on, the next start finds the unsent records and shows a toast (kit) "wubble closed unexpectedly" with a Report button and a Dismiss button. Report opens a new tab at `https://github.com/<repo>/issues/new?title=...&body=...`, the scrubbed body in the URL and cut to GitHub's URL limit, so the user reviews and submits it; wubble sends nothing. The minidump stays local, and the body says where it is. All platforms.

## wubble.cc: feature section recomposed, readable diagrams and install capsule on phones (`site-layout-fixes`)

Key: `site-layout-fixes` · engine patch: no · state: not written.
Note: The owner calls the website finished (2026-11-23: "it looks good right now"): close this unless something in it is still wanted.

TODO lines 25-26. The site in `site/` (plain HTML/CSS/JS, `site/README.md`) shows its feature section as a list: bullets on the left, an image on the right, four times, which leaves empty left columns on wide screens. Recompose it from the owner's design pages (`xuckless/the-wallpaper-project` `src/wubble/pages/`, cloned beside the repo; pull it first). Use alternating or grid feature blocks as the design lays them out, with the same copy (no new sentences: copy is the owner's). Also fix two narrow-screen problems. (1) The inline SVG diagrams' text cannot be read at 360 px: give them a viewBox that scales, larger minimum font sizes under a `@media (max-width: 480px)` rule, or a stacked phone variant of each diagram. (2) The install command is cut off in its capsule: let the capsule scroll sideways (`overflow-x:auto`, `white-space:nowrap`, the copy button pinned) or wrap at the shell separators. Files: `site/index.html`, `site/assets/css/*.css`, the SVGs under `site/assets/`. Keep a 16 px side gutter and no horizontal page scroll at 360 px. Do not deploy: `npx wrangler deploy` is the owner's step.

