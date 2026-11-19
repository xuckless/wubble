// Geometry comparison of specimens (pure).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { compareRects, normalize, formatRows } from '../lib/rects.mjs';

const web = { specimen: 'toggle-lg/on', elements: {
  track: { rect: [0, 0, 89.24, 50.44], layout: [0, 0, 92, 52], opacity: 1 },
  knob: { rect: [45, 5, 42, 42], layout: [45, 5, 42, 42], opacity: 1 },
  bubble: null,
} };
const wub = (over = {}) => ({ specimen: 'toggle-lg/on', scale: 1, root: 'Gallery::toggle-lg', elements: { track: { x: 0, y: 0, w: 92, h: 52, opacity: 1 }, knob: { x: 45, y: 5, w: 42, h: 42 }, ...over } });

test('equal geometry passes; the layout box is the default', () => {
  const r = compareRects(web, wub());
  assert.equal(r.failed, false);
  assert.deepEqual(r.rows.map(x => x.status), ['ok', 'ok', 'absent']);
  // against the drawn box the transformed track is off
  assert.equal(compareRects(web, wub(), { use: 'rect' }).failed, true);
});

test('deltas are wubble minus web; exactly the tolerance passes', () => {
  const r = compareRects(web, wub({ knob: { x: 46, y: 4, w: 42, h: 43 } }));
  const k = r.rows.find(x => x.name === 'knob');
  assert.deepEqual([k.dx, k.dy, k.dw, k.dh], [1, -1, 0, 1]);
  assert.equal(k.status, 'ok');
  assert.equal(compareRects(web, wub({ knob: { x: 46.01, y: 5, w: 42, h: 42 } })).failed, true);
  assert.equal(compareRects(web, wub({ knob: { x: 47, y: 5, w: 42, h: 42 } }), { tol: 2 }).failed, false);
});

test('a missing element fails, an extra one is only reported', () => {
  const missing = compareRects(web, { specimen: 'x', elements: { track: { x: 0, y: 0, w: 92, h: 52 } } });
  assert.equal(missing.failed, true);
  assert.equal(missing.rows.find(x => x.name === 'knob').status, 'missing');
  const extra = compareRects(web, wub({ glow: { x: 0, y: 0, w: 1, h: 1 } }));
  assert.equal(extra.failed, false);
  assert.equal(extra.rows.find(x => x.name === 'glow').status, 'extra');
});

test('an export at a scale factor is divided back to logical px', () => {
  const r = compareRects(web, { scale: 2, elements: { track: { x: 0, y: 0, w: 184, h: 104 }, knob: { x: 90, y: 10, w: 84, h: 84 } } });
  assert.equal(r.failed, false);
});

test('normalize reads both file shapes; the table names each element', () => {
  assert.deepEqual(normalize(web).knob, { x: 45, y: 5, w: 42, h: 42, opacity: 1 });
  assert.deepEqual(normalize(wub()).track, { x: 0, y: 0, w: 92, h: 52, opacity: 1 });
  const text = formatRows('toggle-lg/on', compareRects(web, wub()), 1);
  assert.match(text, /toggle-lg\/on: ok/);
  assert.match(text, /knob/);
});
