// SPDX-License-Identifier: GPL-3.0-only
//
// wubble's busy probes: what in this frame must keep its tab awake.
//
// Runs in every frame next to autofill.js, in the same context script (welding change 26):
// evaluated before any page script, called once with the native `send`. It never registers a
// receiver; the browser sends it nothing. CEF 156 tells the browser none of this (no capture,
// WebRTC or unsaved-input callback), so the page itself is asked.
//
// It reports the frame's state only when it changes, as `wubble.probe <keys>`: the
// comma-separated keys of wubble-core `busy::REASONS` (capture, webrtc, input, unload, pip),
// empty when idle. The first report, at document start, is that empty "hello": every frame
// message carries the renderer's pid (welding change 28), so it tells the browser which
// process the frame runs in. The browser keeps only these keys from a probe; it never trusts a
// page with its own reasons.
//
// What counts, after Chromium's freezing policy (CannotFreezeReason) and Firefox's tab
// unloader: a live camera, microphone or screen track; a WebRTC connection that is connecting
// or connected; text the user typed into a field (passwords excluded) and has not submitted; a
// `beforeunload` handler once the user has interacted (Chrome's own condition for its prompt);
// picture-in-picture.
//
// Isolation: closure capture, as autofill.js. The page shares the heap; this script patches
// `getUserMedia`, `getDisplayMedia`, `RTCPeerConnection` and window's `addEventListener` in the
// page's world, where a page can see it. It is protection against sleeping a busy tab, not a
// security boundary.
(function (send) {
  "use strict";
  const W = window;
  const D = document;
  const PREFIX = "wubble.probe ";
  const ET = W.EventTarget.prototype;
  const listen = ET.addEventListener;
  const unlisten = ET.removeEventListener;
  const apply = Reflect.apply;
  const construct = Reflect.construct;
  const then = W.Promise.prototype.then;
  const setTimer = W.setTimeout.bind(W);
  const getTracks = W.MediaStream ? W.MediaStream.prototype.getTracks : null;
  const valueGetter = (proto) => {
    const d = proto && Object.getOwnPropertyDescriptor(proto, "value");
    return d && d.get;
  };
  const inputValue = valueGetter(W.HTMLInputElement && W.HTMLInputElement.prototype);
  const areaValue = valueGetter(W.HTMLTextAreaElement && W.HTMLTextAreaElement.prototype);
  const activation = W.navigator.userActivation;
  const TEXT_TYPES = new Set(["text", "search", "email", "url", "tel", "number", ""]);

  const tracks = new Set();
  const peers = new Set();
  const dirty = new Set();
  const unloads = new Set();
  let last = null;
  let timer = null;

  function typed(el) {
    if (W.HTMLInputElement && el instanceof W.HTMLInputElement) {
      return TEXT_TYPES.has(el.type) && inputValue.call(el) !== "";
    }
    if (W.HTMLTextAreaElement && el instanceof W.HTMLTextAreaElement) {
      return areaValue.call(el) !== "";
    }
    return !!el.isContentEditable && (el.textContent || "").trim() !== "";
  }

  function pip() {
    const dp = W.documentPictureInPicture;
    return !!(D.pictureInPictureElement || (dp && dp.window));
  }

  function state() {
    for (const t of tracks) {
      if (t.readyState !== "live") tracks.delete(t);
    }
    for (const p of peers) {
      if (p.connectionState === "closed" || p.connectionState === "failed") peers.delete(p);
    }
    // A page can empty a field in code (a chat box after sending) without an input event:
    // look again at what each one holds.
    for (const el of dirty) {
      if (!el.isConnected || !typed(el)) dirty.delete(el);
    }
    const on = [];
    if (tracks.size) on.push("capture");
    for (const p of peers) {
      const s = p.connectionState;
      if (s === "connecting" || s === "connected" || s === "disconnected") {
        on.push("webrtc");
        break;
      }
    }
    if (dirty.size) on.push("input");
    if ((unloads.size || W.onbeforeunload) && activation && activation.hasBeenActive) {
      on.push("unload");
    }
    if (pip()) on.push("pip");
    return on.join(",");
  }

  function report() {
    const now = state();
    if (now !== last) {
      last = now;
      send(PREFIX + now);
    }
    // Tracks can stop and connections close without an event this script hears: look again
    // while any is held.
    if (timer === null && (tracks.size || peers.size)) {
      timer = setTimer(() => {
        timer = null;
        report();
      }, 2000);
    }
  }

  // Capture: keep the tracks the page asked for.
  const md = W.MediaDevices && W.MediaDevices.prototype;
  for (const name of ["getUserMedia", "getDisplayMedia"]) {
    const original = md && md[name];
    if (typeof original !== "function" || !getTracks) continue;
    md[name] = function () {
      const promise = apply(original, this, arguments);
      apply(then, promise, [
        (stream) => {
          for (const t of apply(getTracks, stream, [])) tracks.add(t);
          report();
        },
        () => {},
      ]);
      return promise;
    };
  }

  // WebRTC: keep every connection the page makes.
  const RPC = W.RTCPeerConnection;
  if (typeof RPC === "function") {
    const wrapped = new Proxy(RPC, {
      construct(target, args, newTarget) {
        const pc = construct(target, args, newTarget);
        peers.add(pc);
        apply(listen, pc, ["connectionstatechange", report]);
        report();
        return pc;
      },
    });
    W.RTCPeerConnection = wrapped;
    if (W.webkitRTCPeerConnection === RPC) W.webkitRTCPeerConnection = wrapped;
  }

  // beforeunload handlers on the window.
  ET.addEventListener = function (type, listener) {
    if (this === W && type === "beforeunload" && listener) {
      unloads.add(listener);
      report();
    }
    return apply(listen, this, arguments);
  };
  ET.removeEventListener = function (type, listener) {
    if (this === W && type === "beforeunload") {
      unloads.delete(listener);
      report();
    }
    return apply(unlisten, this, arguments);
  };

  // Typed input: trusted events only; the element in a shadow root is the path's first.
  const on = (type, handler) => apply(listen, W, [type, handler, true]);
  on("input", (e) => {
    if (!e.isTrusted) return;
    const el = e.composedPath()[0];
    if (!el || !(el instanceof W.Element)) return;
    if (typed(el)) dirty.add(el);
    else dirty.delete(el);
    report();
  });
  on("submit", (e) => {
    for (const el of dirty) {
      if (el.form === e.target) dirty.delete(el);
    }
    report();
  });
  // User activation (which arms beforeunload) and picture-in-picture change on these.
  for (const type of ["pointerdown", "keydown", "enterpictureinpicture", "leavepictureinpicture"]) {
    on(type, (e) => {
      if (e.isTrusted) report();
    });
  }
  // Hidden: the sleep policy starts to care, so look again (a field emptied in code).
  apply(listen, D, [
    "visibilitychange",
    () => {
      if (D.visibilityState === "hidden") report();
    },
  ]);
  // Leaving: the frame holds nothing any more; back from the back/forward cache, say again.
  on("pagehide", () => {
    last = "";
    send(PREFIX);
  });
  on("pageshow", (e) => {
    if (e.persisted) {
      last = null;
      report();
    }
  });

  report();
})
