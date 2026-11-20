#!/usr/bin/env node
// Samples the design's own thinking-orb engine (Ultra Reactivate,
// src/ultra-reactivate/vendor/thinking-orbs: presets.ts, engine/*.ts, imported as they are) and
// writes the fixture wubble_core::orb replays (crates/wubble-core/src/orb/tests.rs):
//
//   node --disable-warning=ExperimentalWarning tools/fidelity/sample-orbs.mjs
//
// For every state at every tuned size, a few instants: the resolved options, how many dots and
// lines the frame has, the sums of each field over them, the first and last dots and the first
// lines in full; plus the ink of a few values on each substrate, with and without a tint (the
// painter's `inkColor`, which is not exported, transcribed below with its citation). No clocks:
// the times are fixed. Re-run it when the design's orb code changes.
import { register } from 'node:module';
import { writeFileSync, mkdirSync } from 'node:fs';
import { resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import { REPO, designSrc, designCommit } from './lib/design.mjs';

register('./lib/ts-loader.mjs', import.meta.url);
const ORBS = resolve(designSrc(), '../ultra-reactivate/vendor/thinking-orbs');
const { commit } = designCommit();
const mod = f => import(pathToFileURL(resolve(ORBS, f)).href);
const { resolvePreset } = await mod('presets.ts');
const { MODE_FRAMES } = await mod('engine/registry.ts');
const { scaleCounts, scaleRadii } = await mod('engine/profiles.ts');

const STATES = ['working', 'searching', 'solving', 'listening', 'connecting', 'weaving', 'composing', 'breathing', 'shaping'];
const SIZES = [64, 32, 20];
// orb times (already × speed): rest, early, a rubik move and a morph mid-blend, later
const TIMES = [0, 0.6, 1.8, 4.1, 9.35, 23.77];
const HEAD = 12, TAIL = 4, LINES = 6;

const sums = (xs, keys) => Object.fromEntries(keys.map(k => [k, xs.reduce((s, d) => s + (d[k] ?? 1), 0)]));
const dotRow = d => [d.x, d.y, d.z, d.r, d.white, d.a ?? null];
const lineRow = l => [l.x1, l.y1, l.x2, l.y2, l.white, l.a ?? null, l.w];

const frames = [];
for (const state of STATES) {
  for (const size of SIZES) {
    const { mode, speed, opts } = resolvePreset(state, size);
    const at = TIMES.map(t => {
      const f = MODE_FRAMES[mode](size, t, opts);
      return {
        t,
        dots: f.dots.length,
        lines: f.lines.length,
        dot_sums: sums(f.dots, ['x', 'y', 'z', 'r', 'white', 'a']),
        line_sums: sums(f.lines, ['x1', 'y1', 'x2', 'y2', 'white', 'a', 'w']),
        head: f.dots.slice(0, HEAD).map(dotRow),
        tail: f.dots.slice(-TAIL).map(dotRow),
        first_lines: f.lines.slice(0, LINES).map(lineRow),
      };
    });
    frames.push({ state, size, mode, speed, opts, at });
  }
}

// the `dots` / `dotSize` knobs (ThinkingOrb.tsx) on one preset
const knobs = [0.5, 2].map(k => {
  const { mode, opts } = resolvePreset('searching', 64);
  const o = scaleRadii(scaleCounts(opts, Math.max(0.1, k)), Math.max(0.1, k));
  const f = MODE_FRAMES[mode](64, 4.1, o);
  return { k, opts: o, dots: f.dots.length, dot_sums: sums(f.dots, ['x', 'y', 'z', 'r', 'white', 'a']) };
});

// engine/core.ts:113-120 inkColor, transcribed (not exported): channels and alpha
function inkColor(w, alpha, dark, tint) {
  if (!tint) {
    const g = Math.round((dark ? 1 - w : w) * 255);
    return [g, g, g, alpha];
  }
  const ramp = c => Math.round(dark ? c * (1 - w) : c + (255 - c) * w);
  return [ramp(tint.r), ramp(tint.g), ramp(tint.b), alpha];
}
const TINT = { r: 0x7c, g: 0xd3, b: 0xff };
const inks = [];
for (const w of [0, 0.05, 0.1, 0.3, 0.42, 0.55, 0.72, 0.78, 1])
  for (const dark of [true, false])
    for (const tint of [null, TINT]) inks.push({ w, dark, tint, ink: inkColor(w, 0.7, dark, tint ?? undefined) });

const OUT = resolve(REPO, 'crates/wubble-core/tests/fixtures');
mkdirSync(OUT, { recursive: true });
writeFileSync(resolve(OUT, 'orb_frames.json'), `${JSON.stringify({ design: commit, frames, knobs, inks })}\n`);
console.log(`sample-orbs: wrote orb_frames.json (${frames.length} presets × ${TIMES.length} times)`);
