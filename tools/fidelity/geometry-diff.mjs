#!/usr/bin/env node
// Geometry parity of kit specimens: the web's element rects against wubble's, per element
// position and size deltas in px (lib/rects.mjs has the file formats).
//
//   node tools/fidelity/geometry-diff.mjs --web WEB.rects.json --wubble WUBBLE.json
//   node tools/fidelity/geometry-diff.mjs --dir-web DIR --dir-wubble DIR    (pairs files by `specimen`)
//       [--tol 1] [--use layout|rect] [--json]
//
// WEB files: `<dpr>x/spec-<control>--<state>.rects.json` of a capture run or the web cache
// (specimens-index.json lists them). WUBBLE files: written by a controls lane's own test from its
// kit gallery scene (Slint testing backend: ElementHandle::find_by_element_id, absolute_position()
// minus the root's, size(), computed_opacity()). Exit 1 when an element is off by more than
// --tol px or missing in wubble; `--use layout` (default) compares the web's transform-free box,
// `rect` its drawn box (for transformed states; informational).
import { readFileSync, readdirSync } from 'node:fs';
import { resolve } from 'node:path';
import { compareRects, formatRows } from './lib/rects.mjs';

const args = process.argv.slice(2);
const opt = (n, d) => (args.includes(n) ? args[args.indexOf(n) + 1] : d);
const tol = Number(opt('--tol', 1));
const use = opt('--use', 'layout');
const read = p => JSON.parse(readFileSync(p, 'utf8'));
const bySpecimen = dir => new Map(readdirSync(dir, { recursive: true }).filter(f => f.endsWith('.json')).map(f => resolve(dir, f)).flatMap(p => {
  try { const j = read(p); return j.specimen && j.elements ? [[j.specimen, j]] : []; } catch { return []; }
}));

const pairs = [];
if (opt('--web') && opt('--wubble')) pairs.push([read(resolve(opt('--web'))), read(resolve(opt('--wubble')))]);
else if (opt('--dir-web') && opt('--dir-wubble')) {
  const web = bySpecimen(resolve(opt('--dir-web')));
  for (const [id, w] of bySpecimen(resolve(opt('--dir-wubble')))) {
    if (web.has(id)) pairs.push([web.get(id), w]);
    else console.error(`geometry-diff: no web rects for ${id}`);
  }
} else {
  console.error('usage: geometry-diff.mjs --web F --wubble F | --dir-web D --dir-wubble D [--tol 1] [--use layout|rect] [--json]');
  process.exit(2);
}

const results = pairs.map(([web, wubble]) => ({ specimen: wubble.specimen ?? web.specimen, ...compareRects(web, wubble, { tol, use }) }));
if (args.includes('--json')) console.log(JSON.stringify(results, null, 1));
else for (const r of results) console.log(formatRows(r.specimen, r, tol));
const failed = results.filter(r => r.failed).length;
if (!args.includes('--json')) console.log(`geometry-diff: ${results.length - failed} of ${results.length} specimens within ${tol} px (${use})`);
process.exit(failed ? 1 : 0);
