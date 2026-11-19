// The sign-off report's model and HTML (pure, no files).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { joinBaseline, regressions, groupBySurface, renderHtml } from '../lib/html-report.mjs';
import { summarize } from '../lib/scores.mjs';

const rows = [
  { scene: 'sidebar-extended', crop: 'window', kind: 'direct', dpr: 1, ssim: 0.83, mean: 4.4, p95: 10, gate: true },
  { scene: 'sidebar-extended', crop: 'address', kind: 'direct', dpr: 1, ssim: 0.95, mean: 1.2, p95: 4, gate: true, masked: 0.3, why: 'URL text masked' },
  { scene: 'command-bar', crop: 'window', kind: 'direct', dpr: 1, ssim: 0.6, mean: 8, p95: 13, gate: true },
  { scene: 'settings', crop: 'page', kind: 'direct', dpr: 1, ssim: 0.5, mean: 6, p95: 20, gate: false },
  { scene: 'settings-anatomy', crop: 'row-toggle', kind: 'derived', dpr: 1, ssim: null, note: 'no wubble rect for @Settings::row-toggle', gate: true },
];
const base = [
  { scene: 'sidebar-extended', crop: 'window', dpr: 1, ssim: 0.86, mean: 4 },
  { scene: 'command-bar', crop: 'window', dpr: 1, ssim: 0.61, mean: 8.4 },
];
const surface = s => ({ 'sidebar-extended': 'Sidebar', 'command-bar': 'Command bar', settings: 'Settings', 'settings-anatomy': 'Settings' })[s];

test('baseline deltas and regressions', () => {
  const j = joinBaseline(rows, base);
  assert.ok(Math.abs(j[0].dSsim + 0.03) < 1e-9);
  assert.ok(Math.abs(j[2].dMean + 0.4) < 1e-9);
  assert.equal(j[1].dSsim, null);
  assert.deepEqual(regressions(j).map(r => r.crop), ['window']);
  assert.equal(regressions(j)[0].scene, 'sidebar-extended');
  assert.equal(joinBaseline(rows, null)[0].base, null);
});

test('surfaces are ordered by their worst gating pair, rows worst first, unscored last', () => {
  const g = groupBySurface(joinBaseline(rows, base), surface);
  assert.deepEqual(g.map(s => s.surface), ['Command bar', 'Sidebar', 'Settings']);
  assert.deepEqual(g[1].rows.map(r => r.crop), ['window', 'address']);
  const st = g.find(s => s.surface === 'Settings');
  assert.deepEqual(st.rows.map(r => r.crop), ['page', 'row-toggle']);
  assert.equal(st.gating, 0);
  assert.equal(st.info, 1);
  // a scene with no surface groups by its name's first word
  assert.equal(groupBySurface([{ ...rows[0], scene: 'spec-x--y' }], () => undefined)[0].surface, 'spec');
});

test('the page holds every pair once and loads nothing from anywhere', () => {
  const j = joinBaseline(rows, base);
  const html = renderHtml({ title: 'T <run>', run: 'r', design: 'abc', date: '2026-11-19', baseline: 'baseline', surfaces: groupBySurface(j, surface), regressions: regressions(j), totals: summarize(rows), src: r => (r.ssim == null ? [] : [`sheets/1x/${r.scene}--${r.crop}.png`]) });
  for (const r of rows) assert.equal(html.split(`<b>${r.scene}</b> / ${r.crop}<br>`).length, 2, `${r.scene}/${r.crop} once`);
  assert.doesNotMatch(html, /https?:\/\//);
  assert.doesNotMatch(html, /<script/);
  assert.doesNotMatch(html, /<link/);
  assert.match(html, /T &lt;run&gt;/);
  assert.match(html, /class="chip fail">FAIL/);
  assert.match(html, /class="chip pass">PASS/);
  assert.match(html, /class="chip info">info/);
  assert.match(html, /masked 30\.0%/);
  assert.match(html, /Regressions since baseline/);
  const inline = renderHtml({ title: 't', run: 'r', design: '', date: '', baseline: null, surfaces: groupBySurface(j, surface), regressions: [], totals: summarize(rows), src: () => ['data:image/png;base64,AAAA'] });
  assert.doesNotMatch(inline, /src="(?!data:)/);
  assert.doesNotMatch(inline, /Regressions since/);
});
