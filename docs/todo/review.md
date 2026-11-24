# Review findings, design fidelity and the feel pass

Part of the roadmap ([TODO.md](../../TODO.md)). Legend: `[ ]` open · `[~]` in progress. Done items are removed: `git log` has them.

## Review findings (perpetual)
The design-fidelity phase is over; this list never closes and gates nothing: findings from reviews, QA rounds and the
owner's eye land here and are fixed as they come up.

Open, for the owner's eye:
- [ ] `GlassKeys.extension-sheet` and `.agent-sheet` are both 1000092
- [ ] Glass over white: faint ink (disabled rows, shortcuts, counts) sits near 2:1; glass reads flat grey
- [ ] Menus grow from the design's 236 to 320 for long rows (derived; on the review sheet)
- [ ] Sidebar mid-collapse shows the rail look below 140 wide (the design's own behaviour; QA read it as a reflow)
- [ ] Rail close badge size (the video strip's collapse question is under TODO.md "Open decisions")
- [ ] Find bar closes on reload (Chromium keeps it)
- [ ] Archive's icon: the stacked trays, shared with the Spaces section (the design's set has no archive glyph);
      Passwords/Sites share the lock, Updates/Downloads the arrow
- [ ] Spaces popup: its alignment (Space tile's left edge), no rename/delete on a row
- [ ] Media downloader: a video a service worker answers is not seen by the detection (the worker's own fetch has no tab; `--smoke media=probe` on fixtures `sw`): the save control says nothing was found; fullscreen shows the strip without its glass (the scene does not render there): look by hand; an embedded YouTube player (an iframe of youtube.com) has no lookup, since the lookup keys on the tab's own address
- [ ] Video strip button reads "Save video"; a second click mid-download re-enters the downloader
- [ ] The derived-pieces review sheet ([docs/fidelity/derived-review.md](../fidelity/derived-review.md))

Open, to build or check:
- [ ] From PR #4's review (2026-11-22), not fixed in the merge: a headless agent session's dialogs are answered for it (alert accepted, confirm and prompt cancelled, leave-page accepted) and the tool call in flight is not told, and the log line says "refused" whatever the answer; a sign-in copy of which no cookie was accepted now stops a starting session (`CookiesNotCopied`): owner's call whether that is wanted; the approval sheet counts its open Selects, so one destroyed while open (the bindings rewritten) leaves Escape dead; a second Save click while a media download runs does nothing and says nothing; a failed ffmpeg join now discards the fetched tracks; `space-switcher.slint` got a spacing the design does not have and `crossfade.slint` a clip that may cut the swap motion (derived-review sheet); a scroll thumb that hides mid-press loses its release
- [ ] The scene's partial marking (PR #4): after a scene render only the strips around the one page and the overlay glass are redrawn, when that page fills the content area; checked with `SLINT_SKIA_PARTIAL_RENDERING=verify` on the video, overlay, page, split and tabs smokes (0 mismatches), not yet on a tab drag preview, a history swipe or a split closing to one pane
- [ ] QA retry by hand: Alt+1/2 Spaces, Shift+click peek, audible badge on a playing YouTube tab (tests pass; not reproduced)
- [ ] The command bar's panel still takes clicks while it shrinks away (176 ms): a click on the page under it then is lost; check the design's exit (pointer-events)
- [ ] In-place rename for a new Space in the sidebar (needs a design); Linux alternates for the chords GNOME takes
- [ ] Audible badge: the probe misses iframes, shadow DOM and WebAudio (a real signal needs the CEF patch, M6)
- [ ] Page copy buttons carry text only (no image items)
- [ ] Start page tiles are stand-ins: the kit tile look and its hover spring
- [ ] Settings against the web: Reset per section; rows folding while searching
- [ ] Gallery: scenes for the hover preview card and the sidebar's empty-area menu; the page menu's position varies by page
- [ ] Spell check in other languages: the English dictionary is bundled; add an opt-in (off by default) download of others, from `redirector.gvt1.com` or a mirror, into `<profile>/Dictionaries`
- [ ] Shift+Insert paste in shell fields (Slint has no such shortcut); tests for the list editor draft and middle-click
- [ ] `--smoke overlay=dark` fails a hue check under the command bar (also before the glass fix)
- [ ] The autofill key chip sits over the page's top left and is not pane-aware in split view: it needs the focused field's rect from the engine
- [ ] Rebinding the sidebar shortcut also unbinds its second chord, Mod+Alt+S (once compact mode's), and the shortcut editor shows only the first chord
- [ ] Narrow Settings (content width 759 px or less): the slider and select rows do not wrap to full width as the design's narrow block does (`settings.css`:120-130); repro `WUBBLE_SHOT_SIZE=960x640 --smoke settings --shot`
- [ ] The built-in pages' list rows take Tab and Enter but not Up/Down, and a focused row is not scrolled into view
- [ ] macOS: a held key's repeats reach the page with `repeat: false` (CEF 156 builds its NSEvent with `isARepeat:NO`): a one-line CEF patch, then drop parity's `A.space-repeat` Mac gap; check YouTube's Space tap and hold-for-2x on the Mac
- [ ] The settings search no longer finds "Your own filter lists" (the setting moved to `Section::Hidden`; the Filter lists section is not searchable)
- [ ] A Space button opened by keyboard does not give the Spaces menu the keyboard focus
- [ ] A popover's entrance marks the whole window dirty (the glass's scene scale and alpha change each frame), so partial rendering does not help during it

## Small and unordered
- [ ] Undo close tab (Mod+Z) and Mod+Shift+T bring back the URL only: no split, no back/forward history (the recently-closed list would need CEF's navigation entries)
- [ ] Build the engine with patch 40, then verify the touchpad pinch road (`--smoke pinch` takes it when the marker is there; unit-tested only so far); macOS engine too, and a real trackpad on GNOME and the Mac (the smoke injects through `Host::pinch`)
- [ ] The Settings Select's list is drawn by Slint (no blur): the scene's window-level glass lies over the page; the design's blur needs the page's lists in a window-level layer (owner's call)
- [ ] Gallery: a Settings section picked far from where the page is is shot mid-jump (`settings-extensions` on the patched engine shows Filter lists passing): wait for the page's spring to land, or jump without it in the gallery
- [ ] Hand check: the rail's sound badge on a tab that really plays audio (the gallery fakes the audible state; no smoke presses mute), and Settings' Spaces list with an agent Space

## Design fidelity
Rules: [docs/agents/design-fidelity.md](../agents/design-fidelity.md).
- [ ] Empty the kit lock baseline (107 rows, mostly `kit/derived.slint` literals); alias the value-equal `Shell` literals
- [ ] Voice glow shader into the kit (no microphone)

## Feel pass (before release)
- [ ] Stretch and squash on every pressable and moving surface (menus, popovers, sheets, toasts, command bar, strip)
- [ ] Liquid glass under every floating surface, with the flat fallback kept
- [ ] The glowing ring and border beam wherever something is focused, loading or newly opened
- [ ] One audit: walk every gallery surface against the three effects
