# Dropped video frames with the page on its layer (macOS)

Shelved by the owner (2026-11-24): "Just ship it. 0.6.2 and shelve this bug for later testing".

Legend: `[ ]` open. Picked up only when the owner says so; how items move: [todo/README.md](../README.md).

- [ ] Seen once on an M2, 1080p60 AV1 video with the page on its layer (window mode): YouTube's stats reported about
  904 dropped frames in about 152 s of a session that included resizes (about 10 %), while wubble's counters showed
  every delivered frame shown (71 to 90 paints a second delivered, 0 pool drops, main process 14 to 15 %). The owner
  thinks the Mac was overloaded at the time. Not compared against `--page-layer=off` on the same build.
- Suspects, unverified: on the layer the begin frames come from a timer not locked to the display; each snapshot is
  presented inside a Core Animation transaction that blocks the main thread (frames arrive late and in bunches: about
  49 a second median in an earlier log); the app logs the display as 60 Hz (`CEF frame rate 60 (display Some(60000) mHz)`)
  although the owner's built-in display runs ProMotion at 120 Hz.
- First checks when picked up: (1) the same video for two minutes without resizing with `--page-layer=off`, and with
  `--page-layer=window --no-begin-frames`; (2) then present with the transaction only when a route or placement
  changes; (3) then why the refresh rate reads 60.
