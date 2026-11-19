#!/usr/bin/env node
// Controls kit B's motion as the web design runs it, sampled on the fixed clock (1/120 s), for
// the curve tests (crates/wubble-ui/tests/controls_b_curves.rs). Writes
// tools/fidelity/fixtures/controls/<name>.json: `{ source, dt, script, frames: { prop: [...] } }`.
//
// Two kinds:
//  - the physics (ElasticSlider, Segmented, the settings swatches' ring): the design's own pages,
//    served by the harness's vite (vite.config.mjs: the fixed clock), driven by real CDP pointer
//    and key events; each step advances the page clock by 1/120 s (one loop frame, dt = 1/120)
//    and reads what the loop painted (the inline transforms). `script` is the input, in the
//    control's own px, so the Rust machine replays it frame for frame;
//  - the CSS tweens (the seam's pill, the scroll thumb, the slider's bubble and label): one probe
//    element per rule with the design's stylesheets, as tools/fidelity/curves.mjs does (every
//    animation paused and set to each step's time through the Web Animations API).
//
//   node tools/fidelity/controls-curves.mjs      (needs the design clone with its node_modules
//                                                 and chromium, like run.sh)
import { spawn } from 'node:child_process';
import { readFileSync, writeFileSync, mkdirSync } from 'node:fs';
import { resolve, dirname } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { REPO, designRoot, designSrc, designCommit } from './lib/design.mjs';
import { launch } from './lib/cdp.mjs';

const here = dirname(fileURLToPath(import.meta.url));
const root = designRoot();
const SRC = designSrc(root);
const { commit } = designCommit(root);
const tokens = await import(pathToFileURL(resolve(SRC, 'core/tokens.ts')).href);
const sheet = f => readFileSync(resolve(SRC, f), 'utf8');
const DT = 1000 / 120;
const PORT = Number(process.env.FIDELITY_PORT || 5198);
const BASE = `http://127.0.0.1:${PORT}`;
const sleep = ms => new Promise(r => setTimeout(r, ms));
const out = resolve(REPO, 'tools/fidelity/fixtures/controls');
mkdirSync(out, { recursive: true });
const round = a => a.map(v => Number(Number(v).toFixed(5)));
const save = (name, data) => {
  const frames = Object.fromEntries(Object.entries(data.frames).map(([k, v]) => [k, round(v)]));
  writeFileSync(resolve(out, `${name}.json`), `${JSON.stringify({ ...data, source: `${data.source} @${commit}`, dt: DT, frames })}\n`);
  console.log(`controls-curves: ${name} (${Object.values(frames)[0].length} frames)`);
};

// ── the design served with the fixed clock (as run.sh) ─────────────────────────────────────
const vite = spawn('npx', ['vite', '--config', resolve(here, 'vite.config.mjs'), '--port', String(PORT), '--strictPort'], {
  cwd: root, env: { ...process.env, WUBBLE_DESIGN_ROOT: root, FIDELITY_PORT: String(PORT) }, stdio: 'ignore', detached: true,
});
const stopVite = () => { try { process.kill(-vite.pid); } catch {} };
process.on('exit', stopVite);
for (let i = 0; ; i++) {
  try { if ((await fetch(`${BASE}/__parity/manifest.json`)).ok) break; } catch {}
  if (i > 120) { stopVite(); throw new Error('controls-curves: vite did not come up'); }
  await sleep(500);
}

const page = await launch({ port: 9334, width: 1360, height: 3600 });
// Node's WebSocket does not hold the event loop open while a slow page load is awaited
const alive = setInterval(() => {}, 1000);
await page.send('Page.enable');
const mouse = (type, x, y, extra = {}) => page.send('Input.dispatchMouseEvent', { type, x, y, button: 'left', clickCount: 1, ...extra });
const KEYS = { End: ['End', 35], ArrowRight: ['ArrowRight', 39], ArrowLeft: ['ArrowLeft', 37], ',': ['Comma', 188] };
const key = async (k, modifiers = 0) => {
  const [code, vk] = KEYS[k];
  await page.send('Input.dispatchKeyEvent', { type: 'keyDown', key: k, code, windowsVirtualKeyCode: vk, modifiers });
  await page.send('Input.dispatchKeyEvent', { type: 'keyUp', key: k, code, windowsVirtualKeyCode: vk, modifiers });
};
const step = () => page.evaluate(`window.__wb.step(${DT})`);
const open = async (url, ready) => {
  await page.send('Emulation.setDeviceMetricsOverride', { width: 1360, height: 3600, deviceScaleFactor: 1, mobile: false });
  await page.send('Page.navigate', { url: BASE + url });
  for (let i = 0; i < 200; i++) {
    await sleep(100);
    try { if (await page.evaluate(`!!document.querySelector(${JSON.stringify(ready)}) && document.fonts.status === 'loaded'`)) return; } catch {}
  }
  throw new Error(`controls-curves: ${url} has no ${ready}`);
};
// settle the page's own motion on its clock
const settle = async ms => { for (let i = 0; i < ms / DT; i++) await step(); };

try {
  // ── ElasticSlider: the touch page's volume slider (value 64 of 0..100) ─────────────────────
  await open('/wubble/pages/_harness.html?page=touch', '.tp-sl');
  await settle(600);
  const SL = `(() => {
    const t = document.querySelector('.tp-sl'), k = t.querySelector('.tp-sl-knob'), b = t.querySelector('.tp-sl-bub');
    const r = t.getBoundingClientRect();
    const m = /translate\\(([-\\d.e]+)px,\\s*-50%\\) translateX\\(-50%\\) scale\\(([-\\d.e]+),\\s*([-\\d.e]+)\\)/.exec(k.style.transform);
    const n = /translateX\\(([-\\d.e]+)px\\) translateX\\(-50%\\) rotate\\(([-\\d.e]+)deg\\)/.exec(b.style.transform);
    return { left: r.x, top: r.y, w: r.width, h: r.height, x: m ? +m[1] : NaN, sx: m ? +m[2] : 1, sy: m ? +m[3] : 1,
      origin: k.style.transformOrigin.startsWith('70') ? 0.7 : 0.3, bx: n ? +n[1] : NaN, lean: n ? +n[2] : 0,
      value: Number(t.getAttribute('aria-valuenow')) };
  })()`;
  const sl = async (name, source, script, n) => {
    await open('/wubble/pages/_harness.html?page=touch', '.tp-sl');
    await settle(600);
    const g = await page.evaluate(SL);
    const y = g.top + g.h / 2;
    const frames = { x: [], sx: [], sy: [], origin: [], bx: [], lean: [], value: [] };
    for (let f = 0; f < n; f++) {
      for (const s of script.filter(s => s.frame === f)) {
        if ('press' in s) { await mouse('mouseMoved', g.left + s.press, y, { button: 'none' }); await mouse('mousePressed', g.left + s.press, y); }
        if ('move' in s) await mouse('mouseMoved', g.left + s.move, y);
        if (s.release) await mouse('mouseReleased', g.left + (s.at ?? 0), y);
        if (s.key) { await page.evaluate(`document.querySelector('.tp-sl').focus()`); await key(s.key); }
      }
      await step();
      const v = await page.evaluate(SL);
      for (const k of Object.keys(frames)) frames[k].push(v[k]);
    }
    save(name, { source, w: g.w, start: g.value, script, frames });
  };
  {
    const g = await page.evaluate(SL);
    const w = g.w;
    // a press away from the knob (catch-up), held still, dragged far left past the end (band),
    // let go there
    await sl('slider', 'pages/touch.tsx ElasticSlider (VolumeSlider), pointer', [
      { frame: 0, press: 0.9 * w },
      { frame: 40, move: 0.3 * w },
      { frame: 41, move: -60 },
      { frame: 70, release: true, at: -60 },
    ], 160);
    await sl('slider-key', 'pages/touch.tsx ElasticSlider (VolumeSlider), End key', [{ frame: 0, key: 'End' }], 120);
  }

  // ── Segmented: the touch page's Space switcher ─────────────────────────────────────────────
  {
    await open('/wubble/pages/_harness.html?page=touch', '.tp-seg');
    await settle(600);
    const SEG = `(() => {
      const row = document.querySelector('.tp-seg'), ind = row.querySelector('.tp-seg-ind'), r = row.getBoundingClientRect();
      const m = /translateX\\(([-\\d.e]+)px\\) scale\\(([-\\d.e]+),\\s*([-\\d.e]+)\\)/.exec(ind.style.transform);
      const w = parseFloat(ind.style.width), o = parseFloat(ind.style.transformOrigin);
      const x = m ? +m[1] : NaN, ax = m ? +m[2] : 1, sy = m ? +m[3] : 1;
      const segs = [...row.querySelectorAll(':scope > button')].map(b => [b.offsetLeft, b.offsetWidth]);
      return { left: r.x, top: r.y, h: r.height, segs, sel: segs.findIndex((_, i) => row.querySelectorAll(':scope > button')[i].getAttribute('aria-selected') === 'true'),
        l: x + o - o * ax, width: w * ax, sy };
    })()`;
    const g = await page.evaluate(SEG);
    const last = g.segs[g.segs.length - 1];
    const px = last[0] + last[1] / 2;
    const script = [{ frame: 0, press: px }, { frame: 1, release: true, at: px }];
    const frames = { left: [], width: [], sy: [] };
    for (let f = 0; f < 120; f++) {
      for (const s of script.filter(s => s.frame === f)) {
        if ('press' in s) { await mouse('mouseMoved', g.left + s.press, g.top + g.h / 2, { button: 'none' }); await mouse('mousePressed', g.left + s.press, g.top + g.h / 2); }
        if (s.release) await mouse('mouseReleased', g.left + s.at, g.top + g.h / 2);
      }
      await step();
      const v = await page.evaluate(SEG);
      frames.left.push(v.l); frames.width.push(v.width); frames.sy.push(v.sy);
    }
    save('segmented', { source: 'pages/touch-kit.tsx Segmented (the Space switcher), a press on the last segment', segs: g.segs, start: g.sel, script, frames });
  }

  // ── the settings swatches' ring: ArrowRight twice from the first ───────────────────────────
  {
    await open('/wubble/', '.wb-window');
    await settle(2000);
    await key(',', 2);
    for (let i = 0; i < 50 && !(await page.evaluate(`!!document.querySelector('.st-swatches')`)); i++) { await sleep(200); await step(); }
    await settle(1500);
    const RING = `(() => {
      const r = document.querySelector('.st-sw-ring');
      const m = /translate3d\\(([-\\d.e]+)px,\\s*0(?:px)?,\\s*0(?:px)?\\)(?: scale\\(([-\\d.e]+),\\s*([-\\d.e]+)\\))?/.exec(r.style.transform);
      const row = r.parentElement, sel = [...row.querySelectorAll('button')].findIndex(b => b.getAttribute('aria-checked') === 'true');
      return { x: m ? +m[1] : NaN, sx: m && m[2] ? +m[2] : 1, sy: m && m[3] ? +m[3] : 1, origin: r.style.transformOrigin.startsWith('30') ? 0.3 : 0.7, sel, n: row.querySelectorAll('button').length };
    })()`;
    const g = await page.evaluate(RING);
    await page.evaluate(`document.querySelectorAll('.st-swatches')[0].querySelectorAll('button')[${g.sel}].focus()`);
    const script = [{ frame: 0, key: 'ArrowRight' }, { frame: 6, key: 'ArrowRight' }];
    const frames = { x: [], sx: [], sy: [], origin: [] };
    for (let f = 0; f < 120; f++) {
      for (const s of script.filter(s => s.frame === f)) await key(s.key);
      await step();
      const v = await page.evaluate(RING);
      for (const k of Object.keys(frames)) frames[k].push(v[k]);
    }
    save('swatches', { source: 'pages/settings.tsx Swatches (the first Space colour), ArrowRight twice', start: g.sel, n: g.n, script, frames });
  }

  // ── the CSS tweens: one probe element per rule ─────────────────────────────────────────────
  await page.send('Page.navigate', { url: 'about:blank' });
  await sleep(300);
  const css = ['core/wubble.css', 'browser/browser.css', 'pages/touch.css', 'pages/settings.css'].map(sheet).join('\n');
  const vars = `${tokens.tokenCss()};--wb-ease-in-out:var(--wb-ease-inOut);--tp-accent:#7CC8FF`;
  // name, markup, `#p` the element read, how it starts (`hover`: the pointer moves onto `#h`), ms
  const PROBES = [
    { name: 'pill-hover-in', html: '<span class="wb-pill"><i id="p"></i></span><div id="h" class="wb-grab"></div>', hover: true, ms: 300, source: 'browser/browser.css .wb-pill:has(+ .wb-grab:hover) i' },
    { name: 'pill-hover-out', html: '<span class="wb-pill"><i id="p"></i></span><div id="h" class="wb-grab"></div>', hover: true, unhover: true, ms: 200, source: 'browser/browser.css .wb-pill i' },
    { name: 'pill-held-in', html: '<span id="s" class="wb-pill"><i id="p"></i></span>', start: 's.classList.add("wb-pill-held")', ms: 350, source: 'browser/browser.css .wb-pill-held i' },
    { name: 'pill-held-out', html: '<span id="s" class="wb-pill wb-pill-held"><i id="p"></i></span>', start: 's.classList.remove("wb-pill-held")', ms: 200, source: 'browser/browser.css .wb-pill i (from held)' },
    { name: 'thumb-on', html: '<div id="p" class="wb-thumb"></div>', start: 'p.classList.add("wb-thumb-on")', ms: 150, source: 'browser/browser.css .wb-thumb.wb-thumb-on' },
    { name: 'thumb-off', html: '<div id="p" class="wb-thumb wb-thumb-on"></div>', start: 'p.classList.remove("wb-thumb-on")', ms: 320, source: 'browser/browser.css .wb-thumb' },
    { name: 'thumb-hover', html: '<div id="p" class="wb-thumb wb-thumb-on"></div>', hoverSelf: true, ms: 180, source: 'browser/browser.css .wb-thumb:hover' },
    { name: 'bubble-in', html: '<div id="s" class="tp-sl-wrap"><div class="tp-sl"><span class="tp-sl-bub"><b id="p">64</b></span></div></div>', start: 's.classList.add("held")', ms: 340, source: 'pages/touch.css .tp-sl-wrap.held .tp-sl-bub b' },
    { name: 'bubble-out', html: '<div id="s" class="tp-sl-wrap held"><div class="tp-sl"><span class="tp-sl-bub"><b id="p">64</b></span></div></div>', start: 's.classList.remove("held")', ms: 180, source: 'pages/touch.css .tp-sl-bub b' },
    { name: 'label-out', html: '<div id="s" class="tp-sl-wrap"><div class="tp-sl-label"><output id="p">64</output></div></div>', start: 's.classList.add("held")', ms: 180, source: 'pages/touch.css .tp-sl-wrap.held .tp-sl-label output' },
    { name: 'label-in', html: '<div id="s" class="tp-sl-wrap held"><div class="tp-sl-label"><output id="p">64</output></div></div>', start: 's.classList.remove("held")', ms: 220, source: 'pages/touch.css .tp-sl-label output' },
  ];
  for (const p of PROBES) {
    await page.evaluate(`(() => {
      document.documentElement.innerHTML = ${JSON.stringify(`<head><style>${css}\n.wb { ${vars} } .wb-pill, .wb-grab, .wb-thumb { position: absolute; left: 40px; top: 40px; } .wb-grab { left: 30px; height: 60px; } .wb-thumb { height: 40px; } .tp-sl-wrap { position: absolute; left: 40px; top: 80px; }</style></head><body style="margin:0"><div class="wb" style="position:relative;width:400px;height:300px">${p.html}</div></body>`)};
      getComputedStyle(document.getElementById('p')).opacity;
    })()`);
    const centre = await page.evaluate(`(() => { const b = (document.getElementById(${JSON.stringify(p.hoverSelf ? 'p' : 'h')}) ?? document.getElementById('p')).getBoundingClientRect(); return [b.x + b.width / 2, b.y + b.height / 2]; })()`);
    if (p.unhover) {
      await mouse('mouseMoved', centre[0], centre[1], { button: 'none' });
      await page.evaluate(`(() => { const p = document.getElementById('p'); getComputedStyle(p).opacity; for (const a of document.getAnimations()) a.finish(); getComputedStyle(p).opacity; })()`);
      await mouse('mouseMoved', 380, 280, { button: 'none' });
    } else if (p.hover || p.hoverSelf) await mouse('mouseMoved', centre[0], centre[1], { button: 'none' });
    const frames = await page.evaluate(`(() => {
      const p = document.getElementById('p'), s = document.getElementById('s');
      ${p.start ?? ''};
      const s0 = getComputedStyle(p); s0.opacity; s0.scale; s0.width; s0.transform;
      const anims = () => document.getAnimations();
      for (const a of anims()) a.pause();
      const frames = { opacity: [], scale: [], ty: [], blur: [], width: [], sky: [] };
      const n = Math.round(${p.ms} / ${DT});
      for (let i = 0; i <= n + 1; i++) {
        for (const a of anims()) { a.pause(); a.currentTime = i * ${DT}; }
        const st = getComputedStyle(p);
        const m = new DOMMatrixReadOnly(st.transform === 'none' ? undefined : st.transform);
        frames.opacity.push(Number(st.opacity));
        frames.scale.push(st.scale === 'none' ? Math.hypot(m.m11, m.m12) : Number(st.scale.split(' ')[0]));
        frames.ty.push(m.m42);
        const b = /blur\\(([\\d.]+)px\\)/.exec(st.filter);
        frames.blur.push(b ? Number(b[1]) : 0);
        frames.width.push(parseFloat(st.width));
        // how far the background is from mist (0) to sky (1), by its red channel
        const c = /rgba?\\((\\d+), (\\d+), (\\d+)/.exec(st.backgroundColor);
        frames.sky.push(c ? (Number(c[1]) - 0xA3) / (0x7C - 0xA3) : 0);
      }
      return frames;
    })()`);
    save(p.name, { source: p.source, ms: p.ms, frames });
  }
} finally {
  clearInterval(alive);
  page.close();
  stopVite();
}
