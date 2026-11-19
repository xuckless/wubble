// The needle reader of the inline-literal manifest (lib/literals.mjs): `node --test tools/fidelity/test/`.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { literal } from '../lib/literals.mjs';

const SRC = [
  'const a = 1;',
  '  this.acc[i] = -180 * (this.ring[i] - target) - 6 * this.vel[i];',
  'const quads = [[1, 2], [3, 4]];',
  'step(this.cx, tx, 260, 10, dt); step(this.cy, ty, 260, 10, dt);',
].join('\n');

test('a needle reads its marked number and the line it is on', () => {
  assert.deepEqual(literal(SRC, 'this.acc[i] = -⟦180⟧ * (this.ring'), { value: 180, line: 2 });
  assert.deepEqual(literal(SRC, '- ⟦6⟧ * this.vel[i]'), { value: 6, line: 2 });
});

test('a needle reads a JSON array', () => {
  assert.deepEqual(literal(SRC, 'const quads = ⟦[[1, 2], [3, 4]]⟧;').value, [[1, 2], [3, 4]]);
});

test('a changed literal misses', () => {
  assert.throws(() => literal(SRC, 'this.acc[i] = -⟦181⟧ * (this.ring'), /not in/);
});

test('a needle that occurs twice is refused', () => {
  assert.throws(() => literal(SRC, '⟦260⟧, 10, dt'), /not unique/);
  assert.deepEqual(literal(SRC, 'tx, ⟦260⟧, 10, dt').value, 260);
});

test('a needle needs exactly one mark pair and a number', () => {
  assert.throws(() => literal(SRC, 'const a = 1;'), /needs one/);
  assert.throws(() => literal(SRC, '⟦const⟧ a = 1;'), /not a number/);
});
