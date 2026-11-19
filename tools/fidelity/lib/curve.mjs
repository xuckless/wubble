// Element geometry and motion read from the DOM: the per-specimen rects (`<id>.rects.json`) and
// the per-frame curves of a sequence (`curve.json`) the controls lanes' geometry and curve tests
// read. The parsers are pure; `sampler` is their source plus a page function, evaluated in the
// page (so node tests the same code the page runs).

/** `matrix(a, b, c, d, e, f)` / `matrix3d(...)` / `none` -> [a, b, c, d, e, f]. */
export function parseMatrix(s) {
  if (!s || s === 'none') return [1, 0, 0, 1, 0, 0];
  const n = (s.match(/-?\d*\.?\d+(?:e-?\d+)?/gi) ?? []).map(Number);
  // the digit in `matrix3d` is matched too: drop it
  if (s.startsWith('matrix3d(') && n.length >= 17) return [n[1], n[2], n[5], n[6], n[13], n[14]];
  if (s.startsWith('matrix(') && n.length >= 6) return n.slice(0, 6);
  return null;
}

/** The individual `scale` property (`none`, `0.97`, `1.2 0.8`) -> [sx, sy]. */
export function parseScale(s) {
  if (!s || s === 'none') return [1, 1];
  const n = s.trim().split(/\s+/).map(v => (v.endsWith('%') ? parseFloat(v) / 100 : parseFloat(v)));
  return [n[0], n[1] ?? n[0]];
}

/** The individual `translate` property (`none`, `4px`, `4px -2px`) -> [tx, ty] in px. */
export function parseTranslate(s) {
  if (!s || s === 'none') return [0, 0];
  const n = s.trim().split(/\s+/).map(parseFloat);
  return [n[0] || 0, n[1] || 0];
}

/** The opacity an element is drawn with: its own times every ancestor's. */
export function opacityProduct(values) {
  return values.reduce((p, v) => p * (Number.isFinite(v) ? v : 1), 1);
}

/** Rounds a rect's numbers to 1/1000 px (DOM floats, readable JSON). */
export const r3 = v => Math.round(v * 1000) / 1000;

/**
 * Page function: per element name (a map name -> selector, matched inside `root` first, then the
 * document), its visual rect (with transforms) and layout rect (offset chain, transforms ignored),
 * both relative to the root's, its drawn opacity, transform matrix, `scale`/`translate`,
 * `clip-path`, background colour and width/height styles. A missing element is null.
 */
function sample(rootSel, elements) {
  const root = document.querySelector(rootSel);
  if (!root) return null;
  const R = root.getBoundingClientRect();
  const offset = e => {
    let x = 0, y = 0;
    for (let n = e; n; n = n.offsetParent) { x += n.offsetLeft - (n !== e ? n.scrollLeft : 0); y += n.offsetTop - (n !== e ? n.scrollTop : 0); }
    return [x, y];
  };
  const [ox, oy] = offset(root);
  const out = {};
  for (const [name, sel] of Object.entries(elements)) {
    const e = root.matches(sel) ? root : root.querySelector(sel) ?? document.querySelector(sel);
    if (!e) { out[name] = null; continue; }
    const b = e.getBoundingClientRect();
    const cs = getComputedStyle(e);
    const ops = [];
    for (let n = e; n && n.nodeType === 1; n = n.parentElement) ops.push(parseFloat(getComputedStyle(n).opacity));
    let layout = null;
    if (e.offsetParent !== undefined && e.offsetWidth !== undefined) {
      const [x, y] = offset(e);
      layout = [x - ox, y - oy, e.offsetWidth, e.offsetHeight].map(r3);
    }
    out[name] = {
      rect: [b.x - R.x, b.y - R.y, b.width, b.height].map(r3),
      layout,
      opacity: r3(opacityProduct(ops)),
      transform: parseMatrix(cs.transform)?.map(r3) ?? cs.transform,
      scale: parseScale(cs.scale),
      translate: parseTranslate(cs.translate),
      clipPath: cs.clipPath === 'none' ? null : cs.clipPath,
      background: cs.backgroundColor,
      width: cs.width,
      height: cs.height,
    };
  }
  return { root: [R.x, R.y, R.width, R.height].map(r3), elements: out };
}

/** The source evaluated in the page: `(${sampler})(root, elements)`. */
export const sampler = `(() => {
  ${parseMatrix.toString()}
  ${parseScale.toString()}
  ${parseTranslate.toString()}
  ${opacityProduct.toString()}
  const r3 = ${r3.toString()};
  return ${sample.toString()};
})()`;

/** The rects file of a specimen still, from one sample. */
export function rectsFile(specimen, sampleOut, dpr) {
  const elements = {};
  for (const [name, v] of Object.entries(sampleOut?.elements ?? {})) elements[name] = v && { rect: v.rect, layout: v.layout, opacity: v.opacity };
  return { specimen, dpr, units: 'css px relative to the root', root: sampleOut?.root ?? null, elements };
}

/** The curve file of a sequence: one sample per 1/120 s frame. */
export function curveFile(specimen, track, samples) {
  return {
    specimen, dt: 1000 / 120, units: 'css px relative to the root; t in ms from the first frame',
    elements: track,
    frames: samples.map((s, k) => ({ t: r3((k * 1000) / 120), root: s?.root ?? null, ...Object.fromEntries(track.map(n => [n, s?.elements?.[n] ?? null])) })),
  };
}
