// Image comparison: SSIM on luma (gaussian window 11, sigma 1.5, as Wang et al. 2004) and
// CIEDE2000 colour difference per pixel (Sharma, Wu, Dalal 2005). Images are RGBA, the same
// size; alpha is flattened over the design's abyss first.

const GROUND = [8, 9, 13];

function flatten(img) {
  const n = img.width * img.height, rgb = new Float32Array(n * 3);
  for (let i = 0; i < n; i++) {
    const a = img.data[i * 4 + 3] / 255;
    for (let c = 0; c < 3; c++) rgb[i * 3 + c] = img.data[i * 4 + c] * a + GROUND[c] * (1 - a);
  }
  return rgb;
}

function gaussian(size = 11, sigma = 1.5) {
  const k = new Float32Array(size), r = (size - 1) / 2;
  let s = 0;
  for (let i = 0; i < size; i++) { k[i] = Math.exp(-((i - r) ** 2) / (2 * sigma * sigma)); s += k[i]; }
  return k.map(v => v / s);
}

/** Separable blur with clamped edges. */
function blur(src, w, h, k) {
  const r = (k.length - 1) / 2, tmp = new Float32Array(w * h), out = new Float32Array(w * h);
  for (let y = 0; y < h; y++) for (let x = 0; x < w; x++) {
    let s = 0;
    for (let i = 0; i < k.length; i++) s += k[i] * src[y * w + Math.min(w - 1, Math.max(0, x + i - r))];
    tmp[y * w + x] = s;
  }
  for (let y = 0; y < h; y++) for (let x = 0; x < w; x++) {
    let s = 0;
    for (let i = 0; i < k.length; i++) s += k[i] * tmp[Math.min(h - 1, Math.max(0, y + i - r)) * w + x];
    out[y * w + x] = s;
  }
  return out;
}

/**
 * `b` with its masked pixels replaced by `a`'s, so a window straddling the mask's edge sees no
 * difference from inside it. `mask` is a Uint8Array (1 = excluded) of the images' size, or null.
 */
function unmasked(a, b, mask) {
  if (!mask) return b;
  const data = Buffer.from(b.data);
  for (let i = 0; i < mask.length; i++) if (mask[i]) for (let c = 0; c < 4; c++) data[i * 4 + c] = a.data[i * 4 + c];
  return { ...b, data };
}

/** Mean SSIM of two images' luma (BT.601), 0..1, over the pixels `mask` (optional) keeps. */
export function ssim(a, b, mask = null) {
  const w = a.width, h = a.height, n = w * h;
  const fa = flatten(a), fb = flatten(unmasked(a, b, mask));
  const x = new Float32Array(n), y = new Float32Array(n);
  for (let i = 0; i < n; i++) {
    x[i] = 0.299 * fa[i * 3] + 0.587 * fa[i * 3 + 1] + 0.114 * fa[i * 3 + 2];
    y[i] = 0.299 * fb[i * 3] + 0.587 * fb[i * 3 + 1] + 0.114 * fb[i * 3 + 2];
  }
  const k = gaussian(), mul = (p, q) => p.map((v, i) => v * q[i]);
  const mx = blur(x, w, h, k), my = blur(y, w, h, k);
  const sxx = blur(mul(x, x), w, h, k), syy = blur(mul(y, y), w, h, k), sxy = blur(mul(x, y), w, h, k);
  const C1 = (0.01 * 255) ** 2, C2 = (0.03 * 255) ** 2;
  let total = 0, kept = 0;
  for (let i = 0; i < n; i++) {
    if (mask?.[i]) continue;
    const vx = sxx[i] - mx[i] * mx[i], vy = syy[i] - my[i] * my[i], cxy = sxy[i] - mx[i] * my[i];
    total += ((2 * mx[i] * my[i] + C1) * (2 * cxy + C2)) / ((mx[i] ** 2 + my[i] ** 2 + C1) * (vx + vy + C2));
    kept++;
  }
  return kept ? total / kept : 1;
}

function lab([r, g, b]) {
  const lin = c => { c /= 255; return c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4; };
  const R = lin(r), G = lin(g), B = lin(b);
  const X = (0.4124564 * R + 0.3575761 * G + 0.1804375 * B) / 0.95047;
  const Y = 0.2126729 * R + 0.7151522 * G + 0.072175 * B;
  const Z = (0.0193339 * R + 0.119192 * G + 0.9503041 * B) / 1.08883;
  const f = t => (t > 216 / 24389 ? Math.cbrt(t) : (24389 / 27 * t + 16) / 116);
  return [116 * f(Y) - 16, 500 * (f(X) - f(Y)), 200 * (f(Y) - f(Z))];
}

/** CIEDE2000 between two Lab colours. */
export function de2000([L1, a1, b1], [L2, a2, b2]) {
  const rad = Math.PI / 180, deg = 180 / Math.PI;
  const C1 = Math.hypot(a1, b1), C2 = Math.hypot(a2, b2), Cb = (C1 + C2) / 2;
  const G = 0.5 * (1 - Math.sqrt(Cb ** 7 / (Cb ** 7 + 25 ** 7)));
  const a1p = (1 + G) * a1, a2p = (1 + G) * a2;
  const C1p = Math.hypot(a1p, b1), C2p = Math.hypot(a2p, b2);
  const h = (b, a) => { if (b === 0 && a === 0) return 0; const v = Math.atan2(b, a) * deg; return v < 0 ? v + 360 : v; };
  const h1p = h(b1, a1p), h2p = h(b2, a2p);
  const dLp = L2 - L1, dCp = C2p - C1p;
  let dhp = 0;
  if (C1p * C2p !== 0) { dhp = h2p - h1p; if (dhp > 180) dhp -= 360; else if (dhp < -180) dhp += 360; }
  const dHp = 2 * Math.sqrt(C1p * C2p) * Math.sin((dhp / 2) * rad);
  const Lbp = (L1 + L2) / 2, Cbp = (C1p + C2p) / 2;
  let hbp = h1p + h2p;
  if (C1p * C2p !== 0) { if (Math.abs(h1p - h2p) > 180) hbp += h1p + h2p < 360 ? 360 : -360; hbp /= 2; }
  const T = 1 - 0.17 * Math.cos((hbp - 30) * rad) + 0.24 * Math.cos(2 * hbp * rad) + 0.32 * Math.cos((3 * hbp + 6) * rad) - 0.2 * Math.cos((4 * hbp - 63) * rad);
  const dTheta = 30 * Math.exp(-(((hbp - 275) / 25) ** 2));
  const Rc = 2 * Math.sqrt(Cbp ** 7 / (Cbp ** 7 + 25 ** 7));
  const Sl = 1 + (0.015 * (Lbp - 50) ** 2) / Math.sqrt(20 + (Lbp - 50) ** 2), Sc = 1 + 0.045 * Cbp, Sh = 1 + 0.015 * Cbp * T;
  const Rt = -Math.sin(2 * dTheta * rad) * Rc;
  return Math.sqrt((dLp / Sl) ** 2 + (dCp / Sc) ** 2 + (dHp / Sh) ** 2 + Rt * (dCp / Sc) * (dHp / Sh));
}

/**
 * Per-pixel CIEDE2000 (`Float32Array`, 0 where masked), its mean and 95th percentile over the
 * pixels `mask` (optional) keeps.
 */
export function deltaE(a, b, mask = null) {
  const n = a.width * a.height, fa = flatten(a), fb = flatten(unmasked(a, b, mask)), map = new Float32Array(n);
  const cache = new Map();
  const labOf = (f, i) => {
    const key = (Math.round(f[i * 3]) << 16) | (Math.round(f[i * 3 + 1]) << 8) | Math.round(f[i * 3 + 2]);
    let v = cache.get(key);
    if (!v) { v = lab([f[i * 3], f[i * 3 + 1], f[i * 3 + 2]]); if (cache.size < 1 << 20) cache.set(key, v); }
    return v;
  };
  let sum = 0;
  const kept = [];
  for (let i = 0; i < n; i++) {
    if (mask?.[i]) continue;
    map[i] = de2000(labOf(fa, i), labOf(fb, i));
    sum += map[i];
    kept.push(map[i]);
  }
  if (!kept.length) return { map, mean: 0, p95: 0 };
  const sorted = Float32Array.from(kept).sort();
  return { map, mean: sum / kept.length, p95: sorted[Math.floor(0.95 * (kept.length - 1))] };
}

/** A heat map of a ΔE map: black at 0, red at 5, yellow at 10 and above. */
export function heat(map, width, height) {
  const data = Buffer.alloc(width * height * 4);
  for (let i = 0; i < map.length; i++) {
    const t = Math.min(1, map[i] / 10);
    data[i * 4] = Math.round(255 * Math.min(1, t * 2));
    data[i * 4 + 1] = Math.round(255 * Math.max(0, t * 2 - 1));
    data[i * 4 + 2] = 0;
    data[i * 4 + 3] = 255;
  }
  return { width, height, data };
}

/** A mask (Uint8Array, 1 = excluded) of a `width` x `height` crop from logical rects scaled by `dpr`. */
export function maskOf(width, height, rects, dpr = 1) {
  const m = new Uint8Array(width * height);
  for (const [x, y, w, h] of rects ?? []) {
    const x0 = Math.max(0, Math.floor(x * dpr)), y0 = Math.max(0, Math.floor(y * dpr));
    const x1 = Math.min(width, Math.ceil((x + w) * dpr)), y1 = Math.min(height, Math.ceil((y + h) * dpr));
    for (let yy = y0; yy < y1; yy++) m.fill(1, yy * width + x0, Math.max(yy * width + x0, yy * width + x1));
  }
  return m;
}

/** The share of a mask's pixels that are excluded, 0..1. */
export const coverage = m => (m.length ? m.reduce((s, v) => s + v, 0) / m.length : 0);

/** The mask drawn over an image as a grey hatch (for contact sheets). */
export function hatch(img, mask) {
  if (!mask) return img;
  const data = Buffer.from(img.data);
  for (let i = 0; i < mask.length; i++) {
    if (!mask[i] || ((i % img.width) + Math.floor(i / img.width)) % 6 > 1) continue;
    data[i * 4] = data[i * 4 + 1] = data[i * 4 + 2] = 128;
    data[i * 4 + 3] = 255;
  }
  return { ...img, data };
}
