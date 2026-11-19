// The DOM readers behind rects.json and curve.json (pure parts; the page runs the same source).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { parseMatrix, parseScale, parseTranslate, opacityProduct, sampler, rectsFile, curveFile } from '../lib/curve.mjs';

test('transform matrices parse to [a, b, c, d, e, f]', () => {
  assert.deepEqual(parseMatrix('none'), [1, 0, 0, 1, 0, 0]);
  assert.deepEqual(parseMatrix('matrix(1.045, 0, 0, 0.978, 28.018, -3)'), [1.045, 0, 0, 0.978, 28.018, -3]);
  assert.deepEqual(parseMatrix('matrix3d(0.97, 0, 0, 0, 0, 1.04, 0, 0, 0, 0, 1, 0, 12, -4, 0, 1)'), [0.97, 0, 0, 1.04, 12, -4]);
  assert.deepEqual(parseMatrix('matrix(1e-05, 0, 0, 1, 0, 0)')[0], 1e-5);
  assert.equal(parseMatrix('rotate(3deg)'), null);
});

test('individual scale and translate parse', () => {
  assert.deepEqual(parseScale('none'), [1, 1]);
  assert.deepEqual(parseScale('0.97'), [0.97, 0.97]);
  assert.deepEqual(parseScale('1.08 0.88'), [1.08, 0.88]);
  assert.deepEqual(parseScale('50%'), [0.5, 0.5]);
  assert.deepEqual(parseTranslate('none'), [0, 0]);
  assert.deepEqual(parseTranslate('4px'), [4, 0]);
  assert.deepEqual(parseTranslate('4px -2.5px'), [4, -2.5]);
});

test('drawn opacity is the product through the ancestors', () => {
  assert.equal(opacityProduct([1, 0.5, 0.5]), 0.25);
  assert.equal(opacityProduct([NaN, 0.8]), 0.8);
  assert.equal(opacityProduct([]), 1);
});

test('the page sampler is self-contained source', () => {
  const f = new Function(`return ${sampler}`)();
  assert.equal(typeof f, 'function');
  assert.equal(f.length, 2);
});

test('rects and curve files have the documented shape', () => {
  const s = { root: [10, 20, 92, 52], elements: { knob: { rect: [5, 5, 42, 42], layout: [5, 5, 42, 42], opacity: 1, transform: [1, 0, 0, 1, 0, 0] }, gone: null } };
  const r = rectsFile('toggle-lg/on', s, 2);
  assert.deepEqual(r.elements.knob, { rect: [5, 5, 42, 42], layout: [5, 5, 42, 42], opacity: 1 });
  assert.equal(r.elements.gone, null);
  assert.equal(r.dpr, 2);
  const c = curveFile('toggle-lg/seq-flip', ['knob'], [s, s, null]);
  assert.equal(c.frames.length, 3);
  assert.equal(c.frames[1].t, 8.333);
  assert.deepEqual(c.frames[0].knob.transform, [1, 0, 0, 1, 0, 0]);
  assert.equal(c.frames[2].knob, null);
  assert.ok(Math.abs(c.dt - 1000 / 120) < 1e-9);
});
