# Build-out batch 3

15 features, one branch each (`feat/<key>`, worktree `~/Dev/wubble-wt/feat/<key>/w`). Status is kept in [README.md](README.md).

## `site-layout-fixes`: wubble.cc: feature section recomposed, readable diagrams and install capsule on phones

Area: todo-a. Needs an engine patch: no.

TODO lines 25-26. The site in `site/` (plain HTML/CSS/JS, `site/README.md`) shows its feature section as a list: bullets on the left, an image on the right, four times, which leaves empty left columns on wide screens. Recompose it from the owner's design pages (`xuckless/the-wallpaper-project` `src/wubble/pages/`, cloned beside the repo; pull it first). Use alternating or grid feature blocks as the design lays them out, with the same copy (no new sentences: copy is the owner's). Also fix two narrow-screen problems. (1) The inline SVG diagrams' text cannot be read at 360 px: give them a viewBox that scales, larger minimum font sizes under a `@media (max-width: 480px)` rule, or a stacked phone variant of each diagram. (2) The install command is cut off in its capsule: let the capsule scroll sideways (`overflow-x:auto`, `white-space:nowrap`, the copy button pinned) or wrap at the shell separators. Files: `site/index.html`, `site/assets/css/*.css`, the SVGs under `site/assets/`. Keep a 16 px side gutter and no horizontal page scroll at 360 px. Do not deploy: `npx wrangler deploy` is the owner's step.

## `agent-ax-lazy-mirror`: Agent sessions: apply hidden tabs' accessibility updates lazily

Area: todo-b. Needs an engine patch: no.

Chrome builds no accessibility tree for a hidden tab unless something reads it. In wubble, with no screen reader running, every tab of an agent session still keeps CEF's tree on, and each tree or location event is parsed and applied at once (`Host::ax_event` in crates/wubble/src/host/ax.rs, `AxValue::from_json` then `PageTree::apply`/`move_nodes`). Ten idle tabs in one session cost 6.6 of the main thread's 8.8 % busy time (TODO Optimization 2a).

To build: in `WebAx` (host/ax.rs), give each tab that is driven only by an agent (`agent_drives(tab) && !self.ax.on`) a queue of raw event strings (`Vec<(String, bool)>`, capped; on overflow, drop the queue and mark the tree for a full re-read). `ax_event` only pushes onto that queue and does no JSON parsing. Replace `ax_tree(&self)` with `ax_tree_mut`/`ax_settle(tab)`, which drains the queue into the `PageTree` the first time a call needs it. Its callers in crates/wubble/src/host/agent.rs (`read_page`, refs, `read_text`, the viewport at ~2396/2511/2562/2583/2885/4657) call it at the start of the call. The navigate wait needs to know that "the new document's tree arrived". Keep that answer cheap: detect a root or document change from the event header without a full parse, or settle the queue only when a waiting call asks. Optional second step: switch CEF's tree off for a session tab that is hidden and that no call has read for N seconds (`View::set_accessibility(false)`), and on again with one fresh full tree on the next call. Unit-test the queue and settle rules as a pure helper (wubble-ax or a small module). Linux and macOS.

## `omni-zero-suggest`: Command bar: suggestions on open (current URL, most visited, copied link)

Area: navigation. Needs an engine patch: no.

Chrome's omnibox shows suggestions before you type anything: the current page's URL ready to edit, most-visited and recent sites, and 'Link you copied' when the clipboard holds a URL. In wubble, omnibox::rank returns nothing for an empty query (crates/wubble-core/src/omnibox.rs, 'An empty query yields no rows'), so an opened bar is empty.

Add a pure `fn zero_rows(current_url: Option<&str>, clipboard_url: Option<&str>, history: &[HistoryEntry], tabs: &[OpenTab], now_ms) -> Vec<OmniResult>` in omnibox.rs, with frecency from the same history_bonus. It gives:
- the current page (Kind::Search with Target::Url, subtitle 'Current page'), only in BarMode::Navigate
- the clipboard link (a new subtitle 'Link you copied'). Read the clipboard only when the bar opens and only for text that classify() makes a Url.
- up to six most-visited history entries, without the current one.

Call it from bar_rows in crates/wubble/src/bar.rs when the query is blank. The host reads the clipboard once per open through crates/wubble/src/clipboard.rs, and never in agent-session tabs (they have their own clipboard). In Navigate mode the field opens filled with the current URL and selected (take-focus already selects all), as in Chrome. The start page capsule (kit/pages/start.slint) uses the same rows. Unit tests for ordering and dedupe.

## `wayland-clipboard-rich`: Wayland: page clipboard carries HTML, images, files and web custom formats

Area: page-interaction. Needs an engine patch: no.

Chrome on Wayland copies a selection as text/plain and text/html. It copies images (Copy image, `navigator.clipboard.write` with image/png) and "web " custom formats, and pastes HTML, images (a screenshot into Gmail or Docs) and copied files (text/uri-list, x-special/gnome-copied-files) as `paste` events with `clipboardData.files`. In wubble the pages' clipboard on Wayland goes through crates/wubble-helper/src/clipboard.js and `PageEvent::Clipboard`/`ClipboardRequest` (crates/wubble-engine/src/page.rs) to the worker in crates/wubble/src/wl/wayland.rs. All of it is text only: `TEXT_TYPES`, `send_selection` refuses other types, and paste is one `insertText`. Images go through `shot_io`'s arboard. TODO.md lists "non-text page clipboard formats; custom MIME to other apps" and "Page copy buttons carry text only". To build: extend the bridge protocol to carry a typed item list. clipboard.js captures `setData` for text/html and custom types on trusted copy/cut, and wraps `navigator.clipboard.write`/`read` (ClipboardItem blobs as base64, capped). The worker stores a multi-type `CopyPasteSource` (text, text/html, image/png, `chromium/x-web-custom-data` from the pickle helper in `wubble_core::dnd`) and on load reads the best types. On paste, clipboard.js dispatches a `paste` event whose DataTransfer holds the HTML and any `File`s (image bytes, or file paths read through the host as Chrome reads them). Unless the page cancels it, the HTML goes in with `insertHTML` (Paste) or plain text goes in (Paste as plain text, cef_id::PASTE_MATCH_STYLE). Move Copy Image onto this path on Wayland so one owner sets the selection. Pure type and priority tables go in wubble-core with tests.

## `container-management`: Containers: create, rename, delete, and assign from the UI

Area: privacy-security. Needs an engine patch: no.

wubble's containers are Firefox-style cookie jars (decision "Containers" and "Web storage per container" in docs/decisions.md). `wubble_core::model` already has `Command::NewContainer`, `DeleteContainer`, `SetSpaceContainer` and `ReopenInContainer` (crates/wubble-core/src/model.rs:186-200), and `Engine::forget_container` deletes a container's storage. But only agent sessions and the media smoke call them, so no user can make one (TODO "No way to create, rename or delete a container").

Build:
- Add `Command::RenameContainer { id, name }` to model.rs. Names are trimmed, non-empty and unique case-insensitively, and Default cannot be renamed or deleted. Agent containers are never listed or editable. Write unit tests in model/tests.rs.
- Add a Settings section "Containers" in crates/wubble-ui/ui/components/settings.slint, made only from kit components: a ListEditor with the columns Name, Used by (the Spaces that use it), and adds/rename/remove. Delete asks first through the existing dialog-sheet ("Delete Work? Its cookies, site data and cache are deleted"). Spaces that used it fall back to Default, which model.rs already does.
- Add a Space menu row "Container ▸" listing containers plus "New container…" (crates/wubble-core/src/spacemenu.rs), dispatching SetSpaceContainer.
- Add a tab menu row "Reopen in container ▸" (crates/wubble-core/src/tabmenu.rs) that dispatches ReopenInContainer.
- Wire it in crates/wubble/src/host/settings.rs and menu.rs. Persist through crates/wubble/src/persist.rs, which already stores containers.
- The routing rules' Container column (`routing.rules`) can then name user containers.

## `screen-share`: getDisplayMedia: ask to share the screen, then show a sharing bar with Stop

Area: web-platform-ui. Needs an engine patch: no.

In Chrome, getDisplayMedia opens a picker, and while capture runs a bar says "<site> is sharing your screen" with Stop sharing and Hide. In wubble, welding decodes the desktop bits (DesktopAudioCapture, DesktopVideoCapture from decode_media), but prompt.rs `permission_of` maps both to None, so every getDisplayMedia is denied. CEF's media_access_query.cc grants the full primary screen (or the id the page asked for), plus "loopback" system audio when desktop audio is requested. On Linux Wayland, Chromium's PipeWire capturer shows the desktop portal's own source picker after the grant.
Build: (1) Add Permission::ScreenCapture to wubble_core::site, with ask_text "See your screen" (Chromium's string) and an optional system-audio line. Never remember an Allow for it: Chrome asks every time, so the prompt shows no remember toggle for this kind. (2) Map DesktopVideoCapture/DesktopAudioCapture to it in prompt.rs. (3) Start a sharing indicator from the grant. It ends when the page's track stops: extend wubble-helper/src/probes.js, which already reports `capture`, to say `display` separately. It also ends when the tab closes or navigates. The indicator is a kit notice in the pane (overlay/a/a-capsule-bar.slint or a-toast.slint as a derived piece, listed on docs/fidelity/derived-review.md) reading "<site> is sharing your screen", with a "Stop sharing" button and a "Hide" button. Stop runs a page script through View::eval that stops every live display track (the probe keeps a WeakRef list of MediaStreamTracks with displaySurface settings). (4) Agent tabs keep denying, as decided.

## `download-multiple-permission`: Automatic (multiple) downloads site permission

Area: media-downloads. Needs an engine patch: no.

When a page starts a second download with no gesture, Chrome's DownloadRequestLimiter asks "This site is trying to download multiple files" (Allow / Block). The answer is remembered per site, and Site settings has an "Automatic downloads" row. In CEF 156 this arrives as CEF_PERMISSION_TYPE_MULTIPLE_DOWNLOADS (1 << 14) on OnShowPermissionPrompt. In wubble the bit is broken today: vendor/welding/src/permissions.rs does not name it, so it decodes to PermissionKind::Other(16384), and wubble_core::site::Permission denies every request naming an unknown kind. A site's second automatic download is therefore always refused, silently. Separately, welding's `can_download` (vendor/welding/src/handlers.rs ~1081) ignores its url argument.

What to build: (1) In welding, mark the changes `wubble change` and list them in WUBBLE_CHANGES.md. Add MULTIPLE_DOWNLOADS = 1 << 14 and `PermissionKind::MultipleDownloads`, with a decode test. Pass the url and the browser id from can_download through a host hook (a `CanDownload` question answered synchronously from a host-set table), so a per-site Block takes effect before anything starts. (2) In wubble-core/src/site.rs, add `Permission::AutomaticDownloads` (stored name "automatic-downloads") to ALL, with the prompt wording "Download multiple files" and tests. (3) In crates/wubble-engine/src/page.rs, map the new kind in the engine permission mapping. In the host's permission routing, use the existing permission prompt (components/permission-prompt.slint). (4) Add the row to Settings' Sites list, where it can be revoked like the others. (5) Agent-driven tabs keep refusing, as today. All platforms.

## `pw-export-csv`: Passwords: export to CSV

Area: passwords-autofill-sync. Needs an engine patch: no.

Chrome's Password Manager exports all saved passwords to a CSV with the header `name,url,username,password,note` after OS re-authentication, warning that the file is unencrypted. wubble only imports (`VaultCommand::ImportCsv`, crates/wubble-vault/src/command.rs; parser in crates/wubble-vault/src/import.rs). Build: a pure `wubble_vault::export::to_csv(rows: &[(Meta, username, password, note)]) -> Zeroizing<String>` writing Chromium's exact format (name = host without scheme, url = origin + "/", RFC 4180 quoting, CRLF as Chromium's `password_csv_writer.cc`, BSD-3, credited in THIRD_PARTY_NOTICES.md); a round-trip test through `parse_csv`. `VaultCommand::ExportCsv(PathBuf)` run on the store thread (crates/wubble/src/persist.rs `vault_command`): decrypt every readable row, write with 0600 permissions (create_new, then rename), zeroize buffers, answer `VaultAnswer::Exported(count)`; unreadable rows are listed as skipped. UI: an "Export" ghost button beside "Import CSV" in `PasswordsPage` (crates/wubble-ui/ui/components/pages.slint), a confirm sheet (existing `dialog-sheet.slint`/kit Sheet) saying the file is not encrypted, then the save dialog via `crate::chooser` with `Pick::Save` (as `passwords_import_requested` in crates/wubble/src/host/pages.rs does for open), default name `wubble Passwords.csv`. Gate it behind the OS re-auth from feature `pw-os-reauth` when that exists (call through one trait so this works without it). Linux, macOS, Windows.

## `devtools-dock-sides`: DevTools dock positions: right, bottom, left, undocked

Area: devtools-extensions-internals. Needs an engine patch: no.

Chrome's DevTools menu (the three-dot menu, then Dock side) offers dock to right, bottom, left, or undock into a separate window. It remembers the choice per profile, and Ctrl+Shift+D cycles through the positions. wubble docks only at the right edge (`host/devtools.rs`: the `PANEL_TAB` pane, with the `SourceSeam` width from `wubble_core::split::SourceSeam`). Detached means CEF's own native window (`View::open_devtools_at`), which is not a wubble window. TODO M4 lists the bottom dock as needing vertical pane rects. To build: (1) In `wubble-core`, add a `DockSide` enum {Right, Bottom, Left, Undocked} to `devtools::Dock::Open { tab, side }`, and add `DockEvent::SetSide(DockSide)` and `CycleSide` with pure transitions and tests. Persist the side as a hidden setting `devtools.dock_side` (`settings.rs`, `Section::Hidden`). (2) Pane geometry: extend `crates/wubble/src/panes.rs` and `host/panes.rs::pane_layout`/`layout_panes` so the panel can take a horizontal strip at the bottom, with a height seam. `SourceSeam` gets an axis, or a second seam instance owned by `Docked`. (3) Undocked: open a wubble window of a new `WindowKind::DevTools` (`wubble_core::window`) that shows only the panel view, so it works on Wayland and composes with the shell. Its close sends `DockEvent::PanelClosed`. CEF's native window stays only for the no-bridge fallback. (4) The frontend's `requestSetDockSide`/`setIsDocked` (from devtools-pipe-bridge) drive `SetSide`, and the frontend is told the current side through `DevToolsAPI.setDockSide`. (5) Add an `Action::DevToolsCycleDock` with the chord Ctrl+Shift+D (Cmd+Shift+D on Mac) in `keys.rs`, PageFirst, active only while the panel has focus. UI: any new seam handle comes from kit components. Shoot the bottom dock and the undocked window with `--shot`.

## `windows-agent-paths`: Windows: agent app config paths and the MCP manifest dir

Area: platform. Needs an engine patch: no.

TODO.md MCP section: "Windows: the agent apps' paths (`agent::harness` lists Linux and macOS only) and the manifest's runtime dir." `crates/wubble-core/src/agent/harness.rs` has `Os::Windows` but every `files`/`detect` table lists only `(Os::Linux, ...)` and `(Os::Mac, ...)` (`LM` at :80). Add the Windows entries per harness, relative to the user profile (`%USERPROFILE%`) or to `%APPDATA%`. Examples: Claude Desktop `AppData/Roaming/Claude/claude_desktop_config.json`; VS Code `AppData/Roaming/Code/User/mcp.json`; Claude Code `~/.claude.json`; Codex `~/.codex/config.toml`; Gemini `~/.gemini/settings.json`; opencode `~/.config/opencode/opencode.json`; Cursor `~/.cursor/mcp.json`; Windsurf `~/.codeium/windsurf/mcp_config.json`. Check each against the app's own docs before adding it. The harness takes a home path today: if an entry needs `%APPDATA%`, give `Os::Windows` entries a base (Home or AppData) and pass both from the edge (`crates/wubble/src/mcp.rs` or wherever `detected`/`file` are called). Manifest: `instance::private_root` already gives `%LOCALAPPDATA%` on Windows. Check that the manifest writer (`agent/manifest.rs` and its caller) creates `<LOCALAPPDATA>/wubble/mcp/`, sets an owner-only ACL there instead of the Unix 0700 mode, and that the relay (`wubble mcp`, `mcp::hosting_browsers`) reads it there. Unit tests in harness.rs for the Windows rows, as the existing Linux and macOS ones do.

## `undo-close-split-entries`: Undo close tab and Mod+Shift+T bring back the split and record the tab's navigation entries

Area: todo-a. Needs an engine patch: no.

TODO line 38. Today `wubble_core::closed` (`Closed { url, closed_at_ms, space, place }`) and the store's `ClosedTab` (`crates/wubble-store/src/closed.rs`: url, title, container, space) restore only the URL. Chrome's TabRestoreService restores the tab's place, its group and its navigation stack. Embedder half: (1) Split. When a tab that sat in a split (`wubble_core::split`) closes, record its partner tab and its side/ratio in `Closed` (an optional `split: Option<SplitPlace>`), keep it in the store row (a new nullable column with a migration in wubble-store), and on `Host::reopen_closed` (crates/wubble/src/host.rs ~2142) re-form the split with the partner if it is still open; else open as a plain tab. Do the same for a whole split pair closed together (Chrome restores a window's tabs together). (2) Navigation entries. On close, collect the view's entries through CEF `BrowserHost::get_navigation_entries` (a `NavigationEntryVisitor` in crates/wubble-engine/src/view.rs; URL, title, current index) and keep them in a new `closed_entries` table keyed by the closed row. Show them in the History page's recently-closed rows and the reopen menu as a tooltip/second line (for example "and 4 earlier pages"). Actually rebuilding the back/forward stack needs the engine patch (feature `nav-history-restore-patch`); until then the reopened tab loads the current entry only. Tests stay out; transition logic stays in `closed::transition`.

## `content-driven-frame-rate`: Content-driven frame rate in wubble_gfx::pacing

Area: todo-b. Needs an engine patch: no.

Chrome paints a page only when it has new content and never more than once per display refresh. A still page sends no frames, a video paints at its own rate, and input or animation paint at vsync. In wubble, CEF's rate is fixed at the display's refresh (`pacing::view_frame_rate`, set by `View::set_frame_rate` in crates/wubble-engine/src/view.rs:876 from host.rs:1136/3641). Under `Pacing::Shell`, the begin-frame driver (`BeginFrames`, `transition`) sends a frame on every vsync until the page has left 4 unanswered (TODO Optimization 1b and "Content-driven frame rate").

To build: a pure lifecycle in crates/wubble-gfx/src/pacing.rs, `enum ContentRate { Still, Video { fps }, Interactive { until } }`, with `fn content_transition(state, ContentEvent) -> (state, Vec<RateEffect>)`. The events are FrameArrived { damage share, at }, Input, ShellMotion, VideoPlaying { fps }, VideoStopped and Tick(now). The effects are SetRate(u32) for CEF's timer, or Hold/Resume begin frames for Shell pacing. Video fps comes from a page script that reports `requestVideoFrameCallback` intervals. Add it to the existing media probe in crates/wubble/src/host/media.rs or videos.rs, through `View::eval` or the helper channel. Interactive lasts for a short window after input or shell motion and then falls back to whatever the content shows. Still drops CEF's timer to 1 fps (CEF's minimum) or stops sending begin frames. The host also presents at most one page frame per refresh: when several arrive between renders, only the newest is shown. Wire it per view next to the existing `view_frame_rate` calls in host.rs, and keep `--measure` counters for each state. Unit tests for the transition. All platforms.

## `address-paste-and-go`: Address field context menu: Paste and go, Paste and search, copy and edit rows

Area: navigation. Needs an engine patch: no.

Right-clicking Chrome's omnibox gives Undo, Cut, Copy, Paste, 'Paste and go to <url>' or 'Paste and search for "text"', Delete, Select all, and 'Always show full URLs'. In wubble, the command bar field (kit/chrome/command-head.slint) and the address pill (kit/chrome/address-pill.slint) have no menu. Chrome's paste-and-go is missing entirely, and Shift+Insert paste in shell fields is an open TODO.

Add a shell menu for the command field and the pill, built as Menu::Shell entries in crates/wubble/src/menu.rs, like the tab row menu:
- one table of rows
- the paste row's label from classify() over the clipboard text (crates/wubble/src/clipboard.rs): 'Paste and go to <elided url>' for Intent::Url, 'Paste and search for "…"' for Search, disabled when the clipboard is empty
- choosing it runs bar.rs target_command in the bar's mode

The pill's menu adds Copy address (Action::CopyUrl), Edit address (open the bar filled) and Paste and go. Add Action::PasteAndGo to action.rs (FIXED table and PALETTE) so the command bar and keymap can reach it; Chrome has no default chord, so leave it unbound. Add a setting `ui.full_urls`, off by default. When on, the pill shows scheme and full path instead of wubble_core::chrome::address_parts' host and path; feed it from shell.rs. Shift+Insert pastes in CommandHead's key-pressed. Unit tests for the menu labels in menu.rs.

## `ctx-menu-link-page-frame`: Context menu: the missing link, selection, page and frame rows

Area: page-interaction. Needs an engine patch: no.

Chrome's `RenderViewContextMenu` has rows that wubble's `ROWS` (crates/wubble/src/menu.rs) lacks. Link: "Open link in new window" (now possible: `Action::NewWindow` and `Command::MoveToWindow` exist; the comment above `ROWS` saying one window is stale), "Open link in split view" (wubble has split: `Command::SplitWith`), an "Open link in Glance" equivalent (`ModelCommand::OpenGlance`), and "Copy link text". Selection: "Copy link to highlight", which puts a `#:~:text=` URL on the clipboard, built as Chromium's `shared_highlighting` text-fragment generator does (port the generator to a helper page script, BSD-3, run with `View::run_script_in` in `ContextMenu::frame`), plus a search row for a selected URL that offers "Go to <url>". Frame (when `frame_url` is non-empty and differs from `page_url`): "Reload frame" (script `location.reload()` in the frame) and "View frame source" (view-source: of frame_url via the existing `Action::ViewSource` path, parameterised by URL). Editable: "Emoji" on macOS (`orderFrontCharacterPalette:`) and Windows (the Win+. panel through `CoreInputView`/SendInput), plus "Writing direction" on macOS. Keep the Chromium group order, with dividers from the `row!` group numbers. Add `Command` variants as needed and execute them where the host runs `menu::Command` (crates/wubble/src/host). Unit-test each row's predicate in menu.rs's tests as the existing ones do.

## `routing-rule-integrity`: Routing rules survive Space/container renames and show broken rules

Area: privacy-security. Needs an engine patch: no.

Open TODO under Privacy and containers. `routing.rules` (crates/wubble-core/src/routing.rs, Settings ListEditor) names Spaces and containers by display name. Renaming or deleting one silently turns the rule off. Space names are not unique, and a name containing `,` or `->` cannot be written in a rule.

Build:
- Store rules by id (SpaceId and ContainerId) and keep names only for display. Change the serialized form in settings.rs (kind List) to `pattern -> space:<id> [container:<id>]`. Add a migration from the old name form that resolves names once at load and keeps the original text for rows it cannot resolve.
- Add a pure `routing::resolve(rules, model) -> Vec<RuleState>` with the states Active, SpaceGone and ContainerGone. The Settings list shows a broken rule with a mark and the text "Space deleted" (a kit ListEditor row state; if no kit piece fits, a derived piece goes on docs/fidelity/derived-review.md).
- Make Space names unique: `model::Command::RenameSpace` / NewSpace append " 2" etc., like Chrome profile names.
- Change the row label to say that typed addresses and page links are not routed.
- Write unit tests in routing.rs and model tests.

