// The sign-off report's model and HTML: pairs grouped by surface, worst first, with pass/fail
// against the bar and the change since a baseline run. Pure (report.mjs does the file work).
import { GATE, passes, pairKey, summarize } from './scores.mjs';

/** Each row with its baseline row's numbers and the deltas (`dSsim`, `dMean`), when there is one. */
export function joinBaseline(rows, baseRows) {
  const base = new Map((baseRows ?? []).map(r => [pairKey(r), r]));
  return rows.map(r => {
    const b = base.get(pairKey(r));
    if (!b || b.ssim == null || r.ssim == null) return { ...r, base: b ?? null, dSsim: null, dMean: null };
    return { ...r, base: b, dSsim: r.ssim - b.ssim, dMean: r.mean - b.mean };
  });
}

/** Gating pairs whose SSIM dropped by more than `by` since the baseline, worst first. */
export function regressions(rows, by = 0.02) {
  return rows.filter(r => r.gate !== false && r.dSsim != null && r.dSsim < -by).sort((a, b) => a.dSsim - b.dSsim);
}

/** Worst first: lowest SSIM; unscored rows last. */
const worst = (a, b) => (a.ssim == null) - (b.ssim == null) || (a.ssim ?? 0) - (b.ssim ?? 0) || b.mean - a.mean;

/**
 * Surfaces in order of their worst gating pair, then the surfaces with only informational pairs
 * in order of their worst, each with its rows worst first. `surfaceOf(scene)` names a scene's surface.
 */
export function groupBySurface(rows, surfaceOf) {
  const groups = new Map();
  for (const r of rows) {
    const s = surfaceOf(r.scene) ?? r.scene.split('-')[0];
    if (!groups.has(s)) groups.set(s, []);
    groups.get(s).push(r);
  }
  const out = [...groups].map(([surface, rs]) => {
    rs.sort(worst);
    const scored = rs.filter(r => r.ssim != null);
    const gating = scored.filter(r => r.gate !== false);
    return { surface, rows: rs, gated: gating.length > 0, worst: (gating.length ? gating : scored)[0]?.ssim ?? Infinity, ...summarize(rs) };
  });
  return out.sort((a, b) => b.gated - a.gated || a.worst - b.worst || a.surface.localeCompare(b.surface));
}

const esc = s => String(s ?? '').replace(/[&<>"']/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[c]);
const n = (v, d) => (v == null ? '—' : v.toFixed(d));
const delta = (v, d, better) => {
  if (v == null) return '<span class="d">—</span>';
  const good = better > 0 ? v > 0 : v < 0;
  const cls = Math.abs(v) < 10 ** -d ? 'd' : good ? 'd up' : 'd down';
  return `<span class="${cls}">${v >= 0 ? '+' : ''}${v.toFixed(d)}</span>`;
};
const chip = r => (r.ssim == null ? '<b class="chip none">unscored</b>' : r.gate === false ? `<b class="chip info">info${passes(r) ? ' · pass' : ''}</b>` : passes(r) ? '<b class="chip pass">PASS</b>' : '<b class="chip fail">FAIL</b>');

const CSS = `
:root { color-scheme: dark; --bg: #0b0c10; --card: #14161d; --line: #262a35; --ink: #eef0f6; --haze: #9aa0b0; --pass: #6fdcae; --fail: #ff7a72; --info: #8a90a3; }
* { box-sizing: border-box; }
body { margin: 0; background: var(--bg); color: var(--ink); font: 14px/1.45 system-ui, sans-serif; }
main { max-width: 1400px; margin: 0 auto; padding: 24px 16px 64px; }
h1 { font-size: 22px; margin: 0 0 4px; } h2 { font-size: 17px; margin: 0; }
.meta, .d, .note { color: var(--haze); }
.sum { display: flex; flex-wrap: wrap; gap: 12px; margin: 16px 0 24px; }
.sum div { background: var(--card); border: 1px solid var(--line); border-radius: 10px; padding: 10px 14px; }
.sum b { display: block; font-size: 20px; }
section, .reg { background: var(--card); border: 1px solid var(--line); border-radius: 12px; margin: 0 0 16px; }
summary { cursor: pointer; padding: 12px 16px; display: flex; gap: 12px; align-items: baseline; flex-wrap: wrap; }
.reg { padding: 12px 16px; } .reg li { margin: 2px 0; }
table { width: 100%; border-collapse: collapse; }
td, th { padding: 8px 10px; border-top: 1px solid var(--line); text-align: left; vertical-align: top; }
th { color: var(--haze); font-weight: 500; font-size: 12px; }
td.num { font-variant-numeric: tabular-nums; white-space: nowrap; }
.chip { font-size: 11px; padding: 2px 7px; border-radius: 6px; white-space: nowrap; }
.pass { background: rgb(111 220 174 / .15); color: var(--pass); } .fail { background: rgb(255 122 114 / .15); color: var(--fail); }
.info, .none { background: rgb(138 144 163 / .15); color: var(--info); }
.up { color: var(--pass); } .down { color: var(--fail); }
.sheet img { display: block; max-width: 100%; max-height: 260px; image-rendering: pixelated; border-radius: 4px; background: #000; }
.strip { display: flex; gap: 4px; flex-wrap: wrap; } .strip img { max-height: 120px; }
@media (max-width: 800px) { td.sheet { display: block; } }
`;

/**
 * The report page. `model` = { title, run, design, date, baseline, gate, surfaces (groupBySurface),
 * regressions, totals (summarize of all rows), src(row) -> [image src] }.
 */
export function renderHtml(m) {
  const rowHtml = r => {
    const srcs = m.src(r);
    const sheet = !srcs.length ? '<span class="note">no sheet</span>'
      : srcs.length === 1 ? `<a href="${esc(srcs[0])}"><img loading="lazy" alt="${esc(`${r.scene} ${r.crop}: web | wubble | ΔE`)}" src="${esc(srcs[0])}"></a>`
        : `<div class="strip">${srcs.map(s => `<a href="${esc(s)}"><img loading="lazy" alt="" src="${esc(s)}"></a>`).join('')}</div>`;
    return `<tr>
<td><b>${esc(r.scene)}</b> / ${esc(r.crop)}<br><span class="note">${esc(r.kind)} · ${r.dpr}x${r.size ? ` · ${esc(r.size)}` : ''}</span></td>
<td>${chip(r)}</td>
<td class="num">${n(r.ssim, 3)}<br>${delta(r.dSsim, 3, 1)}</td>
<td class="num">${n(r.mean, 2)} / ${n(r.p95, 2)}<br>${delta(r.dMean, 2, -1)}</td>
<td class="sheet">${sheet}${r.note || r.why || r.masked ? `<div class="note">${esc([r.note, r.masked ? `masked ${(r.masked * 100).toFixed(1)}%` : '', r.why].filter(Boolean).join(' · '))}</div>` : ''}</td>
</tr>`;
  };
  const t = m.totals;
  return `<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<title>${esc(m.title)}</title><style>${CSS}</style></head>
<body><main>
<h1>${esc(m.title)}</h1>
<div class="meta">run ${esc(m.run)} · design ${esc(m.design || 'unknown')} · ${esc(m.date)}${m.baseline ? ` · compared with ${esc(m.baseline)}` : ''}</div>
<div class="sum">
<div><b>${t.gatingPass} / ${t.gating}</b>gating pairs pass</div>
<div><b>${t.infoPass} / ${t.info}</b>informational pairs pass</div>
<div><b>${t.unscored}</b>unscored</div>
<div><b>SSIM ≥ ${GATE.ssim.toFixed(2)}</b>and mean ΔE ≤ ${GATE.de}</div>
</div>
${m.baseline ? `<div class="reg"><h2>Regressions since ${esc(m.baseline)}</h2>${m.regressions.length ? `<ul>${m.regressions.map(r => `<li>${esc(r.scene)} / ${esc(r.crop)} ${r.dpr}x: SSIM ${n(r.base.ssim, 3)} → ${n(r.ssim, 3)}</li>`).join('')}</ul>` : '<p class="note">No gating pair lost more than 0.02 SSIM.</p>'}</div>` : ''}
${m.surfaces.map(s => `<section><details open><summary><h2>${esc(s.surface)}</h2><span class="note">${s.gatingPass} of ${s.gating} gating pass${s.info ? ` · ${s.info} informational` : ''}${s.unscored ? ` · ${s.unscored} unscored` : ''}</span></summary>
<table><thead><tr><th>pair</th><th>bar</th><th>SSIM<br>Δ</th><th>ΔE mean / p95<br>Δ mean</th><th>web | wubble | ΔE</th></tr></thead><tbody>
${s.rows.map(rowHtml).join('\n')}
</tbody></table></details></section>`).join('\n')}
</main></body></html>
`;
}
