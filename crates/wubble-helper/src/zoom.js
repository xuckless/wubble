// SPDX-License-Identifier: GPL-3.0-only
//
// wubble's Ctrl+wheel zoom content script. Chromium zooms on Ctrl+wheel only
// when the page leaves the wheel alone (`RenderWidgetHostViewEventHandler`
// hands an unconsumed Ctrl+wheel to `WebContentsDelegate`); windowless CEF
// tells the embedder neither that nor whether a wheel was consumed. So the
// page side reports it: after a trusted Ctrl+wheel has been dispatched (a
// task later), unless the page cancelled it, `in` or `out` goes to the
// browser, which zooms one step (wubble `Action::ZoomIn` / `ZoomOut`); a
// cancelled one says `kept`. A trackpad pinch's own Ctrl+wheel is answered
// the same way and the engine keeps that answer (`wubble-engine/src/pinch.rs`:
// left alone, the pinch zooms the visual viewport; kept, the page has it).
//
// Called once per frame with this channel's `send` (main.rs `compose`). The
// listener is passive, so it never changes whether the page may cancel.
(function (send) {
  'use strict';
  const later = setTimeout;
  addEventListener('wheel', (e) => {
    if (!e.isTrusted || !e.ctrlKey || e.deltaY === 0) return;
    const step = e.deltaY < 0 ? 'in' : 'out';
    later(() => send(e.defaultPrevented ? 'kept' : step));
  }, { passive: true });
})
