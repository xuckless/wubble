// SPDX-License-Identifier: GPL-3.0-only AND MPL-2.0
//
// wubble's cosmetic filtering content script: the page half of the blocker's
// element hiding. Browser half: wubble-engine `cosmetic.rs` and `view.rs`.
//
// Ported from Brave (brave-core `components/cosmetic_filters/resources/data/`,
// MPL-2.0; THIRD_PARTY_NOTICES.md):
//  - `content_cosmetic.ts`: the class and id collector (`queueElementIdAndClasses`,
//    `fetchNewClassIdRules`, `useMutationObserver`): dedupe sets, one batched
//    query per throttle window, a MutationObserver on `id` and `class`.
//  - `procedural_filters.ts`: the operators adblock-rust can emit (`has-text`,
//    `matches-attr`, `matches-css`, `matches-path`, `min-text-length`, `upward`,
//    `xpath`, nested `css-selector`) and the actions (`remove`, `style`,
//    `remove-attr`, `remove-class`).
// Left out: Brave's first-party ad heuristics, its mutation-score scheduler and
// the C++ bridge (replaced by `send` and `register`).
//
// Protocol (all JSON, no code from the browser is evaluated here):
//   page -> browser: `hello` when the frame's context is created, then
//     {"c":[class names],"i":[ids]} batches of names not asked about yet;
//   browser -> page: {"css":[rules],"procedural":[filters],"generic":bool}.
//
// Called once per frame with this channel's `send` and `register` (main.rs
// `compose`), before any page script, so everything used is captured here.
(function (send, register) {
  'use strict';
  const MAX_BATCH = 512;     // names per message
  const MAX_TOTAL = 20000;   // names per document, then the page stops asking
  const MAX_LEN = 128;       // longer names are skipped
  const THROTTLE = 100;      // ms between messages
  const PROCEDURAL_EVERY = 250; // ms between procedural passes
  const later = setTimeout, now = Date.now, parse = JSON.parse, stringify = JSON.stringify;
  const Observer = MutationObserver, Sheet = CSSStyleSheet, doc = document, win = window;
  const getAttr = Element.prototype.getAttribute, setAttr = Element.prototype.setAttribute;
  const hasAttr = Element.prototype.hasAttribute, matchesSel = Element.prototype.matches;
  const queryAll = Element.prototype.querySelectorAll, docQueryAll = Document.prototype.querySelectorAll;
  const closestOf = Element.prototype.closest, evaluate = Document.prototype.evaluate;
  const computed = getComputedStyle, rand = Math.random;
  const marker = 'wb' + (rand() * 1e9 | 0).toString(36); // attribute for styled elements

  const queriedClasses = new Set(), queriedIds = new Set();
  let pendingClasses = [], pendingIds = [], total = 0;
  let timer = 0, lastSend = 0, lastRun = 0, generic = false, filters = [], dirty = false;
  let observer = null, sheet = null, backlog = [], waiting = false;
  const styles = new Map(); // declarations -> index in `marker` rules

  // ---- stylesheet: one constructed sheet, one rule at a time (a bad selector voids only itself)
  function flushRules() {
    if (!backlog.length) return;
    try {
      if (!sheet) {
        sheet = new Sheet();
        doc.adoptedStyleSheets = [...doc.adoptedStyleSheets, sheet];
      }
    } catch (e) {
      sheet = null;
      if (!waiting) {
        waiting = true;
        doc.addEventListener('readystatechange', () => { waiting = false; flushRules(); }, { once: true });
      }
      return;
    }
    const rules = backlog;
    backlog = [];
    for (const rule of rules) {
      try { sheet.insertRule(rule, sheet.cssRules.length); } catch (e) {}
    }
  }
  function addRules(rules) {
    for (const rule of rules) if (typeof rule === 'string') backlog.push(rule);
    flushRules();
  }

  // ---- collector (Brave `queueElementIdAndClasses`)
  function queueName(name, seen, pending) {
    if (!name || name.length > MAX_LEN || seen.has(name)) return;
    seen.add(name);
    pending.push(name);
    total++;
  }
  function queueElement(el) {
    if (total >= MAX_TOTAL || el.nodeType !== 1) return;
    const id = apply(getAttr, el, 'id');
    if (id) queueName(id, queriedIds, pendingIds);
    const cls = apply(getAttr, el, 'class');
    if (cls) for (const c of cls.split(/\s+/)) queueName(c, queriedClasses, pendingClasses);
  }
  function apply(fn, self, ...args) { return Reflect.apply(fn, self, args); }
  function scanAll() {
    const root = doc.documentElement;
    if (!root) return;
    queueElement(root);
    for (const el of apply(docQueryAll, doc, '[id],[class]')) queueElement(el);
  }
  function onMutations(mutations) {
    dirty = true;
    if (generic) {
      for (const m of mutations) {
        if (m.type === 'attributes') queueElement(m.target);
        else for (const node of m.addedNodes) {
          if (node.nodeType !== 1) continue;
          queueElement(node);
          for (const el of apply(queryAll, node, '[id],[class]')) queueElement(el);
        }
      }
    }
    schedule();
  }
  function schedule() {
    if (timer) return;
    timer = later(tick, Math.max(0, THROTTLE - (now() - lastSend)));
  }
  function tick() {
    timer = 0;
    if (generic && (pendingClasses.length || pendingIds.length)) {
      const c = pendingClasses.splice(0, MAX_BATCH), i = pendingIds.splice(0, MAX_BATCH);
      lastSend = now();
      send(stringify({ c, i }));
      if (pendingClasses.length || pendingIds.length) schedule();
    }
    if (filters.length && dirty && now() - lastRun >= PROCEDURAL_EVERY) {
      dirty = false;
      lastRun = now();
      runProcedural();
    } else if (filters.length && dirty && !timer) {
      timer = later(tick, PROCEDURAL_EVERY - (now() - lastRun));
    }
    if (total >= MAX_TOTAL) generic = false;
  }
  function observe() {
    if (observer) return;
    const root = doc.documentElement;
    if (!root) {
      doc.addEventListener('readystatechange', observe, { once: true });
      return;
    }
    observer = new Observer(onMutations);
    observer.observe(root, { subtree: true, childList: true, attributeFilter: ['id', 'class'] });
    if (generic) scanAll();
    dirty = true;
    schedule();
  }

  // ---- procedural filters (Brave `procedural_filters.ts`)
  function regexOf(arg) {
    const m = /^\/(.+)\/([a-z]*)$/.exec(arg);
    if (!m) return null;
    try { return new RegExp(m[1], m[2]); } catch (e) { return null; }
  }
  function textMatcher(arg) {
    const re = regexOf(arg);
    return re ? (s) => re.test(s) : (s) => s.includes(arg);
  }
  function exactMatcher(arg) {
    const re = regexOf(arg);
    if (re) return (s) => re.test(s);
    const unquoted = arg.replace(/^(["'])(.*)\1$/, '$2');
    if (unquoted.includes('*')) {
      const esc = unquoted.replace(/[.+?^${}()|[\]\\]/g, '\\$&').replace(/\*/g, '.*');
      const wild = new RegExp('^' + esc + '$');
      return (s) => wild.test(s);
    }
    return (s) => s === unquoted;
  }
  // `name`, `name=value`, `"name"="value"`, either side a /regex/
  function attrMatcher(arg) {
    const at = arg.indexOf('=');
    const name = exactMatcher(at < 0 ? arg : arg.slice(0, at));
    const value = at < 0 ? null : exactMatcher(arg.slice(at + 1));
    return (el) => {
      for (const a of el.attributes) {
        if (name(a.name) && (!value || value(a.value))) return true;
      }
      return false;
    };
  }
  function cssMatcher(arg, pseudo) {
    const at = arg.indexOf(':');
    if (at < 0) return () => false;
    const prop = arg.slice(0, at).trim();
    const want = exactMatcher(arg.slice(at + 1).trim());
    return (el) => want(computed(el, pseudo).getPropertyValue(prop));
  }
  // One operator as a function from an element list to an element list.
  function operator(op) {
    const arg = op.arg;
    switch (op.type) {
      case 'css-selector':
        return (els) => els.flatMap((el) => [...apply(queryAll, el, arg)]);
      case 'has-text': {
        const m = textMatcher(arg);
        return (els) => els.filter((el) => m(el.textContent || ''));
      }
      case 'matches-attr': {
        const m = attrMatcher(arg);
        return (els) => els.filter(m);
      }
      case 'matches-css': case 'matches-css-before': case 'matches-css-after': {
        const m = cssMatcher(arg, op.type === 'matches-css' ? null :
          op.type === 'matches-css-before' ? '::before' : '::after');
        return (els) => els.filter(m);
      }
      case 'matches-path': {
        const m = textMatcher(arg);
        return (els) => (m(win.location.pathname + win.location.search) ? els : []);
      }
      case 'min-text-length': {
        const n = parseInt(arg, 10);
        return (els) => els.filter((el) => (el.textContent || '').length >= n);
      }
      case 'upward': {
        const n = /^\d+$/.test(arg) ? parseInt(arg, 10) : 0;
        return (els) => els.map((el) => {
          if (n) {
            let up = el;
            for (let k = 0; k < n && up; k++) up = up.parentElement;
            return up;
          }
          return el.parentElement ? apply(closestOf, el.parentElement, arg) : null;
        }).filter(Boolean);
      }
      case 'xpath':
        return (els) => els.flatMap((el) => {
          const out = [];
          const snap = apply(evaluate, doc, arg, el, null, 7, null);
          for (let k = 0; k < snap.snapshotLength; k++) {
            const node = snap.snapshotItem(k);
            if (node && node.nodeType === 1) out.push(node);
          }
          return out;
        });
      default:
        return () => [];
    }
  }
  function styleIndex(decls) {
    let index = styles.get(decls);
    if (index === undefined) {
      index = styles.size;
      styles.set(decls, index);
      addRules(['[' + marker + '="' + index + '"]{' + decls + '}']);
    }
    return index;
  }
  function compile(json) {
    const f = typeof json === 'string' ? parse(json) : json;
    const ops = f.selector.map(operator);
    const first = f.selector[0];
    const start = first && first.type === 'css-selector'
      ? () => [...apply(docQueryAll, doc, first.arg)] : () => [doc.documentElement];
    const rest = first && first.type === 'css-selector' ? ops.slice(1) : ops;
    const act = f.action ? f.action : { type: 'style', arg: 'display: none !important' };
    return () => {
      let els = start().filter(Boolean);
      for (const op of rest) {
        if (!els.length) return;
        els = op(els);
      }
      for (const el of new Set(els)) {
        switch (act.type) {
          case 'remove': el.remove(); break;
          case 'style': apply(setAttr, el, marker, styleIndex(act.arg)); break;
          case 'remove-attr': el.removeAttribute(act.arg); break;
          case 'remove-class': el.classList.remove(act.arg); break;
        }
      }
    };
  }
  function runProcedural() {
    for (const run of filters) {
      try { run(); } catch (e) {}
    }
  }

  // ---- the browser's answers
  register((text) => {
    let reply;
    try { reply = parse(text); } catch (e) { return; }
    if (!reply || typeof reply !== 'object') return;
    if (Array.isArray(reply.css)) addRules(reply.css);
    if (Array.isArray(reply.procedural)) {
      for (const json of reply.procedural) {
        try { filters.push(compile(json)); } catch (e) {}
      }
    }
    // Only the first "generic" starts the collector: the page's own batches
    // are answered without it, and a spent MAX_TOTAL stays off.
    const starting = reply.generic === true && !generic && total < MAX_TOTAL;
    if (starting) generic = true;
    const watching = observer;
    if (generic || filters.length) observe();
    // A new observer scans by itself; a running one must be told.
    if (starting && watching) { scanAll(); schedule(); }
  });
  send('hello');
})
