#!/usr/bin/env node
// Web side of the harness: shoots every scene of scenes.json from the design served by the
// harness's vite (vite.config.mjs), headless, on the fixed clock.
//
//   node tools/fidelity/capture.mjs --out DIR [--dpr 1,2] [--only a,b] [--base http://127.0.0.1:5199]
//
// Per scene and DPR: DIR/<dpr>x/<name>.png (the window, or the scene's `frame` element), and
// DIR/<dpr>x/<name>.json (the frame's rect in the viewport, each crop's rect relative to it, in
// CSS px). A scene with `frames` is shot as DIR/<dpr>x/<name>/fNNN.png, one per 1/120 s.
//
// Time: the page's clock is the harness's (`__wb.step`), and every CSS animation and transition
// is paused and set to the same clock each step (Web Animations API), so a frame sequence is
// exact however slow SwiftShader renders.
import { mkdirSync, writeFileSync, readFileSync } from 'node:fs';
import { resolve, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { launch } from './lib/cdp.mjs';

const here = dirname(fileURLToPath(import.meta.url));
const args = process.argv.slice(2);
const opt = (n, d) => (args.includes(n) ? args[args.indexOf(n) + 1] : d);
const out = resolve(opt('--out', 'fidelity-out/web'));
const dprs = opt('--dpr', '1,2').split(',').map(Number);
const only = opt('--only', '') ? opt('--only').split(',') : null;
const base = opt('--base', 'http://127.0.0.1:5199');
const spec = JSON.parse(readFileSync(resolve(here, opt('--scenes', 'scenes.json')), 'utf8'));
const DT = 1000 / 120;
const sleep = ms => new Promise(r => setTimeout(r, ms));

// The in-page stepper: the clock, then every animation set to it.
const STEPPER = `window.__wbAdvance = (ms) => {
  const n = Math.max(1, Math.round(ms / ${DT}));
  for (let i = 0; i < n; i++) {
    window.__wb.step(${DT});
    for (const a of document.getAnimations()) {
      if (a.__t0 === undefined) a.__t0 = window.__wb.now() - (a.currentTime ?? 0);
      if (a.playState !== 'paused') a.pause();
      a.currentTime = window.__wb.now() - a.__t0;
    }
  }
  return window.__wb.now();
};`;

const KEYS = { ',': ['Comma', 188], '\\': ['Backslash', 220], Escape: ['Escape', 27], Enter: ['Enter', 13], ArrowDown: ['ArrowDown', 40], ArrowUp: ['ArrowUp', 38] };
const keyInfo = k => KEYS[k] ?? (k.length === 1 ? [`Key${k.toUpperCase()}`, k.toUpperCase().charCodeAt(0)] : [k, 0]);

async function run(page, scene, dpr) {
  // A 2x window must fit the screen wubble's window opens on (2560x1440 on the Linux box), so
  // scenes without their own viewport use `viewportByDpr` there.
  const [vw, vh] = scene.viewport ?? spec.viewportByDpr?.[dpr] ?? spec.viewport;
  await page.send('Emulation.setDeviceMetricsOverride', { width: vw, height: vh, deviceScaleFactor: dpr, mobile: false });
  await page.send('Page.navigate', { url: base + scene.url });
  // real time for the page's modules to load; the page clock does not move meanwhile
  const ready = scene.ready ?? spec.frame;
  for (let i = 0; i < 200; i++) {
    await sleep(100);
    try { if (await page.evaluate(`!!document.querySelector(${JSON.stringify(ready)}) && document.fonts.status === 'loaded'`)) break; } catch {}
  }
  await page.evaluate(STEPPER);
  const advance = ms => page.evaluate(`window.__wbAdvance(${ms})`);
  const center = async sel => {
    const r = await page.evaluate(`(() => { const e = document.querySelector(${JSON.stringify(sel)}); if (!e) return null; e.scrollIntoView?.({ block: 'nearest' }); const b = e.getBoundingClientRect(); return [b.x + b.width / 2, b.y + b.height / 2]; })()`);
    if (!r) throw new Error(`${scene.name}: no ${sel}`);
    return r;
  };
  const mouse = (type, [x, y], extra = {}) => page.send('Input.dispatchMouseEvent', { type, x, y, button: 'left', clickCount: 1, ...extra });
  if (scene.frozen ?? spec.frozen) await page.evaluate('globalThis.__wbFrozen = false');
  await advance(scene.settle ?? spec.settle);
  if (scene.frozen ?? spec.frozen) await page.evaluate('globalThis.__wbFrozen = true');
  for (const step of scene.steps ?? []) {
    if (step.key) {
      const [code, vk] = keyInfo(step.key);
      const modifiers = (step.ctrl ? 2 : 0) | (step.shift ? 8 : 0);
      await page.send('Input.dispatchKeyEvent', { type: 'keyDown', key: step.key, code, windowsVirtualKeyCode: vk, modifiers });
      await page.send('Input.dispatchKeyEvent', { type: 'keyUp', key: step.key, code, windowsVirtualKeyCode: vk, modifiers });
    } else if (step.type) await page.send('Input.insertText', { text: step.type });
    else if (step.hover) await mouse('mouseMoved', await center(step.hover), { button: 'none' });
    else if (step.click) { const p = await center(step.click); await mouse('mouseMoved', p, { button: 'none' }); await mouse('mousePressed', p); await mouse('mouseReleased', p); }
    else if (step.drag) {
      const p = await center(step.drag.from);
      await mouse('mousePressed', p);
      const to = [p[0] + step.drag.by[0], p[1] + step.drag.by[1]];
      for (let i = 1; i <= 12; i++) { await mouse('mouseMoved', [p[0] + (to[0] - p[0]) * i / 12, p[1] + (to[1] - p[1]) * i / 12]); await advance(DT); }
      await mouse('mouseReleased', to);
    } else if (step.wheel) {
      const p = await center(step.wheel.over);
      await page.send('Input.dispatchMouseEvent', { type: 'mouseWheel', x: p[0], y: p[1], deltaX: 0, deltaY: step.wheel.y });
    } else if (step.advance) await advance(step.advance);
    else if (step.wait) await sleep(step.wait);
    else if (step.until) {
      // real-time turns (a throwaway paint, a clock frame) until the element exists, up to 5 s
      for (let i = 0; i < 25 && !(await page.evaluate(`!!document.querySelector(${JSON.stringify(step.until)})`)); i++) {
        await page.send('Page.captureScreenshot', { format: 'png', clip: { x: 0, y: 0, width: 1, height: 1, scale: 1 } });
        await sleep(200);
        await advance(DT);
      }
    }
    else if (step.eval) await page.evaluate(step.eval);
    else throw new Error(`${scene.name}: unknown step ${JSON.stringify(step)}`);
  }
  const frame = scene.frame ?? spec.frame;
  const rects = async () => page.evaluate(`(() => {
    const f = document.querySelector(${JSON.stringify(frame)});
    if (!f) return null;
    const F = f.getBoundingClientRect();
    const crops = {};
    for (const c of ${JSON.stringify(scene.crops ?? [])}) {
      if (!c.web) { crops[c.name] = [0, 0, F.width, F.height]; continue; }
      const e = document.querySelector(c.web);
      if (!e) { crops[c.name] = null; continue; }
      const b = e.getBoundingClientRect();
      crops[c.name] = [b.x - F.x, b.y - F.y, b.width, b.height];
    }
    return { frame: [F.x, F.y, F.width, F.height], crops };
  })()`);
  const dir = resolve(out, `${dpr}x`);
  mkdirSync(dir, { recursive: true });
  const shoot = async (path, meta) => {
    const [x, y, width, height] = meta.frame;
    const r = await page.send('Page.captureScreenshot', { format: 'png', clip: { x, y, width, height, scale: 1 }, captureBeyondViewport: false });
    writeFileSync(path, Buffer.from(r.data, 'base64'));
  };
  if (!scene.frames) await advance(scene.after ?? spec.after);
  // Lazily loaded pages (settings) render on a real-time turn: wait, in real time, until every
  // crop's element exists (up to 5 s), stepping the clock a frame each turn.
  const wanted = JSON.stringify((scene.crops ?? []).filter(c => c.web).map(c => c.web));
  let waited = false;
  for (let i = 0; i < 25 && !(await page.evaluate(`${wanted}.every(s => document.querySelector(s))`)); i++) {
    await page.send('Page.captureScreenshot', { format: 'png', clip: { x: 0, y: 0, width: 1, height: 1, scale: 1 } });
    await sleep(200);
    await advance(DT);
    waited = true;
  }
  if (waited && !scene.frames) await advance(scene.after ?? spec.after);
  const meta = await rects();
  if (!meta) throw new Error(`${scene.name}: no ${frame}`);
  writeFileSync(resolve(dir, `${scene.name}.json`), JSON.stringify({ ...meta, dpr, clock: await page.evaluate('window.__wb.now()') }, null, 1));
  if (scene.frames) {
    mkdirSync(resolve(dir, scene.name), { recursive: true });
    for (let k = 0; k < scene.frames; k++) {
      await shoot(resolve(dir, scene.name, `f${String(k).padStart(3, '0')}.png`), meta);
      await advance(DT);
    }
  } else await shoot(resolve(dir, `${scene.name}.png`), meta);
}

const page = await launch({ width: spec.viewport[0], height: spec.viewport[1] });
await page.send('Page.enable');
let failed = 0;
try {
  for (const dpr of dprs) {
    for (const scene of spec.scenes) {
      if (only && !only.includes(scene.name)) continue;
      const t = Date.now();
      try {
        await run(page, scene, dpr);
        console.log(`capture: ${dpr}x ${scene.name} (${((Date.now() - t) / 1000).toFixed(1)} s)`);
      } catch (e) {
        failed++;
        console.error(`capture: ${dpr}x ${scene.name} failed: ${e.message}`);
      }
    }
  }
} finally {
  page.close();
}
process.exit(failed ? 1 : 0);
