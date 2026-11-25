# 05 Media and downloads

Items of this area that need an engine rebuild: [engine-build.md](engine-build.md#media-and-downloads).

Legend: `[ ]` open · `[~]` in progress. How items move: [todo/README.md](../README.md).

Discarded by the owner on 2026-11-23 (not shelved): the Widevine CDM loader, Cast and live captions. HDR is shelved, and so is DRM through a WebKit tab: [engine-build.md](../shelved/engine-build.md).

## From the old roadmap (M1)
- [ ] Page context menu: the Picture in Picture row; media rows act through a page script (no `MediaPlayerAction`)
- [ ] Media session (metadata, action handlers, OS controls); its real audible signal is `audible-signal-patch` in [engine-build.md](engine-build.md#media-and-downloads)

## Build-out features (2026-11-22)

## Drag a finished download to another app (`download-drag-out`)

Key: `download-drag-out` · engine patch: no · state: not written.

In Chrome a finished download in the bubble or on chrome://downloads can be dragged out as a file: to the desktop, a file manager, a mail composer or a chat app. wubble's rows cannot be dragged. wubble already starts OS drags of page content: Wayland in crates/wubble/src/wl (wl_data_source with text/uri-list), macOS in wl/macos.rs (AppKit dragging session), with the type table in wubble_core::dnd.

What to build: (1) wubble_core::dnd: a pure `fn file_offer(path) -> Vec<(mime, bytes)>` giving `text/uri-list` (file:// URI, RFC 3986 escaped), `text/plain` (the path) and, on macOS, `public.file-url`, with unit tests. (2) UI: the popover row (kit/overlay/a/a-download-row.slint) and the page row (download-row.slint) report a press-and-move past the drag threshold on a complete, not-removed row as a `drag-out(id, x, y)` callback. Use the existing pointer-threshold logic if the kit has one (tab drag uses wubble_core::tabdrag), else add it to core as a pure helper. (3) Host: start the platform drag with the file offer and the file-type glyph as the drag image, through the same entry point page drags use (host/drag.rs), with the operation copy. Windows stays a stub like the other drags (wl::Null). (4) Dropping back onto wubble's own window is ignored. Linux Wayland and macOS; X11 only if the existing drag path covers it.

## Dangerous and insecure download warnings (Keep / Discard) (`download-danger-warnings`)

Key: `download-danger-warnings` · engine patch: no · state: not written.

Chrome holds a download it considers dangerous until the user picks Keep or Discard. It uses a file-type policy (chrome/browser/resources/safe_browsing/download_file_types.asciipb, BSD-3: per-extension danger level per platform, e.g. .exe/.msi/.scr on Windows, .dmg/.pkg/.app on macOS, .deb/.rpm/.sh/.desktop/.AppImage on Linux), insecure downloads (an http: file started from an https: page) and downloads started with no user gesture. wubble has none of this: wubble-core/src/download.rs DownloadState has no dangerous state, and host/downloads.rs gives every download its final path at once. Safe Browsing stays out, because it would contact a server.

What to build: (1) Port the policy table as data into a new module, wubble_core::download::danger, with a pure `fn danger(name, url, initiator_url, platform) -> Danger { Safe, DangerousFile, Insecure }` and unit tests. (2) Add `DownloadState::Dangerous(Danger)` plus the events `Keep` and `Discard` to the transition table, and update the exhaustive pair test. (3) In host/downloads.rs `download_event` DownloadRequested: when the file is dangerous, answer CEF with an intermediate path, `Unconfirmed <n>.crdownload`, in the downloads folder, reserve the real name, and keep a `held: Option<PathBuf>` on Inflight. On Finished, go to Dangerous instead of Complete. Keep renames the file to its reserved name and moves to Complete; Discard deletes the file and forgets the entry. (4) Store: a migration in crates/wubble-store/sql for a danger column, read in wubble-store/src/downloads.rs. (5) UI: DownloadStatus.dangerous in model.slint, and a warning line plus Keep and Delete buttons in kit/overlay/a/a-download-row.slint and download-row.slint. This is a derived kit piece and goes on docs/fidelity/derived-review.md. (6) The rows open the popover by themselves, as Chrome's bubble does. All platforms.

## Automatic (multiple) downloads site permission (`download-multiple-permission`)

Key: `download-multiple-permission` · engine patch: no · state: not written.

When a page starts a second download with no gesture, Chrome's DownloadRequestLimiter asks "This site is trying to download multiple files" (Allow / Block). The answer is remembered per site, and Site settings has an "Automatic downloads" row. In CEF 156 this arrives as CEF_PERMISSION_TYPE_MULTIPLE_DOWNLOADS (1 << 14) on OnShowPermissionPrompt. In wubble the bit is broken today: vendor/welding/src/permissions.rs does not name it, so it decodes to PermissionKind::Other(16384), and wubble_core::site::Permission denies every request naming an unknown kind. A site's second automatic download is therefore always refused, silently. Separately, welding's `can_download` (vendor/welding/src/handlers.rs ~1081) ignores its url argument.

What to build: (1) In welding, mark the changes `wubble change` and list them in WUBBLE_CHANGES.md. Add MULTIPLE_DOWNLOADS = 1 << 14 and `PermissionKind::MultipleDownloads`, with a decode test. Pass the url and the browser id from can_download through a host hook (a `CanDownload` question answered synchronously from a host-set table), so a per-site Block takes effect before anything starts. (2) In wubble-core/src/site.rs, add `Permission::AutomaticDownloads` (stored name "automatic-downloads") to ALL, with the prompt wording "Download multiple files" and tests. (3) In crates/wubble-engine/src/page.rs, map the new kind in the engine permission mapping. In the host's permission routing, use the existing permission prompt (components/permission-prompt.slint). (4) Add the row to Settings' Sites list, where it can be revoked like the others. (5) Agent-driven tabs keep refusing, as today. All platforms.

## Open when done, always open this type, open safe files (`download-auto-open`)

Key: `download-auto-open` · engine patch: no · state: not written.

Chrome lets a running download be marked "Open when done". On a finished one it offers "Always open files of this type", stored per extension (download.extensions_to_open) and refused for dangerous types. The design's Settings (ref/the-wallpaper-project/src/wubble/pages/settings.tsx, `openSafe`) also has "Open safe files after downloading". wubble opens a file only on a click of Open (host/downloads.rs download_action DownloadAction::Open).

What to build: (1) wubble-core/src/download.rs: an `open_when_done: bool` on DownloadEntry, a pure `fn opens_on_finish(entry, always_types, open_safe) -> bool` (never for a dangerous type, which comes from the danger table if that feature has landed, else from a small built-in list of executable extensions), and a `DownloadEffect::OpenFile` emitted on the Finished transition when it returns true. Write unit tests. (2) Settings: a `downloads.open_types` list setting (Section::Hidden, edited from the row menu) and a `downloads.open_safe` Bool row, default off, in wubble-core/src/settings.rs, with SETTINGS-table tests. (3) Host: run OpenFile with `opener::open`. Persist open_when_done in a store migration, so it survives a restart for a resumed item. (4) UI: DownloadAction gets `open-when-done` and `always-open-type`. A small row menu on the popover's ADownloadRow and the page's DownloadRow lists Open when done (checked), Always open files of this type, Copy link and Show in folder. Compose it from the kit menu (kit/controls/menu.slint) and put it on derived-review.md. All platforms.

## Downloads outlive their tab; warn on quit with downloads running (`download-lifetime`)

Key: `download-lifetime` · engine patch: no · state: not written.

In Chrome a download keeps going after its tab closes. Quitting while one runs asks "N downloads in progress. Cancel downloads and quit?". wubble cancels instead: host/downloads.rs `downloads_closing` cancels every running download of a closing tab (toast "Download cancelled: its tab closed"), because the CEF download callbacks belong to the view. Quit does not check for running downloads.

What to build: (1) Parked views. When a tab with running downloads (`Downloads::holds`) closes, its wubble_engine::view::View moves out of `self.views` into a `parked: HashMap<DownloadId-owner, View>`: hidden (`was_hidden`), muted, navigated to nothing new, and absent from the model, from extensions and from MCP. The view is closed when its last inflight download ends, which is the one place in `apply_download` where inflight is removed. Pause, resume and cancel reach the parked view through `running()`. The sleep policy and memory reports count it. (2) Quit. Add a pure `wubble_core::download::quit_check(entries) -> Option<usize>`. The quit path (Action::Quit and the window close in host/window.rs, plus Cmd+Q on macOS) opens the existing dialog sheet (components/dialog-sheet.slint, DialogView): "Cancel N downloads and quit?" with Quit and Keep downloading. Media download jobs (host/media.rs) count too. (3) Tests: a unit test for quit_check, and a host test that closing a tab keeps its download's entry Running. All platforms.

## Download button progress ring and auto-opening bubble (`download-button-progress`)

Key: `download-button-progress` · engine patch: no · state: not written.

Chrome's toolbar download button shows a ring that fills with the combined progress of the running downloads. It animates when a download starts or ends, and the download bubble opens by itself on a new download for a few seconds, showing that row. In wubble the sidebar's downloads IconButton (kit/chrome/side-bottom.slint, visible only while there are downloads) carries no progress, and a new download only raises a toast ("Downloading <name>", host/downloads.rs DownloadStarted and add_download_entry).

What to build: (1) wubble-core/src/download.rs: a pure `fn aggregate(entries) -> Option<f32>` (the received/total sum over running entries with a known size, else None for indeterminate) and an `unseen_complete` flag: true after a finish until the popover opens. Unit tests. (2) An app.slint property `downloads-progress` (float, -1 for none) and `downloads-unseen` (bool), set in render_downloads. (3) side-bottom.slint: the download button gets a progress ring and a dot. The design has no ring, so derive it from the kit's progress parts (kit/pages/list-progress.slint, AProgress) and put it on docs/fidelity/derived-review.md. If a new primitive is needed, it waits for the owner. (4) Auto-open: a new download opens the downloads popover (`downloads-open`) anchored to the button. It closes after a few seconds unless hovered, and replaces the toast. The timing is one enum and a pure transition in core. An agent-tab download never opens it. In compact or collapsed sidebar mode, fall back to the toast. All platforms.

## Downloads page and popover rows: Chrome parity (`downloads-page-parity`)

Key: `downloads-page-parity` · engine patch: no · state: not written.

chrome://downloads groups rows by date. Each row shows the source site as a link, a file whose file was moved or deleted reads "Deleted" with Open turned off, and a header button opens the downloads folder. Each row also has Copy download link, and a click on a finished row opens the file. In wubble, components/pages.slint DownloadsPage and download-row.slint show one flat list with name and detail, a row click does nothing, and a missing file is noticed only when Open fails ("File not found" toast in host/downloads.rs).

What to build: (1) Core (wubble-core/src/download.rs): add `fn groups(entries, today_ms) -> Vec<(label, Vec<id>)>`, with Today, Yesterday, then the date, reusing the History page's day-label helper if one exists in wubble-core. Add `DownloadEntry::removed: bool`, set by the host from a `path.exists()` check when the list renders (cheap, no watcher), and `can_open` returns false when removed. Add the source host for the second line. Unit tests. (2) Store: an end time column in a migration (Chrome shows it), set on Complete. (3) UI: DownloadItem gets `source`, `removed` and `group`. Add an Open downloads folder button in DownloadsPage's header, open on a row click (Enter too) for complete rows, and a Copy link action. Use the page kit's group headers, as the History page already does. (4) Host: DownloadAction `copy-link` (clipboard via crates/wubble/src/clipboard.rs) and `open-folder`. The list keyboard gaps already in TODO stay with that item. All platforms.

## Download progress on the Dock, taskbar and launcher (`download-os-progress`)

Key: `download-os-progress` · engine patch: no · state: not written.

Chrome shows download progress outside the window. On macOS the Dock tile gets a progress bar and the count of running downloads, and the icon bounces once when one finishes. On Windows the taskbar button shows progress (ITaskbarList3::SetProgressValue / SetProgressState). On Linux it uses the Unity launcher API (DBus `com.canonical.Unity.LauncherEntry` signal `Update` with `progress`, `progress-visible`, `count`, `count-visible`), which GNOME's Dash to Dock, KDE's task manager and Plasma read. wubble shows nothing outside its window.

What to build: (1) Core: a pure `fn os_badge(entries) -> OsBadge { count, progress: Option<f64>, finished_since_last: u32 }` in wubble-core/src/download.rs, with unit tests. (2) One shared trait, `trait DownloadBadge { fn show(&mut self, b: &OsBadge); }`, in crates/wubble/src (e.g. a new os_badge.rs). Implement it per platform. Linux: send the LauncherEntry signal on the session bus with the app URI `application://wubble.desktop` (match the .desktop name the installer writes), using the DBus crate souvlaki already pulls in, if any. macOS: NSDockTile with a custom content view drawing the bar, badgeLabel, and requestUserAttention(informational) on a finish, via objc2 (already used by wl/macos.rs). Windows: ITaskbarList3 through the windows crate, behind cfg. (3) Host: call it from render_downloads only when the badge changed. Media jobs count. Nothing is sent while there are no downloads.

## Retry and Save As from the originating tab with its referrer (`download-retry-origin`)

Key: `download-retry-origin` · engine patch: no · state: not written.

TODO.md: "Downloads: Retry restarts from any tab of the download's container, not the tab it came from (a login-gated file can come back as a sign-in page)". Chrome retries with the original request's referrer and site context. wubble's host/downloads.rs `apply_download` StartAgain calls `view_for_download(container)`, which takes the shown tab or any live tab of the container. It then runs `View::start_download(&url)`, which sends no referrer.

What to build: (1) DownloadEntry (wubble-core/src/download.rs) gains `tab: Option<TabId>` (the tab that started it) and `referrer: String` (the page's committed URL when it started), stored through a migration in crates/wubble-store/sql and wubble-store/src/downloads.rs. (2) Core: a pure `fn retry_view(entry, live: &[(TabId, ContainerId, agent: bool)], shown) -> Option<TabId>` that prefers the origin tab, then a tab of the same container on the referrer's site, then any tab of the container, never an agent-driven tab. Unit tests. (3) Engine: `View::start_download_with_referrer(url, referrer)`. CEF's StartDownload has no referrer, so use a CefURLRequest-free road: DevTools `Page.navigate` of a hidden frame is wrong, so use `Network.setExtraHTTPHeaders` around the start, or `Browser.setDownloadBehavior` plus `Runtime.evaluate` of an `<a download>` click in the origin page, which carries the page's referrer policy. Choose the one that keeps cookies in the container. (4) A retried download keeps its list position, as Chrome does, instead of appearing as a new top row. All platforms.

## Media Session API and OS controls: seek, position, raise, active session (`media-session-complete`)

Key: `media-session-complete` · engine patch: no · state: not written.

In Chrome, MPRIS, Now Playing and SMTC show a position bar and accept seeks. The action handlers seekbackward, seekforward, seekto, skipad, togglemicrophone, togglecamera and hangup are honoured, Raise focuses the tab, and the active session is the one that most recently started playing; a paused session stays controllable. In wubble, crates/wubble/src/media_keys.rs and kit.js have only play, pause, toggle, next, previous and stop. MediaKey::of drops souvlaki's Seek, SeekBy, SetPosition, Raise and SetVolume. kit.js does not wrap `navigator.mediaSession.setPositionState` or `playbackState`. Keys::show passes `progress: None` and `duration: None`. `key_target` in host/videos.rs takes the first audible tab in map order and loses a paused session. Windows gets `hwnd: None`, so souvlaki refuses and the keys are off.

What to build: (1) kit.js: wrap setPositionState and playbackState, report them in `meta` (duration, position, rate, state), and extend `act(action, details)` with seek actions (seekTime, seekOffset) that fall back to the video element. (2) Move the active-session choice into a pure machine in a new wubble-core/src/mediasession.rs: an enum of sessions per tab, events (started playing, paused, closed, user pressed) and the chosen tab, with unit tests. (3) media_keys.rs: map Seek, SeekBy, SetPosition and Raise. Raise focuses the tab's window and tab. Pass MediaPosition and the duration, and pick the artwork size closest to 256 px. (4) Windows: pass the main window's HWND from winit. (5) Media in an iframe: the poll asks each same-origin frame's kit through `View::run_script_in`.

## Global media controls (Chrome's media hub) (`media-hub`)

Key: `media-hub` · engine patch: no · state: not written.

While any tab has a media session, Chrome's toolbar shows a media button. Its dialog lists every playing or paused session across tabs, each with artwork, title, artist and source site, play/pause, previous/next (when the page has handlers), a seek bar with times, a Picture-in-picture button and a click that jumps to the tab. wubble has none: media keys act on one target tab, and the only per-tab media UI is the rail's sound badge (wubble_core::tabmarks) and the video strip.

What to build: (1) Core: a list model in the new or existing media-session module (wubble-core), `fn hub_rows(sessions, tabs) -> Vec<HubRow>`, from the kit's `meta` snapshots that host/videos.rs already polls. Poll paused-session tabs too, at the existing poll's rate, never more. Unit tests. (2) UI: a footer button in kit/chrome/side-bottom.slint, shown while there are sessions, opening a popover like downloads-popover.slint. Build it from kit parts: APopover, menu rows, IconButton, the video strip's seek track (video-controls.slint), and artwork as an image fetched by CEF's DownloadImage the way favicons are (decisions.md "Favicons": no HTTP client in wubble). The design has no media hub: it is a derived piece for docs/fidelity/derived-review.md, built only from existing primitives. (3) Host: route row actions to `kit.act` / `kit.cmd` in the right tab, PiP through the pip machine, and the jump through the model's focus command. (4) Gallery scene for the shot set. All platforms.

## Audible probe across iframes and WebAudio on a stock engine (`audible-probe-frames`)

Key: `audible-probe-frames` · engine patch: no · state: not written.

Until a patched engine ships, wubble's audible probe (crates/wubble-engine/src/media.rs AUDIBLE_PROBE, run every 2 s as a main-frame eval) sees only `<audio>` and `<video>` in the top document. Embedded players (YouTube or Spotify iframes, which are cross-origin OOPIFs), media inside shadow roots and WebAudio games never light the sound badge or protect the tab from sleep.

What to build, without an engine change: (1) Shadow DOM. The probe walks open shadow roots as kit.js `shadowVideos` already does, capped as kit.js caps it. (2) WebAudio. kit.js wraps AudioContext and webkitAudioContext construction and `resume`, counting contexts in the `running` state with a non-zero destination connection. The probe reports `audible` when one is running and was started after a gesture. (3) iframes. The probe asks every frame, not just the main one: same-process frames through CEF frame iteration and `View::run_script_in`, and out-of-process frames through DevTools `Target.setAutoAttach {flatten:true}` and `Runtime.evaluate` with each attached session (the view's cdp() channel, crates/wubble-engine/src/cdp.rs). (4) Keep the ProbeState machine to one probe round in flight per view, and OR the per-frame answers in a pure fold with unit tests. When the engine audible patch exists (marker found), all of this is skipped. Measure by hand that the extra frames do not raise idle cost. Nothing in tests asserts timing. All platforms.

## Tab alert indicators: recording, screen capture, PiP (`tab-alert-indicators`)

Key: `tab-alert-indicators` · engine patch: no · state: not written.

Chrome's tab strip shows one alert indicator per tab, in priority order: media recording (camera/mic, a red dot), desktop/tab capturing, audio playing, audio muted, Bluetooth, USB, HID, serial and VR connected, and picture-in-picture (chrome/browser/ui/tabs/tab_utils.cc GetTabAlertStatesForContents, BSD). The indicators have tooltips ("This tab is using your camera or microphone"). wubble's wubble_core::tabmarks has only the sound badge (Sound::Playing, Sound::Muted). The busy set (wubble_core::busy) already knows Reason::Capture (a live capture track, from the renderer probes) and Reason::Pip, but no tab row shows them.

What to build: (1) wubble-core/src/tabmarks.rs: a `TabAlert` enum with Chrome's priority (Recording, Capturing, Pip, Playing, Muted), derived in `Marks::of` from the busy set, the mute state and the pip owner. Split Capture into camera/mic versus display capture: extend the probes script (crates/wubble-helper/src/probes.js) to report `getDisplayMedia` tracks under a new key, plus a busy::Reason, kept in the one Reason table. Unit tests for the priority. (2) UI: the badge in the extended row and the rail (the kit's tab row parts) draws the right glyph and tint, with a tooltip and an accessible label. Recording uses Palette's alert tint. If the kit has no glyph for it, it is a derived piece and goes on derived-review.md; the badge press does nothing except for sound. (3) Gallery scenes for each state. All platforms.

## Page Picture-in-Picture API and auto-PiP through wubble's Pip window (`pip-page-api`)

Key: `pip-page-api` · engine patch: no · state: not written.

In Chrome, `HTMLVideoElement.requestPictureInPicture()` opens the PiP window and resolves with a PictureInPictureWindow (width, height, onresize). `document.pictureInPictureElement`, `exitPictureInPicture()` and the enterpictureinpicture and leavepictureinpicture events all work. The media session action `enterpictureinpicture` lets video calls auto-enter PiP on a tab switch. In wubble's windowless CEF the API rejects. wubble's own PiP (wubble_core::pip, host/videos.rs pip_event, kit.js `k.pip`) is started only from the strip, the setting PIP_ON_TAB_SWITCH, or the shell.

What to build: (1) kit.js: replace `HTMLVideoElement.prototype.requestPictureInPicture`, `document.exitPictureInPicture`, `pictureInPictureEnabled` and the `pictureInPictureElement` getter, plus the events. A request with a gesture (navigator.userActivation.isActive) records the element and returns a pending promise. The next snapshot reports `pipAsk`. The host feeds it into the pip machine as a new `PipInput::PageAsked(tab)`, and later answers with a kit call `k.pipDone(ok, w, h)` that resolves or rejects and fires the events. `disablePictureInPicture` refuses. (2) wubble-core/src/pip.rs: the new input and the effect `AnswerPage`, added to the transition table with unit tests. (3) Auto-PiP: a page that registered an `enterpictureinpicture` media session handler (kit.js already keeps handlers) is popped out on a tab switch even with PIP_ON_TAB_SWITCH off, as Chrome does for video calls. Its handler is called instead of the element fullscreen. All platforms.

## Picture-in-Picture window controls parity (`pip-controls-complete`)

Key: `pip-controls-complete` · engine patch: no · state: not written.

Chrome's video PiP window has play/pause and back and forward 10 s. It adds previous and next track when the page has those media session handlers, plus mute and unmute, a progress bar with the current and total time, Skip Ad (skipad), the video-call buttons (togglemicrophone, togglecamera, hangup), back to tab and close. The window can be resized from its edges, and its size and position are remembered. In wubble, components/pip-controls.slint has back 10 s, play, forward 10 s, return and close only (host/videos.rs pip_command: play, seek-by, return, close). Resize and placement memory are not handled.

What to build: (1) Host: feed the Pip window's controls from the same kit `meta` and `v` snapshot the strip uses, which already carries t, dur, muted and the next/prev flags. Extend kit.js `meta` with flags for skipad, togglemicrophone, togglecamera and hangup handlers. (2) pip-controls.slint: add the conditional buttons and a seek track. Reuse the video strip's track from video-controls.slint, and its kit parts, BDock and IconButton. Commands go out as new pip_command names: mute, next, prev, skipad, mic, camera, hangup, seek-to; route the media-session ones to `kit.act`. (3) Window: resizable from its edges, keeping the video's aspect ratio. Remember its size and position in settings (Section::Hidden) and restore them next time, clamped to the current display (wubble_core::window owns the Pip window's state). (4) Keys while the PiP window is focused: Space toggles, the arrows seek, M mutes. The design has no PiP (wubble-only.md §28), so add the new pieces to derived-review.md. All platforms.

## Linux print handler: window.print() and the PDF viewer's print (`pdf-print-handler`)

Key: `pdf-print-handler` · engine patch: no · state: not written.

In Chrome, a page's `window.print()`, the PDF viewer's Print button and Ctrl+P on a PDF all open print preview and print what is shown. wubble's welding implements no CefPrintHandler (no print_handler in vendor/welding/src/linux_cef). On Linux, CEF needs one for printing, so a page's window.print() does nothing. Ctrl+P on a PDF is worked around in crates/wubble/src/host/print.rs `print()`: an `is_pdf` tab is downloaded instead of printed ("printing the viewer gives a blank page").

What to build: (1) welding (Linux): a WeldPrintHandler (cef::wrap_print_handler!) registered beside the find handler in linux_cef/mod.rs, marked `wubble change`. OnPrintDialog holds its callback and reports `NavigationEvent::PrintRequested { id, has_selection }`. OnPrintSettings fills the settings from the host's answer. OnPrintJob hands the host the PDF path Chromium wrote, and the job callback is completed after the host is done. OnGetPdfPaperSize returns the chosen paper. (2) Engine: a PageEvent::PrintRequested and `View::answer_print(id, Option<PrintSettings>)`. (3) Core: wubble_core::print::Sheet gains an origin `FromPage(id)`, so the existing sheet answers the page instead of calling PrintToPDF. Its Effect becomes AnswerPrint, and a job PDF goes to `lp` (or to Downloads for Save as PDF). Unit tests for the new transitions. (4) Host: remove the is_pdf download workaround. Ctrl+P on any tab calls CefBrowserHost::Print, so the PDF viewer's own document is printed through the sheet. macOS uses CEF's native print panel and needs no change; Windows stays Save as PDF.

## Print sheet: preview, two-sided, colour, pages per sheet, headers (`print-sheet-parity`)

Key: `print-sheet-parity` · engine patch: no · state: not written.

Chrome's print preview shows the rendered pages and offers destination, pages, copies, layout, colour/black and white, paper size, pages per sheet, margins (including custom), scale, two-sided, headers and footers, and background graphics. wubble's sheet (wubble_core::print PrintSettings: destination, pages, landscape, paper, margins, scale, background, copies; components/print-sheet.slint; host/print.rs runs `lp`) has no preview, two-sided, colour mode, pages per sheet or headers and footers.

What to build: (1) Core (wubble-core/src/print.rs): add `two_sided: Sides { One, LongEdge, ShortEdge }`, `color: bool`, `per_sheet: u8` (1/2/4/6/9/16) and `headers: bool` to PrintSettings. Map them in the pure lp command line builder (`-o sides=…`, `-o print-color-mode=monochrome`, `-o number-up=N`) and in PdfOptions (CEF display_header_footer with Chromium's default header and footer templates: date, title, URL, page numbers). Hide the printer-only options for Save as PDF, as Chrome does. Unit tests for every mapping. (2) Preview: on each settings change, debounced, PrintToPDF into the sheet's temp dir. Show the result in a windowless CEF view inside the sheet, using Chromium's own PDF viewer with its toolbar hidden (`#toolbar=0`), so no PDF rasteriser is added. The preview's lifecycle is a Stage in the Sheet machine. (3) UI: the new rows in print-sheet.slint from kit controls (select, toggle) and the preview pane. Lay it out from the kit, and put the derived pieces on derived-review.md. (4) Remember the last-used printer and options per printer in settings. Linux and macOS (CUPS); Windows stays Save as PDF.

