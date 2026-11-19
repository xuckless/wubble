// Gating, anatomy rects and the scores.csv round trip (pure).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { gateOf, whyOf, wubbleRect, summarize, toCsv, parseScores, passes } from '../lib/scores.mjs';

test('gate: the crop decides, else the scene, else gating', () => {
  assert.equal(gateOf({}, {}), true);
  assert.equal(gateOf({ gate: false }, {}), false);
  assert.equal(gateOf({ gate: false }, { gate: true }), true);
  assert.equal(whyOf({ why: 's' }, { why: 'c' }), 'c');
});

test('informational pairs are not counted with the gating ones', () => {
  const rows = [
    { ssim: 0.95, mean: 2, gate: true }, { ssim: 0.8, mean: 2, gate: true }, { ssim: 0.95, mean: 4, gate: true },
    { ssim: 0.99, mean: 1, gate: false }, { ssim: 0.5, mean: 9, gate: false }, { note: 'no web capture' },
  ];
  assert.deepEqual(summarize(rows), { gating: 3, gatingPass: 1, info: 2, infoPass: 1, unscored: 1 });
  assert.equal(passes({ ssim: 0.9, mean: 3 }), true);
});

test('@id crops take wubble rects from the gallery scene\'s rects file', () => {
  const files = [{ scene: 'settings', elements: { 'Settings::row-toggle': { x: 300, y: 120, w: 700, h: 64 } } }];
  const scene = { wubble: 'settings' };
  assert.deepEqual(wubbleRect({ wubble: '@Settings::row-toggle' }, scene, files), [300, 120, 700, 64]);
  assert.match(wubbleRect({ wubble: '@Settings::nope' }, scene, files), /no wubble rect for @Settings::nope$/);
  assert.match(wubbleRect({ wubble: '@Settings::row-toggle' }, { wubble: 'settings-3' }, files), /no rects file/);
  assert.deepEqual(wubbleRect({ wubble: [1, 2, 3, 4] }, scene, files), [1, 2, 3, 4]);
  assert.equal(wubbleRect({}, scene, files), undefined);
});

test('scores.csv round-trips, and an L0 file reads as gating', () => {
  const rows = [{ scene: 'a', crop: 'b', kind: 'direct', dpr: 1, ssim: 0.91234, mean: 2.5, p95: 6, size: '10x10', note: 'x, "y"', gate: false, masked: 0.25, why: 'w' }];
  const back = parseScores(toCsv(rows))[0];
  assert.equal(back.gate, false);
  assert.equal(back.masked, 0.25);
  assert.equal(back.note, "x, 'y'");
  assert.equal(back.ssim, 0.9123);
  const l0 = parseScores('scene,crop,kind,dpr,ssim,de_mean,de_p95,size,note\nsidebar-extended,window,direct,1,0.8309,4.38,10.32,1288x816,""\nx,*,,2,,,,,"no web capture"\n');
  assert.equal(l0[0].gate, true);
  assert.equal(l0[0].mean, 4.38);
  assert.equal(l0[1].ssim, null);
  assert.equal(l0[1].note, 'no web capture');
});
