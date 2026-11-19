#!/usr/bin/env node
// Web side of the harness: shoots every scene of scenes.json (and the specimen scenes its
// `specimens` table expands to) from the design served by the harness's vite (vite.config.mjs),
// headless, on the fixed clock, in parallel browsers, through the web cache.
//
//   node tools/fidelity/capture.mjs --out DIR [--dpr 1,2] [--only a,spec-toggle-*] [--jobs N]
//       [--base http://127.0.0.1:5199] [--cache DIR | --no-cache] [--prune]
//       [--check-selectors] [--probe-cache]
//
// Per scene and DPR: DIR/<dpr>x/<name>.png (the window, the scene's `frame` element, or the union
// of its `clip` selectors plus `pad`) and DIR/<dpr>x/<name>.json (the shot's rect in the viewport,
// each crop's rect and mask rects relative to it, CSS px). A scene with `frames` is shot as
// DIR/<dpr>x/<name>/fNNN.png, one per 1/120 s, plus <name>/curve.json when it has a `track`. A
// specimen still also writes <name>.rects.json; DIR/specimens-index.json lists every specimen.
//
// Time: the page's clock is the harness's (`__wb.step`), and every CSS animation and transition
// is paused and set to the same clock each step (Web Animations API), so a frame sequence is exact
// however slow SwiftShader renders, and scenes in separate browsers are independent.
//
// Cache (lib/cache.mjs): a task whose key (design state, harness files, scene, DPR, gl) is in the
// cache is copied from it, not shot. --probe-cache exits 0 when every task would hit (run.sh then
// skips the vite server); --prune drops keys no task and no recent run uses.
import { mkdirSync, writeFileSync, readFileSync, existsSync, cpSync, renameSync, rmSync } from 'node:fs';
import { resolve, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { homedir } from 'node:os';
import { launch, chromiumVersion } from './lib/cdp.mjs';
import { withSpecimens, sourceIndex, resolveSource } from './lib/specimen.mjs';
import { sampler, rectsFile, curveFile } from './lib/curve.mjs';
import * as cache from './lib/cache.mjs';

const here = dirname(fileURLToPath(import.meta.url));
const args = process.argv.slice(2);
const opt = (n, d) => (args.includes(n) ? args[args.indexOf(n) + 1] : d);
const flag = n => args.includes(n);
const out = resolve(opt('--out', 'fidelity-out/web'));
const dprs = opt('--dpr', '1,2').split(',').map(Number);
const only = opt('--only', '') ? opt('--only').split(',').map(p => new RegExp(`^${p.replace(/[.+?^${}()|[\]\\]/g, '\\$&').replace(/\*/g, '.*')}$`)) : null;
const base = opt('--base', 'http://127.0.0.1:5199');
const jobs = Math.max(1, Number(opt('--jobs', 2)));
const design = process.env.WUBBLE_DESIGN || resolve(homedir(), 'Dev/wubble-wt/ref/the-wallpaper-project');
const cacheDir = flag('--no-cache') ? null : resolve(opt('--cache', resolve(homedir(), 'Dev/wubble-wt/notes/fidelity/web-cache')));
const checkOnly = flag('--check-selectors');
const scenesPath = resolve(here, opt('--scenes', 'scenes.json'));
const spec = withSpecimens(JSON.parse(readFileSync(scenesPath, 'utf8')), f => JSON.parse(readFileSync(resolve(dirname(scenesPath), f), 'utf8')));
const DT = 1000 / 120;
const sleep = ms => new Promise(r => setTimeout(r, ms));

/** An error of the scene itself (a missing selector): never retried. */
class SceneError extends Error {}

// The in-page stepper: the clock, then every animation set to it. An animation starts on the
// clock when the stepper first sees it (a transition an input event started, an entrance at
// load), never at its real-time progress, and every shot is preceded by a sync, so a transition
// started by the last input is drawn at the clock's time, not at however much real time passed
// (that made L0's first frames of a sequence differ run to run).
const STEPPER = `window.__wbSync = () => {
  for (const a of document.getAnimations()) {
    if (a.__t0 === undefined) a.__t0 = window.__wb.now();
    if (a.playState !== 'paused') a.pause();
    a.currentTime = window.__wb.now() - a.__t0;
  }
};
window.__wbAdvance = (ms) => {
  const n = Math.max(1, Math.round(ms / ${DT}));
  for (let i = 0; i < n; i++) { window.__wb.step(${DT}); window.__wbSync(); }
  return window.__wb.now();
};`;

// In-page: the first element matching `sel` that lies inside rect F (else the first match), so a
// crop of a repeated part (a settings row) takes the one in the shot.
const PICK = `(sel, F) => { const all = [...document.querySelectorAll(sel)];
  return all.find(e => { const b = e.getBoundingClientRect(); return b.width > 0 && b.height > 0 && b.x >= F.x - 0.5 && b.y >= F.y - 0.5 && b.right <= F.x + F.width + 0.5 && b.bottom <= F.y + F.height + 0.5; }) ?? all[0] ?? null; }`;

const KEYS = { ',': ['Comma', 188], '\\': ['Backslash', 220], ' ': ['Space', 32], Escape: ['Escape', 27], Enter: ['Enter', 13], Tab: ['Tab', 9], ArrowDown: ['ArrowDown', 40], ArrowUp: ['ArrowUp', 38], ArrowLeft: ['ArrowLeft', 37], ArrowRight: ['ArrowRight', 39], Home: ['Home', 36], End: ['End', 35] };
const keyInfo = k => KEYS[k] ?? (k.length === 1 ? [`Key${k.toUpperCase()}`, k.toUpperCase().charCodeAt(0)] : [k, 0]);

/** Runs one scene at one DPR in `page`, writing its files into `dir` (or only checking selectors). */
async function run(page, scene, dpr, dir) {
  // A 2x window must fit the screen wubble's window opens on (2560x1440 on the Linux box), so
  // scenes without their own viewport use `viewportByDpr` there.
  const [vw, vh] = scene.viewport ?? spec.viewportByDpr?.[dpr] ?? spec.viewport;
  await page.send('Emulation.setDeviceMetricsOverride', { width: vw, height: vh, deviceScaleFactor: dpr, mobile: false });
  await page.send('Page.navigate', { url: base + scene.url });
  // real time for the page's modules to load; the page clock does not move meanwhile
  const ready = scene.ready ?? spec.frame;
  let loaded = false;
  for (let i = 0; i < 300 && !loaded; i++) {
    await sleep(100);
    try { loaded = await page.evaluate(`!!document.querySelector(${JSON.stringify(ready)}) && document.fonts.status === 'loaded'`); } catch {}
  }
  if (!loaded) throw new Error(`${scene.name}: page never showed ${ready}`);
  await page.evaluate(STEPPER);
  const advance = ms => page.evaluate(`window.__wbAdvance(${ms})`);
  const exists = sel => page.evaluate(`!!document.querySelector(${JSON.stringify(sel)})`);
  // real-time turns (a throwaway paint, a clock frame) until `test` holds, up to 20 s: a poll
  // for lazily loaded modules, not a retry
  const until = async test => {
    for (let i = 0; i < 100; i++) {
      if (await test()) return true;
      await page.send('Page.captureScreenshot', { format: 'png', clip: { x: 0, y: 0, width: 1, height: 1, scale: 1 } });
      await sleep(200);
      await advance(DT);
    }
    return false;
  };
  const center = async sel => {
    const r = await page.evaluate(`(() => { const e = document.querySelector(${JSON.stringify(sel)}); if (!e) return null; e.scrollIntoView?.({ block: 'nearest' }); const b = e.getBoundingClientRect(); return [b.x + b.width / 2, b.y + b.height / 2, b.width, b.height]; })()`);
    if (!r) throw new SceneError(`${scene.name}: no ${sel}`);
    return r;
  };
  // the pointer: where it is and whether the left button is down
  const ptr = { x: 0, y: 0, down: false };
  const mouse = (type, [x, y]) => {
    ptr.x = x; ptr.y = y;
    if (type === 'mousePressed') ptr.down = true;
    if (type === 'mouseReleased') ptr.down = false;
    const button = type === 'mouseMoved' && !ptr.down ? 'none' : 'left';
    return page.send('Input.dispatchMouseEvent', { type, x, y, button, buttons: ptr.down ? 1 : 0, clickCount: 1 });
  };
  // a pointer path to `to` in `n` moves, one clock frame each
  const path = async (to, n = 12) => {
    const [x0, y0] = [ptr.x, ptr.y];
    for (let i = 1; i <= n; i++) { await mouse('mouseMoved', [x0 + (to[0] - x0) * i / n, y0 + (to[1] - y0) * i / n]); await advance(DT); }
  };
  const key = async (k, step = {}) => {
    const [code, vk] = keyInfo(k);
    const modifiers = (step.alt ? 1 : 0) | (step.ctrl ? 2 : 0) | (step.shift ? 8 : 0);
    await page.send('Input.dispatchKeyEvent', { type: 'keyDown', key: k, code, windowsVirtualKeyCode: vk, modifiers });
    await page.send('Input.dispatchKeyEvent', { type: 'keyUp', key: k, code, windowsVirtualKeyCode: vk, modifiers });
  };
  // a target point: a selector's centre plus `dx`/`dy`, or `fx` (a fraction of its width from its left)
  const point = async t => {
    const [x, y, w] = await center(t.to);
    return [(t.fx !== undefined ? x - w / 2 + w * t.fx : x) + (t.dx ?? 0), y + (t.dy ?? 0)];
  };
  if (scene.frozen ?? spec.frozen) await page.evaluate('globalThis.__wbFrozen = false');
  await advance(scene.settle ?? spec.settle);
  if (scene.frozen ?? spec.frozen) await page.evaluate('globalThis.__wbFrozen = true');
  for (const step of scene.steps ?? []) {
    if (step.key) await key(step.key, step);
    else if (step.type) await page.send('Input.insertText', { text: step.type });
    else if (step.hover) await mouse('mouseMoved', await center(step.hover));
    else if (step.click) { const p = await center(step.click); await mouse('mouseMoved', p); await mouse('mousePressed', p); await mouse('mouseReleased', p); }
    else if (step.press) { const p = await center(step.press); await mouse('mouseMoved', p); await mouse('mousePressed', p); await advance(DT); }
    else if (step.release) { await mouse('mouseReleased', [ptr.x, ptr.y]); await advance(DT); }
    else if (step.move) await path(step.move.to ? await point(step.move) : [ptr.x + (step.move.dx ?? 0), ptr.y + (step.move.dy ?? 0)], step.move.steps ?? 6);
    else if (step.drag) {
      const p = await center(step.drag.from);
      await mouse('mouseMoved', p);
      await mouse('mousePressed', p);
      await path(step.drag.to ? await point(step.drag.to) : [p[0] + step.drag.by[0], p[1] + step.drag.by[1]], step.drag.steps ?? 12);
      if (!step.drag.hold) await mouse('mouseReleased', [ptr.x, ptr.y]);
    } else if (step.wheel) {
      const p = await center(step.wheel.over);
      await page.send('Input.dispatchMouseEvent', { type: 'mouseWheel', x: p[0], y: p[1], deltaX: 0, deltaY: step.wheel.y });
    } else if (step.focus) {
      // keyboard modality first, so the focus matches :focus-visible as a Tab focus would
      await key('Tab');
      const focus = `(() => { const e = document.querySelector(${JSON.stringify(step.focus)}); if (!e) return null; e.focus(); return e.matches(':focus-visible'); })()`;
      let visible = await page.evaluate(focus);
      if (visible === null) throw new SceneError(`${scene.name}: no ${step.focus}`);
      if (!visible) {
        // Chromium kept mouse modality: leave by Shift+Tab and come back by Tab, a real keyboard focus
        await key('Tab', { shift: true });
        await key('Tab');
        visible = await page.evaluate(`document.activeElement === document.querySelector(${JSON.stringify(step.focus)}) && document.activeElement.matches(':focus-visible')`);
        if (!visible) visible = await page.evaluate(focus);
      }
      if (!visible) console.error(`capture: ${scene.name}: ${step.focus} is focused but not :focus-visible`);
      await advance(DT);
    } else if (step.disable) {
      if (!(await page.evaluate(`(() => { const e = document.querySelector(${JSON.stringify(step.disable)}); if (!e) return false; e.disabled = true; e.setAttribute('aria-disabled', 'true'); return true; })()`))) throw new SceneError(`${scene.name}: no ${step.disable}`);
      await advance(DT);
    } else if (step.holdFor) {
      if (!ptr.down) throw new SceneError(`${scene.name}: holdFor with no button down`);
      await advance(step.holdFor);
    } else if (step.reveal) {
      // wheel `in` until the element is inside it (the fallback for rows a search cannot isolate)
      const { el, in: box = '.wb-main', dy = 200 } = typeof step.reveal === 'string' ? { el: step.reveal } : step.reveal;
      const inside = () => page.evaluate(`(() => { const e = document.querySelector(${JSON.stringify(el)}), c = document.querySelector(${JSON.stringify(box)}); if (!e || !c) return null; const b = e.getBoundingClientRect(), r = c.getBoundingClientRect(); return b.top >= r.top && b.bottom <= r.bottom; })()`);
      for (let i = 0; i < 40 && !(await inside()); i++) {
        const p = await center(box);
        await page.send('Input.dispatchMouseEvent', { type: 'mouseWheel', x: p[0], y: p[1], deltaX: 0, deltaY: dy });
        await advance(200);
      }
      if (!(await inside())) throw new SceneError(`${scene.name}: could not reveal ${el} in ${box}`);
    } else if (step.advance) await advance(step.advance);
    else if (step.wait) await sleep(step.wait);
    else if (step.until) { if (!(await until(() => exists(step.until)))) throw new SceneError(`${scene.name}: ${step.until} never appeared`); }
    else if (step.eval) await page.evaluate(step.eval);
    else throw new SceneError(`${scene.name}: unknown step ${JSON.stringify(step)}`);
    await page.evaluate('window.__wbSync()');
  }
  const frame = scene.frame ?? spec.frame;
  const crops = scene.crops ?? [];
  if (!scene.frames) await advance(scene.after ?? spec.after);
  // Lazily loaded pages (settings) render on a real-time turn: wait, in real time, until every
  // element the shot needs exists, stepping the clock a frame each turn.
  const wanted = JSON.stringify([frame, ...(scene.clip ? [scene.clip[0]] : []), ...crops.filter(c => c.web).map(c => c.web)]);
  const all = () => page.evaluate(`${wanted}.every(s => document.querySelector(s))`);
  if (!(await all()) && (await until(all)) && !scene.frames) await advance(scene.after ?? spec.after);
  const meta = await page.evaluate(`(() => {
    const pick = ${PICK};
    const f = document.querySelector(${JSON.stringify(frame)});
    if (!f) return null;
    let F = f.getBoundingClientRect();
    const clip = ${JSON.stringify(scene.clip ?? null)};
    if (clip) {
      const bs = clip.map(s => document.querySelector(s)).filter(Boolean).map(e => e.getBoundingClientRect());
      if (!bs.length) return { absent: clip[0] };
      const pad = ${Number(scene.pad ?? 0)};
      const x0 = Math.max(0, Math.min(...bs.map(b => b.left)) - pad), y0 = Math.max(0, Math.min(...bs.map(b => b.top)) - pad);
      const x1 = Math.min(innerWidth, Math.max(...bs.map(b => b.right)) + pad), y1 = Math.min(innerHeight, Math.max(...bs.map(b => b.bottom)) + pad);
      F = new DOMRect(x0, y0, x1 - x0, y1 - y0);
    }
    const crops = {}, masks = {}, missing = [];
    for (const c of ${JSON.stringify(crops)}) {
      let C = [0, 0, F.width, F.height];
      if (c.web) {
        const e = pick(c.web, F);
        if (!e) { crops[c.name] = null; missing.push(c.web); continue; }
        const b = e.getBoundingClientRect();
        C = [b.x - F.x, b.y - F.y, b.width, b.height];
      }
      crops[c.name] = C;
      // mask rects relative to the crop: every element of a selector that meets the crop (grown by
      // \`pad\`, and stretched to \`toRight\` px from the crop's right edge), or a rect
      if (c.mask) masks[c.name] = c.mask.flatMap(m => m.rect ? [m.rect] : [...document.querySelectorAll(m.web)].map(e => e.getBoundingClientRect())
        .filter(b => b.width > 0 && b.right > F.x + C[0] && b.left < F.x + C[0] + C[2] && b.bottom > F.y + C[1] && b.top < F.y + C[1] + C[3])
        .map(b => {
          const p = m.pad ?? 0, r = [b.x - F.x - C[0] - p, b.y - F.y - C[1] - p, b.width + 2 * p, b.height + 2 * p];
          if (m.toRight !== undefined) r[2] = C[2] - m.toRight - r[0];
          return r;
        }));
    }
    return { frame: [F.x, F.y, F.width, F.height], crops, masks, missing };
  })()`);
  if (!meta) throw new SceneError(`${scene.name}: no ${frame}`);
  if (meta.absent) throw new SceneError(`${scene.name}: no ${meta.absent}`);
  const sp = scene.specimen;
  const sample = sp ? () => page.evaluate(`${sampler}(${JSON.stringify(sp.root)}, ${JSON.stringify(sp.elements)})`) : null;
  // a specimen's element may be absent in one state (a closed menu): --check-selectors flags it
  // only when it is absent in every state of its control
  if (checkOnly) return { missing: meta.missing, present: sp ? Object.fromEntries(Object.entries((await sample())?.elements ?? {}).map(([n, v]) => [n, !!v])) : {} };
  if (!Object.keys(meta.masks).length) delete meta.masks;
  delete meta.missing;
  mkdirSync(dir, { recursive: true });
  const shoot = async path => {
    await page.evaluate('window.__wbSync()');
    const [x, y, width, height] = meta.frame;
    const r = await page.send('Page.captureScreenshot', { format: 'png', clip: { x, y, width, height, scale: 1 }, captureBeyondViewport: false });
    writeFileSync(path, Buffer.from(r.data, 'base64'));
  };
  writeFileSync(resolve(dir, `${scene.name}.json`), JSON.stringify({ ...meta, dpr, clock: await page.evaluate('window.__wb.now()') }, null, 1));
  if (scene.frames) {
    mkdirSync(resolve(dir, scene.name), { recursive: true });
    const samples = [];
    for (let k = 0; k < scene.frames; k++) {
      if (sample && scene.track) samples.push(await sample());
      await shoot(resolve(dir, scene.name, `f${String(k).padStart(3, '0')}.png`));
      await advance(DT);
    }
    if (sample && scene.track) writeFileSync(resolve(dir, scene.name, 'curve.json'), JSON.stringify(curveFile(sp.id, scene.track, samples), null, 1));
  } else {
    await shoot(resolve(dir, `${scene.name}.png`));
    if (sample) writeFileSync(resolve(dir, `${scene.name}.rects.json`), JSON.stringify(rectsFile(sp.id, await sample(), dpr), null, 1));
  }
}

// ── tasks ──
const scenes = spec.scenes.filter(s => !only || only.some(r => r.test(s.name)));
const tasks = [];
for (const dpr of checkOnly ? [dprs[0]] : dprs) for (const scene of scenes) tasks.push({ scene, dpr, gl: scene.gl ?? 'swiftshader', task: `${scene.name}@${dpr}` });

let designAt = null;
if (cacheDir && !checkOnly) {
  designAt = cache.designState(design);
  const harness = cache.harnessHash(['capture.mjs', 'lib/cdp.mjs', 'lib/specimen.mjs', 'lib/curve.mjs', 'vite.config.mjs'].map(f => resolve(here, f)), chromiumVersion());
  for (const t of tasks) t.key = cache.cacheKey({ design: designAt, harness, scene: cache.sceneHash(t.scene, spec), dpr: t.dpr, gl: t.gl });
  if (flag('--prune')) console.log(`capture: pruned ${cache.prune(cacheDir)} cache entries`);
}
const entry = t => resolve(cacheDir, t.key);
const hit = t => !!t.key && existsSync(resolve(entry(t), 'meta.json'));
if (flag('--probe-cache')) {
  const miss = tasks.filter(t => !hit(t)).length;
  console.log(`capture: ${tasks.length - miss} of ${tasks.length} tasks in the cache`);
  process.exit(miss ? 1 : 0);
}
/** Copies a cache entry's files into the run (all but its meta.json). */
const restore = (t, dir) => cpSync(entry(t), dir, { recursive: true, filter: s => s !== resolve(entry(t), 'meta.json') });

const t0 = Date.now();
let failed = 0, hits = 0;
const checks = [];
// cache hits first (instant), then the slow SwiftShader scenes before the quick GPU ones
const queue = [...tasks].sort((a, b) => hit(b) - hit(a) || (b.gl === 'swiftshader') - (a.gl === 'swiftshader'));
const isCrash = e => !(e instanceof SceneError) && /browser closed|target closed|navigated or closed|websocket|session|chromium did not start|is not a function|never showed/i.test(e.message);

async function worker() {
  const browsers = new Map();
  const browser = async gl => {
    if (!browsers.has(gl)) {
      const b = await launch({ gl, width: spec.viewport[0], height: spec.viewport[1] });
      await b.send('Page.enable');
      browsers.set(gl, b);
    }
    return browsers.get(gl);
  };
  try {
    while (queue.length) {
      const t = queue.shift();
      const dir = resolve(out, `${t.dpr}x`);
      if (hit(t)) { restore(t, dir); hits++; continue; }
      const started = Date.now();
      const into = t.key ? `${entry(t)}.tmp-${process.pid}-${Math.random().toString(36).slice(2)}` : dir;
      for (let attempt = 0; ; attempt++) {
        try {
          const found = await run(await browser(t.gl), t.scene, t.dpr, into);
          if (checkOnly) { checks.push({ scene: t.scene, ...found }); break; }
          if (t.key) {
            writeFileSync(resolve(into, 'meta.json'), JSON.stringify({ task: t.task, gl: t.gl, design: designAt, time: Date.now() }));
            try { renameSync(into, entry(t)); } catch { rmSync(into, { recursive: true, force: true }); } // another run cached it first
            restore(t, dir);
          }
          console.log(`capture: ${t.dpr}x ${t.scene.name} [${t.gl}] (${((Date.now() - started) / 1000).toFixed(1)} s)`);
          break;
        } catch (e) {
          if (t.key) rmSync(into, { recursive: true, force: true });
          // a crashed or hung browser: one more try in a fresh one (not a timing retry)
          if (attempt === 0 && isCrash(e)) {
            console.error(`capture: ${t.dpr}x ${t.scene.name}: browser lost (${e.message}), once more in a fresh one`);
            browsers.get(t.gl)?.close();
            browsers.delete(t.gl);
            continue;
          }
          failed++;
          console.error(`capture: ${t.dpr}x ${t.scene.name} failed: ${e.message}`);
          break;
        }
      }
    }
  } finally {
    for (const b of browsers.values()) b.close();
  }
}

if (cacheDir) mkdirSync(cacheDir, { recursive: true });
await Promise.all(Array.from({ length: Math.min(jobs, tasks.length) }, worker));

if (checkOnly) {
  let bad = 0;
  for (const c of checks.sort((a, b) => a.scene.name.localeCompare(b.scene.name))) if (c.missing.length) { bad++; console.log(`check: ${c.scene.name}: missing ${c.missing.join(', ')}`); }
  const seen = new Map(); // control -> element -> present in some state
  for (const c of checks) {
    const sp = c.scene.specimen;
    if (!sp) continue;
    const m = seen.get(sp.control) ?? new Map();
    for (const [n, v] of Object.entries(c.present)) m.set(n, m.get(n) || v);
    seen.set(sp.control, m);
  }
  for (const [control, m] of seen) for (const [n, v] of m) if (!v) { bad++; console.log(`check: specimen ${control}: element ${n} is in no state`); }
  console.log(`check: ${checks.length} scenes run, ${bad} problems, ${failed} failed to run`);
  process.exit(bad || failed ? 1 : 0);
}

// ── the specimen index: one row per specimen state or sequence and DPR ──
const specTasks = tasks.filter(t => t.scene.specimen);
if (specTasks.length) {
  const src = sourceIndex(design);
  const rows = [];
  for (const t of specTasks) {
    const s = t.scene, sp = s.specimen;
    const metaPath = resolve(out, `${t.dpr}x`, `${s.name}.json`);
    if (!existsSync(metaPath)) { rows.push({ id: sp.id, dpr: t.dpr, scene: s.name, status: 'missing' }); continue; }
    const meta = JSON.parse(readFileSync(metaPath, 'utf8'));
    const source = resolveSource(src, sp.source);
    if (sp.source && !source) console.error(`capture: ${sp.id}: source ${sp.source} not found in the design`);
    rows.push({
      id: sp.id, control: sp.control, state: sp.state, sequence: sp.sequence, kind: sp.kind, scene: s.name, dpr: t.dpr, gl: t.gl,
      png: s.frames ? `${t.dpr}x/${s.name}/` : `${t.dpr}x/${s.name}.png`, frames: s.frames ?? null, crop: meta.frame,
      rects: s.frames ? null : `${t.dpr}x/${s.name}.rects.json`, curve: s.frames && s.track ? `${t.dpr}x/${s.name}/curve.json` : null,
      cache: t.key ? entry(t) : null, source, status: 'ok',
    });
  }
  const index = { design: designAt, written: new Date().toISOString(), note: 'png, rects and curve are relative to this file\'s directory; `cache` is the cache entry holding the same files (without the <dpr>x/ prefix); crop is the shot rect in CSS px of the viewport', specimens: rows };
  writeFileSync(resolve(out, 'specimens-index.json'), JSON.stringify(index, null, 1));
  if (cacheDir) writeFileSync(resolve(cacheDir, 'specimens-index.json'), JSON.stringify({ ...index, run: out }, null, 1));
}
if (cacheDir) {
  const used = tasks.filter(hit).map(t => ({ key: t.key, task: t.task }));
  cache.writeIndex(cacheDir, cache.recordRun(cache.readIndex(cacheDir), used, { run: out, time: Date.now(), design: designAt }));
}
console.log(`capture: ${tasks.length} tasks, ${hits} from the cache, ${failed} failed, ${jobs} jobs, ${((Date.now() - t0) / 1000).toFixed(1)} s`);
process.exit(failed ? 1 : 0);
