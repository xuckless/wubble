// Geometry comparison of one specimen: the web's element rects (capture.mjs `<scene>.rects.json`)
// against wubble's (exported by a controls lane's test from its kit gallery scene). Pure.
//
// wubble's file (logical px, origin at the specimen's root element):
//   { "specimen": "toggle-lg/on", "scale": 1, "root": "Gallery::toggle-lg",
//     "elements": { "track": { "x": 0, "y": 0, "w": 92, "h": 52, "opacity": 1 }, ... } }
// Element names are the keys of specimens.json `elements`; the lane maps its Slint element ids
// to them. The web's file has, per element, `rect` (visual, transforms applied) and `layout`
// (offset box, transforms ignored: what Slint's `size()` and position mean).

/** `{ name: { x, y, w, h, opacity } }` of either file; `use` picks the web's layout or rect box. */
export function normalize(file, use = 'layout') {
  const out = {};
  for (const [name, v] of Object.entries(file?.elements ?? {})) {
    if (!v) { out[name] = null; continue; }
    if ('x' in v) { out[name] = { x: v.x, y: v.y, w: v.w, h: v.h, opacity: v.opacity ?? null }; continue; }
    const r = (use === 'rect' ? v.rect : v.layout ?? v.rect) ?? null;
    out[name] = r && { x: r[0], y: r[1], w: r[2], h: r[3], opacity: v.opacity ?? null };
  }
  return out;
}

/**
 * Per-element deltas (wubble minus web, px) and a status: `ok` when every |delta| <= tol,
 * `off` when one exceeds it, `missing` when wubble lacks an element the web has, `extra` when
 * wubble has one the web lacks (reported, never failing), `absent` when the web element is not
 * drawn in this state (null in the web file). `scale` of the wubble file divides its numbers
 * (an export taken at a scale factor).
 */
export function compareRects(web, wubble, { tol = 1, use = 'layout' } = {}) {
  const a = normalize(web, use), b = normalize(wubble, use);
  const k = wubble?.scale && wubble.scale !== 1 ? wubble.scale : 1;
  const rows = [];
  for (const [name, r] of Object.entries(a)) {
    const s = b[name];
    if (!r) { rows.push({ name, status: 'absent' }); continue; }
    if (!s) { rows.push({ name, status: 'missing', web: r }); continue; }
    const d = { dx: s.x / k - r.x, dy: s.y / k - r.y, dw: s.w / k - r.w, dh: s.h / k - r.h };
    const max = Math.max(...Object.values(d).map(Math.abs));
    // compare at 1/1000 px, the precision of the web file, so a delta of exactly `tol` passes
    const status = Math.round(max * 1000) <= Math.round(tol * 1000) ? 'ok' : 'off';
    rows.push({ name, status, ...d, max, web: r, wubble: { x: s.x / k, y: s.y / k, w: s.w / k, h: s.h / k }, dOpacity: r.opacity != null && s.opacity != null ? s.opacity - r.opacity : null });
  }
  for (const name of Object.keys(b)) if (!(name in a)) rows.push({ name, status: 'extra' });
  const failed = rows.some(r => r.status === 'off' || r.status === 'missing');
  return { rows, failed };
}

/** A plain-text table of a comparison. */
export function formatRows(specimen, { rows, failed }, tol) {
  const f = v => (v === undefined || v === null ? '' : (v >= 0 ? '+' : '') + v.toFixed(2));
  const lines = [`${specimen}: ${failed ? 'FAIL' : 'ok'} (tolerance ${tol} px)`, '  element        dx      dy      dw      dh     max  status'];
  for (const r of rows) lines.push(`  ${r.name.padEnd(12)} ${f(r.dx).padStart(7)} ${f(r.dy).padStart(7)} ${f(r.dw).padStart(7)} ${f(r.dh).padStart(7)} ${(r.max === undefined ? '' : r.max.toFixed(2)).padStart(7)}  ${r.status}`);
  return lines.join('\n');
}
