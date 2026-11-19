// The DevToolsActivePort reader (no browser).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { parseActivePort } from '../lib/cdp.mjs';

test('DevToolsActivePort: line 1 is the port', () => {
  assert.equal(parseActivePort('41237\n/devtools/browser/0b1c-2d\n'), 41237);
  assert.equal(parseActivePort('9333'), 9333);
  assert.equal(parseActivePort(''), null);
  assert.equal(parseActivePort('0\n/x'), null);
  assert.equal(parseActivePort('abc'), null);
});
