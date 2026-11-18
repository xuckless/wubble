// wubble's link hints, in the page (SPDX-License-Identifier: GPL-3.0-only).
//
// Measures and draws only: which label goes where, and what typing does, is the host's
// (`wubble_core::hints`). Runs as part of one expression the host builds (`pagescripts.rs`);
// nothing is put on `window`, and each call stands alone (the labels are found again by their
// element's name).
//
// Which elements are clickable and whether one is really visible is ported from Vimium
// (`content_scripts/link_hints.js` `getLocalHintsForElement`, `content_scripts/dom_utils.js`
// `getVisibleClientRect`; MIT, Phil Crosby and Ilya Sukhar), trimmed to the main frame.
var wubbleHints = (function () {
  var CLICKABLE =
    "a[href],area[href],button,input:not([type=hidden]),select,textarea,summary,label[for]," +
    "details,[onclick],[contenteditable=''],[contenteditable=true],[tabindex]:not([tabindex='-1'])," +
    "[role=button],[role=link],[role=tab],[role=checkbox],[role=radio],[role=menuitem]," +
    "[role=menuitemcheckbox],[role=menuitemradio],[role=option],[role=switch],[role=textbox]";
  var MAX = 300;
  var TAG = "wubble-hints";

  // The part of `el` on screen and not covered by something else, viewport CSS px.
  function visibleRect(el, vw, vh) {
    var rects = el.getClientRects();
    for (var i = 0; i < rects.length; i++) {
      var r = rects[i];
      var x0 = Math.max(0, r.left),
        y0 = Math.max(0, r.top);
      var x1 = Math.min(vw, r.right),
        y1 = Math.min(vh, r.bottom);
      if (x1 - x0 < 3 || y1 - y0 < 3) continue;
      var hit = document.elementFromPoint((x0 + x1) / 2, (y0 + y1) / 2);
      if (hit && (hit === el || el.contains(hit) || hit.contains(el))) {
        return { x: x0, y: y0, w: x1 - x0, h: y1 - y0 };
      }
    }
    return null;
  }

  function collect() {
    var vw = innerWidth,
      vh = innerHeight,
      out = [];
    var all = document.querySelectorAll(CLICKABLE);
    for (var i = 0; i < all.length && out.length < MAX; i++) {
      var el = all[i];
      if (el.disabled || el.closest("[aria-disabled=true]")) continue;
      var style = getComputedStyle(el);
      if (style.visibility !== "visible" || style.pointerEvents === "none") continue;
      var r = visibleRect(el, vw, vh);
      if (!r) continue;
      r.href = typeof el.href === "string" ? el.href : "";
      out.push(r);
    }
    return out;
  }

  function clear() {
    var old = document.querySelectorAll(TAG);
    for (var i = 0; i < old.length; i++) old[i].remove();
    return true;
  }

  // `items`: [{x, y, label}]; the first `typed` letters of each label are shown as typed.
  function draw(items, typed, c) {
    clear();
    var host = document.createElement(TAG);
    var s = host.style;
    s.setProperty("all", "initial", "important");
    s.setProperty("position", "fixed", "important");
    s.setProperty("inset", "0", "important");
    s.setProperty("pointer-events", "none", "important");
    s.setProperty("z-index", "2147483647", "important");
    var root = host.attachShadow({ mode: "closed" });
    var sheet = new CSSStyleSheet();
    sheet.replaceSync(
      ".h{position:absolute;padding:1px 4px;border-radius:5px;background:" + c.label +
        ";color:" + c.ink + ";font:600 11px/1.3 " + c.font + ";letter-spacing:.04em;" +
        "text-transform:uppercase;box-shadow:0 1px 4px rgba(0,0,0,.35);white-space:nowrap}" +
        ".h b{font-weight:600;opacity:.45}"
    );
    root.adoptedStyleSheets = [sheet];
    items.forEach(function (it) {
      var span = document.createElement("span");
      span.className = "h";
      span.style.left = it.x + "px";
      span.style.top = it.y + "px";
      var done = document.createElement("b");
      done.textContent = typed;
      span.append(done, it.label.slice(typed.length));
      root.append(span);
    });
    document.documentElement.append(host);
    return true;
  }

  return { collect: collect, draw: draw, clear: clear };
})();
