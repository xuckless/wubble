// The web cache's key and pruning (no browser, no git): `node --test tools/fidelity/test/`.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { canonical, sceneHash, cacheKey, pruneKeep, recordRun } from '../lib/cache.mjs';

const spec = { viewport: [1360, 880], viewportByDpr: { 2: [1280, 760] }, frame: '.wb-window', settle: 2000, after: 600, frozen: true };
const scene = { name: 'command-bar', url: '/wubble/', steps: [{ key: 't', ctrl: true }], crops: [{ name: 'window' }] };
const key = (over = {}) => cacheKey({ design: 'abc', harness: 'h1', scene: sceneHash(scene, spec), dpr: 1, gl: 'swiftshader', ...over });

test('canonical JSON ignores key order', () => {
  assert.equal(canonical({ b: 1, a: [{ d: 2, c: 3 }] }), canonical({ a: [{ c: 3, d: 2 }], b: 1 }));
  assert.notEqual(canonical({ a: [1, 2] }), canonical({ a: [2, 1] }));
});

test('the key changes with the design, the harness, the scene, the DPR and the renderer', () => {
  const k = key();
  assert.notEqual(key({ design: 'abd' }), k);
  assert.notEqual(key({ design: 'abc+dirty-123' }), k);
  assert.notEqual(key({ harness: 'h2' }), k);
  assert.notEqual(key({ dpr: 2 }), k);
  assert.notEqual(key({ gl: 'gpu' }), k);
  assert.notEqual(key({ scene: sceneHash({ ...scene, steps: [{ key: 't', ctrl: true }, { advance: 10 }] }, spec) }), k);
  // a global the scene inherits re-captures it
  assert.notEqual(key({ scene: sceneHash(scene, { ...spec, settle: 1000 }) }), k);
});

test('the key ignores what only the scorer reads and the order of keys', () => {
  const k = key();
  const reordered = { crops: [{ name: 'window' }], steps: [{ ctrl: true, key: 't' }], url: '/wubble/', name: 'command-bar' };
  assert.equal(key({ scene: sceneHash(reordered, spec) }), k);
  assert.equal(key({ scene: sceneHash({ ...scene, wubble: 'command-bar', surface: 'Command bar', gate: false, why: 'x' }, spec) }), k);
  assert.equal(key({ scene: sceneHash({ ...scene, crops: [{ name: 'window', wubble: '@X::y', gate: false }] }, spec) }), k);
  // a mask is read by the capture (it measures the mask's rects)
  assert.notEqual(key({ scene: sceneHash({ ...scene, crops: [{ name: 'window', mask: [{ web: '.x' }] }] }, spec) }), k);
});

test('prune keeps the newest key of each task and the keys of the last runs', () => {
  let index = { entries: [] };
  index = recordRun(index, [{ key: 'k1', task: 'a@1' }, { key: 'k2', task: 'b@1' }], { run: 'r1', time: 1, design: 'd' });
  index = recordRun(index, [{ key: 'k3', task: 'a@1' }, { key: 'k2', task: 'b@1' }], { run: 'r2', time: 2, design: 'd' });
  index = recordRun(index, [{ key: 'k4', task: 'a@1' }], { run: 'r3', time: 3, design: 'd' });
  assert.deepEqual([...pruneKeep(index, 2)].sort(), ['k2', 'k3', 'k4']);
  assert.deepEqual([...pruneKeep(index, 1)].sort(), ['k2', 'k4']);
  // a key used again is recorded once, with its latest run
  assert.equal(index.entries.filter(e => e.key === 'k2').length, 1);
});
