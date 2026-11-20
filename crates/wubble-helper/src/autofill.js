// SPDX-License-Identifier: GPL-3.0-only
//
// wubble's password autofill content script.
//
// welding (change 26) evaluates this function expression in every frame's V8
// context as the context is created, before any page script, and calls it once
// with two native functions that are never put on `window`:
//   send(text)     posts `text` to the browser from this frame;
//   register(fn)   names the function the browser's fill messages are given to.
// The browser reads the frame and its origin from CEF, not from `text`.
//
// The script only reports what the page holds and fills what the browser names:
// which field is the username or a password is decided once, in Rust
// (wubble-core `autofill.rs`). It fills only after the user picked a row in
// wubble's own menu, never a hidden, read-only or disabled field, and only while
// the frame is still on the origin the browser named. A sandboxed
// (opaque-origin) frame gets nothing: no reports, no fill, since the browser
// would take its reports for the frame URL's origin.
//
// Isolation: closure capture, not an isolated world. The page shares the heap and
// the DOM; the builtins used below are captured before page scripts run, and
// event listeners ignore synthetic (untrusted) events.
//
// The fill sequence (focus, key events, value, key events, input and change, blur)
// is ported from Bitwarden's clients repository,
// apps/browser/src/autofill/services/insert-autofill-content.service.ts
// (insertValueIntoField, triggerPreInsertEventsOnElement,
// triggerPostInsertEventsOnElement, simulateUserKeyboardEventInteractions,
// simulateInputElementChangedEvent), GPL-3.0. Setting the value through the
// prototype's native setter (so frameworks that wrap `value` see the change) is
// what Firefox's LoginManagerChild does through `setUserInput`.
(function (send, register) {
  "use strict";
  const W = window;
  const D = document;
  const loc = W.location;
  if ((loc.protocol !== "http:" && loc.protocol !== "https:") || W.origin === "null") {
    return;
  }

  const listen = EventTarget.prototype.addEventListener;
  const dispatch = EventTarget.prototype.dispatchEvent;
  const stringify = JSON.stringify;
  const parse = JSON.parse;
  const inputValue = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value");
  const getValue = inputValue.get;
  const setValue = inputValue.set;
  const query = Document.prototype.querySelectorAll;
  const closest = Element.prototype.closest;
  const getAttribute = Element.prototype.getAttribute;
  const clientRects = Element.prototype.getClientRects;
  const boundingRect = Element.prototype.getBoundingClientRect;
  const computedStyle = W.getComputedStyle;
  const focus = HTMLElement.prototype.focus;
  const blur = HTMLElement.prototype.blur;
  const InputElement = HTMLInputElement;
  const FormElement = HTMLFormElement;
  const Observer = W.MutationObserver;
  const frame = W.requestAnimationFrame.bind(W);
  const Plain = W.Event;
  const Key = W.KeyboardEvent;
  const Weak = W.WeakRef;

  const MAX_FORMS = 32;
  const MAX_FIELDS = 128;
  const SKIPPED_TYPES = new Set([
    "hidden", "submit", "button", "reset", "image", "checkbox", "radio", "file", "range", "color",
  ]);

  // Stable ids per element for this document; the browser names fields by them.
  const ids = new WeakMap();
  const byId = new Map();
  let nextId = 1;
  function idOf(el) {
    let id = ids.get(el);
    if (id === undefined) {
      id = nextId++;
      ids.set(el, id);
      byId.set(id, new Weak(el));
    }
    return id;
  }

  function visible(el) {
    if (!el.isConnected || clientRects.call(el).length === 0) {
      return false;
    }
    const style = computedStyle.call(W, el);
    if (style.visibility === "hidden" || style.display === "none" || style.opacity === "0") {
      return false;
    }
    const box = boundingRect.call(el);
    return box.width >= 2 && box.height >= 2;
  }

  function fillable(el) {
    return el instanceof InputElement && !SKIPPED_TYPES.has((el.type || "text").toLowerCase());
  }

  function fieldSnapshot(el, withValue) {
    return {
      id: idOf(el),
      type: (el.type || "text").toLowerCase(),
      name: el.name || "",
      idAttr: el.id || "",
      autocomplete: getAttribute.call(el, "autocomplete") || "",
      visible: visible(el),
      readonly: el.readOnly || el.disabled,
      value: withValue ? getValue.call(el) : "",
    };
  }

  // A form's fields, or the page's fields outside any form (`form` null, id 0).
  function fieldsOf(form) {
    const all = form ? Array.from(form.elements) : Array.from(query.call(D, "input"));
    return all.filter((el) => fillable(el) && (form || !el.form)).slice(0, MAX_FIELDS);
  }

  function formSnapshot(form, withValue) {
    return {
      id: form ? idOf(form) : 0,
      fields: fieldsOf(form).map((el) => fieldSnapshot(el, withValue)),
    };
  }

  // Every form (or the formless group) that holds a password field.
  function forms() {
    if (!D.documentElement) {
      return [];
    }
    const seen = new Set();
    const out = [];
    for (const pw of query.call(D, "input[type=password]")) {
      const form = pw.form || null;
      if (seen.has(form)) {
        continue;
      }
      seen.add(form);
      out.push(formSnapshot(form, false));
      if (out.length >= MAX_FORMS) {
        break;
      }
    }
    return out;
  }

  function post(message) {
    try {
      send(stringify(message));
    } catch (e) {
      // The frame is going away.
    }
  }

  // A new document: what the frame's last one offered is gone.
  post({ t: "start" });

  // ----- reports

  let lastForms = stringify([]);
  function report() {
    const now = forms();
    const text = stringify(now);
    if (text !== lastForms) {
      lastForms = text;
      post({ t: "forms", forms: now });
    }
  }

  // The last submitted form and its password fields, until they leave the page.
  let submitted = null;
  // This document submitted a sign-in: its own "settled" (on load) would describe the page as
  // it was before the submit's outcome, so it is not sent (the browser would read the form
  // still there as a failed sign-in and drop the save). The next document reports the outcome.
  let submittedHere = false;
  function submitLike(form) {
    const snapshot = formSnapshot(form, true);
    const passwords = fieldsOf(form).filter(
      (el) => el.type === "password" && getValue.call(el) !== "",
    );
    if (passwords.length === 0) {
      return;
    }
    submitted = { snapshot, passwords };
    submittedHere = true;
    post({ t: "submit", form: snapshot });
  }

  // A single-page sign-in: the submitted password fields left (or hid).
  function checkGone() {
    if (submitted && submitted.passwords.every((el) => !visible(el))) {
      const { snapshot } = submitted;
      submitted = null;
      post({ t: "gone", form: snapshot });
    }
  }

  let scheduled = false;
  function changed() {
    if (scheduled) {
      return;
    }
    scheduled = true;
    frame(() => {
      scheduled = false;
      checkGone();
      report();
    });
  }

  listen.call(W, "submit", (e) => {
    if (e.isTrusted && e.target instanceof FormElement) {
      submitLike(e.target);
    }
  }, true);
  listen.call(W, "keydown", (e) => {
    const t = e.target;
    if (e.isTrusted && e.key === "Enter" && fillable(t)) {
      submitLike(t.form || null);
    }
  }, true);
  listen.call(W, "click", (e) => {
    if (!e.isTrusted || !(e.target instanceof Element)) {
      return;
    }
    // Submit controls only: a show-password toggle (type=button) or any other
    // button is no submit.
    const control = closest.call(e.target, "button, input[type=submit], input[type=image]");
    if (control && (control.type === "submit" || control.type === "image")) {
      submitLike(control.form || closest.call(control, "form"));
    }
  }, true);
  listen.call(D, "DOMContentLoaded", (e) => {
    if (!e.isTrusted) {
      return;
    }
    report();
    new Observer(changed).observe(D.documentElement, {
      childList: true,
      subtree: true,
      attributes: true,
      attributeFilter: ["type", "style", "class", "hidden"],
    });
  });
  listen.call(W, "load", (e) => {
    if (e.isTrusted) {
      frame(() => {
        lastForms = stringify(forms());
        if (!submittedHere) {
          post({ t: "settled", forms: parse(lastForms) });
        }
      });
    }
  });

  // ----- fill (the browser's message, after the user's pick)

  function keys(el) {
    for (const type of ["keydown", "keypress", "keyup"]) {
      dispatch.call(el, new Key(type, { bubbles: true, cancelable: true }));
    }
  }

  function fillField(el, value) {
    if (getValue.call(el) === value) {
      return;
    }
    // Bitwarden triggerPreInsertEventsOnElement.
    const before = getValue.call(el);
    focus.call(el);
    keys(el);
    if (getValue.call(el) !== before) {
      setValue.call(el, before);
    }
    setValue.call(el, value);
    // Bitwarden triggerPostInsertEventsOnElement.
    keys(el);
    if (getValue.call(el) !== value) {
      setValue.call(el, value);
    }
    for (const type of ["input", "change"]) {
      dispatch.call(el, new Plain(type, { bubbles: true }));
    }
    blur.call(el);
  }

  register((text) => {
    let message;
    try {
      message = parse(text);
    } catch (e) {
      return;
    }
    if (!message || message.origin !== loc.origin || !Array.isArray(message.fields)) {
      return;
    }
    for (const [id, value] of message.fields) {
      const ref = byId.get(id);
      const el = ref && ref.deref();
      if (!el || !fillable(el) || el.readOnly || el.disabled || !visible(el)) {
        continue;
      }
      fillField(el, String(value));
    }
  });
});
