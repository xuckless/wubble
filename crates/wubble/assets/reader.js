// wubble's reader mode, in the page (SPDX-License-Identifier: GPL-3.0-only).
//
// Runs as part of one expression the host builds (`pagescripts.rs`): Mozilla's
// `isProbablyReaderable` and `Readability` (assets/reader/, Apache-2.0) are declared in the same
// function scope, so nothing is put on `window`. Each entry returns plain JSON.
//
// The reader replaces the page's document element in place with DOM calls only: no HTML strings
// (Trusted Types), no inline <style> or style attributes (CSP): the stylesheet is adopted through
// CSSOM, the typography is custom properties set on the root's style. Leaving the reader reloads
// the page. The colours and sizes come from the host (`wubble_core::reader::css_vars`).
//
// The reading position: the reader opens at the passage the page showed at its top, and the
// page comes back where it was. Chromium would restore the reloaded page to the reader's scroll
// offset (the reader shared its history entry), so `leave` turns that off before the host
// reloads (a page may not reload itself to a `data:` URL) and `restore` scrolls the loaded page
// itself, then turns it back on.
var wubbleReader = (function () {
  var CSS =
    ":root{background:var(--wb-bg);color-scheme:light dark}" +
    "body{margin:0;background:var(--wb-bg);color:var(--wb-ink);font-family:var(--wb-font);" +
    "font-size:var(--wb-size);line-height:1.65;-webkit-font-smoothing:antialiased;" +
    "text-rendering:optimizeLegibility}" +
    "main{max-width:var(--wb-width);margin:0 auto;padding:56px 24px 120px}" +
    "header{border-bottom:1px solid var(--wb-rule);padding-bottom:1.1em;margin-bottom:1.6em}" +
    ".wb-site,.wb-byline{margin:0 0 .5em;color:var(--wb-muted);" +
    "font:500 .78em/1.4 system-ui,-apple-system,sans-serif;letter-spacing:.01em}" +
    "h1{font-size:1.9em;line-height:1.18;margin:0 0 .45em;letter-spacing:-.01em}" +
    "h2,h3,h4,h5,h6{line-height:1.3;margin:1.7em 0 .5em}" +
    "p,ul,ol,dl,table,pre,blockquote,figure{margin:0 0 1.1em}" +
    "a{color:var(--wb-link);text-decoration-thickness:1px;text-underline-offset:.16em}" +
    "img,video,svg,canvas,iframe{max-width:100%;height:auto}" +
    "img{display:block;margin:1.4em auto;border-radius:12px}" +
    "figure{margin:1.6em 0}figcaption{color:var(--wb-muted);font-size:.84em;text-align:center}" +
    "blockquote{padding:0 0 0 1em;border-left:3px solid var(--wb-rule);color:var(--wb-muted)}" +
    "pre,code,kbd,samp{font-family:ui-monospace,'Geist Mono',SFMono-Regular,Menlo,monospace;" +
    "font-size:.86em}" +
    "pre{padding:1em 1.1em;border-radius:12px;overflow-x:auto;line-height:1.5;" +
    "background:color-mix(in srgb,var(--wb-ink) 6%,transparent)}" +
    "table{border-collapse:collapse;display:block;overflow-x:auto}" +
    "td,th{border:1px solid var(--wb-rule);padding:.35em .6em}" +
    "hr{border:0;border-top:1px solid var(--wb-rule);margin:2em 0}" +
    "::selection{background:color-mix(in srgb,var(--wb-link) 35%,transparent)}";

  var URL_ATTRS = ["href", "src", "action", "formaction", "xlink:href", "poster"];

  // Readability does not sanitise: drop handlers, script URLs and anything active.
  function clean(root) {
    var gone = root.querySelectorAll("script,style,link,iframe,object,embed,form,noscript");
    for (var i = 0; i < gone.length; i++) gone[i].remove();
    var all = [root].concat(Array.prototype.slice.call(root.querySelectorAll("*")));
    all.forEach(function (el) {
      Array.prototype.slice.call(el.attributes).forEach(function (a) {
        var name = a.name.toLowerCase();
        var script = URL_ATTRS.indexOf(name) >= 0 && /^\s*javascript:/i.test(a.value);
        if (name.indexOf("on") === 0 || name === "style" || script) el.removeAttribute(a.name);
      });
    });
  }

  function el(tag, cls, text) {
    var e = document.createElement(tag);
    if (cls) e.className = cls;
    if (text) e.textContent = text;
    return e;
  }

  function set(vars) {
    var s = document.documentElement.style;
    Object.keys(vars).forEach(function (k) {
      s.setProperty(k, vars[k]);
    });
    return true;
  }

  function probe() {
    return isProbablyReaderable(document);
  }

  var BLOCKS = "p,li,h2,h3,h4,h5,h6,blockquote,pre,figcaption,dd";

  function words(e) {
    return (e.textContent || "").replace(/\s+/g, " ").trim();
  }

  // The first passage of text at or below the top of the view, its opening words; "" at the
  // top of the page, which the reader opens at anyway.
  function passageAtTop() {
    if (scrollY <= 0) return "";
    var all = document.querySelectorAll(BLOCKS);
    for (var i = 0; i < all.length; i++) {
      var r = all[i].getBoundingClientRect();
      if (r.bottom <= 0 || r.height === 0) continue;
      if (r.top >= innerHeight) return "";
      var text = words(all[i]);
      if (text.length >= 20) return text.slice(0, 60);
    }
    return "";
  }

  // The reader's block that starts with `passage`, scrolled to just below the top.
  function scrollToPassage(passage) {
    if (!passage) return;
    var all = document.querySelectorAll(BLOCKS);
    for (var i = 0; i < all.length; i++) {
      if (words(all[i]).indexOf(passage) === 0) {
        scrollTo(0, Math.max(0, all[i].getBoundingClientRect().top + scrollY - 24));
        return;
      }
    }
  }

  // The article's text for an agent (`read_text`), the page left as it is: the title, the
  // byline and the passages (the innermost blocks, one per paragraph); the page's own text when
  // Readability finds no article.
  function text() {
    var article = null;
    try {
      article = new Readability(document.cloneNode(true), {
        serializer: function (node) {
          return node;
        },
      }).parse();
    } catch (e) {}
    if (!article || !article.content) {
      var body = document.body ? document.body.innerText : "";
      return { article: false, title: document.title, byline: "", text: body };
    }
    var parts = [];
    var blocks = article.content.querySelectorAll(BLOCKS + ",h1");
    for (var i = 0; i < blocks.length; i++) {
      if (blocks[i].querySelector(BLOCKS)) continue;
      var t = words(blocks[i]);
      if (t) parts.push(t);
    }
    var all = parts.length ? parts.join("\n\n") : words(article.content);
    return {
      article: true,
      title: article.title || document.title,
      byline: article.byline || "",
      text: all,
    };
  }

  function leave() {
    try {
      history.scrollRestoration = "manual";
    } catch (e) {}
    return true;
  }

  function restore(x, y) {
    try {
      history.scrollRestoration = "auto";
    } catch (e) {}
    scrollTo(x, y);
    return true;
  }

  function open(vars) {
    var article = new Readability(document.cloneNode(true), {
      serializer: function (node) {
        return node;
      },
    }).parse();
    if (!article || !article.content) return null;
    var at = { x: Math.round(scrollX), y: Math.round(scrollY) };
    var passage = passageAtTop();
    var content = article.content;
    clean(content);
    var title = article.title || document.title;
    // The page stops: its loads, then its timers (ids count up from 1 in Blink).
    window.stop();
    var last = setTimeout(function () {}, 0);
    for (var t = 0; t <= last; t++) {
      clearTimeout(t);
      clearInterval(t);
    }
    var html = el("html");
    if (article.lang) html.lang = article.lang;
    if (article.dir) html.dir = article.dir;
    var head = el("head");
    var meta = el("meta");
    meta.name = "viewport";
    meta.content = "width=device-width, initial-scale=1";
    head.append(meta, el("title", "", title));
    var header = el("header");
    if (article.siteName) header.append(el("p", "wb-site", article.siteName));
    header.append(el("h1", "", title));
    if (article.byline) header.append(el("p", "wb-byline", article.byline));
    var main = el("main", "wb-reader");
    var body = el("article");
    body.append(content);
    main.append(header, body);
    var page = el("body");
    page.append(main);
    html.append(head, page);
    document.replaceChild(html, document.documentElement);
    var sheet = new CSSStyleSheet();
    sheet.replaceSync(CSS);
    document.adoptedStyleSheets = [sheet];
    set(vars);
    scrollTo(0, 0);
    scrollToPassage(passage);
    return { title: title, length: article.length, x: at.x, y: at.y };
  }

  return { probe: probe, open: open, set: set, leave: leave, restore: restore, text: text };
})();
