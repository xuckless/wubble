// Inline literals of the design's code, read by needle: the exact source text around the value,
// with the value wrapped in ⟦ ⟧ (`'this.acc[i] = -⟦180⟧ * (this.ring[i] - target)'`). The needle
// without its marks must occur exactly once in the file, so a changed value or a duplicated line
// fails the generator (the drift check a table gets from its name).

export const OPEN = '⟦';
export const CLOSE = '⟧';

/**
 * The value a needle marks in `text`: `{ value, line }`. The marked text is a number or a JSON
 * array of numbers. Throws when the needle is malformed, missing or not unique.
 */
export function literal(text, needle, where = 'the file') {
  const a = needle.indexOf(OPEN), b = needle.indexOf(CLOSE);
  if (a < 0 || b < a || needle.indexOf(OPEN, a + 1) >= 0 || needle.indexOf(CLOSE, b + 1) >= 0) {
    throw new Error(`needle needs one ${OPEN}value${CLOSE}: ${needle}`);
  }
  const marked = needle.slice(a + 1, b);
  const plain = needle.slice(0, a) + marked + needle.slice(b + 1);
  const at = text.indexOf(plain);
  if (at < 0) throw new Error(`needle not in ${where} (the design changed?): ${plain}`);
  if (text.indexOf(plain, at + 1) >= 0) throw new Error(`needle not unique in ${where}: ${plain}`);
  const value = marked.trim().startsWith('[') ? JSON.parse(marked) : Number(marked);
  if (typeof value === 'number' && !Number.isFinite(value)) throw new Error(`not a number: ${marked} (${plain})`);
  const line = text.slice(0, at + a).split('\n').length;
  return { value, line };
}
