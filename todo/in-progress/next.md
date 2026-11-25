# Next round (loaded 2026-11-25)

Legend: `[ ]` open · `[~]` in progress. Loaded by the owner while the sprint compiles; written blind now on their own branches, integrated after the sprint passes.

## A. Written branches to integrate, and more search engines
- [ ] More search engines (owner): Chrome's list plus privacy ones: Google, Bing, DuckDuckGo, Kagi, Brave Search, Startpage, Ecosia, Qwant, Yahoo, Yandex, Baidu, Naver, Perplexity, plus Wikipedia, YouTube and GitHub as keyword engines; Chromium's regional defaults table

## Command bar: local calculator and unit conversion answers (`omni-calculator-units`)

Key: `omni-calculator-units` · engine patch: no · state: written on `feat/omni-calculator-units`, awaiting integration (its report: [batch-1-reports.md](batch-1-reports.md)).

Chrome's omnibox answers '12*7+3', 'sqrt(2)', '5 km in miles' and '70 F to C' as a row above the search. Chrome gets these from the server; wubble must compute them on the device with no request. Today omnibox::Kind (crates/wubble-core/src/omnibox.rs) has OpenTab/History/Bookmark/Search/Action and there is no answer row.

Add a pure module wubble-core/src/answer.rs with:
- a small expression parser: + - * / ^ %, parentheses, unary minus, constants pi and e, the functions sqrt, sin, cos, tan, log, ln, abs and round, and percent-of. Write it new or port an MIT/Apache crate such as `meval` (licence check, source comment, THIRD_PARTY_NOTICES entry).
- a units table as data: length, mass, volume, temperature, speed, area, data size and time, with the forms '<n> <unit> in|to <unit>'.
- number formatting that trims trailing zeros.

Add Kind::Answer and Target::Copy(String). Choosing the row copies the result through crates/wubble/src/clipboard.rs and shows a toast. Map Kind::Answer to a new CommandKind `answer` in crates/wubble-ui/ui/components/model.slint and kit command-row.slint, reusing an existing glyph (equals/calculator from kit gen icons if present, else note it on derived-review.md). In rank, the answer goes first when the query parses as math with at least one operator, and second, under the search intent, for unit forms. A query that is only a number gives no answer. No currency: it would need a network rate source. Unit tests for parsing, precedence, division by zero (no row) and each unit family.

## Command bar: inline autocomplete and Chrome's omnibox keys (`omni-inline-autocomplete`)

Key: `omni-inline-autocomplete` · engine patch: no · state: written on `feat/omni-inline-autocomplete`, awaiting integration (its report: [batch-1-reports.md](batch-1-reports.md)).

Chrome finishes a typed address inside the field. Type 'git' and it shows 'github.com/' with the added part selected. Right or End accepts it, Backspace drops it, Enter goes to the completed URL. Only a history entry whose host or URL starts with the typed text and whose typed_count is above 0 (or that has a high visit_count) is offered. wubble has no inline completion. CommandHead (crates/wubble-ui/ui/kit/chrome/command-head.slint) handles only Up, Down, Enter and Escape, and omnibox::rank (crates/wubble-core/src/omnibox.rs) returns rows only.

Add a pure fn `inline_completion(query, &[HistoryEntry], &[Bookmark]) -> Option<String>` to wubble-core/omnibox.rs, ported from Chromium's HistoryURLProvider and ShortcutsProvider inline rules (BSD-3, behaviour only). It returns the suffix, which is never offered for a query that ends in a space or right after a deletion. Make rank's first row the completed URL when there is one. Add an `inline` property to CommandHead and set the selection with TextInput.set-selection-offsets.

Add Chrome's keys there, passed up as new callbacks:
- Tab and Shift+Tab move between rows.
- Shift+Delete on a History row removes it: HistoryClear::Url through host/pages.rs history_deleted, then re-rank.
- Alt+Enter (Option on macOS) opens in a new tab: BarMode::NewTab in bar.rs target_command.
- Shift+Enter opens a new window.
- Ctrl+Enter wraps a bare word as www.<word>.com before classify.

Wire it in crates/wubble/src/bar.rs and the host's command-bar handling. Add unit tests in omnibox.rs for the completion rule and Ctrl+Enter.

## Search engines: more built-ins, custom engines with keywords, management in Settings (`search-engines-keywords`)

Key: `search-engines-keywords` · engine patch: no · state: written on `feat/search-engines-keywords`, awaiting integration (its report: [batch-1-reports.md](batch-1-reports.md)).

Chrome keeps a list of search engines, each with a name, a keyword and a URL with %s. You can set any of them as the default, add and edit your own, and site searches are picked up from pages. wubble has only `ENGINES: [EngineInfo; 2]` (Google, DuckDuckGo) in crates/wubble-core/src/omnibox.rs, chosen by the `search.engine` Select in settings.rs.

1. Extend ENGINES with Bing, Brave Search, Ecosia, Startpage, Qwant, Kagi and Wikipedia (search and suggest templates from Chromium's prepopulated_engines.json, BSD-3), each with a default keyword. Never rename the shipped ids.
2. Add a setting `search.custom` as a list setting (the list-editor kind with columns_of in pages.rs: name, keyword, URL with %s or {q}). Validate it in a pure fn: exactly one placeholder, http(s) only, unique keyword.
3. Let the default be any built-in or custom engine. Replace the SearchEngine enum where it is used (bar.rs, suggest.rs suggest_url, the start page capsule, menu.rs search_label) with an `EngineRef` resolved from settings. Live suggestions stay behind `suggest.live`, and only for engines with a suggest URL.
4. OpenSearch discovery: a page script reads <link rel=search type=application/opensearchdescription+xml>. Fetching the OSDD (crates/wubble-engine/src/fetch.rs) happens only behind a new setting `search.discover_sites`, off by default. Discovered engines are listed as inactive in Settings, never made the default.

The Settings section shows the default engine, the built-in list and the custom list editor. Unit tests for the template checks and engine resolution.

## Startup, home page and quit settings (`startup-home-settings`)

Key: `startup-home-settings` · engine patch: no · state: written on `feat/startup-home-settings`, awaiting integration (its report: [batch-1-reports.md](batch-1-reports.md)).

Chrome's 'On startup' setting offers: open the New Tab page, continue where you left off, or open a specific set of pages. There is a Home page (Alt+Home / Cmd+Shift+H), a 'Restore pages?' offer after a crash, and on macOS 'Warn before quitting'. The design's settings (pages/settings.tsx row `startup`: 'When Wubble opens', Start page | Last session) have the first two. wubble has no such setting: it always restores the stored session (wubble-store session.rs load_session). Build: settings rows in crates/wubble-core/src/settings.rs. `startup.mode` is a Select of Last session (default), Start page, or Specific pages. `startup.pages` is a list editor of URLs, shown only for Specific pages. `home.url` is empty for the start page. `quit.warn` (macOS default on) requires a second Cmd+Q or a hold. At start, in crates/wubble/src (where the loaded Model is handed to the host): with Start page, keep Spaces, pinned tabs and folders, but open a start tab in each window. The unpinned tabs of the last session go to the archive or recently closed, following Chrome's 'new tab page' semantics: they are not restored. Pinned tabs always stay. With Specific pages, open those pages as new tabs. Record a clean-exit flag in the store. When the last run did not exit cleanly and the mode is not Last session, show a kit banner 'Restore pages?' that restores the saved session. Add `Action::GoHome` (Alt+Home on Linux/Windows, Cmd+Shift+H on macOS) and a palette title 'Home'. Add a command-bar row 'Open pages from last session'.

## Recently closed windows, folders and Spaces, and a recently-closed list UI (`recently-closed-full`)

Key: `recently-closed-full` · engine patch: no · state: written on `feat/recently-closed-full`, awaiting integration (its report: [batch-1-reports.md](batch-1-reports.md)).

Chrome's TabRestoreService keeps tabs, whole windows and tab groups. Ctrl+Shift+T reopens the newest entry of any kind, and the History menu and tab search list 'Recently closed' with titles. Today crates/wubble-core/src/closed.rs and wubble-store/src/closed.rs keep tab entries only, and nothing lists them (`Store::recently_closed` has no caller in crates/wubble). A closed Main window keeps its Space, so it is not remembered at all (model/windows.rs close_window), and a closed folder or deleted Space adds its tabs one by one, if at all. Build: generalize the store row into an entry enum, `Tab | Window { space, geometry } | Folder { name, tabs } | Space { name, tint, icon, container, tabs }`, with a migration. Grouped entries carry child tab rows with url, title, container, place and, once captured, history. Record a Window entry in `close_window` (and a Space entry from `DeleteSpace` and `FolderRow::Close`, when they actually close tabs). Extend the `closed::Event` and `Effect` machine: `Restore` gains a window variant (reopen a Main window on that Space at its geometry, or recreate the Space if it is gone) and a folder or Space variant. Mod+Shift+T and Mod+Z pop any kind. UI: a 'Recently closed' block at the top of wubble://history (pages.rs, history page model), and command-bar rows (bar.rs) listing the entries with titles. Each row reopens its entry and can be deleted on its own. Keep RECENTLY_CLOSED_MAX at 25 entries. Agent and private tabs stay off the list.

## B. Settings that are mostly one Chromium preference

## HTTPS-First and HTTPS-Only mode (`https-upgrade-mode`)

Key: `https-upgrade-mode` · engine patch: no · state: not written.

Chrome's "Always use secure connections" setting tries https for every http navigation. If https fails, it shows an interstitial ("Connection is not secure" … "Continue to site") and remembers the choice per site for a while. Chrome also upgrades automatically with a silent fallback (HTTPS-Upgrades). wubble only upgrades typed scheme-less public hosts (decision "Typed local addresses") and has no mode.

Build:
- Add a setting `privacy.https_mode`: off|upgrade|only. The default is "upgrade", to match Chrome's automatic upgrades; it makes no new request to a new party. Put it in crates/wubble-core/src/settings.rs.
- Add a pure module `wubble_core::https`: `plan(url, mode, exempt) -> Upgrade|Keep` (exempt: IP literals, localhost, single labels, .local, non-default ports, the per-container allow list) and a lifecycle enum per navigation (Upgraded → Loaded | Failed → Fallback or Warn) with `transition`. Unit-test it.
- Engine path: main-frame Document requests go through the request filter (crates/wubble-engine/src/filter.rs `RequestAction::Redirect`). The host's ContentFilter (crates/wubble/src/blocking.rs) consults an Arc snapshot of the mode and the allow list and redirects http→https, recording the upgrade per browser_id.
- On a load or certificate error of an upgraded navigation: in upgrade mode, reload the http URL marked exempt for that navigation. In only mode, show a new `PageError::HttpsOnly { url }` in crates/wubble/src/page_error.rs with "Go back" and "Continue to site". Continue adds the host to the container's allow list (session plus 15 days, as Chrome does) and loads http.
- Set Chromium's own prefs `https_only_mode_enabled`/`https_upgrades` false so the two do not both act.

## Secure DNS (DNS-over-HTTPS) setting (`secure-dns`)

Key: `secure-dns` · engine patch: no · state: not written.

Chrome Settings > Security > "Use secure DNS" offers Off, "With your current service provider" (automatic upgrade), or a chosen provider (Cloudflare, Google, Quad9, CleanBrowsing, NextDNS, OpenDNS, …) or a custom template. wubble has no row, and Chromium's default (automatic) applies silently.

Build:
- Add a setting `privacy.secure_dns`: automatic|off|provider:<id>|custom:<template>. The default is automatic, matching Chromium's default (decision "Network stack: Chromium's defaults stay"); choosing a provider is the user's own act.
- wubble-core: a provider table ported from Chromium `net/dns/public/doh_provider_entry.cc` (BSD-3-Clause, with a source comment and a THIRD_PARTY_NOTICES.md entry), with id, display name, template and privacy-policy link. Add `fn validate_template(s)` (https, parses as a URI template with `{?dns}` or a POST form, like Chromium's `dns_over_https_config`) and `fn prefs(setting) -> (mode: &str, templates: String)`. Unit-test it.
- Engine: these are local-state (global) prefs. Apply `dns_over_https.mode` ("off"/"automatic"/"secure") and `dns_over_https.templates` with `cef::preference_manager_get_global()` and `set_preference` after CEF starts and on change. Add `Engine::set_global_pref` in crates/wubble-engine/src/app.rs.
- UI: a Settings Privacy row with a select and a custom-template field (the kit select and input that already exist for settings rows), with an error text when invalid.
- net_audit.rs smokes must allow the chosen provider's host only when it is set; only note this, write no test.

## Global Privacy Control (Sec-GPC header and navigator.globalPrivacyControl) (`global-privacy-control`)

Key: `global-privacy-control` · engine patch: no · state: not written.

GPC sends `Sec-GPC: 1` on every request and exposes `navigator.globalPrivacyControl === true`. Brave and Firefox ship it, and Chrome has it behind a feature. wubble has neither.

Build:
- Add a setting `privacy.gpc` (bool, default off, Section::Privacy) in crates/wubble-core/src/settings.rs.
- First check whether the prebuilt libcef 156 has Chromium's feature: search the CEF binary for the feature name ("GlobalPrivacyControl") with `strings` on the downloaded libcef. If present, add it to the PageFeatures enable list in crates/wubble-engine/src/app.rs (it takes effect at the next start; say so on the row). Otherwise, use the embedder route:
  (a) A request header. Extend welding's request filter (vendor/welding/src, OnBeforeResourceLoad) and `wubble_engine::filter::RequestAction` with a header set. Simplest is an `extra_headers: &'static [(&str,&str)]` returned by a new `ContentFilter::headers(&RequestInfo)` that defaults to none, mark it a wubble change, and have crates/wubble/src/blocking.rs return Sec-GPC when the setting is on (an Arc atomic read on the IO thread).
  (b) The script: a main-world getter `navigator.globalPrivacyControl` installed in every frame at context creation through the helper's page scripts (crates/wubble/src/pagescripts.rs and wubble-helper), only when on. The engine passes the flag with the frame `hello`, like cosmetic.js.
- Write unit tests for the header decision.

## Privacy preferences table: Do Not Track, preload pages, ad privacy, WebRTC IP (`privacy-prefs-table`)

Key: `privacy-prefs-table` · engine patch: no · state: not written.

Chrome's privacy settings are profile prefs: "Send a Do Not Track request" (`enable_do_not_track`, off by default), "Preload pages" (`net.network_prediction_options`), "Allow sites to check if you have payment methods saved" (`payments.can_make_payment_enabled`), the Ad privacy toggles (`privacy_sandbox.m1.topics_enabled`, `privacy_sandbox.m1.fledge_enabled`, `privacy_sandbox.m1.ad_measurement_enabled`), and the WebRTC IP handling policy (`webrtc.ip_handling_policy`, with no UI in Chrome; Brave exposes it). wubble sets one pref only (`webkit.webprefs.minimum_font_size`, crates/wubble-engine/src/view.rs:848) and exposes none of these.

Build:
- wubble-core: a data table `privacy::PREFS: &[(setting key, chromium pref, Value mapping)]` with matching SettingDefs in settings.rs (Section::Privacy). Defaults: DNT off (Chrome), preload "standard" (Chrome's default, since no request of wubble's own is added), can-make-payment on, Privacy Sandbox APIs off (Chrome asks for consent; wubble never shows the consent), WebRTC `default`. Unit-test that every row has a SettingDef and a valid mapping.
- Engine: a generic `Engine::set_profile_pref(container, name, value: PrefValue)` that lifts the `set_preference` code out of view.rs to a request-context helper in storage.rs or app.rs. Apply the whole table when each container context comes up (`Profiles::get`) and on each setting change, and keep the min-font-size path on the same helper.
- UI: the rows come from the settings table automatically.

## "Download PDFs instead of opening them" site setting (`pdf-download-setting`)

Key: `pdf-download-setting` · engine patch: no · state: not written.

Chrome's Settings → Privacy → Site settings → PDF documents offers "Open PDFs in Chrome" or "Download PDFs", stored as the pref plugins.always_open_pdf_externally. When it is on, a PDF navigation becomes a download that the system viewer can open. wubble always shows PDFs in Chromium's viewer (host/builtins.rs `pdf` set) and has no such setting.

What to build: (1) wubble-core/src/settings.rs: `PDF_DOWNLOAD` (key pdf.download, Bool, default "0", label "Download PDFs instead of opening them", Section::Privacy, matching Chrome's place), with settings tests. (2) Host: apply it to every request context (each container's) through the existing preference setter (crates/wubble-engine/src/view.rs `set_preference`, ~line 865), with the name "plugins.always_open_pdf_externally". Apply it at start and live on change from host/settings.rs. (3) Downloads: a PDF downloaded this way follows the normal download path. If the auto-open feature has landed, it may open automatically, since PDF is a safe type. (4) Check by hand that CEF 156 honours the pref for Alloy-style windowless browsers. If it does not, fall back to the response hook: a top-level navigation answered with application/pdf is cancelled and started again as `View::start_download` in the same tab. Keep that decision a pure function with a unit test. All platforms.

## Downloads settings section with a folder picker (`download-location-setting`)

Key: `download-location-setting` · engine patch: no · state: not written.

Chrome's Settings → Downloads has Location [Change] and "Ask where to save each file". The design has a Downloads section with "Save files to" (a folder choice), "Ask where to save each file" and "Open safe files after downloading" (ref/the-wallpaper-project/src/wubble/pages/settings.tsx lines ~55-123). In wubble the downloads folder can only be set on the command line (crates/wubble/src/profile.rs downloads_dir: the CLI, else dirs::download_dir). DOWNLOADS_ASK_WHERE and the media rows sit under Section::General, and there is no Downloads section.

What to build: (1) wubble-core/src/settings.rs: add `Section::Downloads` (title "Downloads"). Move DOWNLOADS_ASK_WHERE and the MEDIA_* rows into it, which changes no stored keys. Add `DOWNLOADS_DIR` (key downloads.dir, empty means the OS default) with a new `SettingKind::Folder`, and update the tests. (2) Settings UI (components/settings.slint, kit pages): a folder row that shows the path and a Change button. It opens the OS folder chooser through crates/wubble/src/chooser.rs (FileChooserMode::OpenFolder, or add that mode). Reuse it for MEDIA_SAVE_DIR, which is a free text field today. (3) Host: Downloads gets `set_dir(Option<PathBuf>)` (host/downloads.rs), applied live from the settings handler (host/settings.rs); a CLI value still wins. The Settings nav gets the section's icon (the design's `download` glyph is already Glyph.download). (4) Settings search finds it ("folder location directory", the design's `find` words). All platforms.

## C. Missing menu rows

## Context menu: the missing link, selection, page and frame rows (`ctx-menu-link-page-frame`)

Key: `ctx-menu-link-page-frame` · engine patch: no · state: not written.
Note: 0.6.1 shipped more context-menu entries: check what is left before writing.

Chrome's `RenderViewContextMenu` has rows that wubble's `ROWS` (crates/wubble/src/menu.rs) lacks. Link: "Open link in new window" (now possible: `Action::NewWindow` and `Command::MoveToWindow` exist; the comment above `ROWS` saying one window is stale), "Open link in split view" (wubble has split: `Command::SplitWith`), an "Open link in Glance" equivalent (`ModelCommand::OpenGlance`), and "Copy link text". Selection: "Copy link to highlight", which puts a `#:~:text=` URL on the clipboard, built as Chromium's `shared_highlighting` text-fragment generator does (port the generator to a helper page script, BSD-3, run with `View::run_script_in` in `ContextMenu::frame`), plus a search row for a selected URL that offers "Go to <url>". Frame (when `frame_url` is non-empty and differs from `page_url`): "Reload frame" (script `location.reload()` in the frame) and "View frame source" (view-source: of frame_url via the existing `Action::ViewSource` path, parameterised by URL). Editable: "Emoji" on macOS (`orderFrontCharacterPalette:`) and Windows (the Win+. panel through `CoreInputView`/SendInput), plus "Writing direction" on macOS. Keep the Chromium group order, with dividers from the `row!` group numbers. Add `Command` variants as needed and execute them where the host runs `menu::Command` (crates/wubble/src/host). Unit-test each row's predicate in menu.rs's tests as the existing ones do.

## Media context menu: Picture in Picture, Mute, video frame rows (`media-context-menu`)

Key: `media-context-menu` · engine patch: no · state: not written.

Chrome's context menu on a video has Play/Pause, Mute/Unmute, Loop, Show controls, Picture in picture, Open video in new tab, Save video as, Copy video address, and since M120 Copy video frame and Save video frame as. wubble's crates/wubble/src/menu.rs ROWS has Play, Loop, Show Controls, the three address rows and the screenshot row. Picture in Picture is left out on purpose (comment at ~line 313 and TODO.md M1 "Page context menu: the Picture in Picture row"), and there is no Mute or frame row, even though wubble now has its own PiP (wubble_core::pip) and a frame grab (kit.js `k.frame`).

What to build in menu.rs, with its label tests updated: (1) A Mute/Unmute row: `MediaOp::Mute`, from the menu's MediaFlags muted bit, run through `media_script`. (2) A Picture in Picture row for videos with CAN_PICTURE_IN_PICTURE, or always for video when the flag is absent. It is a new Command that feeds `PipInput` for the tab, targeting the element under the pointer: pass its point, and the kit picks that element instead of `primary()`; add `k.pipAt(x, y)`. (3) Copy Video Frame and Save Video Frame As…: grab the frame at the point (extend `k.frame` with an element argument), then put a PNG on the clipboard through the screenshot worker (decisions.md "Copy Image" path) or write it to the downloads folder as a finished download (`add_finished_download`). A cross-origin video whose canvas is tainted falls back to a crop of the view's screenshot of the element's rect. (4) The rows follow Chromium's AppendMediaItems order. All platforms.

## Video strip save button: wording and in-progress state (`video-save-button-states`)

Key: `video-save-button-states` · engine patch: no · state: not written.

TODO.md review findings: "Video strip button reads 'Save video'; a second click mid-download re-enters the downloader". Chrome has no such control. The rules for this one come from wubble (decisions.md "Save control"). The strip's save state is wubble_core::video::Save (host/videos.rs video_save_now, video_save_state, video_save_redraw), and the click goes to `video_save(x, y)`, which starts the downloader again even while that video's job runs.

What to build: (1) Core (wubble-core/src/video.rs): make `Save` carry the job's state for this tab's video: Idle, Saving{fraction}, Saved, Failed. Derive it from the downloads entry the media job owns, which has one owner, the downloads list. The click maps to a pure effect: Idle → start the downloader; Saving → open the downloads popover at that row, never a second job; Saved → show in folder or open the menu; Failed → retry. Unit tests over the states. (2) Label: the button's accessible label and tooltip read from that state ("Save video", "Saving video, 40%", "Saved, show in folder", "Save failed, retry"), and the glyph shows progress with the kit's progress part. Settle the wording with the owner's decision on wording if one exists; otherwise use these and flag them. (3) video-controls.slint binds the new fields only (no new drawing outside kit parts). All platforms.

## D. Video downloader: wider and harder (owner: "more sites and formats, quality and codec choice, and a way to detect subtitle files too")
- [ ] More sites and formats: embedded players (iframes, YouTube and Vimeo embeds), service-worker-served media, multi-period DASH, audio-only
- [ ] Quality and codec choice: the best quality this machine plays by default, a quality menu, audio and video merged
- [ ] Subtitles: detect subtitle tracks and files (WebVTT, SRT, TTML, HLS/DASH subtitle renditions) and offer them with the video

## Media downloader: multi-period DASH and codec-aware default quality (`media-dash-periods-codec`)

Key: `media-dash-periods-codec` · engine patch: no · state: not written.

TODO.md: "Media downloader: a DASH manifest of several periods is refused ('Not supported'); the default quality takes the first row of a height whatever its codec (HEVC or AV1 before H.264)". In wubble-core/src/media/dash.rs, `let [period] = mpd.periods.as_slice() else { … "a manifest with several periods" }` refuses multi-period manifests. In wubble-core/src/media/select.rs, `choose` takes the first choice of the wanted height.

What to build: (1) dash.rs: for several periods, match representations across periods by AdaptationSet content type, then by Representation id, then by (height, codecs, bandwidth). Build one Variant per matched chain, whose segment list concatenates each period's segments, with time offsets from Period@start and @duration. Periods that match nothing (a pre-roll ad period) are skipped, and the row says "ad periods left out". Refuse only when no chain spans the main content. Port dash.js's period handling rules, BSD-3, with a source comment and a THIRD_PARTY_NOTICES.md entry if code is copied. Unit tests with multi-period fixtures. (2) select.rs: at equal height, the default prefers the codec most players open: H.264 (avc1) first, then VP9, AV1 and HEVC. For audio, AAC then Opus. Only can_join and ffmpeg availability override this. Put the order in one data table, used by `choose`, and keep the menu's labels (codec_name) unchanged. Unit tests for the ladder. (3) The writer's join (crates/wubble-engine/src/save.rs) handles period-concatenated fMP4 init changes: a new init segment per period goes through ffmpeg's concat. All platforms.

## Media detection for service-worker answers and embedded YouTube (`media-detect-sw-iframe`)

Key: `media-detect-sw-iframe` · engine patch: no · state: not written.

TODO.md review findings: "a video a service worker answers is not seen by the detection (the worker's own fetch has no tab; `--smoke media=probe` on fixtures `sw`): the save control says nothing was found" and "an embedded YouTube player (an iframe of youtube.com) has no lookup, since the lookup keys on the tab's own address". The response hook (host/media.rs, on CEF's IO thread) attributes responses by browser, and a service worker's fetch has none. The YouTube lookup (`media_youtube_page`) runs only for the tab's committed URL (youtube::video_id).

What to build: (1) Service workers: in the response hook, a response with no browser but with a service worker's scope or initiator origin is kept in a small per-origin queue. On the host side, it is credited to every live tab of the same container whose committed origin equals the worker's origin, and the newest active tab of that origin wins when several match. Put the attribution in a pure function in wubble_core::media::detect with unit tests. (2) Embedded YouTube: kit.js's `bigFrame()` also reports the iframe's `src` when it is a youtube.com or youtube-nocookie.com /embed/ID URL. The host feeds that ID to the existing lookup (behind media.youtube, still off by default) and lists the result for the tab as Kind::YouTube, so the strip's save control over the iframe works. Extend youtube::video_id with the embed forms and nocookie hosts, with unit tests. (3) Add the fixture rows to scripts/media-fixtures.py for hand runs. All platforms.

## E. Ad blocker: uBlock Origin parity and click abuse (owner)
- [ ] uBlock Origin parity: scriptlets at document start, procedural cosmetic filters (`:has-text`, `:upward`, `:remove`, ...), redirect resources, `$removeparam`, `$csp`, `$popup`/`$popunder`
- [ ] "Weird invisible panes on websites that open up another tab when clicked": the invisible full-page overlay that opens a new tab (popunder) on the first click: block it
- [ ] "Not allowing arbitrary downloads and scripts to happen when clicking something": a click must not start a download or a navigation to another site the user did not aim at
- [ ] Content blocker: uBO scriptlet coverage
- [ ] Content blocker start-up race: with no compiled-engine cache (first run, list change), requests pass until the lists compile (seconds); hold the first main-frame navigation at the host until the blocker reports ready (a start-up state, bounded by an event), since CEF's IO thread must never wait

## Content blocker start-up gate holds the first navigation (`blocker-startup-gate`)

Key: `blocker-startup-gate` · engine patch: no · state: not written.

With no compiled-engine cache (first run, or after a list change), requests go through unfiltered for several seconds until the lists compile. Chrome extensions such as uBO hold requests at start-up. wubble cannot wait on CEF's IO thread, so the hold belongs at the host. The swap point is `Blocking::swap` around crates/wubble/src/blocking.rs:477-481 (`measure::Stage::BlockerReady`).

To build: a pure lifecycle in wubble-core (for example `blocker_gate.rs`: `enum Gate { Waiting { held: Vec<Held> }, Ready }`, `fn transition(Gate, GateEvent) -> (Gate, Vec<GateEffect>)`). The events are Navigate { view, url } (a main-frame load the host is about to start: restored active tab, typed address, start URL, `wubble <url>` from the instance socket), BlockerReady, BlockerOff, ListsFailed and Deadline. The effects are Load(view, url) and ShowWaiting(view). It is Ready from the start when the cache loaded or the blocker is off (store `BlockerEnabled`). While Waiting, the host creates the view but delays `load_url`, and the pane shows the existing loading state. The gate is bounded by events: blocker ready, compile failure, or a deadline (for example 10 s) that loads anyway and logs it. Page-initiated navigations inside already-loaded pages are not held. Wire it into crates/wubble/src/host.rs where views get their first URL, and send BlockerReady from blocking.rs through the existing UI-thread channel. Unit tests for the transition. All platforms.

## Blocker scriptlets run at document start (`blocker-scriptlets-at-start`)

Key: `blocker-scriptlets-at-start` · engine patch: no · state: not written.

uBO injects scriptlets (`##+js(...)`) at document_start, before any page script runs. In wubble the frame's `cosmetic.js` sends `hello` when its context is created; the browser answers, and only then runs the scriptlets with `run_script_in` (crates/wubble-engine/src/view.rs `answer_cosmetic`, around line 1099). By then the page's own scripts have often run, so many scriptlets (set-constant, abort-on-property-read, json-prune, no-fetch-if) come too late. That is the "uBO scriptlet coverage" gap.

To build: when a view's main frame is about to navigate (`OnBeforeBrowse`/nav commit start in crates/wubble-engine/src/nav.rs or view.rs), ask the filter for the target URL's scriptlets (`filter.frame_cosmetic(top, url).script`, which comes from `Cosmetic::scriptlets` in crates/wubble-blocker/src/blocker.rs). Register them through the view's DevTools channel with `Page.addScriptToEvaluateOnNewDocument` (with `runImmediately` where supported), and remove the previous identifier with `Page.removeScriptToEvaluateOnNewDocument`. Same-site subframes are covered by the same registration. For cross-site subframes, keep the current hello path as a fallback, and skip it when the document-start copy has already run (the scriptlet wrapper sets a guard global inside its closure scope, in the page world). Respect per-site off (`SiteOverrides`) and agent tabs. Add a page fixture to the block smoke with a set-constant rule that is checked by an inline script running before load. All platforms.

## Dangerous-site interstitial for blocker badware/malware document blocks (`dangerous-site-interstitial`)

Key: `dangerous-site-interstitial` · engine patch: no · state: not written.

Chrome shows a red full-page "Dangerous site" warning with "Back to safety" and, under Details, "visit this unsafe site". uBlock Origin's strict blocking does the same for `$doc` and badware rules. wubble's blocker ships the uBO badware list (crates/wubble-blocker/src/catalog.rs `ublock-badware`). A blocked main-frame document ends as ERR_BLOCKED_BY_CLIENT in the generic error panel (crates/wubble-core/src/neterror.rs -20), with no way through and no explanation.

Build:
- wubble-blocker: decisions for Document requests report which list matched (`Decision` in crates/wubble-blocker/src/decision.rs gets a `list: Option<&'static str>` or a `Category {Badware, Privacy, Ads}`). Unit-test it.
- crates/wubble/src/blocking.rs keeps the last document block per browser_id, holding the URL, list and rule.
- crates/wubble/src/page_error.rs: when the load error is -20 and a document block is recorded for that URL, the panel becomes `PageError::Blocked { url, category, rule }`. Its text is Chrome's register ("Dangerous site" for badware, "Page blocked" for others), with "Go back" (the default) and "Proceed anyway" under Advanced.
- Proceed adds a session exception (per container and host) that the filter consults for Document requests only, then reloads. The site's Shields toggle (`blocker.exceptions`) stays the permanent way.
- UI: reuse kit a-error-panel with a danger tint if the kit has one, else a derived piece on derived-review.md.
- No network lookups: Safe Browsing stays out.

## F. Linux measurement against Firefox (owner: "make sure the performance is better than at least Firefox")
- [ ] The Mac harness's states on Linux (idle, a still page, video, video under glass, scrolling, typing: CPU per process, memory, GPU, energy where Linux exposes it), the same run in Firefox, a comparison table; wubble must beat Firefox in every state

## G. Leftovers from the sprint and this round (owner, 2026-11-25)
- [ ] macOS: the search region from the system's region setting (`NSLocale`), not `LANG` (a GUI app has none; today it falls back to Google, Bing, Yahoo)
- [ ] `$csp` filters: main-frame responses the blocker names get the CSP through DevTools `Fetch` (CEF cannot change response headers)
- [ ] Picture-in-picture and the video frame rows for a video inside a subframe
- [ ] Auto-reload offline never repeats a form submission (a failed load does not say whether it was a POST: find where to learn it)
- [ ] Rebuild the bundled FFmpeg with `concat` before the release that ships multi-period DASH
- [ ] macOS Keychain prompts (owner, 2026-11-25: on a new device, and on every test build): test builds and the harness use Chromium's mock keychain and a separate vault entry; check the release signature's designated requirement is team and bundle id (not a cdhash) so updates never ask again; consider the data-protection keychain with an access group
