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
// The script reports what the page holds and fills what the browser names:
// which field is the username or a password is decided once, in Rust
// (wubble-core `autofill.rs`). The script decides only which forms are about
// signing in (it reports those, and sends values only in the submit and gone
// snapshots of those): a form (or the page's fields outside any form) holding a
// password field, or a username-only form. It fills only after the user picked
// a row in wubble's own menu, never a hidden, read-only or disabled field, and
// only while the frame is still on the origin the browser named. A sandboxed
// (opaque-origin) frame gets nothing: no reports, no fill, since the browser
// would take its reports for the frame URL's origin.
//
// Isolation: closure capture, not an isolated world. The page shares the heap and
// the DOM; the builtins used below are captured before page scripts run, and
// event listeners ignore synthetic (untrusted) events.
//
// Ported, with changes marked "wubble:":
// - Fields in open shadow roots, and the roots observed: Bitwarden clients
//   apps/browser/src/autofill/services/dom-query.service.ts
//   (buildTreeWalkerNodesQueryResults, scanForNewShadowRootInSubtree,
//   visitShadowHostCandidate, enrollShadowRoot, checkForNewShadowRoots), GPL-3.0.
// - Whether a field can be seen: Bitwarden dom-element-visibility.service.ts
//   (isElementViewable, isElementHiddenByCss, isElementOutsideViewportBounds,
//   formFieldIsNotHiddenBehindAnotherElement), GPL-3.0.
// - Which fields belong together, and username-only forms: Firefox
//   toolkit/components/passwordmgr LoginFormFactory.sys.mjs (createFromField),
//   toolkit/modules/FormLikeFactory.sys.mjs (findRootForField,
//   closestFormIgnoringShadowRoots), LoginManagerChild.sys.mjs
//   (getUsernameFieldFromUsernameOnlyForm, #isProbablyAUsernameLoginForm,
//   formless capture on page navigation), LoginHelper.sys.mjs
//   (isInferredUsernameField, isInferredNonUsernameField, isInferredLoginForm),
//   LoginManager.shared.sys.mjs (isUsernameFieldType,
//   inputTypeIsCompatibleWithUsername, elementAttrsMatchRegex,
//   hasLabelMatchingRegex), shared/NewPasswordModel.sys.mjs (loginRegex,
//   loginFormAttrRegex), and `hasBeenTypePassword`, MPL-2.0.
// - The fill sequence (focus, key events, value, key events, input and change,
//   blur): Bitwarden insert-autofill-content.service.ts (insertValueIntoField,
//   triggerPreInsertEventsOnElement, triggerPostInsertEventsOnElement,
//   simulateUserKeyboardEventInteractions, simulateInputElementChangedEvent),
//   GPL-3.0. Setting the value through the prototype's native setter (so
//   frameworks that wrap `value` see the change) is what Firefox's
//   LoginManagerChild does through `setUserInput`.
// - Submits: a form's submit, Enter, a click on its submit control (Chromium
//   SubmissionIndicatorEvent); the password fields leaving the page after a
//   click in their form (Chromium's probable submission, "gone").
(function (send, register) {
  "use strict";
  const W = window;
  // The window's document, read each time: a context's first document can be replaced under
  // the same window (its `load` still reaches the listeners on `window`), so it is not captured.
  const doc = () => W.document;
  const DocumentType = W.Document;
  const loc = W.location;
  if ((loc.protocol !== "http:" && loc.protocol !== "https:") || W.origin === "null") {
    return;
  }

  const getter = (proto, name) => Object.getOwnPropertyDescriptor(proto, name).get;
  const listen = EventTarget.prototype.addEventListener;
  const dispatch = EventTarget.prototype.dispatchEvent;
  const stringify = JSON.stringify;
  const parse = JSON.parse;
  const inputValue = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value");
  const getValue = inputValue.get;
  const setValue = inputValue.set;
  const inputForm = getter(HTMLInputElement.prototype, "form");
  const inputLabels = getter(HTMLInputElement.prototype, "labels");
  const shadowOf = getter(Element.prototype, "shadowRoot");
  const hostOf = getter(ShadowRoot.prototype, "host");
  const localName = getter(Element.prototype, "localName");
  const parentElement = getter(Node.prototype, "parentElement");
  const textContent = getter(Node.prototype, "textContent");
  const isConnected = getter(Node.prototype, "isConnected");
  const rootNode = Node.prototype.getRootNode;
  const position = Node.prototype.compareDocumentPosition;
  const queryDocument = Document.prototype.querySelectorAll;
  const queryFragment = DocumentFragment.prototype.querySelectorAll;
  const queryElement = Element.prototype.querySelectorAll;
  const closest = Element.prototype.closest;
  const getAttribute = Element.prototype.getAttribute;
  const clientRects = Element.prototype.getClientRects;
  const boundingRect = Element.prototype.getBoundingClientRect;
  const computedStyle = W.getComputedStyle;
  const documentFromPoint = Document.prototype.elementFromPoint;
  const rootFromPoint = ShadowRoot.prototype.elementFromPoint;
  const composedPath = Event.prototype.composedPath;
  const observe = MutationObserver.prototype.observe;
  const whenDefined = W.CustomElementRegistry && W.CustomElementRegistry.prototype.whenDefined;
  const registry = W.customElements;
  const test = RegExp.prototype.test;
  const focus = HTMLElement.prototype.focus;
  const blur = HTMLElement.prototype.blur;
  const InputElement = HTMLInputElement;
  const FormElement = HTMLFormElement;
  const ElementType = W.Element;
  const ShadowRootType = W.ShadowRoot;
  const Observer = W.MutationObserver;
  const frame = W.requestAnimationFrame.bind(W);
  const Plain = W.Event;
  const Key = W.KeyboardEvent;
  const Weak = W.WeakRef;

  const MAX_FORMS = 32;
  const MAX_FIELDS = 128;
  // Inputs read per scan, shadow roots observed, added subtrees scanned per frame (beyond
  // it, the next frame scans the whole document once), custom element names waited for.
  const MAX_INPUTS = 512;
  const MAX_ROOTS = 256;
  const MAX_ADDED = 64;
  const MAX_WAITING = 64;
  const SKIPPED_TYPES = new Set([
    "hidden", "submit", "button", "reset", "image", "checkbox", "radio", "file", "range", "color",
  ]);
  const OBSERVED = {
    childList: true,
    subtree: true,
    attributes: true,
    attributeFilter: ["type", "style", "class", "hidden", "aria-hidden", "tabindex"],
  };
  // Firefox NewPasswordModel loginRegex and loginFormAttrRegex; LoginHelper
  // isInferredUsernameField and isInferredNonUsernameField.
  const LOGIN_RE = /login|log in|log on|log-on|Войти|sign in|sigin|sign\/in|sign-in|sign on|sign-on|ورود|登录|Přihlásit se|Přihlaste|Авторизоваться|Авторизация|entrar|ログイン|로그인|inloggen|Συνδέσου|accedi|ログオン|Giriş Yap|登入|connecter|connectez-vous|Connexion|Вход/i;
  const LOGIN_BUTTON_RE = /login|log in|log on|log-on|sign in|sigin|sign\/in|sign-in|sign on|sign-on/i;
  const USERNAME_RE = /username/i;
  const NOT_USERNAME_RE = /\b(search|code|add)\b/i;
  const matches = (re, s) => typeof s === "string" && test.call(re, s);

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

  // ----- what the page holds

  // Firefox `hasBeenTypePassword`: a field shown as text by a show-password toggle is
  // still the password.
  const wasPassword = new WeakSet();
  function typeOf(el) {
    const type = (el.type || "text").toLowerCase();
    if (type === "password") {
      wasPassword.add(el);
    }
    return wasPassword.has(el) ? "password" : type;
  }

  function fillable(el) {
    return el instanceof InputElement && !SKIPPED_TYPES.has(typeOf(el));
  }

  // The autocomplete token that decides: the last, `webauthn` aside unless alone (Chromium
  // ParseAutocomplete; Firefox's field name is "webauthn" then).
  function autocomplete(el) {
    const tokens = (getAttribute.call(el, "autocomplete") || "").toLowerCase().split(/\s+/).filter((t) => t !== "");
    const kept = tokens.filter((t) => t !== "webauthn");
    return kept.length ? kept[kept.length - 1] : tokens.length ? "webauthn" : "";
  }

  // Open shadow roots, observed as they are found (Bitwarden enrollShadowRoot); their
  // `submit` events do not leave them, so each gets the submit listener too.
  const roots = [];
  const enrolled = new WeakSet();
  let observer = null;
  function enroll(root) {
    if (enrolled.has(root) || roots.length >= MAX_ROOTS) {
      return false;
    }
    enrolled.add(root);
    roots.push(root);
    if (observer) {
      observe.call(observer, root, OBSERVED);
    }
    listen.call(root, "submit", onSubmit, true);
    return true;
  }

  // Bitwarden scanForNewShadowRootInSubtree / visitShadowHostCandidate: every element
  // of `scope` (and `scope` itself) that hosts an open shadow root, recursively. wubble:
  // a custom element without its root yet is waited for (`customElements.whenDefined`)
  // instead of Bitwarden's ShadowHostHydrationTracker deadlines.
  function scanRoots(scope) {
    const all =
      scope instanceof DocumentType ? queryDocument.call(scope, "*")
        : scope instanceof ShadowRootType ? queryFragment.call(scope, "*")
          : queryElement.call(scope, "*");
    if (scope instanceof ElementType) {
      visitHost(scope);
    }
    for (const el of all) {
      visitHost(el);
    }
  }
  const waiting = new Set();
  function visitHost(el) {
    const root = shadowOf.call(el);
    if (root) {
      // Into a known root too: a custom element in it may have its own root by now.
      enroll(root);
      scanRoots(root);
      return;
    }
    const name = localName.call(el);
    if (name.includes("-") && !waiting.has(name) && waiting.size < MAX_WAITING && whenDefined) {
      waiting.add(name);
      try {
        whenDefined.call(registry, name).then(() => {
          whole = true;
          changed();
        });
      } catch (e) {
        // Not a valid custom element name.
      }
    }
  }

  // The page's inputs in tree order, each open shadow root's where its host stands
  // (Bitwarden buildTreeWalkerNodesQueryResults; wubble: per tree, the inputs and the known
  // hosts merged in document order instead of a walk over every element).
  function inputs() {
    const out = [];
    const into = (scope) => {
      const found = Array.from(scope instanceof DocumentType ? queryDocument.call(scope, "input") : queryFragment.call(scope, "input"));
      const hosts = [];
      for (const root of roots) {
        const host = hostOf.call(root);
        if (rootNode.call(host) === scope) {
          hosts.push(host);
        }
      }
      const all = hosts.length ? found.concat(hosts).sort((a, b) => (position.call(a, b) & 4 ? -1 : 1)) : found;
      for (const el of all) {
        if (out.length >= MAX_INPUTS) {
          return;
        }
        const root = el instanceof InputElement ? null : shadowOf.call(el);
        if (root) {
          into(root);
        } else if (el instanceof InputElement) {
          out.push(el);
        }
      }
    };
    into(doc());
    return out;
  }

  // Firefox FormLikeFactory.findRootForField with closestFormIgnoringShadowRoots: the
  // outermost form holding `node` (a field's own form first), across shadow hosts; null
  // for the page's fields outside any form.
  function formOf(node) {
    let form = (node instanceof InputElement && inputForm.call(node)) || closest.call(node, "form");
    let current = node;
    while (!form) {
      const root = rootNode.call(current);
      if (!(root instanceof ShadowRootType)) {
        break;
      }
      current = hostOf.call(root);
      form = closest.call(current, "form");
    }
    if (form) {
      for (let p = parentElement.call(form); p; p = parentElement.call(p)) {
        if (p instanceof FormElement) {
          form = p;
        }
      }
    }
    return form || null;
  }

  // Each form-like and its fillable fields, in tree order.
  function groups() {
    const out = new Map();
    for (const el of inputs()) {
      if (!fillable(el)) {
        continue;
      }
      const form = formOf(el);
      const fields = out.get(form) || [];
      if (fields.length < MAX_FIELDS) {
        fields.push(el);
      }
      out.set(form, fields);
    }
    return out;
  }

  // Firefox LoginManager.shared isUsernameFieldType.
  function usernameType(el) {
    if (wasPassword.has(el) || !isConnected.call(el)) {
      return false;
    }
    const type = (getAttribute.call(el, "type") || el.type || "").toLowerCase();
    const compatible = ["text", "email", "url", "tel", "number", "search"].includes(type) || type.includes("user");
    const ac = autocomplete(el);
    return compatible && ["username", "webauthn", "email", "tel", "tel-national", "off", "on", ""].includes(ac);
  }

  // Firefox Logic.elementAttrsMatchRegex and hasLabelMatchingRegex.
  function attrsMatch(el, re) {
    return ["id", "name", "class", "aria-label", "placeholder"].some((a) => matches(re, getAttribute.call(el, a)));
  }
  function labelMatches(el, re) {
    const labels = el instanceof InputElement ? inputLabels.call(el) : null;
    return !!(labels && labels.length && matches(re, textContent.call(labels[0])));
  }

  // Firefox LoginHelper.isInferredLoginForm.
  function loginForm(form) {
    if (attrsMatch(form, LOGIN_RE)) {
      return true;
    }
    const buttons = queryElement.call(form, "button[type=submit]");
    return buttons.length === 1 && matches(LOGIN_BUTTON_RE, textContent.call(buttons[0]));
  }

  // Firefox getUsernameFieldFromUsernameOnlyForm: no password field, one username-type
  // field, which looks like a username or sits in what looks like a sign-in form, and does
  // not look like a search or a code. A field outside any form counts only with
  // autocomplete=username, as Firefox's formless username-only rule has it; and, from
  // Chromium's single-username rule, autocomplete=username decides in a form too.
  function usernameOnly(form, fields) {
    let candidate = null;
    for (const el of fields) {
      if (wasPassword.has(el)) {
        return null;
      }
      if (!usernameType(el)) {
        continue;
      }
      if (candidate) {
        return null;
      }
      candidate = el;
    }
    if (!candidate) {
      return null;
    }
    if (autocomplete(candidate) === "username") {
      return candidate;
    }
    if (!form) {
      return null;
    }
    const username = autocomplete(candidate) === "webauthn" || attrsMatch(candidate, USERNAME_RE) || labelMatches(candidate, USERNAME_RE);
    const nonUsername = attrsMatch(candidate, NOT_USERNAME_RE) || labelMatches(candidate, NOT_USERNAME_RE);
    return (username || loginForm(form)) && !nonUsername ? candidate : null;
  }

  // A form-like about signing in: a password field, or a username-only form.
  function signIn(form, fields) {
    return fields.some((el) => wasPassword.has(el)) || usernameOnly(form, fields) !== null;
  }

  // Bitwarden isElementViewable. wubble: a field is placed against the document, not the
  // viewport (Bitwarden re-checks fields scrolled into view; this script reports once per
  // change), a field outside the viewport is not tested for being covered, the transparent
  // ancestors are followed across shadow hosts, and aria-hidden with tabindex -1 together
  // mark a decoy (Bitwarden collects both attributes without using them for this).
  function visible(el) {
    if (!isConnected.call(el) || clientRects.call(el).length === 0) {
      return false;
    }
    if (getAttribute.call(el, "aria-hidden") === "true" && el.tabIndex < 0) {
      return false;
    }
    const box = boundingRect.call(el);
    const de = doc().documentElement;
    const left = box.left + W.scrollX - de.clientLeft;
    const top = box.top + W.scrollY - de.clientTop;
    if (
      box.width < 10 || box.height < 10 ||
      Math.ceil(left) < 0 || Math.floor(left + box.width) > de.scrollWidth ||
      Math.ceil(top) < 0 || Math.floor(top + box.height) > de.scrollHeight
    ) {
      return false;
    }
    const style = computedStyle.call(W, el);
    if (
      parseFloat(style.opacity) < 0.1 || style.display === "none" ||
      style.visibility === "hidden" || style.visibility === "collapse" ||
      ["inset(50%)", "inset(100%)", "circle(0)", "circle(0px)", "circle(0px at 50% 50%)",
        "polygon(0 0, 0 0, 0 0, 0 0)", "polygon(0px 0px, 0px 0px, 0px 0px, 0px 0px)"].includes(style.clipPath)
    ) {
      return false;
    }
    for (let p = up(el); p && p !== de; p = up(p)) {
      if (parseFloat(computedStyle.call(W, p).opacity) < 0.1) {
        return false;
      }
    }
    return notCovered(el, box);
  }
  function up(el) {
    const p = parentElement.call(el);
    if (p) {
      return p;
    }
    const root = rootNode.call(el);
    return root instanceof ShadowRootType ? hostOf.call(root) : null;
  }
  // Bitwarden formFieldIsNotHiddenBehindAnotherElement.
  function notCovered(el, box) {
    const x = box.left + box.width / 2;
    const y = box.top + box.height / 2;
    if (x < 0 || y < 0 || x >= W.innerWidth || y >= W.innerHeight) {
      return true;
    }
    const root = rootNode.call(el);
    const top = root instanceof ShadowRootType ? rootFromPoint.call(root, x, y) : documentFromPoint.call(doc(), x, y);
    if (top === el) {
      return true;
    }
    const labels = Array.from(inputLabels.call(el) || []);
    if (labels.includes(top)) {
      return true;
    }
    const label = top && parentElement.call(top) && closest.call(parentElement.call(top), "label");
    return !!label && labels.includes(label);
  }

  function fieldSnapshot(el, withValue) {
    return {
      id: idOf(el),
      type: typeOf(el),
      name: el.name || "",
      idAttr: el.id || "",
      autocomplete: getAttribute.call(el, "autocomplete") || "",
      visible: visible(el),
      readonly: el.readOnly || el.disabled,
      value: withValue ? getValue.call(el) : "",
    };
  }

  function formSnapshot(form, fields, withValue) {
    return { id: form ? idOf(form) : 0, fields: fields.map((el) => fieldSnapshot(el, withValue)) };
  }

  // Every form-like about signing in.
  function forms() {
    if (!doc().documentElement) {
      return [];
    }
    const out = [];
    for (const [form, fields] of groups()) {
      if (signIn(form, fields)) {
        out.push(formSnapshot(form, fields, false));
        if (out.length >= MAX_FORMS) {
          break;
        }
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

  // A password form the user submitted or clicked in, until its password fields leave the
  // page (then "gone": a single-page sign-in) or the page goes (then "submit": Firefox's
  // formless capture on navigation).
  let armed = null;
  // This document submitted a sign-in: its own "settled" (on load) would describe the page as
  // it was before the submit's outcome, so it is not sent (the browser would read the form
  // still there as a failed sign-in and drop the save). The next document reports the outcome.
  let submittedHere = false;
  const typed = (el) => getValue.call(el) !== "";

  // A submit of the form-like holding `node`: `how` is "submit" (a form's submit, its submit
  // control, Enter) or "button" (another button: a username step's Next, or a click that
  // only arms the gone check of a password form). A button outside any form stands for the
  // one sign-in form-like of the page with something typed ("next to the form").
  function act(node, how) {
    const all = groups();
    let form = formOf(node);
    let fields = all.get(form);
    if (!fields || !signIn(form, fields)) {
      const typedIn = Array.from(all).filter(([f, els]) => signIn(f, els) && els.some(typed));
      if (how !== "button" || form || typedIn.length !== 1) {
        return;
      }
      [form, fields] = typedIn[0];
    }
    const passwords = fields.filter((el) => wasPassword.has(el));
    if (passwords.some(typed)) {
      armed = { form, fields: fields.slice(), passwords };
      if (how === "submit") {
        submittedHere = true;
        post({ t: "submit", form: formSnapshot(form, fields, true) });
      }
      return;
    }
    // A username step: a username-only form, or (Chromium) autocomplete=username (or Firefox's
    // webauthn alone) next to password fields that are not shown (a decoy, or one shown after
    // this step).
    const user = !passwords.length ? usernameOnly(form, fields)
      : passwords.every((el) => !visible(el)) ? fields.find((el) => ["username", "webauthn"].includes(autocomplete(el)))
        : null;
    if (user && typed(user)) {
      post({ t: "submit", form: formSnapshot(form, fields, true) });
    }
  }

  function checkGone() {
    if (armed && armed.passwords.every((el) => !visible(el))) {
      const { form, fields } = armed;
      armed = null;
      post({ t: "gone", form: formSnapshot(form, fields, true) });
    }
  }

  // Bitwarden handleMutationObserverMutation, coalesced to one scan per frame: the added
  // subtrees are searched for shadow roots (the whole document once when too many were
  // added, or a custom element was defined), then the forms are reported again.
  const added = [];
  let whole = false;
  let scheduled = false;
  function mutated(records) {
    for (const record of records) {
      for (const node of record.addedNodes) {
        if (!(node instanceof ElementType)) {
          continue;
        }
        if (added.length < MAX_ADDED) {
          added.push(node);
        } else {
          whole = true;
        }
      }
    }
    changed();
  }
  function changed() {
    if (scheduled) {
      return;
    }
    scheduled = true;
    frame(() => {
      scheduled = false;
      const nodes = added.splice(0);
      if (whole) {
        whole = false;
        scanRoots(doc());
      } else {
        for (const node of nodes) {
          if (isConnected.call(node)) {
            scanRoots(node);
          }
        }
      }
      checkGone();
      report();
    });
  }

  // Events from inside shadow roots reach the window retargeted to their host; the
  // first node of the composed path is the real target.
  const target = (e) => composedPath.call(e)[0] || e.target;
  function onSubmit(e) {
    if (e.isTrusted && e.target instanceof FormElement) {
      act(e.target, "submit");
    }
  }
  listen.call(W, "submit", onSubmit, true);
  listen.call(W, "keydown", (e) => {
    const t = target(e);
    if (e.isTrusted && e.key === "Enter" && fillable(t)) {
      act(t, "submit");
    }
  }, true);
  listen.call(W, "click", (e) => {
    const t = target(e);
    if (!e.isTrusted || !(t instanceof ElementType)) {
      return;
    }
    const control = closest.call(t, "button, input[type=submit], input[type=image], input[type=button], [role=button]");
    if (!control) {
      return;
    }
    act(control, control.type === "submit" || control.type === "image" ? "submit" : "button");
  }, true);
  listen.call(W, "pagehide", (e) => {
    if (e.isTrusted && armed && armed.passwords.some(typed)) {
      const { form, fields } = armed;
      armed = null;
      post({ t: "submit", form: formSnapshot(form, fields, true) });
    }
  }, true);
  // The document is parsed: watch it and report. On the window, where DOMContentLoaded
  // bubbles from whichever document the window holds; on load too, if that was missed.
  function begin() {
    if (observer || !doc().documentElement) {
      return;
    }
    observer = new Observer(mutated);
    observe.call(observer, doc().documentElement, OBSERVED);
    scanRoots(doc());
    for (const root of roots) {
      observe.call(observer, root, OBSERVED);
    }
    report();
  }
  listen.call(W, "DOMContentLoaded", (e) => {
    if (e.isTrusted) {
      begin();
    }
  });
  listen.call(W, "load", (e) => {
    if (e.isTrusted) {
      frame(() => {
        begin();
        scanRoots(doc());
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
      dispatch.call(el, new Key(type, { bubbles: true, cancelable: true, composed: true }));
    }
  }

  // wubble: the events are composed as the browser's own are (input is, change is not),
  // so a listener on a shadow host sees the input.
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
    dispatch.call(el, new Plain("input", { bubbles: true, composed: true }));
    dispatch.call(el, new Plain("change", { bubbles: true }));
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
