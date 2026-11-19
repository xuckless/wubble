// The web-capture cache: a directory per capture task, keyed by everything the task's pixels
// depend on, so a run re-captures the web only when the design, the harness or the scene changed.
//
//   key = sha1(design state, harness hash, scene hash, dpr, gl)
//
// - design state: the clone's HEAD, plus (when src/wubble is dirty) a hash of its diff and the
//   names of its untracked files, so a local edit of the design never hits a stale entry;
// - harness hash: the files that drive the browser (capture.mjs, lib/cdp.mjs, lib/specimen.mjs,
//   lib/curve.mjs, vite.config.mjs) and `chromium --version`;
// - scene hash: the canonical JSON of the scene with the globals it inherits, without the fields
//   only the scorer reads (`wubble`, `surface`, `gate`, `why`), so editing one scene re-captures
//   only it and editing a global (`settle`) re-captures all.
//
// Layout: <cache>/<key>/ holds exactly what the task writes into <out>/<dpr>x/ (the PNG or frame
// folder, the meta JSON, rects and curves) plus meta.json; <cache>/index.json records which key each
// scene@dpr used and when, for --prune.
import { createHash } from 'node:crypto';
import { execFileSync } from 'node:child_process';
import { readFileSync, existsSync, writeFileSync, readdirSync, rmSync, mkdirSync } from 'node:fs';
import { resolve } from 'node:path';

export const sha1 = s => createHash('sha1').update(s).digest('hex');

/** JSON with object keys sorted, so key order never changes a hash. */
export function canonical(v) {
  if (Array.isArray(v)) return `[${v.map(canonical).join(',')}]`;
  if (v && typeof v === 'object') return `{${Object.keys(v).sort().filter(k => v[k] !== undefined).map(k => `${JSON.stringify(k)}:${canonical(v[k])}`).join(',')}}`;
  return JSON.stringify(v);
}

/** Scene and crop fields the capture never reads. */
const SCORE_ONLY = new Set(['wubble', 'surface', 'gate', 'why']);
const strip = v => {
  if (Array.isArray(v)) return v.map(strip);
  if (v && typeof v === 'object') return Object.fromEntries(Object.entries(v).filter(([k]) => !SCORE_ONLY.has(k)).map(([k, x]) => [k, strip(x)]));
  return v;
};
/** The spec-level fields a scene inherits when it does not set them. */
export const GLOBALS = ['viewport', 'viewportByDpr', 'frame', 'settle', 'after', 'frozen'];

/** The hash of what a scene captures: its own fields and the globals it inherits. */
export function sceneHash(scene, spec) {
  const globals = Object.fromEntries(GLOBALS.map(k => [k, spec[k]]));
  return sha1(canonical({ scene: strip(scene), globals }));
}

/** The task's cache key. */
export function cacheKey({ design, harness, scene, dpr, gl }) {
  return sha1(canonical({ design, harness, scene, dpr, gl }));
}

/** The design clone's state: HEAD, plus a hash of the diff and untracked names when dirty. */
export function designState(clone, sub = 'src/wubble') {
  const git = (...a) => execFileSync('git', ['-C', clone, ...a], { encoding: 'utf8', maxBuffer: 1 << 28 });
  const head = git('rev-parse', 'HEAD').trim();
  const diff = git('diff', 'HEAD', '--', sub);
  const untracked = git('ls-files', '--others', '--exclude-standard', '--', sub);
  return diff || untracked ? `${head}+dirty-${sha1(diff + '\0' + untracked).slice(0, 12)}` : head;
}

/** The hash of the harness files that drive the browser, and the browser's version. */
export function harnessHash(files, extra = '') {
  return sha1(files.map(f => `${f}\0${readFileSync(f, 'utf8')}`).join('\0') + '\0' + extra);
}

/** Reads `<cache>/index.json` (`{ entries: [{ key, task, design, time, run }] }`). */
export function readIndex(cache) {
  const p = resolve(cache, 'index.json');
  try { return JSON.parse(readFileSync(p, 'utf8')); } catch { return { entries: [] }; }
}

/** Records the keys a run used (one entry per task, newest replaces older of the same key). */
export function recordRun(index, used, { run, time, design }) {
  const keys = new Set(used.map(u => u.key));
  const entries = index.entries.filter(e => !keys.has(e.key));
  for (const u of used) entries.push({ key: u.key, task: u.task, design, time, run });
  return { entries };
}

/**
 * The keys --prune keeps: the newest key of each task (scene@dpr), and every key the last
 * `runs` runs used. Pure.
 */
export function pruneKeep(index, runs = 2) {
  const keep = new Set();
  const newest = new Map();
  for (const e of index.entries) if (!newest.has(e.task) || newest.get(e.task).time < e.time) newest.set(e.task, e);
  for (const e of newest.values()) keep.add(e.key);
  const lastRuns = [...new Set([...index.entries].sort((a, b) => b.time - a.time).map(e => e.run))].slice(0, runs);
  for (const e of index.entries) if (lastRuns.includes(e.run)) keep.add(e.key);
  return keep;
}

/** Removes every key directory --prune does not keep; returns how many it removed. */
export function prune(cache, runs = 2) {
  const index = readIndex(cache);
  const keep = pruneKeep(index, runs);
  let n = 0;
  if (!existsSync(cache)) return 0;
  for (const d of readdirSync(cache, { withFileTypes: true })) {
    if (!d.isDirectory() || !/^[0-9a-f]{40}$/.test(d.name) || keep.has(d.name)) continue;
    rmSync(resolve(cache, d.name), { recursive: true, force: true });
    n++;
  }
  writeIndex(cache, { entries: index.entries.filter(e => keep.has(e.key)) });
  return n;
}

export function writeIndex(cache, index) {
  mkdirSync(cache, { recursive: true });
  writeFileSync(resolve(cache, 'index.json'), JSON.stringify(index, null, 1));
}
