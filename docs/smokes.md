# Smokes, measurements and examples

Status (2026-11-19): every hand-run check, moved out of [AGENTS.md](../AGENTS.md). Window smokes need a GUI
session; over ssh they hang creating the window, so wrap them in `timeout`; on Linux run them on an invisible
display instead ([Headless](#headless-linux)). The Mac: [macos.md](macos.md).
Numbers from the `--measure` runs: [measurements.md](measurements.md).

## Commands

    cargo check --workspace        # first run downloads prebuilt CEF and Skia (rust-skia binaries)
    cargo build --workspace        # builds wubble-helper too; needed before cargo run
    cargo test -p wubble-core
    cargo run -p wubble
    cargo run -p wubble -- --smoke boot   # CEF boots and shuts down, no window
    cargo run -p wubble -- --smoke update   # windowless, no CEF: a signed fake release of the next version on loopback, checked, downloaded, verified and staged into a temp install root; a tampered archive refused; exits
    cargo run -p wubble -- --smoke quit   # Inspect on a selection and DevTools windows (shown, counted), one more just before the quit; clean quit; exits; run by hand only
    cargo run -p wubble -- --smoke spell   # windowless, throwaway profile, loopback `assets/spell.html`: a misspelt word ("teh") typed into a focused textarea is right-clicked until Chromium's menu names it with suggestions ("the" among them: the bundled en-US dictionary, `wubble_engine::dictionary`); then the net log (`log-net-log`) must hold the page's own loopback request and no `gvt1.com` or `.bdic` URL (the other hosts Chromium asked for, refused at name resolution, are listed); exits; run by hand only
    cargo run -p wubble -- --smoke frames=300   # window shows 300 zero-copy frames, exits; run by hand only
    cargo run -p wubble -- --smoke nav   # window navigates between two bundled pages, exits; run by hand only
    cargo run -p wubble -- --smoke tabs   # window opens, switches, sleeps, restores and closes tabs, exits; run by hand only
    cargo run -p wubble -- --smoke windows   # a second window, a tab moved into it, both checked, closed; a Little window promoted; exits; run by hand only
    cargo run -p wubble -- --smoke split   # window splits two bundled pages, checks each pane, unsplits, exits; run by hand only
    cargo run -p wubble -- --smoke pinch   # loopback `pinch.html`: a trackpad pinch through `Host::pinch` (the gesture events' entry): a page keeping the Ctrl+wheel stays at scale 1, Ctrl + zooms the page and not the visual scale, a new page starts at 1, then split: the pinched pane's `visualViewport.scale` grows past 1.5 with its layout width and page zoom unchanged, no click, selection or pressed element, hover back, `pointer: coarse` never matched, the other pane at 1; pinched out: exactly 1; `--shot PNG` shows it zoomed; prints what touch left on the page; exits; run by hand only
    cargo run -p wubble -- --smoke page   # window drives a select popup, an alert, a context menu, find and zoom, exits; run by hand only
    cargo run -p wubble -- --smoke session   # two tabs saved, restarted and restored on a temp profile; two windows, exits; run by hand only
    cargo run -p wubble -- --smoke passwords   # wubble://passwords on a temp profile, three runs: add, edit, reveal, delete, leave (locks) and add again; restart, unlock, the same logins; a keyring that cannot be reached says so and saves nothing. The key lives in a file keyring on the profile; WUBBLE_SMOKE_KEYRING=os uses the OS one for the first two runs: only inside a private `dbus-run-session` with a throwaway gnome-keyring, never the desktop's; run by hand only
    cargo run -p wubble -- --smoke start   # Ctrl+T's start page: its bubble's pixels, a poke, a pop, the capsule's command bar, a tile; `--shot-dir DIR` for one PNG a step; exits; run by hand only
    cargo run -p wubble -- --smoke settings   # window opens wubble://settings, flips a setting through its toggle, reads the store back, exits; run by hand only
    cargo run -p wubble -- --smoke autofill   # a corpus of loopback sign-in pages (GitHub, WordPress, Google and Amazon two-step, Microsoft same-page, Reddit shadow DOM, React, sign-up, change password, formless, late, iframes, honeypot, one-time code, picker), fake keyring: per fixture the chip's rows, a pick, the page's own check of the fill, a trusted submit, the save or update prompt, the vault rows; one `smoke: autofill <fixture> ok` line each; exits; run by hand only
    cargo run -p wubble -- --smoke autofill=probe --url URL   # a real sign-in page, read only, fake keyring and a fake login for its origin: the autofill trace (frame origins, form classes, field kinds, never values) and what the chip would offer; nothing typed or submitted; exits; run by hand only
    cargo run -p wubble-engine --example block_smoke   # windowless: the request-filter hook cancels a script and runs the commit script (a stand-in filter, not wubble-blocker), exits; run by hand only
    cargo run -p wubble-engine --example gpu_crash_smoke   # windowless: kills the GPU process (chrome://gpucrash), frames must resume; run by hand only
    cargo run -p wubble -- --shot-dir DIR --smoke gallery   # walks the shell through its surfaces on a seeded profile, one PNG each; run by hand only
    cargo run -p wubble-engine --example cookie_smoke   # windowless, two processes on one temp profile: a container's tabs share cookies (persistent and session) and localStorage, other containers see none, they survive a restart, clearing and forgetting a site work; prints `smoke: cookies ok`; run by hand only
    WUBBLE_CODEC_CLIPS=DIR cargo run -p wubble -- --smoke codecs   # windowless: which codecs and media sources play with the browser's blocker, observer and kit (`codecs=bare`: without them); clips from scripts/codec-clips.sh DIR (none: the bundled clip only); exits
    cargo run -p wubble -- --smoke extensions   # windowless, throwaway profile: the bundled test extension installed as a directory and as a .crx (twice; the start-time sweep must leave only the copy the list names), a loopback page shows both content-script marks with their service workers' answers, the ids are wubble-core's, the workers are listed by the DevTools endpoint; with `CEF_PATH` on a CEF built with `patches/chromium` it also checks the tab APIs (`windows.getCurrent/getAll/getLastFocused`, `tabs.query`, `tabs.create` answered with a new view and its `onUpdated` `complete`, an active `tabs.create` and its `onActivated`, `tabs.create({})` opening the start page, all served by the host's own request code on a model, `tabs.sendMessage`, the badge `7`, an action click setting `hit`) and prints `tab APIs ok`; on an engine without the patches (the release's codec engine) it says extensions are not available in this engine and passes once nothing was loaded, no extension process runs and the footer button's rule says hidden; `extensions-none`: nothing installed, no mark, no extension target; exits
    cargo run -p wubble-engine --example local_probe   # windowless: localhost, 127.0.0.1, [::1] and the LAN address load with fetch, cookies, WebSocket and a service worker; exits
    cargo run -p wubble -- --smoke pages   # opens history, bookmarks, downloads, passwords on a seeded profile, exits; run by hand only
    cargo run -p wubble -- --smoke data   # the built-in pages and their data against a loopback server (`data_smoke.rs`, `host/data.rs`), two runs on throwaway profiles, one ending in a quit and one in a SIGKILL, each followed by the database compared with what the pages showed: visits recorded (title, time), search in any script's case, a row deleted, bookmarks (sheet, folder, duplicate, rename), clearing history keeps bookmarks, clearing cookies and cache (the cookie no longer reaches the server, the cached page is fetched again), downloads (progress, pause, resume, cancel, retry, a cut-off one, a missing and a read-only folder, Open and Show in folder through a logging stand-in `xdg-open` and no session bus), the archive (setting, filed, restored, deleted), the Task manager, the agent activity page updating while open, every Settings Pages link; run by hand only; it sets the child's `PATH` and `DBUS_SESSION_BUS_ADDRESS`, so never run `data-phase=` directly
    cargo run -p wubble -- --smoke ax   # a11y.web on: the page's accessible items, a click and a field's value through them, exits; run by hand only
    cargo run -p wubble -- --smoke mcp   # agent apps in a scratch home under the profile (Codex without a file, opencode with a comment, VS Code), then hosting turned on through its setting at run time (`agent.mcp_enabled`): wubble added to Codex and opencode with a backup, VS Code untouched, Remove and Add on Codex; then: an MCP client on the profile's socket drives every tool on loopback pages (`mcp_smoke.rs`), one line per step: discover, tools/list (20), an earlier-revision client on its own connection (`initialize` at 2025-11-25, its list and a call in its shape); approval sheets answered by pointer presses as listed in `mcp_smoke::SHEETS` (Deny; Allow headless on 127.0.0.1; two request_more sheets showing only what is new: Deny, then a site typed in the Sites draft, Escape through the host's key path (must drop the draft only), Allow; Windowed then Allow); navigate (spelt HTTP://), read_page (long text and a link whole with its target, a cut list noted, a stale ref an error), list_tabs (active, shown), isolation (other tab handles refused by five tools), click, type, evaluate (statements, return, promises, await, a node named, a throw with its stack, timeouts), wait_for (field values, text gone, an in-page navigation, url), type submit (waits for the next page; no form: says no load followed), press_key (Alt+ArrowLeft back, Control+L, Control+Shift+T refused), a GitHub-like sticky bar's links clicked at two scrolls, screenshots cropped to an element at scroll 0/50/150/300 (colour checked) and of one far down, full page, hover, drag in a canvas by its coordinates and between buttons, coordinates outside a canvas refused, scroll (into view, the wheel up and down), read_text (an article, capped, one element), safety (file: and chrome: by location, window.open, a server's redirect, navigate: no tab off the web), sites (a link, `location.href`, a form, `window.open` and a server's redirect to a.localhost, not on the session's sites: each refused before it is sent, the call's error naming the host and request_more, the tab where it was; once request_more allows a.localhost the link goes), open_tab waits for its load, request_more, stop_session keep_tabs (the tab stays, the owner's; the handle is dead), focus_tab; sign-ins (the owner's first tab signed in to signin.localhost and other.localhost by the server's cookies; a session on signin.localhost with "Use my sign-ins" switched on by pointer and both secret names bound to the seeded login: the server sees the copied cookies, the httpOnly one too, which the page's script does not; fill_secret types the username and password there, every reply and the activity log carry `[secret]`, never a value; request_more for other.localhost, unshared: its page gets no cookie and fill_secret is refused there; the host checks 2 of 3 owner cookies copied, the owner keeps both, the copy logged, the stopped session's container gone); a second session left for the quit (its two tabs switched in the window, each back at its pane's size) and a third, headless; the Spaces popup's dustbins pressed by pointer (the kept Space: gone; the third session's: it alone stops, its Space gone; the user's Space has none; the popup stays open); then hosting turned off through its setting with the second session running (it ends, its Space closes, the socket and the manifest go, a new connection is refused, wubble leaves the apps' files, opencode's back to the same bytes); the user's closed tabs and history hold nothing of the agent's; exits; run by hand only
    echo '{"jsonrpc":"2.0","id":1,"method":"server/discover"}' | cargo run -p wubble -- mcp --profile DIR   # the stdio relay a harness starts; no browser on DIR: an MCP error saying so
    cargo run -p wubble-engine --example ax_smoke   # windowless: CEF's accessibility tree of a bundled page, mapped and printed (`-- --dump FILE` writes the JSON), exits; run by hand only
    cargo run -p wubble -- --smoke media   # loopback page: detects a file, HLS (plain + AES-128) and a DRM stream, saves three, checks size and SHA-256, exits; run by hand only
    cargo run -p wubble -- --smoke overlay   # green/red page: menu, command bar and peek glass frost the page, the gutter takes its light, no page blur while nothing is over it; then the overlays' rules (Print replaces Add bookmark, Escape closes each kind topmost first, a tab switch closes the sheet and keeps the find bar on its own tab, a navigation closes it); `overlay=dark`, `--shot-dir DIR` for one PNG a step; exits; run by hand only
    cargo run -p wubble -- --smoke builtins   # page dark mode, reader, link hints, screenshots, the PDF viewer on bundled pages, exits; run by hand only
    cargo run -p wubble -- --smoke video   # loopback video page: controls, speed per site, SponsorBlock against a fake endpoint, a media key, PiP spike line, the Pip window opened, returned and closed, the strip's save button (absent with nothing to save and on a ClearKey-protected video, a click saves the clip through the media downloader); exits; run by hand only; its pointer finds the strip by element ids, so a release build (`--release`, `--profile dist`) runs it only when built with `SLINT_EMIT_DEBUG_INFO=1`
    cargo run -p wubble -- --smoke boost   # loopback pages: a Boost's CSS, tint and document-start script, a CSP page, turned off; exits; run by hand only
    scripts/build-cef.sh WORK   # Linux x64: CEF 156.0.30 with proprietary codecs into WORK (~38 GB, ~2.5 h, no root); run by hand only
    CEF_PATH=WORK/cef_path cargo build --workspace   # wubble on that CEF (H.264, AAC, HLS: `--smoke codecs` then asserts them); unset CEF_PATH for the prebuilt
    cargo run -p wubble -- --measure load   # windowed, release: loopback pages cold and warm, in-page timings and first frame; `--load-variant`, `--runs`, `--load-page`, `--load-back`, `--serve-only`; `scripts/measure-load.sh` (a process per cold sample) and `scripts/measure-load-chromium.sh` compare plain Chromium; hand run only
    cargo run -p wubble -- --measure memory --out mem.csv   # process tree memory with 0/1/10/50 tabs, frozen, discarded; window; run by hand only
    scripts/mem-compare.sh chromium-browser 10   # another browser's PSS sum on the same page; opens its window; run by hand only
    cargo run -p wubble -- --smoke parity   # windowless-mode gaps as expected-fail rows (a gap that passes fails); owns the window's input while it runs; run by hand only
    cargo run -p wubble -- --smoke block   # bundled page, injected list: two blocked, one `$redirect` stub that must run; run by hand only
    cargo run -p wubble -- --measure scroll   # also video, resize, typing: per-frame paint, copy and render times (`--seconds N`, `--out CSV`); hand run only
    cargo run -p wubble -- --measure idle --seconds 20   # still page after a warm-up (`--warmup N`): renders, timers, process CPU; hand run only
    cargo run -p wubble -- --measure startup   # stages from `main` to first page pixels; hand run only
    cargo run -p wubble -- --measure blocker   # compile and cache-load cost of the default lists; hand run only
    cargo run -p wubble-engine --example frame_smoke   # windowless zero-copy and colour check
    cargo run -p wubble -- example.com   # the browser on a page (the bundled one without a URL)
    cargo run -p wubble -- --profile DIR   # the browser on another profile (database + CEF cache)
    cargo run -p wubble -- --downloads DIR   # downloads go to DIR instead of the OS downloads folder
    cargo run -p wubble -- --no-sandbox   # CEF without its sandbox, debugging only (warned at start)

## The gallery

`--shot-dir DIR --smoke gallery` (`crates/wubble/src/gallery_smoke.rs`, `host/gallery.rs`) writes
`DIR/NN-name.png` per scene in the order of `gallery_smoke::SCENES`: sidebar extended, rail and hidden;
a tab row and the new-tab row hovered; the rail's sound badge (playing, muted, the row hovered, the badge hovered); an agent's tabs on the rail; the Spaces popup with the user's Space, a running session's and a kept agent Space (their dustbins), then the pointer on a dustbin; the command bar opening (a burst: `DIR/NN-name/fKKK.png` with
`times.txt`), empty and with results; tab-row menu; site menu; downloads popover (and a row hovered);
find bar; permission prompt (and Block hovered); autofill menu (and a login hovered) and save prompt;
dialog sheet and prompt; bookmark sheet; print sheet; toast; link status bubble; error and certificate
error pages; over a plain white page (`assets/ground.html`) the sidebar, page menu, tab menu, command bar,
find bar, bookmark sheet, reload tooltip and peeking sidebar, and over its dark variant the menus, command
bar and find bar (the glass's contrast); Settings at four scroll positions and its section list scrolled to its end; Settings' list editors (routing rules with a row
selected, a new rule's draft, Spaces with the second selected, Sites); Settings' Agents (MCP) section (hosting is on from the run's start, with Claude Code, Codex and VS Code in a scratch home under the profile, never the owner's); history, bookmarks, downloads,
passwords (locked, unlocked), archive, task manager; an agent's approval sheet, its sheet asking for more (`request_more`: only what is new), an approved agent's marked tabs and Space, the agent activity page; the start page; a history row hovered (its
actions shown); the rail with its tab hovered (the close badge). The agent widgets' specimen: every thinking orb state at 16, 20, 24, 32 and 64 on glass, the library's stage, its paper and the page (white, then dark), at 160, paused, and a burst of the large ones moving (`--frozen-backdrop` holds the orbs' clock at 2 s).

`WUBBLE_SEED_SCALE=5000` seeds that many extra visits and bookmarks (a tenth as many downloads, half as many archived tabs) for a hand look at long lists; the smoke's own counts then do not hold, so use it with `--smoke gallery` only.

Add a surface by adding a `Scene` row (data over existing host state, never `AppWindow` properties:
the host rewrites those). Size and scale come from the environment: `WUBBLE_SHOT_SIZE=900x600`
(logical; the design's 1360x880 by default) and `SLINT_SCALE_FACTOR=2` (the PNG is then physical, 2x).
Surfaces that need a live page (split panes, DevTools docked, video strip, reader, boost editor,
Little and Popup windows) are not scenes: use `--shot PNG` with their own smoke. Compare each PNG by
eye with its design counterpart (`browser/*.css`, `pages/*.css` in the design repo) before UI work is
handed back.

## Headless (Linux)

`scripts/headless.sh <command...>` runs a command on an invisible Wayland display, so a smoke's
window never shows on the desktop or takes its focus; prefix every window smoke, `--shot` and
`--shot-dir` run with it, e.g. `scripts/headless.sh timeout 240 target/debug/wubble --smoke page`.
Each call starts its own headless mutter (1360x880 virtual monitor; `WUBBLE_HEADLESS_SIZE`) on a
private D-Bus session that starts no services, and tears it down when the command exits, so
parallel callers never share focus or a clipboard and nothing reaches the desktop's display,
clipboard, D-Bus session or settings (`WUBBLE_HEADLESS_LOG=FILE` keeps mutter's output). The
command keeps the caller's environment apart from `WAYLAND_DISPLAY`, `DISPLAY`, `XAUTHORITY` and
`XDG_SESSION_TYPE`. Needs mutter, dbus-run-session, python3-gobject and GTK 4 (Fedora Workstation
has them). What the display adds to a bare `mutter --headless`, and why:

- A keyboard: a RemoteDesktop session on the private bus adds mutter's virtual input devices and
  presses Shift once. Without one no window gets keyboard focus or an input serial, and Wayland
  lets only the focused client set the selection: every clipboard check failed ("clipboard holds
  None"). The pointer is never moved, so screenshots show no hover.
- A ticker: a small software-drawn GTK window drawing every frame under the command's windows.
  Mutter 50's headless frame clock can park ("scheduled-later") on a commit that carries a
  commit-timing timestamp (NVIDIA's Vulkan and EGL presents send `wp_commit_timer_v1` with
  `wp_fifo_v1`) after an empty frame, and never wake: the window then gets no frame callback and
  stops drawing (`--smoke start` stuck at Poked, `--smoke page` at PopupShown/PopupFrame). A
  desktop always has something else damaging the stage. `CLUTTER_PAINT=continuous-redraw` does
  not help on mutter 50.
- Xwayland, mutter's own: Copy Image goes through arboard, which on GNOME (no data-control
  protocol) falls back to X11; without a `DISPLAY` `B.copy-image` failed after X11 timeouts.

Frames stay GPU-accelerated there (`--smoke frames=300`: accelerated=300 cpu_fallback=0, 16.7 ms).

Every `--smoke` on `scripts/headless.sh`, 2026-11-19, debug build of `qa1/headless`, three runs
each, three at once (each its own compositor):

| Smoke | Runs | Smoke | Runs | Smoke | Runs |
|---|---|---|---|---|---|
| boot | 3/3 | session | 3/3 | ax | 3/3 |
| quit | 3/3 | start | 3/3 | media | 3/3 |
| frames=300 | 3/3 | settings | 3/3 | overlay | 3/3 |
| nav | 3/3 | autofill | 3/3 | overlay=dark | 3/3 |
| tabs | 3/3 | gallery (`--shot-dir`) | 3/3 | builtins | 3/3 |
| windows | 3/3 | codecs | 3/3 | video | 3/3 |
| split | 3/3 | codecs=bare | 3/3 | boost | 3/3 |
| page | 3/3 | pages | 3/3 | block | 3/3 |
| parity | 3/3 (11/12 more) | | | | |

Known flake: `--smoke parity` fails about one run in six on this display (7 of 40), always early in
`A.type-input` (its first item) or `A.pagedown-scrolls`, and never with the keyboard left out (0 of
18): the page sees every keydown but not its default action (no keypress, nothing typed; no
scroll). Cause not found yet. Also fixed on the way: `--smoke autofill` failed about one
run in five (the submitting page's own load report read as a failed sign-in), and parallel
`--smoke parity` runs stalled three minutes (the loopback server answered one connection at a time).
