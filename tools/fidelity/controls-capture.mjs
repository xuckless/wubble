#!/usr/bin/env node
// The press lane's motion fixtures: the design's own controls driven in headless Chromium on the
// harness's fixed clock (vite.config.mjs injects `__wb.step`: rAF, timers and performance.now;
// every CSS animation and transition is paused and set to the same clock each step), sampled
// every 1/120 s into tools/fidelity/fixtures/controls/<control>/<name>.json:
// `{ source, dt, frames: { prop: [...] } }`. The kit's curve tests
// (crates/wubble-ui/tests/press_curves.rs) step wubble's controls by the same 1/120 s and compare.
//
//   node tools/fidelity/controls-capture.mjs [--base http://127.0.0.1:5199] [--geometry]
//
// `--geometry` only measures the controls' boxes at rest (fixtures/controls/geometry.json, CSS px:
// width and height per control), which the specimen's geometry test compares.
//
// Without --base it serves the design clone itself through the harness's vite (as run.sh does)
// and stops it at the end. Needs the clone (with its node_modules) and chromium, like capture.mjs.
// The JS controls (toggle, hold) are read from the inline transform and clip-path their loops
// write; the CSS ones (check box, button squash, tooltip) from computed style.
import { mkdirSync, writeFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { spawn } from 'node:child_process';
import { REPO, designRoot, designCommit } from './lib/design.mjs';
import { launch } from './lib/cdp.mjs';

const args = process.argv.slice(2);
const opt = (n, d) => (args.includes(n) ? args[args.indexOf(n) + 1] : d);
const port = Number(process.env.FIDELITY_PORT ?? 5198);
let base = opt('--base', null);
const root = designRoot();
const { commit } = designCommit(root);
const DT = 1000 / 120;
const sleep = ms => new Promise(r => setTimeout(r, ms));
const OUT = resolve(REPO, 'tools/fidelity/fixtures/controls');

let vite = null;
if (!base) {
  base = `http://127.0.0.1:${port}`;
  vite = spawn('npx', ['vite', '--config', resolve(REPO, 'tools/fidelity/vite.config.mjs')], {
    cwd: root, env: { ...process.env, WUBBLE_DESIGN_ROOT: root, FIDELITY_PORT: String(port) }, stdio: 'ignore', detached: true,
  });
  let up = false;
  for (let i = 0; i < 120 && !up; i++) {
    try { up = (await fetch(`${base}/__parity/manifest.json`)).ok; } catch { await sleep(500); }
  }
  if (!up) { process.kill(-vite.pid); throw new Error('controls-capture: vite did not come up'); }
}

// The in-page stepper (capture.mjs's): one 1/120 s frame of the clock, then every animation set
// to it.
const STEPPER = `window.__wbAdvance = (n = 1) => {
  for (let i = 0; i < n; i++) {
    window.__wb.step(${DT});
    for (const a of document.getAnimations()) {
      if (a.__t0 === undefined) a.__t0 = window.__wb.now() - (a.currentTime ?? 0);
      if (a.playState !== 'paused') a.pause();
      a.currentTime = window.__wb.now() - a.__t0;
    }
  }
  return window.__wb.now();
};
window.__wbFix = () => { for (const a of document.getAnimations()) if (a.__t0 === undefined) { a.__t0 = window.__wb.now() - (a.currentTime ?? 0); a.pause(); a.currentTime = window.__wb.now() - a.__t0; } };
window.__num = (s, re) => { const m = re.exec(s); return m ? m.slice(1).map(Number) : null; };
window.__mat = s => { const m = new DOMMatrixReadOnly(s === 'none' ? undefined : s); return [Math.hypot(m.m11, m.m12), Math.hypot(m.m21, m.m22), m.m41, m.m42]; };`;

const page = await launch({ port: 9334, width: 1360, height: 880 });
const ev = e => page.evaluate(e);
const mouse = (type, [x, y], extra = {}) => page.send('Input.dispatchMouseEvent', { type, x, y, button: 'left', clickCount: 1, ...extra });
const key = (type, k) => page.send('Input.dispatchKeyEvent', { type, key: k, code: k === ' ' ? 'Space' : k, windowsVirtualKeyCode: k === ' ' ? 32 : 13, text: type === 'keyDown' ? (k === ' ' ? ' ' : '\r') : undefined });

async function open(url, ready, steps = async () => {}) {
  await page.send('Emulation.setDeviceMetricsOverride', { width: 1360, height: 3600, deviceScaleFactor: 1, mobile: false });
  await page.send('Page.navigate', { url: base + url });
  for (let i = 0; i < 200; i++) {
    await sleep(100);
    try { if (await ev(`!!document.querySelector(${JSON.stringify(ready)}) && document.fonts.status === 'loaded'`)) break; } catch {}
  }
  await ev(STEPPER);
  await ev('window.__wbAdvance(240)');
  // the backdrop's liquid stops after the settle (as capture.mjs's frozen scenes): the controls
  // do not depend on it, and SwiftShader steps it slowly
  await ev('globalThis.__wbFrozen = true');
  await steps();
}
async function center(sel, i = 0) {
  const r = await ev(`(() => { const e = document.querySelectorAll(${JSON.stringify(sel)})[${i}]; if (!e) return null; e.scrollIntoView({ block: 'center' }); const b = e.getBoundingClientRect(); return [b.x + b.width / 2, b.y + b.height / 2]; })()`);
  if (!r) throw new Error(`no ${sel}[${i}]`);
  await ev('window.__wbAdvance(1)');
  return ev(`(() => { const b = document.querySelectorAll(${JSON.stringify(sel)})[${i}].getBoundingClientRect(); return [b.x + b.width / 2, b.y + b.height / 2]; })()`);
}
/**
 * `n` frames of `probe` (an expression returning { prop: value }), one per 1/120 s after the
 * first. One evaluation per frame, so the page's state updates (Preact renders on a microtask)
 * land between frames as they would between animation frames.
 */
async function sample(probe, n) {
  const frames = {};
  for (let i = 0; i < n; i++) {
    const v = await ev(`(() => { ${i > 0 ? 'window.__wbAdvance(1);' : 'window.__wbFix();'} return (${probe})(); })()`);
    for (const [k, x] of Object.entries(v)) (frames[k] ??= []).push(Number(x.toFixed(5)));
  }
  return frames;
}
function save(control, name, source, frames) {
  const dir = resolve(OUT, control);
  mkdirSync(dir, { recursive: true });
  writeFileSync(resolve(dir, `${name}.json`), `${JSON.stringify({ source: `${source} @${commit}`, dt: DT, frames })}\n`);
  console.log(`controls-capture: ${control}/${name} (${Object.values(frames)[0].length} frames)`);
}

// The toggle's paint (touch.tsx:166-180): its inline transform and clip-path, as numbers.
const TOGGLE = sel => `() => {
  const b = document.querySelector(${JSON.stringify(sel)}), k = b.querySelector('.tp-tg-knob'), f = b.querySelector('.tp-tg-fill');
  const t = window.__num(k.style.transform, /translateX\\(([-\\d.e]+)px\\) translateX\\(([-\\d.e]+)px\\) scale\\(([-\\d.e]+), ?([-\\d.e]+)\\) translateX\\([-\\d.e]+px\\) translateX\\(([-\\d.e]+)px\\) scale\\(([-\\d.e]+), ?([-\\d.e]+)\\)/);
  // \`inset(v r v l round R)\`, which the browser shortens when the sides agree
  const c = window.__num(f.style.clipPath, /inset\\(([-\\d.e]+)px[^r]*round ([-\\d.e]+)px\\)/);
  const K = k.offsetWidth;
  return { x: t[0], o1: t[1] / K, k: t[2], ky: t[3], o2: t[4] / K, qx: t[5], qy: t[6], fill: 1 - c[0] / c[1] };
}`;

/** Each selector's first (or `i`th) box at rest, [width, height]. */
async function boxes(list) {
  const out = {};
  for (const [name, sel, i = 0] of list) {
    await center(sel, i);
    out[name] = await ev(`(() => { const b = document.querySelectorAll(${JSON.stringify(sel)})[${i}].getBoundingClientRect(); return [b.width, b.height]; })()`);
  }
  return out;
}
async function openSettings() {
  await open('/wubble/', '.wb-window', async () => {
    await page.send('Input.dispatchKeyEvent', { type: 'keyDown', key: ',', code: 'Comma', windowsVirtualKeyCode: 188, modifiers: 2 });
    await page.send('Input.dispatchKeyEvent', { type: 'keyUp', key: ',', code: 'Comma', windowsVirtualKeyCode: 188, modifiers: 2 });
    for (let i = 0; i < 50 && !(await ev("!!document.querySelector('.st-hold')")); i++) {
      await page.send('Page.captureScreenshot', { format: 'png', clip: { x: 0, y: 0, width: 1, height: 1, scale: 1 } });
      await sleep(200);
      await ev('window.__wbAdvance(1)');
    }
    await ev('window.__wbAdvance(180)');
  });
}

try {
  if (args.includes('--geometry')) {
    await open('/wubble/pages/_harness.html?page=touch', '.tp-grid');
    const touch = await boxes([
      ['button-pri', '.tp-btn-body.pri'], ['button-glass', '.tp-btn-body.glass'], ['button-ghost', '.tp-btn-body.ghost'],
      ['toggle-lg', '.tp-tg'], ['check-row', '.tp-ck', 1], ['check-box', '.tp-ck-box', 1], ['checks', '.tp-checks'],
    ]);
    await openSettings();
    const settings = await boxes([['toggle-sm', '.tp-tg-sm'], ['hold', '.st-hold'], ['kbd', '.st-page kbd'], ['icon-button', '.wb-nav .wb-ib']]);
    const ci = await ev(`[...document.querySelectorAll('.st-confirm')].findIndex(e => e.textContent.includes('updates'))`);
    const confirm = await boxes([['confirm', '.st-confirm', ci]]);
    mkdirSync(OUT, { recursive: true });
    writeFileSync(resolve(OUT, 'geometry.json'), `${JSON.stringify({ source: `pages/touch.tsx (touch harness), pages/settings.tsx (Ctrl+,), browser/chrome.tsx @${commit}: [width, height] CSS px at rest`, boxes: { ...touch, ...settings, ...confirm } }, null, 1)}\n`);
    console.log('controls-capture: geometry.json');
    process.exitCode = 0;
  } else {
  let p;
  if (!args.includes('--confirm-only')) {
  // ── touch page: toggle (lg), check box, buttons, dock tooltip ─────────────────────────────
  await open('/wubble/pages/_harness.html?page=touch', '.tp-grid');
  // the specimen toggle starts on; a tap turns it off, a second on
  p = await center('.tp-tg');
  for (const [name, to] of [['tap-off', 'off'], ['tap-on', 'on']]) {
    await mouse('mousePressed', p);
    await mouse('mouseReleased', p);
    save('toggle', `lg-${name}`, `pages/touch.tsx:161-224 Toggle lg, a tap ${to}`, await sample(TOGGLE('.tp-tg'), 90));
  }
  // held, then dragged 30 px right in 12 frames and released: band, stretch, fling
  await mouse('mousePressed', p);
  await mouse('mouseReleased', p);
  await ev('window.__wbAdvance(120)');
  await mouse('mousePressed', p);
  const held = await sample(TOGGLE('.tp-tg'), 24);
  const drag = { x: [], k: [], ky: [], o1: [], qx: [], qy: [], o2: [], fill: [] };
  for (let i = 1; i <= 12; i++) {
    await mouse('mouseMoved', [p[0] + 30 * i / 12, p[1]]);
    const f = await sample(TOGGLE('.tp-tg'), 2);
    for (const k of Object.keys(drag)) drag[k].push(f[k][1]);
  }
  await mouse('mouseReleased', [p[0] + 30, p[1]]);
  const after = await sample(TOGGLE('.tp-tg'), 72);
  save('toggle', 'lg-hold', 'pages/touch.tsx:161-224 Toggle lg, pressed and held 24 frames', held);
  save('toggle', 'lg-drag', 'pages/touch.tsx:161-224 Toggle lg, dragged 30 px right over 12 frames (2 frames each, the second sampled)', drag);
  save('toggle', 'lg-fling', 'pages/touch.tsx:161-224 Toggle lg, released after the drag', after);

  // check box: the second row starts off
  const CK = `() => {
    const b = document.querySelectorAll('.tp-ck')[1], box = b.querySelector('.tp-ck-box'), s = getComputedStyle(box);
    const fill = window.__num(getComputedStyle(b.querySelector('.tp-ck-fill')).clipPath, /circle\\(([-\\d.e]+)(%|px)/);
    const dash = parseFloat(getComputedStyle(b.querySelector('path')).strokeDashoffset);
    const m = window.__mat(s.transform), sc = (s.scale === 'none' ? '1 1' : s.scale).split(' ').map(Number);
    return { fill: fill ? fill[0] / 100 : 0, tick: 1 - dash / 20, sx: m[0], sy: m[1], px: sc[0], py: sc[1] ?? sc[0] };
  }`;
  p = await center('.tp-ck', 1);
  await mouse('mousePressed', p);
  await mouse('mouseReleased', p);
  save('check-box', 'on', 'pages/touch.css:163-174 .tp-ck turned on (fill, tick, squash)', await sample(CK, 72));
  await mouse('mousePressed', p);
  await mouse('mouseReleased', p);
  save('check-box', 'off', 'pages/touch.css:163-174 .tp-ck turned off (fill, tick)', await sample(CK, 40));
  await mouse('mousePressed', p);
  save('check-box', 'press', 'pages/touch.css:165-167 .tp-ck:active .tp-ck-box scale', await sample(CK, 30));
  await mouse('mouseReleased', p);
  save('check-box', 'release', 'pages/touch.css:165-167 .tp-ck-box scale back (and the squash as it turns on)', await sample(CK, 60));

  // button: the release squash on the body (`tp-bq-a`), the press on the button
  const BTN = `() => {
    const b = document.querySelectorAll('.tp-btn')[1], body = b.querySelector('.tp-btn-body');
    const m = window.__mat(getComputedStyle(body).transform), n = window.__mat(getComputedStyle(b).transform);
    return { sx: m[0], sy: m[1], press: n[0] };
  }`;
  p = await center('.tp-btn', 1);
  await mouse('mousePressed', p);
  save('button', 'press', 'core/wubble.css:46-49 .wb-press:active on .tp-btn', await sample(BTN, 24));
  await mouse('mouseReleased', p);
  save('button', 'release', 'pages/touch.css:128-131 tp-bq-a on .tp-btn-body after pointerup, and the press back', await sample(BTN, 72));

  // dock tooltip: hover the first button; after the intent it shows; leave and it hides
  const TIP = `() => {
    const t = document.querySelector('.tp-tip'), s = getComputedStyle(t), m = window.__mat(s.transform);
    return { opacity: Number(s.opacity), ty: m[3], scale: m[1] };
  }`;
  p = await center('.tp-dock-btn', 0);
  await mouse('mouseMoved', p, { button: 'none' });
  for (let i = 0; i < 120 && !(await ev("!!document.querySelector('.tp-tip.show')")); i++) await ev('window.__wbAdvance(1)');
  save('tooltip', 'show', 'pages/touch.css:103-106 .tp-tip -> .tp-tip.show', await sample(TIP, 30));
  await mouse('mouseMoved', [p[0], p[1] - 200], { button: 'none' });
  save('tooltip', 'hide', 'pages/touch.css:103-105 .tp-tip.show -> .tp-tip', await sample(TIP, 24));
  }

  // ── settings: hold and confirm ────────────────────────────────────────────────────────────
  await openSettings();
  if (!args.includes('--confirm-only')) {
  // the settings row's sm toggle: a tap each way (real px: travel 20, the same band and slop)
  p = await center('.tp-tg-sm');
  for (const name of ['tap-1', 'tap-2']) {
    await mouse('mousePressed', p);
    await mouse('mouseReleased', p);
    save('toggle', `sm-${name}`, 'pages/touch.tsx:161-224 Toggle sm (settings row), a tap', await sample(TOGGLE('.tp-tg-sm'), 72));
  }
  const smOn = await ev("document.querySelector('.tp-tg-sm').getAttribute('aria-checked')");
  writeFileSync(resolve(OUT, 'toggle/sm-start.json'), `${JSON.stringify({ source: `the sm toggle's state after both taps @${commit}`, on: smOn === 'true' })}\n`);

  const HOLD = `() => {
    const b = document.querySelector('.st-hold'), f = b.querySelector('.st-hold-fill');
    const c = window.__num(f.style.clipPath, /inset\\(\\S+ ([-\\d.e]+)%/);
    const t = window.__num(b.style.transform, /scale\\(([-\\d.e]+), ?([-\\d.e]+)\\)/);
    return { p: c ? 1 - c[0] / 100 : 0, k: t ? t[0] : 1, ky: t ? t[1] : 1, done: b.classList.contains('done') ? 1 : 0 };
  }`;
  p = await center('.st-hold');
  await mouse('mousePressed', p);
  save('hold', 'full', 'pages/settings.tsx:308-352 HoldButton held to done', await sample(HOLD, 160));
  await mouse('mouseReleased', p);
  save('hold', 'done', 'pages/settings.tsx:308-352 HoldButton after done (strike, reset after doneMs)', await sample(HOLD, 320));
  await mouse('mousePressed', p);
  const early = await sample(HOLD, 60);
  await mouse('mouseReleased', p);
  const drain = await sample(HOLD, 90);
  save('hold', 'early', 'pages/settings.tsx:308-352 HoldButton held 60 frames', early);
  save('hold', 'drain', 'pages/settings.tsx:308-352 HoldButton let go early: the drain', drain);
  }

  const CONFIRM = `() => {
    const b = [...document.querySelectorAll('.st-confirm')].find(e => e.textContent.includes('updates') || e.textContent.includes('Checking') || e.textContent.includes('Up to date'));
    const items = [...b.querySelectorAll('.tp-sw-i')].filter(i => i.dataset.s !== 'out');
    const cur = items[items.length - 1];
    return { phase: cur.querySelector('.st-busy') ? 1 : cur.querySelector('.st-ok') ? 2 : 0 };
  }`;
  const ci = await ev(`[...document.querySelectorAll('.st-confirm')].findIndex(e => e.textContent.includes('updates'))`);
  p = await center('.st-confirm', ci);
  await mouse('mousePressed', p);
  await mouse('mouseReleased', p);
  save('confirm', 'run', 'pages/settings.tsx:354-373 ConfirmButton ms=1400: idle -> busy -> done -> idle', await sample(CONFIRM, 520));
  }
} finally {
  page.close();
  if (vite) process.kill(-vite.pid);
}
