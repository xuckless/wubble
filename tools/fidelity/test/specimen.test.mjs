// The specimen table's expansion into scenes, and source anchors (no browser).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { resolve, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { expand, anchorsOf, resolveSource, withSpecimens } from '../lib/specimen.mjs';

const here = dirname(fileURLToPath(import.meta.url));
const table = {
  pages: {
    touch: { url: '/t', viewport: [100, 200], ready: '.g', frozen: false, gl: 'gpu', stage: '.spec:has({root}) .stage', after: 600 },
    shell: { url: '/wubble/', gl: 'swiftshader', stage: '.wb-window', steps: [{ key: ',', ctrl: true }] },
  },
  controls: [
    { control: 'toggle', page: 'touch', root: '.tg', source: 'touch-toggle', elements: { track: '{root}', knob: '{root} .knob' },
      states: [{ state: 'on' }, { state: 'press', steps: [{ press: '{root}' }], after: 0 }],
      sequences: [{ name: 'flip', steps: [{ click: '{root}' }], frames: 60, track: ['knob'] }] },
    { control: 'ib', page: 'shell', root: '.ib', clip: ['.tip'], pad: 4, before: [{ type: 'x' }], states: [{ state: 'rest', wubble: 'kit-ib' }] },
  ],
};

test('a control expands into one still per state and one burst per sequence', () => {
  const s = expand(table);
  assert.deepEqual(s.map(x => x.name), ['spec-toggle--on', 'spec-toggle--press', 'spec-toggle--seq-flip', 'spec-ib--rest']);
  const [on, press, flip, ib] = s;
  assert.equal(on.url, '/t');
  assert.equal(on.gl, 'gpu');
  assert.equal(on.frame, '.spec:has(.tg) .stage');
  assert.deepEqual(on.clip, ['.tg']);
  assert.equal(on.pad, 8);
  assert.equal(on.after, 600);
  assert.deepEqual(on.viewport, [100, 200]);
  assert.equal(on.specimen.id, 'toggle/on');
  assert.deepEqual(on.specimen.elements, { track: '.tg', knob: '.tg .knob' });
  assert.equal(on.frames, undefined);
  assert.equal(press.after, 0);
  assert.deepEqual(press.steps, [{ press: '.tg' }]);
  assert.equal(flip.frames, 60);
  assert.deepEqual(flip.track, ['knob']);
  assert.equal(flip.specimen.sequence, 'flip');
  assert.deepEqual(ib.steps, [{ key: ',', ctrl: true }, { type: 'x' }]);
  assert.deepEqual(ib.clip, ['.ib', '.tip']);
  assert.equal(ib.pad, 4);
  assert.equal(ib.wubble, 'kit-ib');
  assert.equal(ib.gl, 'swiftshader');
});

test('an unknown page or tracked element throws', () => {
  assert.throws(() => expand({ pages: {}, controls: [{ control: 'x', page: 'nope', root: '.x' }] }), /unknown page/);
  assert.throws(() => expand({ ...table, controls: [{ ...table.controls[0], sequences: [{ name: 's', track: ['nope'] }] }] }), /unknown element/);
});

test('@source anchors and path#text resolve to file:line', () => {
  const texts = new Map([['pages/touch.tsx', 'a\n// @source touch-toggle\nfunction Toggle() {}\n'], ['browser/chrome.tsx', 'x\ny\n<button aria-label="Source">']]);
  const idx = { texts, anchors: anchorsOf(texts) };
  assert.equal(resolveSource(idx, 'touch-toggle'), 'pages/touch.tsx:2');
  assert.equal(resolveSource(idx, 'browser/chrome.tsx#aria-label="Source"'), 'browser/chrome.tsx:3');
  assert.equal(resolveSource(idx, 'gone'), null);
  assert.equal(resolveSource(idx, 'browser/nope.tsx#x'), null);
  assert.equal(resolveSource(idx, null), null);
});

test('the real table expands, tracks known elements, and only pages without WebGL ask for the GPU', () => {
  const spec = JSON.parse(readFileSync(resolve(here, '../scenes.json'), 'utf8'));
  const all = withSpecimens(spec, f => JSON.parse(readFileSync(resolve(here, '..', f), 'utf8'))).scenes;
  assert.ok(all.filter(s => s.specimen).length > 100);
  const names = new Set();
  for (const s of all) {
    assert.ok(!names.has(s.name), `${s.name} twice`);
    names.add(s.name);
    // the shell and settings run the liquid (core/gl/scene.ts): rendered wrong on the GPU
    if (s.gl === 'gpu') assert.ok(s.url.includes('/pages/'), `${s.name} loads the shell on the GPU`);
  }
  for (const c of ['button-pri', 'button-glass', 'button-ghost', 'iconbutton', 'toggle-lg', 'toggle-sm', 'slider', 'segmented', 'checkbox', 'swatches', 'menu', 'select', 'select-up', 'hold', 'confirm', 'search', 'kbd', 'scroller', 'seam-pill', 'tooltip']) {
    assert.ok(all.some(s => s.specimen?.control === c), `no specimen of ${c}`);
  }
});
