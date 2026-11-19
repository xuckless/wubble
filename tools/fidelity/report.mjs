#!/usr/bin/env node
// The sign-off report: one static HTML file from a run's scores.csv and contact sheets, grouped by
// surface (scenes.json `surface`), worst pair first, with PASS/FAIL against SSIM >= 0.90 and mean
// ΔE <= 3 (informational pairs marked), and the change since a baseline run. No network, no
// external assets: sheets are linked next to the run, or embedded with --inline.
//
//   node tools/fidelity/report.mjs RUN [--baseline DIR | --baseline-name NAME] [--out FILE] [--inline]
//
// --baseline-name NAME reads ~/Dev/wubble-wt/notes/fidelity/NAME (e.g. `baseline`, the L0 run).
import { readFileSync, writeFileSync, existsSync, readdirSync } from 'node:fs';
import { resolve, dirname, relative, basename } from 'node:path';
import { fileURLToPath } from 'node:url';
import { homedir } from 'node:os';
import { parseScores, summarize } from './lib/scores.mjs';
import { joinBaseline, groupBySurface, regressions, renderHtml } from './lib/html-report.mjs';
import { withSpecimens } from './lib/specimen.mjs';

const here = dirname(fileURLToPath(import.meta.url));
const args = process.argv.slice(2);
const opt = (n, d) => (args.includes(n) ? args[args.indexOf(n) + 1] : d);
const run = resolve(args.find((a, i) => !a.startsWith('--') && !['--baseline', '--baseline-name', '--out', '--scenes'].includes(args[i - 1])) ?? '.');
const outFile = resolve(opt('--out', resolve(run, 'report.html')));
const inline = args.includes('--inline');
const baseDir = opt('--baseline') ? resolve(opt('--baseline')) : opt('--baseline-name') ? resolve(homedir(), 'Dev/wubble-wt/notes/fidelity', opt('--baseline-name')) : null;
const scenesPath = resolve(here, opt('--scenes', 'scenes.json'));
const spec = withSpecimens(JSON.parse(readFileSync(scenesPath, 'utf8')), f => JSON.parse(readFileSync(resolve(dirname(scenesPath), f), 'utf8')));

if (!existsSync(resolve(run, 'scores.csv'))) { console.error(`report: no ${resolve(run, 'scores.csv')}`); process.exit(1); }
const rows = parseScores(readFileSync(resolve(run, 'scores.csv'), 'utf8'));
const baseRows = baseDir && existsSync(resolve(baseDir, 'scores.csv')) ? parseScores(readFileSync(resolve(baseDir, 'scores.csv'), 'utf8')) : null;
if (baseDir && !baseRows) console.error(`report: no ${resolve(baseDir, 'scores.csv')}; no baseline deltas`);
const joined = joinBaseline(rows, baseRows);
const surfaces = new Map(spec.scenes.map(s => [s.name, s.surface]));

/** The contact sheets of a row: one per still pair, every frame's for a motion pair. */
const sheetsOf = r => {
  const dir = resolve(run, 'sheets', `${r.dpr}x`);
  if (r.kind === 'motion') {
    if (!existsSync(dir)) return [];
    const re = new RegExp(`^${r.scene.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}--f\\d+\\.png$`);
    const all = readdirSync(dir).filter(f => re.test(f)).sort();
    // at most eight evenly spaced frames
    const pick = all.length <= 8 ? all : Array.from({ length: 8 }, (_, i) => all[Math.round((i * (all.length - 1)) / 7)]);
    return pick.map(f => resolve(dir, f));
  }
  const p = resolve(dir, `${r.scene}--${r.crop}.png`);
  return existsSync(p) ? [p] : [];
};
const src = r => sheetsOf(r).map(p => (inline ? `data:image/png;base64,${readFileSync(p).toString('base64')}` : relative(dirname(outFile), p)));

let design = '';
try { design = JSON.parse(readFileSync(resolve(run, 'web', 'specimens-index.json'), 'utf8')).design ?? ''; } catch {}
const html = renderHtml({
  title: `Design fidelity: ${basename(run)}`, run, design, date: new Date().toISOString().slice(0, 10),
  baseline: baseRows ? basename(baseDir) : null,
  surfaces: groupBySurface(joined, s => surfaces.get(s)), regressions: regressions(joined), totals: summarize(rows), src,
});
writeFileSync(outFile, html);
const t = summarize(rows);
console.log(`report: ${t.gatingPass} of ${t.gating} gating pairs pass; wrote ${outFile}`);
