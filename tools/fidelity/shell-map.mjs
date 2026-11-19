// Proposes, for each literal of `kit/derived.slint`'s `Shell` global, the generated design value
// it should alias: a property of gen/tokens.slint or gen/css.slint with the same type and value,
// ranked by the CSS rule the nearby comment cites (`.wb-ib`, `.st-row`...) and by the words the
// two names share. Prints a table; writes nothing (the aliases are written by hand from it).
//   node tools/fidelity/shell-map.mjs [--accepted]
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { REPO } from './lib/design.mjs';

const ui = p => readFileSync(resolve(REPO, 'crates/wubble-ui/ui', p), 'utf8');
const norm = v => v.replace(/\s+/g, ' ').trim().toLowerCase();

/** `Global.name` -> { type, value } for every generated property. */
export function genProps() {
  const out = [];
  for (const f of ['kit/gen/tokens.slint', 'kit/gen/css.slint']) {
    let g = null;
    for (const ln of ui(f).split('\n')) {
      const m = /^export global (\w+)/.exec(ln);
      if (m) g = m[1];
      const p = /^\s*out property <(\w+)> ([\w-]+): ([^;{]+);/.exec(ln);
      if (p && g) out.push({ id: `${g}.${p[2]}`, name: p[2], type: p[1], value: norm(p[3]) });
    }
  }
  return out;
}

// The CSS properties a Shell name's last word can stand for (a radius is a border-radius...).
const KINDS = [
  [/(^|-)(font|title|text|label|note)$/, /--font-size$/],
  [/radius$/, /--border-radius$/],
  [/(^|-)(height|row|head)$/, /--(min-)?height$/],
  [/(^|-)(width|max)$/, /--(max-|min-)?width$/],
  [/(^|-)pad(-x|-y|-start|-end)?$/, /--padding(-left|-right|-top|-bottom)?$/],
  [/(gap|above|below|in|top|bottom|end)$/, /--(gap(-column|-row)?|margin(-left|-right|-top|-bottom)?|padding(-left|-right|-top|-bottom)?)$/],
  [/shadow-y$/, /-shadow-\d-y$/],
  [/shadow-blur$/, /-shadow-\d-blur$/],
  [/(shadow|shade)$/, /-shadow-\d-color$/],
  [/(highlight|rim)$/, /-shadow-\d-color$/],
  [/(fill|hover|card)$/, /--background(-color)?$/],
  [/-ms$|-in$|-out$/, /-duration$/],
  [/icon$/, /--(width|height|font-size)$/],
];
const kindOk = (name, gen) => {
  const k = KINDS.find(([n]) => n.test(name));
  return !k || k[1].test(gen);
};
const slugSel = s => s.replace(/^\./, '').replace(/[^A-Za-z0-9]+/g, '-').toLowerCase();
const words = s => new Set(s.toLowerCase().split(/[^a-z0-9]+/).filter(w => w.length > 1));

export function proposals() {
  const gen = genProps();
  const rows = [];
  let cites = [];
  for (const ln of ui('kit/derived.slint').split('\n')) {
    if (/^\s*\/\//.test(ln)) {
      const found = [...ln.matchAll(/\.((?:wb|st|tp)-[\w-]+)/g)].map(m => slugSel(m[1]));
      cites = [...cites, ...found];
      continue;
    }
    if (!ln.trim()) { cites = []; continue; }
    const p = /^\s*out property <(\w+)> ([\w-]+): ([^;]+);(.*)$/.exec(ln);
    if (!p) continue;
    const block = cites;
    const [, type, name, raw, tail] = p;
    const local = [...block, ...[...tail.matchAll(/\.((?:wb|st|tp)-[\w-]+)/g)].map(m => slugSel(m[1]))];
    const value = norm(raw);
    if (!/^[-\d.#r]/.test(value)) continue;
    const cands = gen.filter(g => g.type === type && g.value === value && g.name.includes('--') && kindOk(name, g.name)).map(g => {
      let score = 0;
      for (const c of local) if (g.name.startsWith(`${c}--`) || g.name.startsWith(`wb-${c}--`)) score += 10;
      const a = words(name), b = words(g.name);
      for (const w of a) if (b.has(w)) score += 1;
      return { ...g, score };
    }).sort((x, y) => y.score - x.score);
    const best = cands[0];
    const accepted = best && best.score >= 10 && (cands.length < 2 || cands[1].score < best.score);
    rows.push({ name, type, value, best, accepted, n: cands.length });
  }
  return rows;
}

if (import.meta.url === `file://${process.argv[1]}`) {
  const rows = proposals();
  const only = process.argv.includes('--accepted');
  for (const r of rows) {
    if (only && !r.accepted) continue;
    console.log(`${r.accepted ? 'ALIAS' : '-    '}\t${r.name}\t${r.value}\t${r.best ? `${r.best.id} (${r.best.score}, of ${r.n})` : 'no design equal'}`);
  }
  console.error(`shell-map: ${rows.filter(r => r.accepted).length} accepted of ${rows.length} literals`);
}
