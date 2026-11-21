// `--smoke autofill`'s fixture pages: what reached their fields through their own listeners,
// told in the title (an iframe tells its parent, which takes the title over). Values a page
// "types" go through the native setter and an input event, as a user's typing would.
"use strict";
const fx = (() => {
  const native = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value");
  const framed = window.parent !== window;
  const fx = {
    get(el) {
      return native.get.call(el);
    },
    $(sel) {
      return typeof sel === "string" ? document.querySelector(sel) : sel;
    },
    say(title) {
      document.title = title;
      if (framed) {
        window.parent.postMessage({ fixtureTitle: title }, "*");
      }
    },
    // A user typing `value` into `el`.
    type(el, value) {
      el = fx.$(el);
      native.set.call(el, value);
      el.dispatchEvent(new Event("input", { bubbles: true, composed: true }));
      el.dispatchEvent(new Event("change", { bubbles: true }));
    },
    // Says `title` once every field's last input or change event carried its value and every
    // field in `empty` is still empty; then runs `then`.
    expect(pairs, title, then, empty = []) {
      const seen = new Map();
      let done = false;
      const check = () => {
        if (done) return;
        const filled = pairs.every(([sel, value]) => seen.get(fx.$(sel)) === value);
        const clean = empty.every((sel) => fx.get(fx.$(sel)) === "");
        if (filled && clean) {
          done = true;
          fx.say(title);
          if (then) then();
        }
      };
      for (const [sel] of pairs) {
        const el = fx.$(sel);
        for (const type of ["input", "change"]) {
          el.addEventListener(type, () => {
            seen.set(el, fx.get(el));
            check();
          });
        }
      }
    },
  };
  // A framed fixture's title becomes the page's.
  window.addEventListener("message", (e) => {
    if (e.data && typeof e.data.fixtureTitle === "string") {
      fx.say(e.data.fixtureTitle);
    }
  });
  return fx;
})();
