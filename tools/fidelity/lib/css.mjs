// A small CSS reader for the design's stylesheets: rules (with the at-rule they sit in), their
// declarations and the line each declaration is on. Enough for the design's own CSS (no
// nesting beyond one at-rule level, no escaped braces); not a general CSS parser.

/** Comments blanked out, keeping every newline so line numbers stay. */
function stripComments(text) {
  return text.replace(/\/\*[\s\S]*?\*\//g, m => m.replace(/[^\n]/g, ' '));
}

/**
 * Parses `text` into `[{ at, selector, decls: [{ prop, value, line }], line }]`; `at` is the
 * enclosing at-rule's prelude (`@keyframes wb-in`, `@media (max-width: 700px)`) or ''.
 * Keyframe steps come out as rules whose selector is the step (`from`, `28%`).
 */
export function parseCss(text) {
  const src = stripComments(text);
  const rules = [];
  let i = 0;
  const lineAt = pos => src.slice(0, pos).split('\n').length;
  // Reads a block body from `i` (just after `{`) to its matching `}`; returns the end index.
  function block(start, at) {
    let j = start;
    while (j < src.length) {
      const close = src.indexOf('}', j);
      const open = src.indexOf('{', j);
      if (close < 0) throw new Error('unbalanced css');
      if (open >= 0 && open < close) {
        // a nested rule: its prelude is the text since the last `;` or `}` or `{`
        let p = open - 1;
        while (p >= j && !';{}'.includes(src[p])) p--;
        const prelude = src.slice(p + 1, open).trim();
        if (prelude.startsWith('@')) {
          j = block(open + 1, prelude) + 1;
        } else {
          const end = src.indexOf('}', open);
          rules.push({ at, selector: prelude, decls: decls(open + 1, end), line: lineAt(open) });
          j = end + 1;
        }
      } else {
        return close;
      }
    }
    return j;
  }
  function decls(start, end) {
    const out = [];
    let k = start;
    for (const part of src.slice(start, end).split(';')) {
      const colon = part.indexOf(':');
      if (colon > 0) {
        const prop = part.slice(0, colon).trim();
        const value = part.slice(colon + 1).trim().replace(/\s*!important$/, '');
        const lead = part.length - part.trimStart().length;
        if (prop && value) out.push({ prop, value, line: lineAt(k + lead) });
      }
      k += part.length + 1;
    }
    return out;
  }
  while (i < src.length) {
    const open = src.indexOf('{', i);
    if (open < 0) break;
    const prelude = src.slice(i, open).trim();
    if (prelude.startsWith('@')) {
      // `@property` and other one-block at-rules hold declarations, not rules
      if (prelude.startsWith('@property') || prelude.startsWith('@font-face')) {
        const end = src.indexOf('}', open);
        rules.push({ at: prelude, selector: '', decls: decls(open + 1, end), line: lineAt(open) });
        i = end + 1;
      } else {
        i = block(open + 1, prelude) + 1;
      }
    } else {
      const end = src.indexOf('}', open);
      rules.push({ at: '', selector: prelude, decls: decls(open + 1, end), line: lineAt(open) });
      i = end + 1;
    }
  }
  return rules;
}

/** Splits `s` on `sep` (a single character) outside parentheses. */
export function splitTop(s, sep) {
  const out = [];
  let depth = 0, cur = '';
  for (const c of s) {
    if (c === '(') depth++;
    if (c === ')') depth--;
    if (depth === 0 && (sep === ' ' ? /\s/.test(c) : c === sep)) {
      if (cur.trim()) out.push(cur.trim());
      cur = '';
    } else cur += c;
  }
  if (cur.trim()) out.push(cur.trim());
  return out;
}
