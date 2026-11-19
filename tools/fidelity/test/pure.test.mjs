// Pure tests of the harness's geometry, metrics, PNG round trip and CSS reading (no timing, no
// browser): `node --test tools/fidelity/test/`.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { de2000, ssim, deltaE, maskOf, coverage } from '../lib/metrics.mjs';
import { encode, decode, blank, crop } from '../lib/png.mjs';
import { parseCss, splitTop } from '../lib/css.mjs';
import { makeReader, readDecl } from '../lib/values.mjs';
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

test('masked pixels change neither SSIM nor ΔE; unmasked differences still score', () => {
  const a = blank(40, 20, [100, 120, 140, 255]);
  for (let i = 0; i < a.data.length; i += 4 * 5) a.data[i] = 30;
  // b differs from a only inside x 10..19 (a "URL text" region)
  const b = blank(40, 20, [0, 0, 0, 255]);
  b.data.set(a.data);
  for (let y = 0; y < 20; y++) for (let x = 10; x < 20; x++) b.data[(y * 40 + x) * 4 + 1] = 250;
  assert.ok(ssim(a, b) < 0.95);
  assert.ok(deltaE(a, b).mean > 5);
  const m = maskOf(40, 20, [[5, 0, 5, 10]], 2); // logical rect at 2x: x 10..19, y 0..19
  assert.equal(coverage(m), 0.25);
  assert.ok(Math.abs(ssim(a, b, m) - 1) < 1e-9);
  const d = deltaE(a, b, m);
  assert.equal(d.mean, 0);
  assert.equal(d.p95, 0);
  // a difference outside the mask still counts
  b.data[(5 * 40 + 30) * 4] = 255;
  assert.ok(ssim(a, b, m) < 1);
  assert.ok(deltaE(a, b, m).mean > 0);
  // everything masked: nothing to compare
  assert.equal(ssim(a, b, maskOf(40, 20, [[0, 0, 40, 20]])), 1);
});

test('a PNG round-trips', () => {
  const a = blank(5, 3, [1, 2, 3, 4]);
  a.data[7] = 200;
  const b = decode(encode(a));
  assert.deepEqual([b.width, b.height], [5, 3]);
  assert.deepEqual([...b.data], [...a.data]);
  assert.deepEqual([...crop(a, [1, 0, 2, 1]).data], [1, 2, 3, 200, 1, 2, 3, 4]);
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
