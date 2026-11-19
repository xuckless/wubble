// Samples the touch kit's menu, the settings Select's chevron and the search capsule's clear
// button in headless Chromium on the fixed clock (1/120 s), from the design's own stylesheets
// (core/wubble.css, pages/touch.css, pages/settings.css with tokenCss and the menu's inline
// variables, pages/touch.tsx `MENU`), into tools/fidelity/fixtures/controls/menu/<name>.json:
// `{ source, dt, ms, frames: { prop: [...] } }`. The clock machinery is curves.mjs's (every
// transition paused and set to each step's time through the Web Animations API, then the
// computed style read); kept apart so that harness file stays as it is. Flagged for the lead: the
// probe loop could be hoisted into lib/ and shared.
// crates/wubble-ui/tests/controls_menu.rs compares the kit against these.
//   node tools/fidelity/controls-curves.mjs     (needs the design clone and chromium)
import { readFileSync, writeFileSync, mkdirSync } from 'node:fs';
import { resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import { REPO, designRoot, designSrc, designCommit } from './lib/design.mjs';
import { launch } from './lib/cdp.mjs';

const root = designRoot();
const SRC = designSrc(root);
const { commit } = designCommit(root);
const tokens = await import(pathToFileURL(resolve(SRC, 'core/tokens.ts')).href);
const sheet = f => readFileSync(resolve(SRC, f), 'utf8');
const css = ['core/wubble.css', 'pages/touch.css', 'pages/settings.css'].map(sheet).join('\n');
const DT = 1000 / 120;
const d = tokens.motion.dropdown;
// the menu's inline variables (touch.tsx `Menu`, the wrapper's style)
const MENU_VARS = `--m-in:${d.in}ms;--m-out:${d.out}ms;--m-from:${d.from};--m-blur:4px;--m-gap:8px`;
const row = (delay, extra = '') => `<div class="tp-menu ${extra}" id="m"><button id="p" class="tp-mrow" style="transition-delay:${delay}ms"></button></div>`;

// name, markup (`#p` is read), how it starts, length (ms)
const PROBES = [
  { name: 'menu-in', html: '<div id="p" class="tp-menu wb-thick"></div>', start: 'p.classList.add("shown");', ms: d.in, source: 'pages/touch.css .tp-menu.shown' },
  { name: 'menu-out', html: '<div id="p" class="tp-menu wb-thick shown"></div>', start: 'p.classList.remove("shown");', ms: d.out, source: 'pages/touch.css .tp-menu' },
  { name: 'mrow-in-0', html: row(40), start: 'm.classList.add("shown");', ms: 40 + 240, source: 'pages/touch.css .tp-menu.shown .tp-mrow, delay 40 ms (row 0)' },
  { name: 'mrow-in-3', html: row(40 + 3 * 30), start: 'm.classList.add("shown");', ms: 130 + 240, source: 'pages/touch.css .tp-menu.shown .tp-mrow, delay 130 ms (row 3)' },
  { name: 'mrow-out', html: row(0, 'shown'), start: 'm.classList.remove("shown");', ms: 120, source: 'pages/touch.css .tp-mrow' },
  { name: 'hl-glide', html: '<span id="p" class="tp-menu-hl on" style="transform: translateY(0px)"></span>', start: 'p.style.transform = "translateY(72px)";', ms: 160, source: 'pages/touch.css .tp-menu-hl (row 0 to row 2)' },
  { name: 'hl-jump', html: '<span id="p" class="tp-menu-hl jump" style="transform: translateY(0px)"></span>', start: 'p.classList.add("on"); p.style.transform = "translateY(72px)";', ms: 120, source: 'pages/touch.css .tp-menu-hl.jump.on (appears at row 2)' },
  { name: 'hl-fade', html: '<span id="p" class="tp-menu-hl on"></span>', start: 'p.classList.remove("on");', ms: 120, source: 'pages/touch.css .tp-menu-hl' },
  { name: 'clear-in', html: '<button id="p" class="st-clear"></button>', start: 'p.classList.add("on");', ms: 240, source: 'pages/settings.css .st-clear.on' },
  { name: 'clear-out', html: '<button id="p" class="st-clear on"></button>', start: 'p.classList.remove("on");', ms: 150, source: 'pages/settings.css .st-clear' },
  { name: 'chev', html: '<div id="p" class="tp-chev"></div>', start: 'p.classList.add("up");', ms: 220, source: 'pages/touch.css .tp-chev.up' },
];

const page = await launch({ width: 400, height: 300 });
try {
  const out = resolve(REPO, 'tools/fidelity/fixtures/controls/menu');
  mkdirSync(out, { recursive: true });
  for (const p of PROBES) {
    const frames = await page.evaluate(`(async () => {
      document.documentElement.innerHTML = ${JSON.stringify(`<head><style>${css}\n.wb { ${tokens.tokenCss()};${MENU_VARS} } #p { width: 100px; height: 40px; }</style></head><body><div class="wb">${p.html}</div></body>`)};
      const p = document.getElementById('p'), m = document.getElementById('m');
      getComputedStyle(p).transform; // the start style
      ${p.start}
      const anims = () => document.getAnimations();
      const frames = { opacity: [], ty: [], sx: [], cos: [], blur: [] };
      const n = Math.round(${p.ms} / ${DT});
      for (let i = 0; i <= n + 1; i++) {
        for (const a of anims()) { a.pause(); a.currentTime = i * ${DT}; }
        const s = getComputedStyle(p);
        const m2 = new DOMMatrixReadOnly(s.transform === 'none' ? undefined : s.transform);
        // the CSS \`scale\` property (the clear button) composes with \`transform\`
        const sc = s.scale === 'none' ? 1 : Number(s.scale.split(' ')[0]);
        frames.opacity.push(Number(s.opacity));
        frames.ty.push(m2.m42);
        frames.sx.push(Math.hypot(m2.m11, m2.m12) * sc);
        frames.cos.push(m2.m11);
        const b = /blur\\(([\\d.]+)px\\)/.exec(s.filter);
        frames.blur.push(b ? Number(b[1]) : 0);
      }
      return frames;
    })()`);
    const round = a => a.map(v => Number(v.toFixed(5)));
    const data = { source: `${p.source} @${commit}`, dt: DT, ms: p.ms, frames: Object.fromEntries(Object.entries(frames).map(([k, v]) => [k, round(v)])) };
    writeFileSync(resolve(out, `${p.name}.json`), `${JSON.stringify(data)}\n`);
    console.log(`controls-curves: ${p.name} (${frames.opacity.length} frames)`);
  }
} finally {
  page.close();
}
