# Hardening: built-in pages and their data

Shelved as on demand by the owner (2026-11-23): "Hardening and review are on demand. And close them as required. Hopefully the redux system would eliminate state errors".

Legend: `[ ]` open. Picked up only when the owner says so; how items move: [todo/README.md](../README.md).

Open findings of the hardening pass (2026-11-21; checklist in its report). Review findings: [review.md](review.md);
private windows: [02-tabs-windows.md](../upcoming/02-tabs-windows.md); containers: [06-privacy-security.md](../upcoming/06-privacy-security.md).
- [ ] Task manager on Linux with the sandbox: each renderer is pid 1 in its own PID namespace, so a tab cannot be tied to its renderer (tab rows have no memory, CPU or End) and renderers are listed as "Zygote"; needs a way to ask CEF's browser process for a tab's renderer pid
- [ ] List pages (History, Bookmarks, Downloads, Archive): a row cannot be opened from the keyboard (only its Delete is a Tab stop; a screen reader's default action opens it); no select, delete selection, undo, copy address or keyboard open in a new tab; a bookmark folder is deleted with its contents by one press; needs a focusable kit row and a design for selection and undo
- [ ] Media downloader: the Mac `wubble-ffmpeg` (scripts/release/build-ffmpeg.sh, signed in package-macos.sh) is written, never run: build and notarise it on the Mac, then try a YouTube pair and an HLS download in the app
- [ ] Media downloader: YouTube's player API answered "Sign in to confirm you're not a bot" to the `android_vr` client from the dev box (shown to the user now, not worked around); its format URLs' own fetch is untried; an owner call: allow one 1 MB ranged fetch from a free video, and say which sites the owner tried
- [ ] Media downloader: a DASH manifest of several periods is refused ("Not supported"); the default quality takes the first row of a height whatever its codec (HEVC or AV1 before H.264)
- [ ] Downloads: Retry restarts from any tab of the download's container, not the tab it came from (a login-gated file can come back as a sign-in page)
- [ ] Agent sessions' pages are filed in the profile's icon table (origin and icon) though their visits are not history; the agent activity log cannot be cleared
- [ ] `--smoke data` runs each process on its own CEF cache: a cookie cleared right before a SIGKILL and read after the next start is not checked
- [ ] `--smoke data` step 52 (a download cut off after 8 KiB) counts pump runs and now takes about 8 minutes with the Quiet pump: wait on a clock
