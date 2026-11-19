// Samples the web design's fixed-curve motions in headless Chromium on the fixed clock (1/120 s)
// and writes them to tools/fidelity/curves/<name>.json: `{ source, dt, frames: { prop: [...] } }`.
// The page is the design's own stylesheet (core/wubble.css) with its token variables
// (core/tokens.ts tokenCss), one probe element per motion; every animation and transition is
// paused and set to each step's time through the Web Animations API, then the computed style is
// read. The kit's curve tests (crates/wubble-ui/tests/motion_curves.rs) compare against these.
//   node tools/fidelity/curves.mjs        (needs the design clone and chromium, like capture.mjs)
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
const css = sheet('core/wubble.css');
const DT = 1000 / 120;

// name, the markup's probe (`#p` is read; `::before` when `pseudo`; `#f`'s height over 40 px is
// `h`), how it starts, length (ms), the page stylesheets it needs beside core/wubble.css
const PROBES = [
  { name: 'overlap-in', html: '<div id="p" class="wb-in"></div>', ms: 240, source: 'core/wubble.css .wb-in (wb-in)' },
  { name: 'overlap-out', html: '<div id="p" class="wb-out"></div>', ms: 190, source: 'core/wubble.css .wb-out (wb-out)' },
  { name: 'press-in', html: '<div id="p" class="wb-press"></div>', start: 'p.style.transitionDuration = ".12s"; p.style.transform = "scale(.97)";', ms: 120, source: 'core/wubble.css .wb-press:active' },
  { name: 'press-out', html: '<div id="p" class="wb-press" style="transform: scale(.97)"></div>', start: 'p.style.transform = "none";', ms: 200, source: 'core/wubble.css .wb-press' },
  { name: 'film-in', html: '<div id="p" class="wb-film"></div>', start: 'p.classList.add("wb-on");', pseudo: true, ms: 200, source: 'core/wubble.css .wb-film.wb-on::before' },
  { name: 'film-out', html: '<div id="p" class="wb-film wb-on"></div>', start: 'p.classList.remove("wb-on");', pseudo: true, ms: 150, source: 'core/wubble.css .wb-film::before' },
  { name: 'wobble', html: '<div class="wb-wob-a"><div id="p" class="wb-icon"></div></div>', ms: 550, source: 'core/wubble.css .wb-wob-a > .wb-icon (wb-wob-a)' },
  { name: 'pulse', html: '<div id="p" class="tp-pulse"></div>', ms: 800, sheets: ['pages/touch.css'], source: 'pages/touch.css .tp-pulse (wb-pulse)' },
  { name: 'fold-in', html: '<div id="f" class="st-fold"><div id="p" class="st-fold-in"><div style="height: 40px"></div></div></div>', start: 'f.classList.add("open");', ms: 240, sheets: ['pages/settings.css'], source: 'pages/settings.css .st-fold.open' },
  { name: 'fold-out', html: '<div id="f" class="st-fold open"><div id="p" class="st-fold-in"><div style="height: 40px"></div></div></div>', start: 'f.classList.remove("open");', ms: 190, sheets: ['pages/settings.css'], source: 'pages/settings.css .st-fold' },
];

const page = await launch({ width: 400, height: 300 });
// Design bug (598f814): core/wubble.css `.wb-out` (and browser.css `.wb-tab-closing`,
// `.wb-cmd-out`) ease on `var(--wb-ease-in-out)`, but tokenCss defines `--wb-ease-inOut`, so in
// the browser those animations are invalid at computed-value time and do not run. The kit
// follows the intent (tokens.ts motion.overlap.out on ease.inOut); the sampler defines the
// missing name as the defined one so the JSON is that intended curve. Flagged for the owner.
const vars = `${tokens.tokenCss()};--wb-ease-in-out:var(--wb-ease-inOut)`;
try {
  const out = resolve(REPO, 'tools/fidelity/curves');
  mkdirSync(out, { recursive: true });
  for (const p of PROBES) {
    const frames = await page.evaluate(`(async () => {
      document.documentElement.innerHTML = ${JSON.stringify(`<head><style>${[css, ...(p.sheets ?? []).map(sheet)].join('\n')}\n.wb { ${vars} } #p { width: 100px; height: 40px; } #p.st-fold-in { height: auto; }</style></head><body><div class="wb">${p.html}</div></body>`)};
      const p = document.getElementById('p'), f = document.getElementById('f');
      getComputedStyle(p).transform; getComputedStyle(p, '::before').opacity; // the start style
      ${p.start ?? ''}
      const anims = () => document.getAnimations();
      for (const a of anims()) a.pause();
      const frames = { opacity: [], ty: [], sx: [], sy: [], rot: [], blur: [], spread: [], alpha: [], h: [] };
      const n = Math.round(${p.ms} / ${DT});
      for (let i = 0; i <= n + 1; i++) {
        for (const a of anims()) { a.pause(); a.currentTime = i * ${DT}; }
        const s = getComputedStyle(p, ${p.pseudo ? "'::before'" : 'null'});
        const m = new DOMMatrixReadOnly(s.transform === 'none' ? undefined : s.transform);
        frames.opacity.push(Number(s.opacity));
        frames.ty.push(m.m42);
        frames.sx.push(Math.hypot(m.m11, m.m12));
        frames.sy.push(Math.hypot(m.m21, m.m22));
        frames.rot.push(Math.atan2(m.m12, m.m11) * 180 / Math.PI);
        const b = /blur\\(([\\d.]+)px\\)/.exec(s.filter);
        frames.blur.push(b ? Number(b[1]) : 0);
        // the first box shadow's spread and colour alpha (\`rgba(r, g, b, a) x y blur spread\`)
        const sh = /rgba?\\(\\d+, \\d+, \\d+(?:, ([\\d.]+))?\\) -?[\\d.]+px -?[\\d.]+px [\\d.]+px (-?[\\d.]+)px/.exec(s.boxShadow);
        frames.spread.push(sh ? Number(sh[2]) : 0);
        frames.alpha.push(sh ? Number(sh[1] ?? 1) : 0);
        frames.h.push(f ? f.getBoundingClientRect().height / 40 : 0);
      }
      return frames;
    })()`);
    const round = a => a.map(v => Number(v.toFixed(5)));
    const data = { source: `${p.source} @${commit}`, dt: DT, ms: p.ms, frames: Object.fromEntries(Object.entries(frames).map(([k, v]) => [k, round(v)])) };
    writeFileSync(resolve(out, `${p.name}.json`), `${JSON.stringify(data)}\n`);
    console.log(`curves: ${p.name} (${frames.opacity.length} frames)`);
  }
} finally {
  page.close();
}
