# 03 Navigation: address bar, bookmarks, history

Legend: `[ ]` open · `[~]` in progress. How items move: [todo/README.md](../README.md).

## From the old roadmap
- [ ] A bookmark dropped into a folder row lands at its end; Chrome's drop line between rows (a position) waits for drag reorder of bookmarks (M1)
- [ ] Bookmarks: drag reorder, a bookmarks bar (M2)
- [ ] Per-site user agent ("request desktop site"); history grouped by task; sidebar web panels; the shell in other languages (scout, 2026-11-22: features no plan has yet; the owner picks)

## Build-out features (2026-11-22)

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

## Command bar: local calculator and unit conversion answers (`omni-calculator-units`)

Key: `omni-calculator-units` · engine patch: no · state: written on `feat/omni-calculator-units`, awaiting integration (its report: [batch-1-reports.md](batch-1-reports.md)).

Chrome's omnibox answers '12*7+3', 'sqrt(2)', '5 km in miles' and '70 F to C' as a row above the search. Chrome gets these from the server; wubble must compute them on the device with no request. Today omnibox::Kind (crates/wubble-core/src/omnibox.rs) has OpenTab/History/Bookmark/Search/Action and there is no answer row.

Add a pure module wubble-core/src/answer.rs with:
- a small expression parser: + - * / ^ %, parentheses, unary minus, constants pi and e, the functions sqrt, sin, cos, tan, log, ln, abs and round, and percent-of. Write it new or port an MIT/Apache crate such as `meval` (licence check, source comment, THIRD_PARTY_NOTICES entry).
- a units table as data: length, mass, volume, temperature, speed, area, data size and time, with the forms '<n> <unit> in|to <unit>'.
- number formatting that trims trailing zeros.

Add Kind::Answer and Target::Copy(String). Choosing the row copies the result through crates/wubble/src/clipboard.rs and shows a toast. Map Kind::Answer to a new CommandKind `answer` in crates/wubble-ui/ui/components/model.slint and kit command-row.slint, reusing an existing glyph (equals/calculator from kit gen icons if present, else note it on derived-review.md). In rank, the answer goes first when the query parses as math with at least one operator, and second, under the search intent, for unit forms. A query that is only a number gives no answer. No currency: it would need a network rate source. Unit tests for parsing, precedence, division by zero (no row) and each unit family.

## Search engines: more built-ins, custom engines with keywords, management in Settings (`search-engines-keywords`)

Key: `search-engines-keywords` · engine patch: no · state: written on `feat/search-engines-keywords`, awaiting integration (its report: [batch-1-reports.md](batch-1-reports.md)).

Chrome keeps a list of search engines, each with a name, a keyword and a URL with %s. You can set any of them as the default, add and edit your own, and site searches are picked up from pages. wubble has only `ENGINES: [EngineInfo; 2]` (Google, DuckDuckGo) in crates/wubble-core/src/omnibox.rs, chosen by the `search.engine` Select in settings.rs.

1. Extend ENGINES with Bing, Brave Search, Ecosia, Startpage, Qwant, Kagi and Wikipedia (search and suggest templates from Chromium's prepopulated_engines.json, BSD-3), each with a default keyword. Never rename the shipped ids.
2. Add a setting `search.custom` as a list setting (the list-editor kind with columns_of in pages.rs: name, keyword, URL with %s or {q}). Validate it in a pure fn: exactly one placeholder, http(s) only, unique keyword.
3. Let the default be any built-in or custom engine. Replace the SearchEngine enum where it is used (bar.rs, suggest.rs suggest_url, the start page capsule, menu.rs search_label) with an `EngineRef` resolved from settings. Live suggestions stay behind `suggest.live`, and only for engines with a suggest URL.
4. OpenSearch discovery: a page script reads <link rel=search type=application/opensearchdescription+xml>. Fetching the OSDD (crates/wubble-engine/src/fetch.rs) happens only behind a new setting `search.discover_sites`, off by default. Discovered engines are listed as inactive in Settings, never made the default.

The Settings section shows the default engine, the built-in list and the custom list editor. Unit tests for the template checks and engine resolution.

## Command bar: keyword mode (tab to search) and @tabs/@bookmarks/@history scopes (`omni-keyword-mode`)

Key: `omni-keyword-mode` · engine patch: no · state: not written.

In Chrome, typing a keyword and then Tab or Space ('yt', 'w') switches the omnibox into a 'Search YouTube' chip mode. The built-in scopes @tabs, @bookmarks and @history limit results to one source. wubble's rank (crates/wubble-core/src/omnibox.rs) always searches with the default engine across every source.

This depends on engine keywords (feature search-engines-keywords); if that has not landed, build on the ENGINES table with a keyword field. Add a pure parse step `fn scope(query, engines) -> (Scope, &str)`, where Scope is one of Default, Engine(EngineRef) or Only(Kind::OpenTab|History|Bookmark), plus an `Actions` scope for '>' commands. rank takes the scope: an engine scope makes the intent row 'Search <name>' with that engine's template, and an Only scope drops the other candidate lists. Keep the tie-break rules.

In CommandHead (crates/wubble-ui/ui/kit/chrome/command-head.slint), Tab or Space after an exact keyword enters the mode and Backspace on an empty field leaves it. The chip is a kit piece: compose an existing chip or badge in kit/chrome, mark it `// derived:` and add it to docs/fidelity/derived-review.md. The scope lives in the host's command-bar state (one owner) and is passed to bar_rows in crates/wubble/src/bar.rs. When a keyword is typed exactly, a row 'Press Tab to search <name>' is offered, as in Chrome. Unit tests for the scope parse and scoped ranking.

## Command bar: suggestions on open (current URL, most visited, copied link) (`omni-zero-suggest`)

Key: `omni-zero-suggest` · engine patch: no · state: not written.

Chrome's omnibox shows suggestions before you type anything: the current page's URL ready to edit, most-visited and recent sites, and 'Link you copied' when the clipboard holds a URL. In wubble, omnibox::rank returns nothing for an empty query (crates/wubble-core/src/omnibox.rs, 'An empty query yields no rows'), so an opened bar is empty.

Add a pure `fn zero_rows(current_url: Option<&str>, clipboard_url: Option<&str>, history: &[HistoryEntry], tabs: &[OpenTab], now_ms) -> Vec<OmniResult>` in omnibox.rs, with frecency from the same history_bonus. It gives:
- the current page (Kind::Search with Target::Url, subtitle 'Current page'), only in BarMode::Navigate
- the clipboard link (a new subtitle 'Link you copied'). Read the clipboard only when the bar opens and only for text that classify() makes a Url.
- up to six most-visited history entries, without the current one.

Call it from bar_rows in crates/wubble/src/bar.rs when the query is blank. The host reads the clipboard once per open through crates/wubble/src/clipboard.rs, and never in agent-session tabs (they have their own clipboard). In Navigate mode the field opens filled with the current URL and selected (take-focus already selects all), as in Chrome. The start page capsule (kit/pages/start.slint) uses the same rows. Unit tests for ordering and dedupe.

## Address field context menu: Paste and go, Paste and search, copy and edit rows (`address-paste-and-go`)

Key: `address-paste-and-go` · engine patch: no · state: not written.

Right-clicking Chrome's omnibox gives Undo, Cut, Copy, Paste, 'Paste and go to <url>' or 'Paste and search for "text"', Delete, Select all, and 'Always show full URLs'. In wubble, the command bar field (kit/chrome/command-head.slint) and the address pill (kit/chrome/address-pill.slint) have no menu. Chrome's paste-and-go is missing entirely, and Shift+Insert paste in shell fields is an open TODO.

Add a shell menu for the command field and the pill, built as Menu::Shell entries in crates/wubble/src/menu.rs, like the tab row menu:
- one table of rows
- the paste row's label from classify() over the clipboard text (crates/wubble/src/clipboard.rs): 'Paste and go to <elided url>' for Intent::Url, 'Paste and search for "…"' for Search, disabled when the clipboard is empty
- choosing it runs bar.rs target_command in the bar's mode

The pill's menu adds Copy address (Action::CopyUrl), Edit address (open the bar filled) and Paste and go. Add Action::PasteAndGo to action.rs (FIXED table and PALETTE) so the command bar and keymap can reach it; Chrome has no default chord, so leave it unbound. Add a setting `ui.full_urls`, off by default. When on, the pill shows scheme and full path instead of wubble_core::chrome::address_parts' host and path; feed it from shell.rs. Shift+Insert pastes in CommandHead's key-pressed. Unit tests for the menu labels in menu.rs.

## Back and forward history list (long press or right-click), middle-click to new tab (`nav-history-menus`)

Key: `nav-history-menus` · engine patch: no · state: not written.

In Chrome, holding or right-clicking Back or Forward opens a list of up to 15 history entries with favicons and titles, plus 'Show full history'. Picking one jumps there, and Ctrl, Cmd or middle click opens it in a new tab. A middle click on Back, Forward or Reload opens that page in a new tab. wubble's NavButtons (crates/wubble-ui/ui/components/nav-buttons.slint) has only clicked, and there is no navigation-entry access (CEF's GetNavigationEntries is not wrapped).

Engine: use the existing CDP channel (crates/wubble-engine/src/cdp.rs, View::cdp). Page.getNavigationHistory returns {currentIndex, entries[id,url,title]} and Page.navigateToHistoryEntry(entryId) jumps. Add View::navigation_history() and View::go_to_entry(id), with the reply mapped in page.rs to a PageEvent::NavigationHistory.

Core: a pure fn in wubble-core (new navlist.rs) splits the entries into back and forward lists, nearest first, capped at 15, each titled by title or URL. It has unit tests.

UI:
- NavButtons gets `pressed-long(id)` and `context(id)` callbacks (a kit hold or press motion from kit/motion/press.slint and controls/hold-button.slint for the 500 ms hold) and `middle-clicked(id)`.
- The host opens a Menu::Shell (crates/wubble/src/menu.rs) under the button with favicon rows (host/favicons) and 'Show full history' (Action::OpenHistory).
- A middle or Mod click on a row, or on Back, Forward or Reload, makes Command::NewTab with that entry's URL (a copy, as Chrome; history is not cloned).

The page context menu's Back and Forward rows keep working. Not for agent-session tabs (agent::may_go still guards the navigation).

## Reload button turns into Stop while loading; reload menu with hard reload and empty cache (`nav-stop-reload`)

Key: `nav-stop-reload` · engine patch: no · state: not written.

While a page loads, Chrome's reload button turns into an X (Stop), and back into Reload when the load ends. Shift-click or Ctrl-click does a hard reload. With DevTools open, a right-click or long press gives Normal reload, Hard reload and 'Empty cache and hard reload'. wubble has Action::Stop (Escape, Cmd+.), Action::HardReload (ViewOp::HardReload, reload_ignore_cache in crates/wubble-engine/src/view.rs) and an `address-loading` property in app.slint, but NavButtons (crates/wubble-ui/ui/components/nav-buttons.slint) always shows Reload.

Add `in property <bool> loading` to NavButtons, bound from the visible tab's loading state (the same source as address-loading in crates/wubble/src/bind.rs). The fourth button's icon, label and id become Glyph.close / 'Stop' / 'stop' while loading, swapped with a kit crossfade (kit/motion/crossfade.slint). If the kit's icon set has no stop glyph, use close and note it in docs/fidelity/derived-review.md. Add a modifiers argument to `action(...)` so the host maps Shift or Mod on reload to hard-reload. The NavButtons button table stays one table.

Add Action::EmptyCacheHardReload (FIXED table and PALETTE). It sends the CDP call Network.clearBrowserCache through View::cdp (crates/wubble-engine/src/cdp.rs), then ViewOp::HardReload. A right-click on the reload button opens a Menu::Shell (crates/wubble/src/menu.rs) with the three rows; the third is shown only when wubble_core::devtools says DevTools is open for that tab. Unit tests in perform.rs for the plans.

## History swipe on macOS and Windows touchpads (and X11) (`swipe-nav-macos-windows`)

Key: `swipe-nav-macos-windows` · engine patch: no · state: not written.

In Chrome, two fingers sideways go back or forward on every platform. On macOS that is the NSEvent scroll phases (it is the 'Swipe between pages' gesture), with the arrow bubble following the fingers. wubble's history swipe (crates/wubble-core/src/gesture.rs SwipeInput, crates/wubble/src/host/gesture.rs, chip in components/swipe-chip.slint) is fed only from Wayland through crates/wubble/src/wl. host/gesture.rs says 'Nothing reports a history swipe elsewhere yet (macOS's would be winit's PanGesture)'.

macOS: feed SwipeInput from winit's WindowEvent::PanGesture (phase Started, Moved, Ended), or from MouseWheel with TouchPhase and PixelDelta when the delta is mainly horizontal. Respect the system 'Swipe between pages' preference: read NSEvent.isSwipeTrackingFromScrollEventsEnabled through objc2 in a platform impl of one shared trait, alongside os_motion.rs and os_scope.rs. Phases end the gesture as the Wayland path does, and the PageScrolled give-way stays.

Windows and X11: precision touchpads arrive as horizontal MouseWheel PixelDelta with no phases. Add a pure idle-timeout end (no delta for 150 ms counts as release) to gesture.rs's transition as a new SwipeInput::Idle, with the timer driven by the host's tick.

Add unit tests for the idle end and the phase mapping in gesture.rs. Keep it to one path: the platform code turns OS events into SwipeInput only, and the rule stays in wubble-core.

## History page: row menu (more from this site, remove, delete site), bulk select and delete (`history-page-site-actions`)

Key: `history-page-site-actions` · engine patch: no · state: not written.

Each row on Chrome's History page has a menu: 'More from this site' (filters to that host), 'Remove from history'. Checkboxes select rows for 'Delete' as a group, and Shift-click selects a range. wubble's history page (crates/wubble/src/host/pages.rs feed_*, history_query, history_deleted; components/pages.slint) has search, a per-row delete and 'Clear history' with ClearRange. The store already has delete_history_site (crates/wubble-store/src/history.rs), but nothing calls it.

Add:
1. A row context menu (Menu::Shell, crates/wubble/src/menu.rs) with Open in new tab, Copy address, More from this site, Remove from history, Delete all from <site>. 'More from this site' sets the query to a host filter: give query_history a `site:` prefix or a host argument. Add HistoryClear::Site(host) in wubble-store, executed in crates/wubble/src/persist.rs. Deleting a site's history also removes its recently-closed entries and orphan favicons, per the 'Clearing history' decision.
2. Selection: a pure set of selected keys in wubble-core/pages.rs, with toggle, range and clear operations and unit tests. It shows as the kit check box in the list row (kit/pages/list-row.slint gains a selectable slot), with a header bar of 'N selected · Delete · Cancel' composed from kit buttons. The delete runs one store call over a list of URLs: add HistoryClear::Urls(Vec).
3. Overlaps the open TODO 'List pages … no select, delete selection': build history first, and keep the selection type generic so Bookmarks, Downloads and Archive can reuse it.

The Archive feed keeps its own delete.

## Error panel: full error table, HSTS refusals, details, HTTP error pages, form resubmission (`error-panel-completeness`)

Key: `error-panel-completeness` · engine patch: no · state: not written.

wubble's error panel (crates/wubble/src/page_error.rs PageError/ErrorText, crates/wubble-core/src/neterror.rs NAMES/describe, components/error-panel.slint) covers about 50 codes. Chrome's net error page and SSL interstitial also do the following, none of which wubble does:
1. Name and describe the rest of net_error_list.h that a page can meet: ERR_ABORTED is skipped; ERR_CACHE_MISS, ERR_BLOCKED_BY_RESPONSE, ERR_BLOCKED_BY_ADMINISTRATOR, ERR_PROXY_CONNECTION_FAILED, ERR_TUNNEL_CONNECTION_FAILED, ERR_HTTP2_PROTOCOL_ERROR, ERR_QUIC_PROTOCOL_ERROR, ERR_SSL_OBSOLETE_VERSION, ERR_BAD_SSL_CLIENT_AUTH_CERT, ERR_SSL_CLIENT_AUTH_CERT_NEEDED, ERR_CERTIFICATE_TRANSPARENCY_REQUIRED (-214), ERR_CERT_SYMANTEC_LEGACY (-215), ERR_SSL_PINNED_KEY_NOT_IN_CERT_CHAIN, ERR_ADDRESS_INVALID, ERR_DNS_* and the rest. Check each number against cef_errorcode_t.
2. Non-overridable certificate errors. When welding's on_certificate_error gets no callback (HSTS, pinned), the load fails and today shows as Load with 'Reload'. Show a Cert-style panel instead that says the site uses HSTS and cannot be visited now, with no Proceed.
3. A 'Details' disclosure (kit fold, kit/motion/fold.slint) with Chrome's suggestions per class: check the connection, proxy and firewall, DNS, clock wrong for ERR_CERT_DATE_INVALID.
4. HTTP errors: welding's LoadEnd carries http_status but the engine drops it. Route it through crates/wubble-engine/src/view.rs and page.rs. When the status is 4xx or 5xx and the body is empty (eval document.body.childElementCount == 0 after load), show Chrome's 'This page isn't working · HTTP ERROR 500' as a new PageError::Http.
5. ERR_CACHE_MISS on back or reload to a POST: Chrome's 'Confirm Form Resubmission' with a button that reloads (CEF's Reload reposts).

The transition stays the single machine. Add unit tests in page_error.rs and neterror.rs.

## Error panel: auto-reload when the network comes back (`offline-auto-reload`)

Key: `offline-auto-reload` · engine patch: no · state: not written.

Chrome's NetErrorAutoReloader reloads a page that failed with a network error (ERR_INTERNET_DISCONNECTED, ERR_NAME_NOT_RESOLVED, ERR_NETWORK_CHANGED, ERR_CONNECTION_*, ERR_ADDRESS_UNREACHABLE, timeouts). It retries on an exponential backoff (1 s, 5 s, 30 s, 1 m, 5 m, 10 m), and at once when the system comes back online, but only while the tab is visible. It never retries certificate errors, a POST, or a load the user stopped. wubble's PageError::Load (crates/wubble/src/page_error.rs) stays until the user presses Reload.

Add the retry schedule to the error machine as data: a `retry: Option<Retry { attempt, due_ms }>` inside PageError::Load. New events: Tick(now_ms), Online, Hidden, Shown and UserStopped. A new effect is ErrorEffect::Reload with a flag marking it as automatic. All of it stays pure in transition(), and the host's existing tick drives Tick. The table of retryable codes lives in wubble-core/neterror.rs (a `retryable(code)` fn ported from Chromium's net_error_auto_reloader.cc IsReloadableError, BSD-3, behaviour only).

Online detection makes no request. On Linux, watch NetworkManager's or systemd-networkd's state over D-Bus only if a crate is already in the tree; otherwise rely on the backoff alone. On macOS use SCNetworkReachability or NWPathMonitor. Put each behind one shared trait in a new small module, next to os_scope.rs. Hidden tabs and sleeping or discarded tabs (wubble_core::sleep) do not retry. The panel shows 'Trying again…' while a retry is due. Unit tests for the schedule, the visibility gating and the refusal cases.

## Connection security: certificate viewer, Not secure and mixed-content states in the address pill (`cert-viewer-security-state`)

Key: `cert-viewer-security-state` · engine patch: no · state: not written.

Chrome shows a security state for each page: secure, 'Not secure' for http or a form on http, 'Dangerous' or crossed-out https after proceeding past a certificate error, and mixed-content warnings. Its page info bubble opens a certificate viewer with subject, issuer, validity, SHA-256 fingerprints and the chain. In wubble, is_secure (crates/wubble-core/src/chrome.rs) is only a check that the URL starts with https://. welding's on_certificate_error throws away ssl_info (vendor/welding/src/handlers.rs, `_ssl_info`), and there is no viewer.

1. welding, marked as a `wubble change` in WUBBLE_CHANGES.md: read CefSSLInfo's X509 certificate in on_certificate_error into a plain `CertInfo { subject, issuer, valid_from, valid_to, sha256, chain: Vec<..>, der }`. Add a call to read the visible navigation entry's CefSSLStatus (is_secure_connection, cert_status, content_status for displayed or ran insecure content, and its certificate) after load end.
2. wubble-core: a pure `security::level(url, ssl_status, proceeded_exception) -> Level { Secure, None, NotSecure, Dangerous, MixedContent }`, with Chromium's security_state.cc rules (BSD-3, behaviour only) and unit tests. Use CertExceptions in neterror.rs to tell when the user has proceeded past an error.
3. Feed Level into the address pill in place of the `secure` bool (crates/wubble/src/shell.rs, bind.rs, kit/chrome/address-pill.slint), with a 'Not secure' or 'Dangerous' label composed from existing kit text and glyphs.
4. A certificate sheet composed from kit sheet and list rows. It opens from the pill's security glyph and from the certificate error panel's Details, and has an Export (.pem) action through the save dialog.

If a site-info scout covers the pill's permissions bubble, share that surface.

## Internal URLs: chrome:// diagnostics, about: aliases, wubble://about index, wubble://version (`internal-pages-chrome-urls`)

Key: `internal-pages-chrome-urls` · engine patch: no · state: not written.

In Chrome you can type chrome://version, gpu, net-internals, net-export, credits, media-internals, webrtc-internals, process-internals, accessibility, histograms, serviceworker-internals, indexeddb-internals, inspect, sandbox and so on. The about:X form maps to chrome://X, and chrome://about lists them all. The decision 'Internal surfaces' already allows Chromium diagnostics as web pages. But omnibox classify (crates/wubble-core/src/omnibox.rs, HIER_SCHEMES) does not take 'chrome', so 'chrome://gpu' becomes a search, and about:version is passed raw.

1. Add 'chrome' to the accepted schemes. Add a pure fn in wubble-core/pages.rs, `resolve_internal(url) -> Resolved { Native(InternalPage) | Chromium(String) | Refused }`, from one data table. Aliases go to wubble's native pages: chrome://settings, history, bookmarks, downloads, newtab, password-manager and about:settings and so on map to the wubble:// InternalPage. The WebUI hosts CEF 156 serves for alloy-style browsers pass through. Pages CEF lacks or that wubble replaces are refused (flags is covered by a separate feature).
2. Add InternalPage::About (wubble://about), a native list of every wubble:// page and the allowed chrome:// hosts, built from PAGE_LINKS and the table with kit list rows (kit/pages).
3. Map wubble://version and wubble://credits to chrome://version and chrome://credits, since CEF fills them with CEF and Chromium data. Settings' About section links them.
4. Agent-session tabs keep refusing chrome: (agent may_go and the 'Agent tabs stay on the web' decision); add a check that resolve_internal is never consulted for them.

The command bar suggests matching internal pages as Action-like rows. Unit tests for the resolver table.

## Experiments (flags-like) section: user-chosen Chromium features and switches (`experiments-flags`)

Key: `experiments-flags` · engine patch: no · state: not written.

chrome://flags lets the user turn Chromium features on and off and asks for a relaunch. CEF does not serve a working flags page for wubble's browsers. wubble builds its command line from one table, PageFeatures::knobs in crates/wubble-engine/src/app.rs (Knob::Enable, Disable, Switch, merged into a single enable-features and disable-features).

1. Add a curated data table in wubble-core (new experiments.rs): id, Chromium feature or switch, description, platforms, default, and whether it needs a relaunch. Start with entries that prebuilt CEF 156 honours and that users ask for: ElasticOverscroll, SmoothScrolling (disable-smooth-scrolling), ParallelDownloading, OverlayScrollbar, ForceDarkMode for web contents (careful: wubble has page themes), WebGPU on Linux (enable-unsafe-webgpu), Vulkan, ignore-gpu-blocklist, enable-features=BackForwardCache variants. Check each name is a string in libcef, as the existing knobs comment does.
2. Add settings `experiments.enabled` (list of ids) and `experiments.custom` (a list-editor of raw `--switch[=value]` and `+Feature` or `-Feature` entries, validated by a pure fn that refuses switches wubble owns: remote-debugging, user-data-dir, the sandbox ones).
3. In app.rs, merge them into the one enable/disable list and the switches, with the table's knobs winning over a conflicting user entry, and log what was applied.
4. Settings section 'Experiments' at wubble://flags (alias chrome://flags), built from settings::ALL rows with toggles and a relaunch toast. Off by default, and a 'Reset all' button.

Unit tests for validation and merging.

## Reading list: save for later, read and unread, page and sidebar access (`reading-list`)

Key: `reading-list` · engine patch: no · state: not written.

Chrome's reading list is a separate list from bookmarks. You add to it with 'Add to reading list' on the bookmark star, the tab context menu or the link context menu. It has 'Unread' and 'Read' sections, an item becomes read when you open it to the end or mark it, and items can be deleted. It lives in the side panel. wubble has nothing like it.

1. Store: migration crates/wubble-store/sql/017_reading_list.sql with `reading_list(id, url UNIQUE, title, added_ms, read_ms NULL)` and fns add, list, mark_read, mark_unread, delete and clear_read in a new wubble-store/src/reading.rs.
2. Core: InternalPage::ReadingList (wubble://reading-list, title 'Reading list') with a PageLink row, and a pure view fn in pages.rs that splits Unread and Read, sorts newest first and filters by query. Add Action::AddToReadingList and Action::OpenReadingList (FIXED table, PALETTE, keywords 'read later').
3. Marking read: when a reading-list URL commits in a tab, mark it read, as Chrome does on open. Do this in host/pages.rs, keyed by wubble_core::url::url_key.
4. UI: a list page from kit/pages (list-card, list-row, list-field) like History, with per-row Open, Mark read or unread and Delete. Add 'Add to reading list' rows in the tab row menu (wubble_core::tabmenu::ROWS), the link context menu (crates/wubble/src/menu.rs, group 1) and the bookmark sheet.
5. The command bar ranks reading-list items as candidates with Kind::Bookmark and subtitle 'Reading list'.

Import and export with bookmarks HTML is not needed. Unit tests for the view fn.

## Bookmarks bar (`bookmarks-bar`)

Key: `bookmarks-bar` · engine patch: no · state: not written.

Chrome's bookmarks bar is a strip under the toolbar. It shows the 'Bookmarks bar' folder's items as chips with favicons, with folders as drop-down menus and an overflow chevron. Mod+Shift+B toggles it, it shows on the new-tab page by default, items can be dragged in, out and around, and right-clicking a chip gives Open, Edit and Delete. wubble has no bookmarks bar (TODO M2 'Bookmarks: drag reorder, a bookmarks bar'), and no top toolbar by decision ('Layout'). Bookmarks have a single root (crates/wubble-store sql 001 bookmarks.parent_id NULL).

1. Store and core: a special root folder, 'Bookmarks bar' (a well-known folder created on migration, its id in settings). Chrome HTML and profile imports map their bar folder to it. pages::bookmark_tree shows it first.
2. Setting `ui.bookmarks_bar`: Never, New tab only (default) or Always. Action::ToggleBookmarksBar bound to Mod+Shift+B in keys.rs.
3. Placement: the decision rules out a toolbar, so the bar is a slim strip across the top of the content area, above the page card and split-aware. Compose it from kit chrome parts (favs chips, menu for folders, the overflow as a menu) and mark it `// derived:` on docs/fidelity/derived-review.md. The owner must approve the look before merge.
4. A folder chip opens a Menu::Shell of its children (crates/wubble/src/menu.rs), nested. Chips take drops of links and tabs, through the drag machine in wubble_core::drag and dnd and the bookmark-drop path in the 'Drops onto the shell's pages' decision. The page area shrinks by the strip's height (host/panes.rs).

Unit tests for the overflow fit and the visibility rule.

## Bookmarks: import and export the Netscape HTML file (`bookmarks-html-import-export`)

Key: `bookmarks-html-import-export` · engine patch: no · state: not written.

Chrome's bookmark manager has 'Import bookmarks' and 'Export bookmarks', using the Netscape Bookmark File format (bookmarks_DATE.html: <DL><DT><A HREF ADD_DATE ICON>, <H3> folders, the PERSONAL_TOOLBAR_FOLDER attribute for the bar). Every browser reads and writes it. wubble's importers (crates/wubble-import: places for Firefox and Zen, arc, session) do not handle HTML, and there is no export.

1. crates/wubble-import: a new html.rs with `parse_netscape(&[u8]) -> Result<Vec<BookmarkNode>, ImportError>`. It is a tolerant tag scanner, since real files are not valid HTML: case-insensitive tags, entity decoding, no closing </DT>. It reads ADD_DATE in seconds into added_ms, takes javascript: and place: URLs only as kept-but-unopened, or drops them as is_web_url does, and marks the toolbar folder. Port from Firefox's BookmarkHTMLUtils (MPL-2.0) or write it new.
2. wubble-core: a pure `export_netscape(rows: &[BookmarkRow], now_ms) -> String` with Chrome's exact header (<!DOCTYPE NETSCAPE-Bookmark-file-1>, META charset, TITLE and H1 'Bookmarks') and HTML escaping. It has unit tests that round-trip with the parser.
3. Wiring: the bookmarks page header gets 'Import' and 'Export' kit buttons. Import uses the file chooser path the host already has (crates/wubble/src/chooser.rs) and applies the nodes into a new folder 'Imported' through the store's import apply (wubble-store/src/import.rs, import_marks so a file is not imported twice). Export uses a save dialog and writes the file.
4. Add the 'Import bookmarks', 'Export bookmarks' and 'bookmarks html' keywords to PALETTE.

## Bookmark editing: edit URL, open all, bookmark all tabs, folder delete confirmation and undo (`bookmark-editing-complete`)

Key: `bookmark-editing-complete` · engine patch: no · state: not written.

Chrome's bookmark editor and manager do the following, and wubble is missing them:
- 'Edit…' changes both the name and the URL. BookmarkEdit in crates/wubble-core/src/pages.rs has Rename (title only), Move, Delete, Add and AddFolder.
- 'Open all (N)', 'Open all in new window' and 'Open all in Incognito', with a confirmation over 15 tabs.
- 'Bookmark all tabs…' (Mod+Shift+D) makes a folder of the window's tabs.
- Deleting a folder is undoable ('Undo' toast). TODO notes that wubble deletes a folder with its contents in one press.
- 'Copy' and 'Paste' of bookmarks, and 'Show in folder' from search results.

Build:
1. BookmarkEdit::Edit { id, title, url } and a store fn (crates/wubble-store/src/bookmarks.rs update_bookmark). The bookmark sheet (components/bookmark-sheet.slint, host/pages.rs sheet_view and sheet_edits) gets a URL field for the Edit kind, validated through omnibox::classify, which must be Intent::Url.
2. Action::BookmarkAllTabs (FIXED, PALETTE, keys.rs Mod+Shift+D). It opens the sheet in a new 'folder of tabs' kind and writes AddFolder plus Adds as one store transaction.
3. A row menu on bookmark and folder rows (Menu::Shell): Open in new tab, Open all (N) for folders, Edit, Move, Copy address, Delete. Open all opens Command::NewTab for each URL. Over 15 asks through the existing dialog-sheet.
4. Undo for deletes: a pure undo slot in wubble-core holding the deleted subtree (BookmarkRow list). A kit toast (components/toast.slint) with 'Undo' re-inserts it at the same parent and positions. The slot clears on the next edit.

Unit tests for subtree capture and restore, and for the sheet kinds.

## Bookmarks: drag reorder with Chrome's drop line between rows (`bookmark-drop-position`)

Key: `bookmark-drop-position` · engine patch: no · state: not written.

TODO line 159 (and the reorder in line 169). A bookmark dropped onto a folder row lands at its end. Chrome's bookmark manager and bar show a drop line between rows and insert at that position, and reorder by drag. Build: a pure `wubble_core` drop-position helper (like `tabdrag.rs`): given a pointer y over a list of row rects, it returns `Before(i)`, `Into(folder)` (the middle third of a folder row) or `After(last)`, with unit tests. Add a store operation to move or insert a bookmark at an index in a folder (`crates/wubble-store/src/bookmarks.rs`, with a position column if there is none). Drag within the Bookmarks page (`kit/pages/list-row.slint` already composes `DropMark`; show it as a line between rows) and drops from tabs or links onto the page or the sidebar's bookmark rows (crates/wubble/src/host/drag.rs) use it. A drop on a folder row's middle still goes into the folder at its end.

