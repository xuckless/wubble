// scores.csv: one row per scored pair, written by score.mjs and read by report.mjs. Pure.
//
// Columns: scene,crop,kind,dpr,ssim,de_mean,de_p95,size,note,gate,masked,why. Rows of runs before
// the gate column existed read as gating with nothing masked.

export const GATE = { ssim: 0.9, de: 3 };
const COLUMNS = ['scene', 'crop', 'kind', 'dpr', 'ssim', 'de_mean', 'de_p95', 'size', 'note', 'gate', 'masked', 'why'];

/** Whether a pair gates: the crop's `gate`, else the scene's, else true. */
export const gateOf = (scene, crop) => crop?.gate ?? scene?.gate ?? true;
/** Why a pair is informational or masked: the crop's `why`, else the scene's. */
export const whyOf = (scene, crop) => crop?.why ?? scene?.why ?? '';

/**
 * Wubble's rect for a derived anatomy crop (`"wubble": "@<element id>"`) from the rects files
 * given to score.mjs (`{ scene, elements: { id: { x, y, w, h } } }`, logical px in wubble's
 * window): [x, y, w, h], or a string saying why there is none.
 */
export function wubbleRect(crop, scene, rectFiles) {
  if (typeof crop.wubble !== 'string' || !crop.wubble.startsWith('@')) return crop.wubble;
  const id = crop.wubble.slice(1);
  const file = rectFiles.find(f => f.scene === scene.wubble);
  const r = file?.elements?.[id];
  if (!r) return `no wubble rect for ${crop.wubble}${file ? '' : ` (no rects file for gallery scene ${scene.wubble})`}`;
  return [r.x, r.y, r.w, r.h];
}

/** A pair passes the bar. */
export const passes = r => r.ssim != null && r.ssim >= GATE.ssim && r.mean <= GATE.de;

/** Counts of gating and informational pairs and how many pass. */
export function summarize(rows) {
  const scored = rows.filter(r => r.ssim != null);
  const gating = scored.filter(r => r.gate !== false);
  const info = scored.filter(r => r.gate === false);
  return { gating: gating.length, gatingPass: gating.filter(passes).length, info: info.length, infoPass: info.filter(passes).length, unscored: rows.length - scored.length };
}

const fmt = (v, n = 4) => (v === null || v === undefined || Number.isNaN(v) ? '' : Number(v).toFixed(n));
const quote = s => `"${String(s ?? '').replace(/"/g, "'")}"`;

/** The CSV text of score rows (`{ scene, crop, kind, dpr, ssim, mean, p95, size, note, gate, masked, why }`). */
export function toCsv(rows) {
  const lines = [COLUMNS.join(',')];
  for (const r of rows) lines.push([r.scene, r.crop, r.kind ?? '', r.dpr, fmt(r.ssim), fmt(r.mean, 2), fmt(r.p95, 2), r.size ?? '', quote(r.note), r.gate === false ? 0 : 1, fmt(r.masked, 3), quote(r.why)].join(','));
  return `${lines.join('\n')}\n`;
}

/** Splits one CSV line (double-quoted fields may hold commas). */
function fields(line) {
  const out = [];
  let cur = '', q = false;
  for (const ch of line) {
    if (ch === '"') q = !q;
    else if (ch === ',' && !q) { out.push(cur); cur = ''; } else cur += ch;
  }
  out.push(cur);
  return out;
}

/** Rows of a scores.csv (any run: columns are read by header name). */
export function parseScores(text) {
  const [head, ...lines] = text.trim().split('\n');
  const cols = fields(head);
  const num = v => (v === undefined || v === '' ? null : Number(v));
  return lines.filter(Boolean).map(l => {
    const f = Object.fromEntries(fields(l).map((v, i) => [cols[i], v]));
    return {
      scene: f.scene, crop: f.crop, kind: f.kind || '', dpr: Number(f.dpr), ssim: num(f.ssim), mean: num(f.de_mean), p95: num(f.de_p95),
      size: f.size || '', note: f.note || '', gate: f.gate === undefined || f.gate === '' ? true : f.gate !== '0', masked: num(f.masked), why: f.why || '',
    };
  });
}

/** The key that pairs a row with the same pair of another run. */
export const pairKey = r => `${r.scene}|${r.crop}|${r.dpr}`;
