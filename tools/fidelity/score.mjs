#!/usr/bin/env node
// The scorer: pairs each web capture with wubble's gallery shot of the same scene, crops both to
// each of the scene's rects, and writes per pair SSIM (luma), mean and 95th-percentile CIEDE2000
// to scores.csv, and a contact sheet PNG (web | wubble | ΔE heat map, black 0, red 5, yellow 10+;
// masked regions hatched grey).
//
//   node tools/fidelity/score.mjs --web DIR/web --wubble DIR/wubble --out DIR [--dpr 1,2]
//       [--wubble-rects FILE ...]
//
// --wubble-rects (repeatable): element rects of a wubble gallery scene, `{ "scene": "<gallery
// scene>", "elements": { "<element id>": { "x", "y", "w", "h" } } }` (logical px in wubble's
// window), for derived anatomy crops (`"wubble": "@<element id>"`).
//
// Scores are reported, not asserted. The bar is SSIM >= 0.90 and mean ΔE <= 3 on gating pairs;
// pairs with `gate: false` (scene or crop) are informational.
import { readFileSync, writeFileSync, mkdirSync, existsSync, readdirSync } from 'node:fs';
import { resolve, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import * as png from './lib/png.mjs';
import { ssim, deltaE, heat, maskOf, coverage, hatch } from './lib/metrics.mjs';
import { cropRects } from './lib/geometry.mjs';
import { toCsv, gateOf, whyOf, wubbleRect, summarize } from './lib/scores.mjs';
import { withSpecimens } from './lib/specimen.mjs';

const here = dirname(fileURLToPath(import.meta.url));
const args = process.argv.slice(2);
const opt = (n, d) => (args.includes(n) ? args[args.indexOf(n) + 1] : d);
const webDir = resolve(opt('--web', 'fidelity-out/web'));
const wubDir = resolve(opt('--wubble', 'fidelity-out/wubble'));
const out = resolve(opt('--out', 'fidelity-out'));
const dprs = opt('--dpr', '1,2').split(',').map(Number);
const scenesPath = resolve(here, opt('--scenes', 'scenes.json'));
const spec = withSpecimens(JSON.parse(readFileSync(scenesPath, 'utf8')), f => JSON.parse(readFileSync(resolve(dirname(scenesPath), f), 'utf8')));
const rectFiles = args.flatMap((a, i) => (a === '--wubble-rects' ? [JSON.parse(readFileSync(resolve(args[i + 1]), 'utf8'))] : []));

/** The gallery's file for scene `name` (`NN-name.png` or the burst folder `NN-name`). */
function gallery(dir, name) {
  if (!existsSync(dir)) return null;
  const f = readdirSync(dir).find(f => f.replace(/^\d+-/, '').replace(/\.png$/, '') === name);
  return f ? resolve(dir, f) : null;
}

function compare(a, b, mask = null) {
  const s = ssim(a, b, mask);
  const d = deltaE(a, b, mask);
  return { ssim: s, mean: d.mean, p95: d.p95, heat: heat(d.map, a.width, a.height) };
}

const rows = [];
const fmt = (v, n = 4) => (v === null || v === undefined ? '' : Number(v).toFixed(n));
for (const dpr of dprs) {
  const sheets = resolve(out, 'sheets', `${dpr}x`);
  mkdirSync(sheets, { recursive: true });
  for (const scene of spec.scenes) {
    if (!scene.wubble) continue;
    const metaPath = resolve(webDir, `${dpr}x`, `${scene.name}.json`);
    const wubPath = gallery(resolve(wubDir, `${dpr}x`), scene.wubble);
    const base = { scene: scene.name, dpr, gate: gateOf(scene), why: whyOf(scene) };
    const webPng = resolve(webDir, `${dpr}x`, scene.frames ? scene.name : `${scene.name}.png`);
    if (!existsSync(metaPath) || !existsSync(webPng)) { rows.push({ ...base, crop: '*', note: 'no web capture' }); continue; }
    if (!wubPath) { rows.push({ ...base, crop: '*', note: `no wubble gallery shot ${scene.wubble}` }); continue; }
    const meta = JSON.parse(readFileSync(metaPath, 'utf8'));
    // a burst: wubble's frames (timed) against the web frame at the same time on the 1/120 s clock
    if (scene.frames) {
      const times = readFileSync(resolve(wubPath, 'times.txt'), 'utf8').trim().split('\n').map(l => l.split(' ').map(Number));
      const acc = [];
      for (const [k, ms] of times) {
        const j = Math.round(ms / (1000 / 120));
        const webF = resolve(webDir, `${dpr}x`, scene.name, `f${String(j).padStart(3, '0')}.png`);
        if (!existsSync(webF)) continue;
        const w = png.read(webF), g = png.read(resolve(wubPath, `f${String(k).padStart(3, '0')}.png`));
        const [rw, rg] = cropRects(meta, { name: 'window' }, dpr, w, g);
        const a = png.crop(w, rw), b = png.crop(g, rg);
        const W = Math.min(a.width, b.width), H = Math.min(a.height, b.height);
        const c = compare(png.crop(a, [0, 0, W, H]), png.crop(b, [0, 0, W, H]));
        acc.push(c);
        png.write(resolve(sheets, `${scene.name}--f${String(k).padStart(3, '0')}.png`), png.hstack([png.crop(a, [0, 0, W, H]), png.crop(b, [0, 0, W, H]), c.heat]));
      }
      const avg = f => (acc.length ? acc.reduce((s, c) => s + c[f], 0) / acc.length : null);
      rows.push({ ...base, crop: 'window (motion)', kind: 'motion', ssim: avg('ssim'), mean: avg('mean'), p95: avg('p95'), note: `${acc.length} of ${times.length} wubble frames inside the web burst` });
      continue;
    }
    const w = png.read(webPng);
    const g = png.read(wubPath);
    for (const crop of scene.crops ?? [{ name: 'window' }]) {
      const at = wubbleRect(crop, scene, rectFiles);
      const kind = at ? 'derived' : 'direct';
      const row = { ...base, crop: crop.name, kind, gate: gateOf(scene, crop), why: whyOf(scene, crop) };
      if (typeof at === 'string') { rows.push({ ...row, note: at }); continue; }
      const rects = cropRects(meta, { ...crop, wubble: at }, dpr, w, g);
      if (!rects) { rows.push({ ...row, note: 'web element missing' }); continue; }
      let a = png.crop(w, rects[0]), b = png.crop(g, rects[1]);
      const W = Math.min(a.width, b.width), H = Math.min(a.height, b.height);
      const note = a.width !== b.width || a.height !== b.height ? `sizes ${a.width}x${a.height} / ${b.width}x${b.height}, compared top-left ${W}x${H}` : '';
      a = png.crop(a, [0, 0, W, H]); b = png.crop(b, [0, 0, W, H]);
      if (W < 8 || H < 8) { rows.push({ ...row, note: `too small (${W}x${H})` }); continue; }
      const mask = meta.masks?.[crop.name] ? maskOf(W, H, meta.masks[crop.name], dpr) : null;
      const c = compare(a, b, mask);
      rows.push({ ...row, ssim: c.ssim, mean: c.mean, p95: c.p95, size: `${W}x${H}`, note, masked: mask ? coverage(mask) : null });
      png.write(resolve(sheets, `${scene.name}--${crop.name}.png`), png.hstack([hatch(a, mask), hatch(b, mask), c.heat]));
    }
  }
}

writeFileSync(resolve(out, 'scores.csv'), toCsv(rows));
const scored = rows.filter(r => r.ssim != null).sort((a, b) => (b.gate !== false) - (a.gate !== false) || a.ssim - b.ssim);
console.log('| scene | crop | kind | dpr | SSIM | mean ΔE | p95 ΔE | gate | masked |');
console.log('|---|---|---|---|---|---|---|---|---|');
for (const r of scored) console.log(`| ${r.scene} | ${r.crop} | ${r.kind} | ${r.dpr} | ${fmt(r.ssim, 3)} | ${fmt(r.mean, 2)} | ${fmt(r.p95, 2)} | ${r.gate === false ? 'info' : 'gate'} | ${r.masked ? `${(r.masked * 100).toFixed(1)}%` : ''} |`);
for (const r of rows.filter(r => r.ssim == null)) console.log(`| ${r.scene} | ${r.crop} | ${r.kind ?? ''} | ${r.dpr} | — | — | — | ${r.gate === false ? 'info' : 'gate'} | | ${r.note}`);
const s = summarize(rows);
console.log(`\nscore: ${s.gatingPass} of ${s.gating} gating pairs at SSIM >= 0.90 and mean ΔE <= 3 (informational: ${s.infoPass} of ${s.info}; unscored: ${s.unscored}); wrote ${resolve(out, 'scores.csv')}`);
