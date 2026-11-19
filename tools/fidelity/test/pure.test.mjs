// Pure tests of the harness's geometry, metrics, PNG round trip and CSS reading (no timing, no
// browser): `node --test tools/fidelity/test/`.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { de2000, ssim, deltaE } from '../lib/metrics.mjs';
import { cropRects } from '../lib/geometry.mjs';
import { encode, decode, blank, crop } from '../lib/png.mjs';
import { parseCss, splitTop } from '../lib/css.mjs';
import { makeReader, readDecl } from '../lib/values.mjs';
import { parseStill, toCanvas } from '../lib/still.mjs';
import { readFileSync } from 'node:fs';

test('CIEDE2000 matches Sharma, Wu, Dalal 2005 test data', () => {
  // pairs 1, 7 and 17 of the paper's table
  assert.ok(Math.abs(de2000([50, 2.6772, -79.7751], [50, 0, -82.7485]) - 2.0425) < 1e-4);
  assert.ok(Math.abs(de2000([50, 0, 0], [50, -1, 2]) - 2.3669) < 1e-4);
  assert.ok(Math.abs(de2000([50, 2.5, 0], [73, 25, -18]) - 27.1492) < 1e-4);
});

test('identical images score SSIM 1 and ΔE 0; different ones less', () => {
  const a = blank(32, 32, [100, 120, 140, 255]);
  for (let i = 0; i < a.data.length; i += 4 * 7) a.data[i] = 20;
  assert.ok(Math.abs(ssim(a, a) - 1) < 1e-9);
  assert.equal(deltaE(a, a).mean, 0);
  const b = blank(32, 32, [140, 120, 100, 255]);
  assert.ok(ssim(a, b) < 1);
  assert.ok(deltaE(a, b).mean > 5);
});

test('a PNG round-trips', () => {
  const a = blank(5, 3, [1, 2, 3, 4]);
  a.data[7] = 200;
  const b = decode(encode(a));
  assert.deepEqual([b.width, b.height], [5, 3]);
  assert.deepEqual([...b.data], [...a.data]);
  assert.deepEqual([...crop(a, [1, 0, 2, 1]).data], [1, 2, 3, 200, 1, 2, 3, 4]);
});

test('a direct crop takes the same logical rect on both sides, scaled by the DPR', () => {
  const meta = { frame: [36, 32, 100, 80], crops: { row: [8, 10, 50, 20] } };
  const img = { width: 200, height: 160 };
  assert.deepEqual(cropRects(meta, { name: 'row', web: '.r' }, 2, img, img), [[16, 20, 100, 40], [16, 20, 100, 40]]);
  assert.deepEqual(cropRects(meta, { name: 'window' }, 1, img, img)[0], [0, 0, 100, 80]);
  assert.deepEqual(cropRects(meta, { name: 'row', web: '.r', wubble: [0, 0, 10, 10] }, 1, img, img)[1], [0, 0, 10, 10]);
  assert.equal(cropRects(meta, { name: 'gone', web: '.g' }, 1, img, img), null);
  // clamped to the image
  assert.deepEqual(cropRects({ frame: [0, 0, 10, 10], crops: { a: [90, 0, 50, 5] } }, { name: 'a', web: '.a' }, 1, img, { width: 100, height: 10 })[1], [90, 0, 10, 5]);
});

test('CSS declarations read as typed values with their lines', () => {
  const rules = parseCss('.a { height: 36px;\n  box-shadow: inset 0 1px 0 rgb(255 255 255 / .2), 0 6px 14px rgb(0 0 0 / .28); }\n@keyframes k { 28% { transform: scale(1.3, .78); } }');
  assert.equal(rules.length, 2);
  assert.equal(rules[0].decls[1].line, 2);
  assert.equal(rules[1].at, '@keyframes k');
  const R = makeReader({ palette: { ink: '#F2F4FA' }, ease: { out: [0.22, 1, 0.36, 1] }, radius: {}, space: {} });
  const shadow = Object.fromEntries(readDecl('box-shadow', rules[0].decls[1].value, R));
  assert.equal(shadow['-shadow-1-inset'].v, true);
  assert.equal(shadow['-shadow-2-blur'].v, 14);
  assert.deepEqual(shadow['-shadow-2-color'].v, { r: 0, g: 0, b: 0, a: 0.28 });
  const t = Object.fromEntries(readDecl('transition', 'opacity .15s, scale .2s var(--wb-ease-out)', R));
  assert.equal(t['-opacity-duration'].v, 150);
  assert.equal(t['-scale-easing'].v, 'cubic-bezier(0.22, 1, 0.36, 1)');
  assert.deepEqual(splitTop('a(b c) d', ' '), ['a(b c)', 'd']);
  assert.equal(readDecl('position', 'absolute', R), null);
});

test('the CSS ease-out table follows cubic-bezier(0, 0, .58, 1)', async () => {
  const { cubicBezier, easeTable, CSS_EASE_OUT } = await import('../lib/ease.mjs');
  const t = easeTable(CSS_EASE_OUT, 32);
  assert.equal(t.length, 33);
  assert.equal(t[0], 0);
  assert.equal(t[32], 1);
  // the bezier at parameter .5 is the point (.3425, .5)
  assert.ok(Math.abs(cubicBezier(CSS_EASE_OUT, 0.3425) - 0.5) < 1e-9);
  // linear interpolation of the table stays within 1e-3 of the curve
  let worst = 0;
  for (let i = 0; i <= 1000; i++) {
    const u = i / 1000, k = Math.min(31, Math.floor(u * 32)), f = u * 32 - k;
    worst = Math.max(worst, Math.abs(t[k] + (t[k + 1] - t[k]) * f - cubicBezier(CSS_EASE_OUT, u)));
  }
  assert.ok(worst < 1e-3, `worst ${worst}`);
});

test('a font rule reads as a text role over what it inherits', async () => {
  const { fontOf } = await import('../lib/text.mjs');
  const base = { family: 'ui', size: 14, weight: 400, lh: 1.45 };
  const d = (prop, value) => ({ prop, value });
  assert.deepEqual(fontOf([d('font-family', 'var(--wb-font-display)'), d('font-size', '19px'), d('font-weight', '600'), d('line-height', '1')], base), { family: 'display', size: 19, weight: 600, lh: 1 });
  assert.deepEqual(fontOf([d('font', '500 12px var(--wb-font-ui)')], base), { family: 'ui', size: 12, weight: 500, lh: 1.45 });
  assert.deepEqual(fontOf([d('font', 'inherit'), d('font-size', '17px')], base), { ...base, size: 17 });
  assert.deepEqual(fontOf([d('color', 'red')], base), base);
});

test('the still dye table parses and maps onto the desktop canvas', () => {
  const text = readFileSync(new URL('../../../crates/wubble-gfx/src/scene/still_dye.txt', import.meta.url), 'utf8');
  const blobs = parseStill(text);
  assert.ok(blobs.length >= 8);
  for (const b of blobs) assert.ok([0, 1, 2].includes(b.color) && b.radius > 0 && b.strength > 0);
  assert.throws(() => parseStill('0.1 0.2 0'));
  const b = { fx: 0.5, fy: 0.25, color: 0, radius: 0.02, strength: 1 };
  // the window filling the canvas: as wubble splats it (y up, radius as written)
  assert.deepEqual(toCanvas(b, { x: 0, y: 0, w: 800, h: 400 }, 800, 400), { x: 0.5, y: 0.75, radius: 0.02 });
  // the design's 1288x816 window at (36, 32) on its 1360x880 desktop
  const p = toCanvas(b, { x: 36, y: 32, w: 1288, h: 816 }, 1360, 880);
  assert.ok(Math.abs(p.x - (36 + 644) / 1360) < 1e-12);
  assert.ok(Math.abs(p.y - (1 - (32 + 204) / 880)) < 1e-12);
  // the same blob in px: d^2 / (H^2 r') = d^2 / (h^2 r)
  assert.ok(Math.abs(p.radius * 880 ** 2 - 0.02 * 816 ** 2) < 1e-9);
});
