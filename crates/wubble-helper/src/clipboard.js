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
// `[kind, text]` as JSON:
//   copy     a trusted copy or cut, `navigator.clipboard.writeText`, or the
//            `text/plain` item of `navigator.clipboard.write`: the text
//   primary  the selection changed: the primary selection's text
//   paste    Chromium's trusted paste was cancelled: answer with the clipboard
//            ("primary" for a middle click in a field)
//   read     `navigator.clipboard.readText` passed its permission check: the id
// From the browser, `read id or ""\ntext`: the paste or read's answer, pasted
// as Chromium pastes plain text (a `paste` event carrying it, then, unless the
// page cancels that, one undoable `insertText`).
//
// Isolation: closure capture, as autofill.js; trusted events only.
(function (send, register) {
  'use strict';
  const apply = Reflect.apply;
  const indexOf = String.prototype.indexOf;
  const slice = String.prototype.slice;
  const own = new WeakSet();
  const reads = new Map();
  let nextRead = 1;
  const post = (kind, text) => send(kind + '\n' + String(text));
  const selected = () => {
    const a = document.activeElement;
    if (a && (a instanceof HTMLInputElement || a instanceof HTMLTextAreaElement)) {
      try {
        if (typeof a.selectionStart === 'number') return a.value.slice(a.selectionStart, a.selectionEnd);
      } catch (e) {}
    }
    return String(getSelection());
  };
  for (const type of ['copy', 'cut']) {
    addEventListener(type, (e) => {
      if (!e.isTrusted) return;
      let set;
      const data = e.clipboardData;
      if (data) {
        const setData = data.setData;
        data.setData = function (format, value) {
          if (format === 'text/plain' || format === 'text' || format === 'Text') set = String(value);
          return setData.call(this, format, value);
        };
      }
      const selection = selected();
      setTimeout(() => {
        const text = e.defaultPrevented ? set : selection;
        if (text) post('copy', text);
      });
    }, true);
  }
  // A middle click in a field pastes the primary selection, as on X11 (Blink's
  // `PasteGlobalSelection`). Chromium's own runs only where its clipboard has a
  // selection buffer, which a windowless Wayland one does not; when it does run,
  // its paste below is answered from the primary selection too.
  let middle = false;
  const editable = (t) => t instanceof Element && (t.isContentEditable ||
    t instanceof HTMLTextAreaElement || (t instanceof HTMLInputElement && !t.readOnly && !t.disabled));
  addEventListener('mousedown', (e) => { middle = e.isTrusted && e.button === 1; }, true);
  addEventListener('mouseup', (e) => {
    if (!e.isTrusted || e.button !== 1 || !middle || !editable(e.target)) return;
    setTimeout(() => { if (middle) { middle = false; post('paste', 'primary'); } });
  }, true);
  addEventListener('keydown', () => { middle = false; }, true);
  addEventListener('paste', (e) => {
    if (!e.isTrusted || own.has(e)) return;
    e.preventDefault();
    e.stopImmediatePropagation();
    post('paste', middle ? 'primary' : '');
    middle = false;
  }, true);
  let primaryTimer = 0;
  document.addEventListener('selectionchange', () => {
    clearTimeout(primaryTimer);
    primaryTimer = setTimeout(() => {
      const text = selected();
      if (text) post('primary', text);
    }, 60);
  }, true);
  // `navigator.clipboard`'s own prototype: Blink puts the `Clipboard` interface object on
  // `window` only after this script ran (a secure-context interface, installed after the
  // context is created), while the instance and its prototype are already there.
  const proto = navigator.clipboard && Object.getPrototypeOf(navigator.clipboard);
  const includes = Array.prototype.includes;
  const blobText = Blob.prototype.text;
  // A `write`'s text: its first item's `text/plain` (Chromium writes one item), or nothing.
  const plainText = (items) => {
    const item = items && items[0];
    if (!item || !apply(includes, item.types || [], ['text/plain'])) return Promise.resolve();
    return item.getType('text/plain').then((blob) => apply(blobText, blob, []));
  };
  // Chromium checks a write first (a focused, secure document, the frame allowed
  // `clipboard-write`, a user activation) and resolves it only when it may: then it goes
  // through the window too.
  if (proto && proto.writeText) {
    const writeText = proto.writeText;
    proto.writeText = function (text) {
      return apply(writeText, this, [text]).then((v) => { post('copy', text); return v; });
    };
  }
  if (proto && proto.write) {
    const write = proto.write;
    proto.write = function (items) {
      return apply(write, this, [items]).then((v) => plainText(items).then(
        (text) => { if (typeof text === 'string') post('copy', text); return v; },
        () => v));
    };
  }
  if (proto && proto.readText) {
    const readText = proto.readText;
    proto.readText = function () {
      return readText.call(this).then(() => new Promise((resolve) => {
        const id = nextRead++;
        reads.set(id, resolve);
        post('read', id);
      }));
    };
  }
  register((message) => {
    const at = apply(indexOf, message, ['\n']);
    if (at < 0) return;
    const id = apply(slice, message, [0, at]);
    const text = apply(slice, message, [at + 1]);
    if (id) {
      const resolve = reads.get(Number(id));
      reads.delete(Number(id));
      if (resolve) resolve(text);
      return;
    }
    const target = document.activeElement || document.body;
    if (!target || !text) return;
    const data = new DataTransfer();
    data.setData('text/plain', text);
    const event = new ClipboardEvent('paste', { clipboardData: data, bubbles: true, cancelable: true, composed: true });
    own.add(event);
    if (target.dispatchEvent(event)) document.execCommand('insertText', false, text);
  });
})
