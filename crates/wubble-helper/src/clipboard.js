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
//            custom data): the page JSON below
//   write    `navigator.clipboard.writeText` or `write` (text, HTML, PNG, `web `
//            formats) passed Chromium's check: the page JSON (the host drops
//            its HTML)
//   image    a copy in an image document: the picture's URL (Chromium copies
//            the picture there)
//   primary  the selection changed: the primary selection's text (never a
//            password field's; at most `PRIMARY_CAP` bytes)
//   paste    Chromium's trusted paste was cancelled: `token from`, answer with
//            the clipboard (`from`: "primary" after a middle click in a field,
//            "plain" after Ctrl+Shift+V, Paste as Plain Text, else empty)
//   read     `navigator.clipboard.readText` passed its permission check: the token
//   items    `navigator.clipboard.read` passed its permission check: the token
// A token is this document's nonce and a count: an answer names it, so a
// document never takes an answer asked for by the one before it in the same
// frame. From the browser, `token or ""\njson`: the answer to that token
// (`null`: refused, a read's promise rejects), or, with no token, a paste the
// browser starts itself (the menu's Paste as Plain Text); a paste is pasted as
// Chromium pastes (a `paste` event carrying it, then, unless the page cancels
// that, one undoable edit: the HTML, else the picture, into rich text; the text
// into a field).
//
// The page JSON is `wubble_core::clipboard::Payload`'s: `{text, html, files:
// [{name, type, data}], custom: [[type, value]], web: [[type, data]]}`, bytes
// as base64.
//
// Isolation: closure capture, as autofill.js; trusted events only. The page
// shares this world's built-ins, so every one this script calls after the
// page's scripts ran is captured here, and nothing iterates (`for...of`,
// spreading and destructuring call the page's iterators): loops go by index.
// No `.then`, `await` or `Promise.all` on a promise (they look up the page's
// `then` and `Promise.resolve`). The DOM it reads (the selection, the focused
// field) is the page's to describe; the bridge is not the page's to drive. The
// host serves only the focused frame of the page the user is in
// (`wubble_core::clipboard::admit`) whatever happens here.
(function (send, register) {
  'use strict';
  const apply = Reflect.apply;
  const indexOf = String.prototype.indexOf;
  const slice = String.prototype.slice;
  const startsWith = String.prototype.startsWith;
  const charCodeAt = String.prototype.charCodeAt;
  const trim = String.prototype.trim;
  const lower = String.prototype.toLowerCase;
  const replace = String.prototype.replace;
  const join = Array.prototype.join;
  const regexTest = RegExp.prototype.test;
  const fromCharCode = String.fromCharCode;
  const parse = JSON.parse;
  const stringify = JSON.stringify;
  const keys = Object.keys;
  const isArray = Array.isArray;
  const encode = TextEncoder.prototype.encode;
  const encoder = new TextEncoder();
  const blobText = Blob.prototype.text;
  const blobBuffer = Blob.prototype.arrayBuffer;
  const getRandomValues = Crypto.prototype.getRandomValues;
  const cryptoObject = crypto;
  const toHex = Number.prototype.toString;
  const weakHas = WeakSet.prototype.has;
  const weakAdd = WeakSet.prototype.add;
  const parseHtml = DOMParser.prototype.parseFromString;
  const queryAll = Document.prototype.querySelectorAll;
  const removeNode = Element.prototype.remove;
  const removeAttribute = Element.prototype.removeAttribute;
  const attributesOf = Object.getOwnPropertyDescriptor(Element.prototype, 'attributes').get;
  const setTransfer = DataTransfer.prototype.setData;
  const itemsOf = Object.getOwnPropertyDescriptor(DataTransfer.prototype, 'items').get;
  const addItem = DataTransferItemList.prototype.add;
  const dispatch = EventTarget.prototype.dispatchEvent;
  const execCommand = Document.prototype.execCommand;
  const Str = String;
  const P = Promise;
  const then = Promise.prototype.then;
  const DomException = DOMException;
  const Bytes = Uint8Array;
  const BlobOf = Blob;
  const FileOf = File;
  const Transfer = DataTransfer;
  const PasteEvent = ClipboardEvent;
  const Parser = DOMParser;
  const toBase64Text = btoa;
  const fromBase64Text = atob;
  // `promise.then(ok, failed)` without the page's `then`.
  const after = (promise, ok, failed) => apply(then, promise, [ok, failed]);
  const byteLength = (text) => apply(encode, encoder, [text]).length;
  // The longest primary selection the host takes, in UTF-8 bytes
  // (`wubble_core::clipboard::PRIMARY_CAP`).
  const PRIMARY_CAP = 1 << 20;
  // The longest message the browser takes (welding's `MAX_FRAME_MESSAGE_BYTES`, which drops a
  // longer one), less room for the channel's and kind's names.
  const MESSAGE_CAP = (16 << 20) - 64;
  const own = new WeakSet();
  // This document's nonce, and what each token waits for: [answered, refused].
  const nonce = (() => {
    const words = apply(getRandomValues, cryptoObject, [new Uint32Array(2)]);
    return apply(toHex, words[0], [16]) + apply(toHex, words[1], [16]);
  })();
  const pending = { __proto__: null };
  let nextToken = 1;
  const post = (kind, text) => send(kind + '\n' + Str(text));
  const toBase64 = (blob) => after(apply(blobBuffer, blob, []), (buffer) => {
    const bytes = new Bytes(buffer);
    let binary = '';
    for (let i = 0; i < bytes.length; i += 0x8000) {
      binary += apply(fromCharCode, null, bytes.subarray(i, i + 0x8000));
    }
    return toBase64Text(binary);
  });
  const fromBase64 = (text) => {
    const binary = fromBase64Text(text);
    const bytes = new Bytes(binary.length);
    for (let i = 0; i < binary.length; i++) bytes[i] = apply(charCodeAt, binary, [i]);
    return bytes;
  };
  // A copy as the page JSON. What would take the message past the cap is left out, the
  // largest kinds first and the text last.
  const DROPS = ['files', 'web', 'custom', 'html', 'text'];
  const postCopy = (kind, payload) => {
    for (let i = 0; i < DROPS.length; i++) {
      const json = stringify(payload);
      if (byteLength(json) <= MESSAGE_CAP) return post(kind, json);
      delete payload[DROPS[i]];
    }
  };
  const field = (a) => a && (a instanceof HTMLInputElement || a instanceof HTMLTextAreaElement);
  // The selected text; none in a password field, as Chromium neither copies one nor puts it
  // on the primary selection.
  const selected = () => {
    const a = document.activeElement;
    if (field(a)) {
      if (a.type === 'password') return '';
      try {
        if (typeof a.selectionStart === 'number') return apply(slice, Str(a.value), [a.selectionStart, a.selectionEnd]);
      } catch (e) {}
    }
    return Str(getSelection());
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
    if (!apply(startsWith, Str(document.contentType), ['image/']) || images.length !== 1) return '';
    return images[0].currentSrc || images[0].src;
  };
  // `setData`'s type as Blink keeps it (`NormalizeType`).
  const ALIASES = { __proto__: null, text: 'text/plain', url: 'text/uri-list' };
  const normalized = (format) => {
    const type = apply(lower, apply(trim, Str(format), []), []);
    if (apply(startsWith, type, ['text/plain;'])) return 'text/plain';
    return ALIASES[type] || type;
  };
  // What a `copy` listener set (by type, in a null-prototype record), as Chromium writes it:
  // text, HTML, the rest as custom data.
  const fromSetData = (set) => {
    const payload = { text: set['text/plain'], html: set['text/html'], custom: [] };
    const types = keys(set);
    for (let i = 0; i < types.length; i++) {
      const type = types[i];
      if (type !== 'text/plain' && type !== 'text/html') {
        payload.custom[payload.custom.length] = [type, set[type]];
      }
    }
    return payload;
  };
  const COPIES = ['copy', 'cut'];
  for (let i = 0; i < COPIES.length; i++) {
    addEventListener(COPIES[i], (e) => {
      if (!e.isTrusted) return;
      let set = { __proto__: null };
      const data = e.clipboardData;
      if (data) {
        const setData = data.setData;
        const clearData = data.clearData;
        data.setData = function (format, value) {
          set[normalized(format)] = Str(value);
          return apply(setData, this, [format, value]);
        };
        data.clearData = function (format) {
          if (format === undefined) set = { __proto__: null };
          else delete set[normalized(format)];
          return apply(clearData, this, format === undefined ? [] : [format]);
        };
      }
      const text = selected();
      const html = selectedHtml();
      const image = imageDocument();
      setTimeout(() => {
        if (e.defaultPrevented) {
          if (keys(set).length) postCopy('copy', fromSetData(set));
        } else if (image) {
          post('image', image);
        } else if (text || html) {
          postCopy('copy', html ? { text, html } : { text });
        }
      });
    }, true);
  }
  // Asks the browser for the clipboard under a new token: `answered` gets the payload,
  // `refused` an error.
  const ask = (kind, text, answered, refused) => {
    const token = nonce + '.' + nextToken++;
    pending[token] = [answered, refused];
    post(kind, text === undefined ? token : token + ' ' + text);
  };
  // Where the next paste comes from: "primary" after a middle press, "plain" after
  // Ctrl+Shift+V (Blink's Paste as Plain Text), "" otherwise. A middle click in a field pastes
  // the primary selection, as on X11 (Blink's `PasteGlobalSelection`). Chromium's own runs
  // only where its clipboard has a selection buffer, which a windowless Wayland one does not;
  // when it does run, its paste below is answered from the primary selection too.
  let from = '';
  const editable = (t) => t instanceof Element && (t.isContentEditable ||
    t instanceof HTMLTextAreaElement || (t instanceof HTMLInputElement && !t.readOnly && !t.disabled));
  const askPaste = (where) => ask('paste', where, (payload) => paste(payload), () => {});
  addEventListener('mousedown', (e) => { from = e.isTrusted && e.button === 1 ? 'primary' : ''; }, true);
  addEventListener('mouseup', (e) => {
    if (!e.isTrusted || e.button !== 1 || from !== 'primary' || !editable(e.target)) return;
    setTimeout(() => { if (from === 'primary') { from = ''; askPaste('primary'); } });
  }, true);
  addEventListener('keydown', (e) => {
    from = e.isTrusted && e.ctrlKey && e.shiftKey && !e.altKey && e.code === 'KeyV' ? 'plain' : '';
  }, true);
  addEventListener('paste', (e) => {
    if (!e.isTrusted || apply(weakHas, own, [e])) return;
    e.preventDefault();
    e.stopImmediatePropagation();
    askPaste(from);
    from = '';
  }, true);
  let primaryTimer = 0;
  document.addEventListener('selectionchange', () => {
    clearTimeout(primaryTimer);
    primaryTimer = setTimeout(() => {
      const text = selected();
      if (text && byteLength(text) <= PRIMARY_CAP) post('primary', text);
    }, 60);
  }, true);
  // Pasted markup without active content: no scripts, frames, plugins, styles or SVG
  // animation, no event handler attributes, no `javascript:` URLs. Parsed in an inert document.
  // (Blink's paste goes further: it keeps only what renders, with its computed style.)
  const DROPPED = apply(join, ['script', 'style', 'link', 'meta', 'title', 'base', 'iframe',
    'frame', 'frameset', 'object', 'embed', 'noscript', 'template', 'animate', 'set',
    'animateMotion', 'animateTransform'], [',']);
  const URLS = { __proto__: null, href: 1, src: 1, action: 1, formaction: 1, 'xlink:href': 1,
    data: 1, poster: 1, background: 1 };
  const CONTROLS = /[\u0000- ]/g;
  const SCRIPT_URL = /^(javascript|vbscript):/;
  const sanitized = (html) => {
    const doc = apply(parseHtml, new Parser(), [html, 'text/html']);
    const dropped = apply(queryAll, doc, [DROPPED]);
    for (let i = dropped.length - 1; i >= 0; i--) apply(removeNode, dropped[i], []);
    const nodes = apply(queryAll, doc, ['*']);
    for (let i = 0; i < nodes.length; i++) {
      const attributes = apply(attributesOf, nodes[i], []);
      for (let j = attributes.length - 1; j >= 0; j--) {
        const name = apply(lower, attributes[j].name, []);
        const url = apply(lower, apply(replace, attributes[j].value, [CONTROLS, '']), []);
        const script = URLS[name] === 1 && apply(regexTest, SCRIPT_URL, [url]);
        if (apply(startsWith, name, ['on']) || script) {
          apply(removeAttribute, nodes[i], [attributes[j].name]);
        }
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
    const files = isArray(payload.files) ? payload.files : [];
    const custom = isArray(payload.custom) ? payload.custom : [];
    if (!target || !(text || html || files.length)) return;
    const data = new Transfer();
    if (text) apply(setTransfer, data, ['text/plain', text]);
    if (html) apply(setTransfer, data, ['text/html', html]);
    for (let i = 0; i < custom.length; i++) {
      apply(setTransfer, data, [Str(custom[i][0]), Str(custom[i][1])]);
    }
    let picture = null;
    for (let i = 0; i < files.length; i++) {
      const file = files[i];
      const type = Str(file.type);
      apply(addItem, apply(itemsOf, data, []), [new FileOf([fromBase64(file.data)], Str(file.name), { type })]);
      if (!picture && apply(startsWith, type, ['image/'])) picture = file;
    }
    const event = new PasteEvent('paste', { clipboardData: data, bubbles: true, cancelable: true, composed: true });
    apply(weakAdd, own, [event]);
    if (!apply(dispatch, target, [event])) return;
    if (target.isContentEditable && html) {
      apply(execCommand, document, ['insertHTML', false, sanitized(html)]);
    } else if (target.isContentEditable && picture) {
      const src = 'data:' + Str(picture.type) + ';base64,' + Str(picture.data);
      apply(execCommand, document, ['insertHTML', false, '<img src="' + src + '">']);
    } else if (text) {
      apply(execCommand, document, ['insertText', false, text]);
    }
  };
  // `navigator.clipboard`'s own prototype: Blink puts the `Clipboard` interface object on
  // `window` only after this script ran (a secure-context interface, installed after the
  // context is created), while the instance and its prototype are already there.
  const proto = navigator.clipboard && Object.getPrototypeOf(navigator.clipboard);
  // How a `write`'s types go into the page JSON; `web ` formats by their prefix.
  const WRITES = {
    __proto__: null,
    'text/plain': (payload, blob) => after(apply(blobText, blob, []), (t) => { payload.text = t; }),
    'text/html': (payload, blob) => after(apply(blobText, blob, []), (t) => { payload.html = t; }),
    'image/png': (payload, blob) => after(toBase64(blob), (data) => {
      payload.files = [{ name: 'image.png', type: 'image/png', data }];
    }),
  };
  const writeWeb = (payload, blob, type) => after(toBase64(blob), (data) => {
    payload.web[payload.web.length] = [apply(slice, type, [4]), data];
  });
  // A `write`'s first item (Chromium writes one) as the page JSON, to `done` once every type
  // is read (by hand: no `Promise.all`). `types` and `getType` are the page's
  // (`ClipboardItem` is installed after this script), and what a page hands its own write is
  // its to choose; only a `getType` that answers with a real promise is waited for.
  const fromItems = (items, done) => {
    const item = items && items[0];
    const types = item && item.types;
    if (!types || !types.length) return done(null);
    const payload = { web: [] };
    let left = types.length;
    const one = () => { if (--left === 0) done(payload); };
    for (let i = 0; i < types.length; i++) {
      const type = Str(types[i]);
      const take = WRITES[type] || (apply(startsWith, type, ['web ']) ? writeWeb : null);
      let blob;
      try { blob = take && item.getType(type); } catch (e) { blob = null; }
      if (!(blob instanceof P)) { one(); continue; }
      after(blob, (b) => {
        if (b instanceof BlobOf) after(take(payload, b, type), one, one);
        else one();
      }, one);
    }
  };
  // What a read answers, as `ClipboardItem`s: the text, HTML, the picture if it is a PNG, and
  // the `web ` formats. `ClipboardItem` is put on `window` after this script ran, as
  // `Clipboard` is.
  const clipboardItems = (payload) => {
    const parts = {};
    let any = false;
    const part = (type, bytes) => { parts[type] = new BlobOf([bytes], { type }); any = true; };
    if (typeof payload.text === 'string') part('text/plain', payload.text);
    if (typeof payload.html === 'string') part('text/html', payload.html);
    const files = isArray(payload.files) ? payload.files : [];
    for (let i = 0; i < files.length; i++) {
      if (files[i].type === 'image/png') { part('image/png', fromBase64(files[i].data)); break; }
    }
    const web = isArray(payload.web) ? payload.web : [];
    for (let i = 0; i < web.length; i++) part('web ' + Str(web[i][0]), fromBase64(web[i][1]));
    return any ? [new window.ClipboardItem(parts)] : [];
  };
  // Chromium checks a write first (a focused, secure document, the frame allowed
  // `clipboard-write`, a user activation) and resolves it only when it may: then it goes
  // through the window too. A read likewise, after its `clipboard-read` check.
  if (proto && proto.writeText) {
    const writeText = proto.writeText;
    proto.writeText = function (text) {
      const checked = apply(writeText, this, [text]);
      const copied = { text: Str(text) };
      return new P((resolve, reject) => after(checked, (v) => {
        postCopy('write', copied);
        resolve(v);
      }, reject));
    };
  }
  if (proto && proto.write) {
    const write = proto.write;
    proto.write = function (items) {
      const checked = apply(write, this, [items]);
      return new P((resolve, reject) => after(checked, (v) => fromItems(items, (payload) => {
        if (payload) postCopy('write', payload);
        resolve(v);
      }), reject));
    };
  }
  if (proto && proto.readText) {
    const readText = proto.readText;
    proto.readText = function () {
      const checked = apply(readText, this, []);
      return new P((resolve, reject) => after(checked, () => ask('read', undefined,
        (payload) => resolve(typeof payload.text === 'string' ? payload.text : ''), reject), reject));
    };
  }
  if (proto && proto.read) {
    const read = proto.read;
    proto.read = function () {
      const checked = apply(read, this, arguments);
      return new P((resolve, reject) => after(checked, () => ask('items', undefined,
        (payload) => resolve(clipboardItems(payload)), reject), reject));
    };
  }
  register((message) => {
    const at = apply(indexOf, message, ['\n']);
    if (at < 0) return;
    const token = apply(slice, message, [0, at]);
    let payload;
    try {
      payload = parse(apply(slice, message, [at + 1]));
    } catch (e) {
      return;
    }
    const answer = payload && typeof payload === 'object' ? payload : null;
    // A paste the browser starts itself (the menu's Paste as Plain Text).
    if (!token) {
      if (answer) paste(answer);
      return;
    }
    // Only this document's own: a token of the document before it in this frame is not here.
    const waiting = pending[token];
    delete pending[token];
    if (!waiting) return;
    if (answer) waiting[0](answer);
    else waiting[1](new DomException('Read permission denied.', 'NotAllowedError'));
  });
})
