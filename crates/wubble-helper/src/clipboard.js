// SPDX-License-Identifier: GPL-3.0-only
//
// wubble's clipboard content script: the page's clipboard through the window,
// for windowless Chromium on Wayland, whose own clipboard cannot reach the
// system's (no surface: no input serial to set the selection with, no keyboard
// focus to be offered one). Runs only when the browser launched the renderer
// with `--wubble-clipboard-bridge` (wubble-engine `EngineConfig`).
//
// Called once per frame with this channel's `send(text)` and `register(fn)`
// (main.rs `compose`, on welding's context script, change 26). To the browser,
// `kind\ntext`:
//   copy     a trusted copy or cut (the selection's text and HTML, or what the
//            page's `copy` listener set: text, HTML, other types as Chromium's
//            custom data), `navigator.clipboard.writeText` or `write` (text,
//            HTML, PNG, `web ` formats): the page JSON below
//   image    a copy in an image document: the picture's URL (Chromium copies
//            the picture there)
//   primary  the selection changed: the primary selection's text
//   paste    Chromium's trusted paste was cancelled: answer with the clipboard
//            ("primary" after a middle click in a field, "plain" after
//            Ctrl+Shift+V, Paste as Plain Text)
//   read     `navigator.clipboard.readText` passed its permission check: the id
//   items    `navigator.clipboard.read` passed its permission check: the id
// From the browser, `read id or ""\njson`: the read's answer, or a paste,
// pasted as Chromium pastes (a `paste` event carrying it, then, unless the page
// cancels that, one undoable edit: the HTML, else the picture, into rich text;
// the text into a field).
//
// The page JSON is `wubble_core::clipboard::Payload`'s: `{text, html, files:
// [{name, type, data}], custom: [[type, value]], web: [[type, data]]}`, bytes
// as base64.
//
// Isolation: closure capture, as autofill.js; trusted events only.
(function (send, register) {
  'use strict';
  const apply = Reflect.apply;
  const indexOf = String.prototype.indexOf;
  const slice = String.prototype.slice;
  const startsWith = String.prototype.startsWith;
  const charCodeAt = String.prototype.charCodeAt;
  const fromCharCode = String.fromCharCode;
  const parse = JSON.parse;
  const stringify = JSON.stringify;
  const encode = TextEncoder.prototype.encode;
  const encoder = new TextEncoder();
  const blobText = Blob.prototype.text;
  const blobBuffer = Blob.prototype.arrayBuffer;
  // The longest message the browser takes (welding's `MAX_FRAME_MESSAGE_BYTES`, which drops a
  // longer one), less room for the channel's and kind's names.
  const MESSAGE_CAP = (16 << 20) - 64;
  const own = new WeakSet();
  const reads = new Map();
  let nextRead = 1;
  const post = (kind, text) => send(kind + '\n' + String(text));
  const toBase64 = (blob) => apply(blobBuffer, blob, []).then((buffer) => {
    const bytes = new Uint8Array(buffer);
    let binary = '';
    for (let i = 0; i < bytes.length; i += 0x8000) {
      binary += apply(fromCharCode, null, bytes.subarray(i, i + 0x8000));
    }
    return btoa(binary);
  });
  const fromBase64 = (text) => {
    const binary = atob(text);
    const bytes = new Uint8Array(binary.length);
    for (let i = 0; i < binary.length; i++) bytes[i] = apply(charCodeAt, binary, [i]);
    return bytes;
  };
  // A copy as the page JSON. What would take the message past the cap is left out, the
  // largest kinds first and the text last.
  const postCopy = (payload) => {
    for (const part of ['files', 'web', 'custom', 'html', 'text']) {
      const json = stringify(payload);
      if (apply(encode, encoder, [json]).length <= MESSAGE_CAP) return post('copy', json);
      delete payload[part];
    }
  };
  const field = (a) => a && (a instanceof HTMLInputElement || a instanceof HTMLTextAreaElement);
  const selected = () => {
    const a = document.activeElement;
    if (field(a)) {
      try {
        if (typeof a.selectionStart === 'number') return a.value.slice(a.selectionStart, a.selectionEnd);
      } catch (e) {}
    }
    return String(getSelection());
  };
  // The selection's markup (a field's has none). Blink also inlines the computed style and
  // wraps the enclosing elements; this is the selected nodes alone.
  const selectedHtml = () => {
    const s = getSelection();
    if (field(document.activeElement) || !s || s.isCollapsed) return '';
    const box = document.createElement('div');
    for (let i = 0; i < s.rangeCount; i++) box.appendChild(s.getRangeAt(i).cloneContents());
    return box.innerHTML;
  };
  // A document that is one picture (Chromium's image document): its URL.
  const imageDocument = () => {
    const images = document.images;
    if (!apply(startsWith, document.contentType, ['image/']) || images.length !== 1) return '';
    return images[0].currentSrc || images[0].src;
  };
  // `setData`'s type as Blink keeps it (`NormalizeType`).
  const ALIASES = { __proto__: null, text: 'text/plain', url: 'text/uri-list' };
  const normalized = (format) => {
    const type = String(format).trim().toLowerCase();
    if (apply(startsWith, type, ['text/plain;'])) return 'text/plain';
    return ALIASES[type] || type;
  };
  // What a `copy` listener set, as Chromium writes it: text, HTML, the rest as custom data.
  const fromSetData = (set) => {
    const payload = { text: set.get('text/plain'), html: set.get('text/html'), custom: [] };
    for (const [type, value] of set) {
      if (type !== 'text/plain' && type !== 'text/html') payload.custom.push([type, value]);
    }
    return payload;
  };
  for (const type of ['copy', 'cut']) {
    addEventListener(type, (e) => {
      if (!e.isTrusted) return;
      const set = new Map();
      const data = e.clipboardData;
      if (data) {
        const setData = data.setData;
        const clearData = data.clearData;
        data.setData = function (format, value) {
          set.set(normalized(format), String(value));
          return apply(setData, this, [format, value]);
        };
        data.clearData = function (format) {
          if (format === undefined) set.clear();
          else set.delete(normalized(format));
          return apply(clearData, this, format === undefined ? [] : [format]);
        };
      }
      const text = selected();
      const html = selectedHtml();
      const image = imageDocument();
      setTimeout(() => {
        if (e.defaultPrevented) {
          if (set.size) postCopy(fromSetData(set));
        } else if (image) {
          post('image', image);
        } else if (text || html) {
          postCopy(html ? { text, html } : { text });
        }
      });
    }, true);
  }
  // Where the next paste comes from: "primary" after a middle press, "plain" after
  // Ctrl+Shift+V (Blink's Paste as Plain Text), "" otherwise. A middle click in a field pastes
  // the primary selection, as on X11 (Blink's `PasteGlobalSelection`). Chromium's own runs
  // only where its clipboard has a selection buffer, which a windowless Wayland one does not;
  // when it does run, its paste below is answered from the primary selection too.
  let from = '';
  const editable = (t) => t instanceof Element && (t.isContentEditable ||
    t instanceof HTMLTextAreaElement || (t instanceof HTMLInputElement && !t.readOnly && !t.disabled));
  addEventListener('mousedown', (e) => { from = e.isTrusted && e.button === 1 ? 'primary' : ''; }, true);
  addEventListener('mouseup', (e) => {
    if (!e.isTrusted || e.button !== 1 || from !== 'primary' || !editable(e.target)) return;
    setTimeout(() => { if (from === 'primary') { from = ''; post('paste', 'primary'); } });
  }, true);
  addEventListener('keydown', (e) => {
    from = e.isTrusted && e.ctrlKey && e.shiftKey && !e.altKey && e.code === 'KeyV' ? 'plain' : '';
  }, true);
  addEventListener('paste', (e) => {
    if (!e.isTrusted || own.has(e)) return;
    e.preventDefault();
    e.stopImmediatePropagation();
    post('paste', from);
    from = '';
  }, true);
  let primaryTimer = 0;
  document.addEventListener('selectionchange', () => {
    clearTimeout(primaryTimer);
    primaryTimer = setTimeout(() => {
      const text = selected();
      if (text) post('primary', text);
    }, 60);
  }, true);
  // Pasted markup without active content: no scripts, frames, plugins, styles or SVG
  // animation, no event handler attributes, no `javascript:` URLs. Parsed in an inert document.
  // (Blink's paste goes further: it keeps only what renders, with its computed style.)
  const DROPPED = ['script', 'style', 'link', 'meta', 'title', 'base', 'iframe', 'frame',
    'frameset', 'object', 'embed', 'noscript', 'template', 'animate', 'set', 'animateMotion',
    'animateTransform'];
  const URLS = ['href', 'src', 'action', 'formaction', 'xlink:href', 'data', 'poster', 'background'];
  const sanitized = (html) => {
    const doc = new DOMParser().parseFromString(html, 'text/html');
    for (const node of doc.querySelectorAll(DROPPED.join(','))) node.remove();
    for (const node of doc.querySelectorAll('*')) {
      for (const attribute of Array.from(node.attributes)) {
        const name = attribute.name.toLowerCase();
        const url = attribute.value.replace(/[\u0000- ]/g, '').toLowerCase();
        const script = URLS.includes(name) && /^(javascript|vbscript):/.test(url);
        if (apply(startsWith, name, ['on']) || script) node.removeAttribute(attribute.name);
      }
    }
    return doc.body ? doc.body.innerHTML : '';
  };
  // A paste from the browser: a `paste` event carrying it, then, unless the page cancels it,
  // the edit Blink makes. Rich text takes the HTML, else the picture (Blink's
  // `ReadImageAsImageMarkup`), else the text; a field takes the text.
  const paste = (payload) => {
    const target = document.activeElement || document.body;
    const text = typeof payload.text === 'string' ? payload.text : '';
    const html = typeof payload.html === 'string' ? payload.html : '';
    const files = Array.isArray(payload.files) ? payload.files : [];
    if (!target || !(text || html || files.length)) return;
    const data = new DataTransfer();
    if (text) data.setData('text/plain', text);
    if (html) data.setData('text/html', html);
    for (const [type, value] of Array.isArray(payload.custom) ? payload.custom : []) {
      data.setData(type, value);
    }
    for (const file of files) {
      data.items.add(new File([fromBase64(file.data)], file.name, { type: file.type }));
    }
    const event = new ClipboardEvent('paste', { clipboardData: data, bubbles: true, cancelable: true, composed: true });
    own.add(event);
    if (!target.dispatchEvent(event)) return;
    const picture = files.find((f) => apply(startsWith, f.type, ['image/']));
    if (target.isContentEditable && html) {
      document.execCommand('insertHTML', false, sanitized(html));
    } else if (target.isContentEditable && picture) {
      const src = 'data:' + picture.type + ';base64,' + picture.data;
      document.execCommand('insertHTML', false, '<img src="' + src + '">');
    } else if (text) {
      document.execCommand('insertText', false, text);
    }
  };
  // `navigator.clipboard`'s own prototype: Blink puts the `Clipboard` interface object on
  // `window` only after this script ran (a secure-context interface, installed after the
  // context is created), while the instance and its prototype are already there.
  const proto = navigator.clipboard && Object.getPrototypeOf(navigator.clipboard);
  // How a `write`'s types go into the page JSON; `web ` formats by their prefix.
  const WRITES = {
    __proto__: null,
    'text/plain': (payload, blob) => apply(blobText, blob, []).then((t) => { payload.text = t; }),
    'text/html': (payload, blob) => apply(blobText, blob, []).then((t) => { payload.html = t; }),
    'image/png': (payload, blob) => toBase64(blob).then((data) => {
      payload.files = [{ name: 'image.png', type: 'image/png', data }];
    }),
  };
  const writeWeb = (payload, blob, type) => toBase64(blob).then((data) => {
    payload.web.push([apply(slice, type, [4]), data]);
  });
  // A `write`'s first item (Chromium writes one) as the page JSON.
  const fromItems = (items) => {
    const item = items && items[0];
    if (!item) return Promise.resolve();
    const payload = { web: [] };
    return Promise.all(Array.from(item.types || [], (type) => {
      const take = WRITES[type] || (apply(startsWith, type, ['web ']) ? writeWeb : null);
      return take && item.getType(type).then((blob) => take(payload, blob, type));
    })).then(() => payload);
  };
  // What a read answers, as `ClipboardItem`s: the text, HTML, the picture if it is a PNG, and
  // the `web ` formats. `ClipboardItem` is put on `window` after this script ran, as
  // `Clipboard` is.
  const clipboardItems = (payload) => {
    const parts = {};
    if (typeof payload.text === 'string') parts['text/plain'] = new Blob([payload.text], { type: 'text/plain' });
    if (typeof payload.html === 'string') parts['text/html'] = new Blob([payload.html], { type: 'text/html' });
    const png = (payload.files || []).find((f) => f.type === 'image/png');
    if (png) parts['image/png'] = new Blob([fromBase64(png.data)], { type: 'image/png' });
    for (const [type, data] of payload.web || []) {
      parts['web ' + type] = new Blob([fromBase64(data)], { type: 'web ' + type });
    }
    return Object.keys(parts).length ? [new window.ClipboardItem(parts)] : [];
  };
  // Asks the browser for the clipboard; the promise has its answer.
  const ask = (kind) => new Promise((resolve) => {
    const id = nextRead++;
    reads.set(id, resolve);
    post(kind, id);
  });
  // Chromium checks a write first (a focused, secure document, the frame allowed
  // `clipboard-write`, a user activation) and resolves it only when it may: then it goes
  // through the window too. A read likewise, after its `clipboard-read` check.
  if (proto && proto.writeText) {
    const writeText = proto.writeText;
    proto.writeText = function (text) {
      return apply(writeText, this, [text]).then((v) => { postCopy({ text: String(text) }); return v; });
    };
  }
  if (proto && proto.write) {
    const write = proto.write;
    proto.write = function (items) {
      return apply(write, this, [items]).then((v) => fromItems(items).then(
        (payload) => { if (payload) postCopy(payload); return v; },
        () => v));
    };
  }
  if (proto && proto.readText) {
    const readText = proto.readText;
    proto.readText = function () {
      return apply(readText, this, []).then(() => ask('read'))
        .then((payload) => (typeof payload.text === 'string' ? payload.text : ''));
    };
  }
  if (proto && proto.read) {
    const read = proto.read;
    proto.read = function (...args) {
      return apply(read, this, args).then(() => ask('items')).then(clipboardItems);
    };
  }
  register((message) => {
    const at = apply(indexOf, message, ['\n']);
    if (at < 0) return;
    const id = apply(slice, message, [0, at]);
    let payload;
    try {
      payload = parse(apply(slice, message, [at + 1]));
    } catch (e) {
      return;
    }
    if (!payload || typeof payload !== 'object') return;
    if (id) {
      const resolve = reads.get(Number(id));
      reads.delete(Number(id));
      if (resolve) resolve(payload);
      return;
    }
    paste(payload);
  });
})
