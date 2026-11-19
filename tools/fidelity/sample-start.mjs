#!/usr/bin/env node
// Samples the design's start-page physics (core/softbody.ts, core/minis.ts, the loop of
// core/living.ts, wTexture() of core/gl/scene.ts) on a fixed 1/120 s clock and writes the
// fixtures wubble_core::start compares its port against:
//
//   node --disable-warning=ExperimentalWarning tools/fidelity/sample-start.mjs
//
// No clocks: pointer times are explicit (ms = step * 1000 / 120). `Math.random` is replaced by a
// seeded list, and the draws each scenario consumed are stored in its fixture so Rust replays
// them in the same order. living.ts can't be imported (it needs a DOM and WebGL), so its loop is
// transcribed below line by line, with citations; SoftBody and Minis are the design's own.
import { register } from 'node:module';
import { readFileSync, writeFileSync, mkdirSync } from 'node:fs';
import { resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import { REPO, designSrc, designCommit } from './lib/design.mjs';

register('./lib/ts-loader.mjs', import.meta.url);
const SRC = designSrc();
const { commit } = designCommit();
const mod = f => import(pathToFileURL(resolve(SRC, f)).href);
const { SoftBody } = await mod('core/softbody.ts');
const { Minis } = await mod('core/minis.ts');

const DT = 1 / 120;
const OUT = resolve(REPO, 'crates/wubble-core/tests/fixtures');
mkdirSync(OUT, { recursive: true });

// ── Math.random as a recorded, seeded stream (mulberry32) ──
let drawn = [];
function seedRandom(seed) {
  let a = seed >>> 0;
  drawn = [];
  Math.random = () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    const v = ((t ^ (t >>> 14)) >>> 0) / 4294967296;
    drawn.push(v);
    return v;
  };
}

const write = (name, data) => {
  writeFileSync(resolve(OUT, `start_${name}.json`), `${JSON.stringify({ design: commit, dt: DT, ...data })}\n`);
  console.log(`sample-start: wrote start_${name}.json`);
};
const ms = i => (i * 1000) / 120;

// ── the soft body alone ──
const HOME = { x: 400, y: 300 };
function bodyCurves() {
  return { ring0: [], ring6: [], ring12: [], ring18: [], cx: [], cy: [], yaw: [], pitch: [], swirl: [], winv0: [], winv1: [], winv2: [], winv3: [], woff0: [], woff1: [], grab: [] };
}
function sampleBody(b, c) {
  const s = b.state;
  c.ring0.push(b.ring[0]); c.ring6.push(b.ring[6]); c.ring12.push(b.ring[12]); c.ring18.push(b.ring[18]);
  c.cx.push(s.x); c.cy.push(s.y); c.yaw.push(b.yaw.x); c.pitch.push(b.pitch.x); c.swirl.push(s.swirl);
  s.w.inv.forEach((v, i) => c[`winv${i}`].push(v));
  c.woff0.push(s.w.off[0]); c.woff1.push(s.w.off[1]);
  c.grab.push(b.dragging ? 2 : b.held ? 1 : 0);
}
/** `events[i]`: what happens before step i (`[kind, x, y]`, kinds down / move / up / leave / poke). */
function bodyScenario(name, steps, events) {
  seedRandom(7);
  const b = new SoftBody({});
  b.home = { ...HOME };
  const c = bodyCurves();
  const log = [];
  for (let i = 0; i < steps; i++) {
    nowMs = ms(i);
    for (const [at, kind, x, y] of events) {
      if (at !== i) continue;
      log.push({ step: i, kind, x, y, t_ms: ms(i) });
      if (kind === 'down') b.down(x, y);
      else if (kind === 'move') b.move(x, y);
      else if (kind === 'up') b.up();
      else if (kind === 'leave') b.leave();
      else if (kind === 'poke') b.poke(x, y);
    }
    b.step(DT);
    sampleBody(b, c);
  }
  write(name, { steps, home: [HOME.x, HOME.y], draws: drawn, events: log, curves: c });
}
// Velocity reads performance.now(): it reads the step's own time instead.
const realNow = performance.now.bind(performance);
let nowMs = 0;
performance.now = () => nowMs;
const at = (i, kind, x, y) => [i, kind, x, y];

const R = 180;
bodyScenario('rest', 360, []);
bodyScenario('lean', 240, [at(0, 'move', HOME.x + 140, HOME.y - 60)]);
bodyScenario('poke_mid', 240, [at(0, 'poke', HOME.x + 10, HOME.y + 5)]);
bodyScenario('poke_rim', 240, [at(0, 'poke', HOME.x + R * 0.9, HOME.y - R * 0.3)]);
bodyScenario('drag_fling', 360, [
  at(0, 'down', HOME.x + 20, HOME.y),
  ...Array.from({ length: 12 }, (_, k) => at(k + 1, 'move', HOME.x + 20 + (k + 1) * 14, HOME.y + (k + 1) * 5)),
  at(13, 'up'),
  at(200, 'leave'),
]);
performance.now = realNow;

// ── the minis alone, with a stub scene (scene.pop draws its seed as scene.ts:623 does) ──
function stubScene() {
  const scene = { bubbles: [], pops: [], time: 0 };
  scene.pop = (x, y, r, angle = -Math.PI / 2) => scene.pops.push({ x, y, r, age: 0, life: 0.36, angle, seed: Math.random() * 100 });
  return scene;
}
{
  seedRandom(11);
  const scene = stubScene();
  const minis = new Minis(scene);
  const avoid = [{ x: 400, y: 300, r: 80 }];
  const steps = 480, snaps = [], pops = [], events = [];
  let nextId = 0;
  const tag = () => { for (const m of minis.list) if (m.id === undefined) m.id = nextId++; };
  for (let i = 0; i < steps; i++) {
    const before = scene.pops.length;
    if (i === 0) { minis.spawn(400, 420, 3, true); minis.spawn(300, 700, 1); tag(); events.push({ step: i, kind: 'spawn' }); }
    if (i === 30) { minis.spawn(420, 330, 5, true, 0.8); tag(); events.push({ step: i, kind: 'spawn' }); }
    if (i === 60) {
      const m = minis.list[minis.list.length - 1];
      // off the centre: a hit dead on it has an angle of atan2(0, ±0), which the last bit decides
      events.push({ step: i, kind: 'hit', x: m.x + 2, y: m.y + 1 });
      minis.hit(m.x + 2, m.y + 1);
    }
    minis.step(DT, i * DT, avoid);
    for (const p of scene.pops.slice(before)) pops.push({ step: i, ...p });
    snaps.push(minis.list.map(m => [m.id, m.x, m.y, m.b.r, m.b.swirl]));
  }
  write('minis', { steps, draws: drawn, events, avoid: avoid[0], snaps, pops });
}

// ── the living loop (core/living.ts), transcribed; SoftBody and Minis are the design's ──
{
  seedRandom(23);
  nowMs = 0;
  performance.now = () => nowMs;
  const W = 1000, H = 800;
  // living.ts:34-39 (the Scene is the stub: no WebGL)
  const scene = stubScene();
  scene.w = 0; scene.h = 0;
  const body = new SoftBody({ wobble: 1 });
  const minis = new Minis(scene);
  scene.bubbles.push(body.state);
  const o = { size: 0.1, at: { x: 0.5, y: 0.4 }, ambient: true }; // pages/start.tsx:24
  const at = o.at;
  let spawnT = 0.6, active = true;
  // living.ts:41-48 layout()
  scene.w = W; scene.h = H;
  body.R = Math.max(40, Math.min(W, H) * (o.size ?? 0.2));
  body.home = { x: W * at.x, y: H * at.y };
  // scene.ts:822-828 wave / ageWaves; 798 pops aged at draw
  const wave = (target, x, y, reach, life = 0.9) => { (target.waves ??= []).push({ x, y, age: 0, life, reach }); if (target.waves.length > 4) target.waves.shift(); };
  const frame = dt => {
    if (!scene.w) return;
    scene.time += dt;
    for (const b of scene.bubbles) if (b.waves?.length) b.waves = b.waves.filter(w => (w.age += dt) < w.life);
    scene.pops = scene.pops.filter(p => (p.age += dt) < p.life);
  };
  const effects = [];
  let step = 0;
  const ripple = (x, y, reach = 260, strength = 1) => effects.push({ step, x, y, reach, strength });
  // living.ts:54-60 onDown
  const onDown = p => {
    if (minis.hit(p.x, p.y)) return;
    if (body.hit(p.x, p.y)) body.down(p.x, p.y);
    else { ripple(p.x, p.y); minis.spawn(p.x, p.y, 4, true, 0.8); }
  };
  // living.ts:62-67 onUp
  const onUp = () => {
    const c = body.center, r = body.up();
    if (r === 'poke' && body.ptr) { ripple(body.ptr.x, body.ptr.y, 300); wave(body.state, body.ptr.x, body.ptr.y, body.R * 2.3); minis.spawn(body.ptr.x, body.ptr.y, 3, true); }
    if (r === 'fling') { ripple(c.x, c.y, body.R * 2.4, 1.6); wave(body.state, c.x, c.y, body.R * 1.6, 1.1); }
  };
  // living.ts:76-85 poke()
  const poke = (rim = Math.random() > 0.4) => {
    const c = body.center, a = Math.random() * Math.PI * 2;
    const x = rim ? c.x + Math.cos(a) * body.R : c.x + (Math.random() - 0.5) * body.R * 0.4;
    const y = rim ? c.y + Math.sin(a) * body.R : c.y + (Math.random() - 0.5) * body.R * 0.4;
    body.poke(x, y);
    ripple(x, y, 300);
    wave(body.state, x, y, body.R * 2.3);
    minis.spawn(x, y, 3, true);
  };
  const steps = 600, events = [], curves = { cx: [], cy: [], minis: [], time: [], waves: [], pops: [] };
  for (let i = 0; i < steps; i++) {
    nowMs = ms(i);
    step = i;
    const ev = e => events.push({ step: i, t_ms: ms(i), ...e });
    if (i === 120) { ev({ kind: 'poke' }); poke(); }
    if (i === 240) { ev({ kind: 'down', x: 60, y: 700 }); onDown({ x: 60, y: 700 }); }
    if (i === 241) { ev({ kind: 'up' }); onUp(); }
    if (i === 300) { const c = body.center; ev({ kind: 'down', x: c.x + 3, y: c.y }); onDown({ x: c.x + 3, y: c.y }); }
    if (i >= 301 && i <= 310) { const c = { x: body.home.x + 3 + (i - 300) * 12, y: body.home.y - (i - 300) * 4 }; ev({ kind: 'move', ...c }); body.move(c.x, c.y); }
    if (i === 311) { ev({ kind: 'up' }); onUp(); }
    if (i === 420 && minis.list.length) { const m = minis.list[minis.list.length - 1], p = { x: m.x + 2, y: m.y + 1 }; ev({ kind: 'down', ...p }); onDown(p); }
    // living.ts:88-116 the loop body (auto off, voice off, not reduced)
    if (active) {
      const dt = DT;
      body.wobble = 1;
      body.step(dt);
      if (o.ambient !== false) {
        spawnT -= dt;
        if (spawnT <= 0) { spawnT = 0.9 + Math.random() * 1.4; minis.spawn(scene.w * (0.12 + Math.random() * 0.76), scene.h + 20, 1); }
      }
      const c = body.center;
      minis.step(dt, scene.time, [{ x: c.x, y: c.y, r: body.R }]);
      frame(dt);
    }
    curves.cx.push(body.state.x); curves.cy.push(body.state.y);
    curves.minis.push(minis.list.length); curves.time.push(scene.time);
    curves.waves.push(body.state.waves?.length ?? 0);
    curves.pops.push(scene.pops.map(p => [p.x, p.y, p.r, p.age, p.angle, p.seed]));
  }
  performance.now = realNow;
  write('living', { steps, size: [W, H], draws: drawn, events, curves, effects });
}

// ── the W field: wTexture() sliced from scene.ts as text, evaluated with a stub gl ──
{
  const text = readFileSync(resolve(SRC, 'core/gl/scene.ts'), 'utf8');
  const start = text.indexOf('function wTexture(gl: WebGL2RenderingContext) {');
  const end = text.indexOf('\n}\n', start) + 2;
  const fnSrc = text.slice(start, end).replace('gl: WebGL2RenderingContext', 'gl').replace('const pts: number[] = []', 'const pts = []').replace('gl.createTexture()!', 'gl.createTexture()');
  let bytes = null;
  const gl = new Proxy({}, { get: (_, k) => (k === 'texImage2D' ? (...a) => { bytes = a[8]; } : typeof k === 'string' && /^[A-Z_0-9]+$/.test(k) ? 0 : () => ({})) });
  new Function('gl', `${fnSrc}\nreturn wTexture(gl);`)(gl);
  let h = 0xcbf29ce484222325n;
  for (const b of bytes) { h ^= BigInt(b); h = (h * 0x100000001b3n) & 0xffffffffffffffffn; }
  const S = 384, texels = [];
  for (const [i, j] of [[0, 0], [383, 383], [192, 192], [100, 100], [150, 250], [200, 120], [96, 256], [288, 256], [192, 144], [50, 300], [330, 60], [123, 210], [260, 230], [140, 270], [240, 270], [10, 370]]) texels.push([i, j, bytes[(j * S + i) * 4 + 3]]);
  write('wfield', { bytes: bytes.length, fnv1a64: h.toString(16).padStart(16, '0'), texels });
}
