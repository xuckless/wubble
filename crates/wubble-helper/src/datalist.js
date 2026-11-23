// SPDX-License-Identifier: GPL-3.0-only
//
// wubble's <datalist> content script. Chromium shows a datalist's options under the field
// as the user types; windowless CEF does not draw that popup, so the page side reports the
// focused field and the browser (wubble_core::datalist) filters and shows them.
//
// welding (change 26) calls this function expression once per frame, before any page
// script, with two native functions that are never put on `window`:
//   send(text)     posts `text` to the browser from this frame;
//   register(fn)   names the function the browser's answers are given to.
//
// Reports (JSON): `{}` when nothing is offered, else `{rect, value, options}`: the field's
// box in physical pixels of the frame's viewport (CSS px times devicePixelRatio, which holds
// the page zoom), the value it holds, and every enabled option as `[value, label]`. A report
// is sent when it differs from the last. Only the main frame reports: CEF does not say where
// a subframe sits, so its offer could not be placed.
// Answer (JSON): `{value}`, the option the user picked: it is set on the field still
// focused, and `input` (inputType insertReplacementText, as Chromium's) and `change` fire.
//
// Closure capture, not an isolated world: the builtins used below are captured before page
// scripts run, and only trusted events count.
(function (send, register) {
  "use strict";
  const W = window;
  if (W.parent !== W || W.origin === "null") {
    return;
  }

  const getter = (proto, name) => Object.getOwnPropertyDescriptor(proto, name).get;
  const listen = EventTarget.prototype.addEventListener;
  const dispatch = EventTarget.prototype.dispatchEvent;
  const composedPath = Event.prototype.composedPath;
  const boundingRect = Element.prototype.getBoundingClientRect;
  const observe = MutationObserver.prototype.observe;
  const disconnect = MutationObserver.prototype.disconnect;
  const isConnected = getter(Node.prototype, "isConnected");
  const inputValue = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value");
  const getValue = inputValue.get;
  const setValue = inputValue.set;
  const inputList = getter(HTMLInputElement.prototype, "list");
  const listOptions = getter(HTMLDataListElement.prototype, "options");
  const optionValue = getter(HTMLOptionElement.prototype, "value");
  const optionLabel = getter(HTMLOptionElement.prototype, "label");
  const optionDisabled = getter(HTMLOptionElement.prototype, "disabled");
  const InputElement = HTMLInputElement;
  const InputEvent = W.InputEvent;
  const Plain = W.Event;
  const Observer = W.MutationObserver;
  const frame = W.requestAnimationFrame.bind(W);
  const stringify = JSON.stringify;
  const parse = JSON.parse;

  // The input types Chromium offers a datalist on with a text box (the others have a picker
  // of their own).
  const TYPES = new Set(["text", "search", "url", "tel", "email", "number"]);
  const MAX_OPTIONS = 200;

  // The field with focus, whose list is offered; `hidden` once the user dismissed the list
  // (Escape) or picked from it, until they type again or focus moves.
  let current = null;
  let hidden = false;
  let last = "{}";
  let scheduled = false;
  const watcher = new Observer(() => later());

  function offered(el) {
    if (!(el instanceof InputElement) || !TYPES.has(el.type) || el.readOnly || el.disabled) {
      return null;
    }
    return inputList.call(el);
  }

  function message() {
    const list = current && !hidden && isConnected.call(current) ? offered(current) : null;
    if (!list) {
      return {};
    }
    const options = [];
    for (const option of listOptions.call(list)) {
      const value = optionValue.call(option);
      if (value !== "" && !optionDisabled.call(option) && options.length < MAX_OPTIONS) {
        options.push([value, optionLabel.call(option)]);
      }
    }
    if (!options.length) {
      return {};
    }
    const box = boundingRect.call(current);
    const scale = W.devicePixelRatio;
    const rect = [box.left, box.top, box.width, box.height].map((n) => Math.round(n * scale));
    return { rect, value: getValue.call(current), options };
  }

  function report() {
    const text = stringify(message());
    if (text !== last) {
      last = text;
      try {
        send(text);
      } catch (e) {
        // The frame is going away.
      }
    }
  }

  // One report per frame, whatever changed (typing, scrolling, the list's options).
  function later() {
    if (!scheduled) {
      scheduled = true;
      frame(() => {
        scheduled = false;
        report();
      });
    }
  }

  function focusOn(el) {
    if (el === current) {
      return;
    }
    current = el;
    hidden = false;
    disconnect.call(watcher);
    const list = offered(el);
    if (list) {
      observe.call(watcher, list, { childList: true, subtree: true, attributes: true, characterData: true });
    }
  }

  // Events from inside shadow roots reach the window retargeted to their host.
  const target = (e) => composedPath.call(e)[0] || e.target;
  listen.call(W, "focusin", (e) => {
    if (e.isTrusted) {
      focusOn(target(e));
      later();
    }
  }, true);
  listen.call(W, "focusout", (e) => {
    if (e.isTrusted && target(e) === current) {
      focusOn(null);
      later();
    }
  }, true);
  listen.call(W, "input", (e) => {
    if (e.isTrusted && target(e) === current) {
      hidden = false;
      later();
    }
  }, true);
  listen.call(W, "keydown", (e) => {
    if (e.isTrusted && e.key === "Escape" && !hidden) {
      hidden = true;
      later();
    }
  }, true);
  // The field moves under the list.
  listen.call(W, "scroll", () => current && later(), { capture: true, passive: true });
  listen.call(W, "resize", () => current && later(), { passive: true });

  register((text) => {
    let answer;
    try {
      answer = parse(text);
    } catch (e) {
      return;
    }
    const el = current;
    if (!answer || typeof answer.value !== "string" || !el || !isConnected.call(el) || !offered(el)) {
      return;
    }
    setValue.call(el, answer.value);
    dispatch.call(el, new InputEvent("input", {
      bubbles: true, composed: true, inputType: "insertReplacementText", data: answer.value,
    }));
    dispatch.call(el, new Plain("change", { bubbles: true }));
    hidden = true;
    later();
  });
})
