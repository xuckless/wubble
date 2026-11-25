# Web view parity with Chrome

Shelved as on demand by the owner (2026-11-23): "Hardening and review are on demand. And close them as required. Hopefully the redux system would eliminate state errors".

Legend: `[ ]` open. Picked up only when the owner says so; how items move: [todo/README.md](../README.md).

The owner wants the page view to feel like Chrome's, 1:1 (2026-11-22). Five lanes of it shipped in `0.6.1` (parity, page,
dnd, popup), built, reviewed and smoke-tested; what no smoke reaches is checked by hand below. Drags inside a window,
between panes, onto the shell and to and from other apps on Wayland worked before. Pop-ups and window state:
[02-tabs-windows.md](../upcoming/02-tabs-windows.md).

- [ ] Written, to check by hand, the riskiest first:
  - [ ] Touch as touch (`wubble_core::touch`, `host/touch.rs`): on a touchscreen, `touchstart` and `pointerType` "touch" fire (else `--touch-events=enabled`), the shell still takes touches, no mouse event made of a touch reaches a page, a tap on a context menu goes to the menu, two-finger pinch
  - [ ] Wayland clipboard, every type (`wubble_core::clipboard`, `clipboard.js`): the parity smoke's B items, rich text into LibreOffice, a screenshot pasted into a page (`clipboardData.files`), files copied in Nautilus, `navigator.clipboard.read()`; Chromium's `web ` format names and the 16 MiB frame message were written from memory
  - [ ] Screen and window geometry (`welding::Desktop`): `screen.*`, `window.screenX/Y`, `outerWidth` against Chrome on two monitors and at 1.25x; a `<select>` near the window's bottom edge; macOS's flipped y; many `Moved` events on X11 during a window drag
  - [ ] Cursors (`host/cursor.rs`): every CSS cursor against Chrome, an image cursor with a hotspot (CEF's buffer is taken as premultiplied BGRA), no hidden pointer over the extension popup card
  - [ ] Wheel: a notched mouse wheel now scrolls with Blink's animation (not marked precise), a touchpad stays precise; the fling after a two-finger scroll on Wayland (`wubble_core::fling`, our own decay: tune it against Chrome)
  - [ ] The new menu rows: Open Link in New Window, Add to Dictionary (CEF's command 206), Search Google for Image (the URL is unchecked), Copy Link to Highlight, the frame rows; Paste as Plain Text now runs as an edit command on every platform
- [ ] Pump: `--measure idle` with an unshown agent tab open (pump wakes, not renders); no smoke opens one
- [ ] The macOS begin-frame throttle by content rate (`begin_frame_due`, `PaceEvent::Skipped`) has never run on a Mac: a calm page at 120 Hz, video CPU
- [ ] The parity smoke has no per-item timeout: one lost page turns into a 400 s run
- [ ] `E.hover-cursor FAIL: cursor 0` once in 10 parity runs with `--begin-frames` on Linux (2026-11-21), 0 in 8 repeats of the E items in either mode: watch for it; cause unknown
- [ ] Clipboard: a page's `navigator.clipboard.write` HTML is dropped (no host-side sanitiser); a page's copy lands a moment later (checked on a thread); hand check: a cross-origin frame with an overridden `Promise.prototype.then` calling `readText()` unfocused gets NotAllowedError, from a background tab too; Paste as Plain Text does nothing in a script-less frame (PDF)
- [ ] `clipboard::plan` and `dnd::plan` are two planners over the same MIME tables (two `Kind`s): merge them
- [ ] Touch: the view origin is frozen at `Landed` (a split spring or resize mid-touch shifts points); X11 sends one stray mouse move before the first Touch; pinch emulation ids 0/1 can collide with real fingers
- [ ] Fling: the decay curve is ours, tune it by eye against Chrome; winit gives no event timestamps; hi-res wheels count as precise and lose smooth scrolling
- [ ] `window.screenX/Y` is the content's corner (off by the title bar under a native frame); a mixed-DPI second display gets slightly wrong `screen.*` origins; the window clamp uses the whole monitor (winit has no work area)
- [ ] The back and forward mouse buttons never reach the page: CEF sends only left, middle and right, and a CDP event does not say whether the page cancelled it
- [ ] `cursor: all-scroll` arrives as CEF's autoscroll cursor and starts the autoscroll overlay: an armed state in `wubble_core::autoscroll`, entered on a middle press
- [ ] Pen: winit 0.30 has no pen type (Windows: a touch; Wayland and X11: the pointer; macOS: nothing): tablet readers per platform (build-out `pen-input`); touch selection handles need kit parts; a drag started by a long press gets no positions
- [ ] Screen: no work area (`screen.availWidth` is the monitor's); a `<select>` that overflows the window slides over its control (no flip: the popup carries no anchor); `window.open`'s `left`/`top` without a size are dropped
- [ ] Wayland clipboard: copied HTML lacks Blink's inlined styles, pasted HTML loses `<style>` formatting, screenshots still copy through arboard
- [ ] Fling: stop it on the touchpad's hold gesture (`zwp_pointer_gesture_hold_v1` in the wl worker)
- [ ] Drags to and from other apps: none on X11 and Windows beyond dropped files (`wl::Null`), macOS written and never run; custom dataTransfer types leave the window nowhere; a page drag to a second wubble window (build-out `x11-xdnd`, `windows-drag-drop`, `drag-custom-types-out`, `cross-window-page-drag`)
