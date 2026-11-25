// wubble's page kit: run at the start of every document (`Page.addScriptToEvaluateOnNewDocument`,
// `host/boost.rs`), before the page's own scripts. It keeps one object on the window under
// `Symbol.for("wubble.kit")` (not enumerable, defined once, so running it again changes nothing)
// for the host's polls (`View::eval`) and commands:
//
// - video: the page's main <video> as a snapshot (`snapshot()`), commands (`cmd`), a frame as a
//   PNG (`frame()`), and `jumps`, the one skipping primitive: a table of [from, to) ranges set by
//   the host (SponsorBlock's policy is all in Rust, `wubble_core::sponsor`); a video playing
//   inside one goes to its end and the range's id is reported once in the next snapshot.
// - media session: the page's own handlers (`navigator.mediaSession.setActionHandler`) are kept
//   so the media keys can call them (`act`), and its metadata is read for the system's controls.
// - Boosts: `css` (one adopted style sheet, so no `style-src` policy blocks it), `ready`
//   (DOMContentLoaded), `matches` (whether this document is one of a Boost's pattern).
// - zap: an element picker; its selector is a port of Chromium DevTools' `DOMPath.ts`
//   `cssPath` (front_end/panels/elements/DOMPath.ts, BSD-3-Clause), shortened.
//
// Nothing here talks back on its own: the host asks. A page can read and call the kit; nothing it
// can do through it makes wubble act, since the host checks every value it reads.
(function () {
  "use strict";
  const KEY = Symbol.for("wubble.kit");
  if (window[KEY]) return;
  const k = {};
  Object.defineProperty(window, KEY, { value: k });

  // ----- the main video: the largest visible one, a playing one first
  function visible(v) {
    const r = v.getBoundingClientRect();
    return r.width > 1 && r.height > 1 && r.bottom > 0 && r.right > 0 &&
      r.top < innerHeight && r.left < innerWidth ? r.width * r.height : 0;
  }
  // The page's videos: the document's own, and when it has none on screen, those inside open
  // shadow roots (web-component players: a `querySelectorAll` does not reach into them).
  function shadowVideos(root, out) {
    for (const el of root.querySelectorAll("*")) {
      const sr = el.shadowRoot;
      if (!sr) continue;
      out.push(...sr.querySelectorAll("video"));
      shadowVideos(sr, out);
    }
    return out;
  }
  // The walk costs a pass over the DOM: at most once a second, the list kept between.
  let shadowList = [], shadowAt = -1e9;
  function primary() {
    let best = pick(document.querySelectorAll("video"));
    if (!best) {
      const now = performance.now();
      if (now - shadowAt > 1000) { shadowList = shadowVideos(document, []); shadowAt = now; }
      best = pick(shadowList);
    }
    return best;
  }
  function pick(list) {
    let best = null, bestScore = 0;
    for (const v of list) {
      const area = visible(v);
      if (!area) continue;
      const score = area * (v.paused ? 1 : 4);
      if (score > bestScore) { best = v; bestScore = score; }
    }
    return best;
  }
  k.primary = primary;
  // The video under a point of the view (physical px, view-relative: the context menu's), else
  // the main one.
  function at(x, y) {
    const e = Number.isFinite(x) && Number.isFinite(y)
      ? document.elementFromPoint(x / devicePixelRatio, y / devicePixelRatio) : null;
    return (e && e.closest("video")) || primary();
  }

  // ----- jumps: the host's skip table
  let jumps = [];
  let fired = [];
  function check(v) {
    const t = v.currentTime;
    for (const j of jumps) {
      if (t >= j.from && t < j.to - 0.05) {
        v.currentTime = j.to;
        if (!fired.includes(j.id)) fired.push(j.id);
        return;
      }
    }
  }
  for (const type of ["timeupdate", "playing", "seeked", "loadedmetadata"]) {
    document.addEventListener(type, (e) => {
      if (jumps.length && e.target instanceof HTMLVideoElement) check(e.target);
    }, true);
  }
  k.jumps = function (table) {
    jumps = (Array.isArray(table) ? table : []).filter((j) =>
      Number.isFinite(j.from) && Number.isFinite(j.to) && j.to > j.from);
    const v = primary();
    if (v && jumps.length) check(v);
    return jumps.length;
  };

  // ----- the page's media session
  const handlers = {};
  if (window.MediaSession && MediaSession.prototype.setActionHandler) {
    const set = MediaSession.prototype.setActionHandler;
    MediaSession.prototype.setActionHandler = function (action, handler) {
      handlers[action] = handler;
      return set.call(this, action, handler);
    };
  }
  k.act = function (action) {
    const h = handlers[action];
    if (typeof h === "function") { h({ action }); return true; }
    const v = primary() || document.querySelector("video, audio");
    if (!v) return false;
    if (action === "play") v.play(); else if (action === "pause") v.pause();
    else if (action === "playpause") (v.paused ? v.play() : v.pause());
    else if (action === "stop") { v.pause(); v.currentTime = 0; }
    else return false;
    return true;
  };
  function meta() {
    const m = navigator.mediaSession && navigator.mediaSession.metadata;
    const art = m && m.artwork && m.artwork.length ? m.artwork[m.artwork.length - 1].src : "";
    return {
      title: (m && m.title) || document.title,
      artist: (m && m.artist) || "",
      album: (m && m.album) || "",
      art: String(art || ""),
      next: typeof handlers.nexttrack === "function",
      prev: typeof handlers.previoustrack === "function",
    };
  }

  // The largest visible embedded frame (a cross-origin player the kit cannot run in): where the
  // strip offers the save control over what the downloader saw of the page. The host shows it
  // only when the downloader lists something, so an ad frame gets nothing. Its address goes
  // with it: an embedded YouTube player is looked up as its watch page would be.
  function bigFrame() {
    let best = null, bestArea = 0;
    for (const f of document.querySelectorAll("iframe, embed, object")) {
      const r = f.getBoundingClientRect();
      const w = Math.min(r.right, innerWidth) - Math.max(r.left, 0);
      const h = Math.min(r.bottom, innerHeight) - Math.max(r.top, 0);
      if (w < 200 || h < 112 || getComputedStyle(f).visibility === "hidden") continue;
      if (w * h > bestArea) { best = { r, src: String(f.src || f.data || "").slice(0, 2048) }; bestArea = w * h; }
    }
    return best;
  }

  // ----- the snapshot the host polls
  k.snapshot = function () {
    const v = primary();
    // The poll checks too: timeupdate comes only every quarter second or so.
    if (v && jumps.length && !v.paused) check(v);
    const out = { v: null, fired, zap: zapState(), fs: !!document.fullscreenElement };
    fired = [];
    // While there is media, playing or not: the system's controls show it paused too.
    const media = v || document.querySelector("video, audio");
    if (media) { out.meta = meta(); out.meta.paused = media.paused; }
    if (v) {
      const r = v.getBoundingClientRect();
      out.v = {
        x: r.left, y: r.top, w: r.width, h: r.height,
        t: v.currentTime, dur: Number.isFinite(v.duration) ? v.duration : null,
        paused: v.paused, rate: v.playbackRate, loop: v.loop, muted: v.muted,
        // Which document answered: unique per document, larger for a later one.
        doc: performance.timeOrigin,
        // EME MediaKeys attached: protected media, which the downloader never touches.
        drm: !!v.mediaKeys,
        // Its subtitle files (`<track>`), which load only when shown: the downloader lists them.
        tracks: [...v.querySelectorAll("track")]
          .filter((t) => (t.kind === "subtitles" || t.kind === "captions") && /^https?:/.test(t.src))
          .slice(0, 16)
          .map((t) => ({
            src: String(t.src).slice(0, 2048),
            lang: String(t.srclang || "").slice(0, 16),
            label: String(t.label || "").slice(0, 80),
          })),
      };
    } else if (window === window.top) {
      const f = bigFrame();
      if (f) {
        const r = f.r;
        out.v = {
          x: r.left, y: r.top, w: r.width, h: r.height, t: 0, dur: null, paused: true,
          rate: 1, loop: false, muted: false, doc: performance.timeOrigin, drm: false,
          frame: true, src: f.src,
        };
      }
    }
    return out;
  };

  k.cmd = function (name, arg) {
    const v = primary() || document.querySelector("video");
    if (!v) return false;
    switch (name) {
      // The default too: loading a source resets the speed to it.
      case "rate": v.defaultPlaybackRate = arg; v.playbackRate = arg; break;
      case "loop": v.loop = !!arg; break;
      case "muted": v.muted = !!arg; break;
      case "toggle": if (v.paused) v.play(); else v.pause(); break;
      case "play": v.play(); break;
      case "pause": v.pause(); break;
      case "seek": v.currentTime = arg; break;
      default: return false;
    }
    return true;
  };

  // The frame on screen as PNG base64 (`null` for a cross-origin video the canvas may not read):
  // the main video's, or with a point the one under it.
  k.frame = function (x, y) {
    const v = at(x, y);
    if (!v || !v.videoWidth) return null;
    try {
      const c = document.createElement("canvas");
      c.width = v.videoWidth;
      c.height = v.videoHeight;
      c.getContext("2d").drawImage(v, 0, 0);
      return c.toDataURL("image/png").split(",")[1];
    } catch (e) {
      return null;
    }
  };

  // Picture-in-picture: the main video alone, fullscreen in the tab's own view; `pipAt`, the
  // one under a point (the context menu's).
  function pipOf(v) {
    if (!v) return Promise.reject(new Error("no video"));
    if (v.disablePictureInPicture) return Promise.reject(new Error("the page disallows it"));
    return v.requestFullscreen().then(() => true);
  }
  k.pip = function () { return pipOf(primary()); };
  k.pipAt = function (x, y) { return pipOf(at(x, y)); };

  // ----- Boosts
  let sheet = null;
  k.css = function (text) {
    try {
      if (!sheet) {
        sheet = new CSSStyleSheet();
        document.adoptedStyleSheets = [...document.adoptedStyleSheets, sheet];
      }
      sheet.replaceSync(String(text || ""));
    } catch (e) {
      console.error("wubble boost css:", e);
    }
  };
  k.ready = function (f) {
    if (document.readyState === "loading") {
      document.addEventListener("DOMContentLoaded", f, { once: true });
    } else {
      f();
    }
  };
  // A Boost's pattern (`wubble_core::boost::Pattern::js`) against this document, by the same
  // rules as `Pattern::matches`: the top document only, http(s), the host exact or a domain and
  // its subdomains, a glob over path and query.
  function glob(p, text) {
    const re = p.split("*").map((s) => s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")).join(".*");
    return new RegExp("^" + re + "$", "s").test(text);
  }
  k.matches = function (p) {
    if (window !== window.top || !p) return false;
    const scheme = location.protocol.slice(0, -1);
    if (scheme !== "http" && scheme !== "https") return false;
    if (p.s !== "web" && p.s !== scheme) return false;
    const h = location.hostname;
    if (p.h === "exact" && h !== p.n) return false;
    if (p.h === "domain" && h !== p.n && !h.endsWith("." + p.n)) return false;
    const href = location.href.split("#")[0];
    const q = href.indexOf("?");
    return glob(String(p.p), location.pathname + (q < 0 ? "" : href.slice(q)));
  };

  // ----- zap: pick an element, get a selector for it
  let zap = null;
  function step(node) {
    const id = node.id;
    if (id && document.querySelectorAll("#" + CSS.escape(id)).length === 1) {
      return { text: "#" + CSS.escape(id), unique: true };
    }
    const tag = node.localName;
    const parent = node.parentElement;
    if (!parent) return { text: tag, unique: false };
    const siblings = [...parent.children];
    const same = siblings.filter((s) => s.localName === tag);
    if (same.length === 1) return { text: tag, unique: false };
    const classes = [...node.classList].filter((c) =>
      same.every((s) => s === node || !s.classList.contains(c)));
    if (classes.length) {
      return { text: tag + "." + classes.map((c) => CSS.escape(c)).join("."), unique: false };
    }
    return { text: tag + ":nth-child(" + (siblings.indexOf(node) + 1) + ")", unique: false };
  }
  function cssPath(node) {
    const steps = [];
    for (let n = node; n && n.nodeType === 1 && n !== document.documentElement; n = n.parentElement) {
      const s = step(n);
      steps.unshift(s.text);
      if (s.unique) break;
    }
    return steps.join(" > ");
  }
  function zapState() {
    if (!zap) return null;
    const r = { on: zap.on, selector: zap.selector };
    if (!zap.on) zap = null;
    return r;
  }
  function stopZap(selector) {
    if (!zap || !zap.on) return;
    removeEventListener("pointermove", zap.move, true);
    removeEventListener("click", zap.click, true);
    removeEventListener("keydown", zap.key, true);
    if (zap.hit) zap.hit.removeAttribute("data-wubble-zap");
    zap.sheet.replaceSync("");
    zap.on = false;
    zap.selector = selector;
  }
  k.zap = function (on) {
    if (!on) { stopZap(null); return true; }
    if (zap && zap.on) return true;
    const outline = new CSSStyleSheet();
    outline.replaceSync("[data-wubble-zap]{outline:2px solid #7cc4ff!important;" +
      "outline-offset:-1px!important;background:rgba(124,196,255,.18)!important}");
    document.adoptedStyleSheets = [...document.adoptedStyleSheets, outline];
    zap = { on: true, selector: null, hit: null, sheet: outline };
    zap.move = (e) => {
      const t = e.target;
      if (!(t instanceof Element) || t === zap.hit) return;
      if (zap.hit) zap.hit.removeAttribute("data-wubble-zap");
      zap.hit = t;
      t.setAttribute("data-wubble-zap", "");
    };
    zap.click = (e) => {
      e.preventDefault();
      e.stopPropagation();
      if (e.target instanceof Element) stopZap(cssPath(e.target));
    };
    zap.key = (e) => {
      if (e.key === "Escape") { e.preventDefault(); stopZap(null); }
    };
    addEventListener("pointermove", zap.move, true);
    addEventListener("click", zap.click, true);
    addEventListener("keydown", zap.key, true);
    return true;
  };
})();
