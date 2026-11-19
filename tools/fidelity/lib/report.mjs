// The differences between the generated kit values and what wubble uses today, for the lanes
// that switch call sites over. Read only: names are matched where wubble kept the design's.
import { readFileSync, readdirSync } from 'node:fs';
import { resolve } from 'node:path';

const num = s => Number(String(s).replace(/px|ms/, ''));

/** `pub mod NAME { pub const key: T = v; }` and `pub const NAME: T = v;` in wubble-core's sources. */
function rustConsts(REPO) {
  const out = new Map(); // "MOD::key" or "NAME" -> { value, file }
  const dir = resolve(REPO, 'crates/wubble-core/src');
  const walk = d => readdirSync(d, { withFileTypes: true }).flatMap(e => (e.isDirectory() ? walk(resolve(d, e.name)) : e.name.endsWith('.rs') && e.name !== 'kit_gen.rs' ? [resolve(d, e.name)] : []));
  for (const f of walk(dir)) {
    const text = readFileSync(f, 'utf8');
    const rel = f.slice(REPO.length + 1);
    for (const m of text.matchAll(/pub mod (\w+) \{([^}]*)\}/g)) {
      for (const c of m[2].matchAll(/pub const (r#)?(\w+): [^=]+= ([^;]+);/g)) out.set(`${m[1]}::${c[2]}`, { value: c[3].trim(), file: rel });
    }
    for (const c of text.matchAll(/^pub const (\w+): [^=]+= ([^;]+);/gm)) out.set(c[1], { value: c[2].trim(), file: rel });
  }
  return out;
}

function flat(prefix, v, out) {
  if (v && typeof v === 'object' && !Array.isArray(v)) for (const [k, x] of Object.entries(v)) flat(`${prefix}::${k}`, x, out);
  else out.set(prefix, v);
  return out;
}
const same = (a, b) => {
  const nums = s => String(s).match(/-?\d*\.?\d+/g)?.map(Number) ?? [];
  const x = Array.isArray(a) ? a.flat(3) : [a];
  const y = nums(b);
  return x.length === y.length && x.every((v, i) => typeof v === 'number' && Math.abs(v - y[i]) < 1e-9);
};

/** `name: value;` of a Slint global in ui/tokens.slint. */
function slintTokens(REPO) {
  let text;
  try { text = readFileSync(resolve(REPO, 'crates/wubble-ui/ui/tokens.slint'), 'utf8'); } catch { return null; }
  const out = new Map();
  let g = null;
  for (const ln of text.split('\n')) {
    const m = /^export global (\w+)/.exec(ln);
    if (m) g = m[1];
    for (const p of ln.matchAll(/out property <[^>]+> (\w+): ([^;]+);/g)) out.set(`${g}.${p[1]}`, p[2].trim());
  }
  return out;
}

export function report({ tokens, physics, css, files, REPO, commit, genTables, slintValue }) {
  const lines = [`# Generated kit values vs wubble today (design ${commit})`, '',
    'Written by `node tools/fidelity/extract.mjs --report`. Each row is a value the later lanes must',
    'switch to the generated one (or record as a wubble change). Call sites were not switched in L0.', ''];
  // Slint tokens.slint (the port copy) vs generated tokens
  const st = slintTokens(REPO) ?? new Map();
  const rows = [];
  const hex = c => c.toUpperCase();
  for (const [k, v] of Object.entries(tokens.color)) if (st.has(`Palette.${k}`) && hex(st.get(`Palette.${k}`)) !== hex(v)) rows.push([`Palette.${k}`, st.get(`Palette.${k}`), v]);
  for (const [k, v] of Object.entries(tokens.radius)) if (st.has(`Radius.${k}`) && num(st.get(`Radius.${k}`)) !== v) rows.push([`Radius.${k}`, st.get(`Radius.${k}`), `${v}px`]);
  for (const [k, v] of Object.entries(tokens.chrome)) if (typeof v === 'number' && st.has(`Chrome.${k}`) && num(st.get(`Chrome.${k}`)) !== v) rows.push([`Chrome.${k}`, st.get(`Chrome.${k}`), `${v}px`]);
  for (const [k, g] of Object.entries(tokens.glass)) {
    const cur = st.get(`Glass.${k}`) ?? '';
    rows.push([`Glass.${k}`, cur, `blur ${g.blur}, saturate ${g.saturate}, brightness ${g.brightness}, fill ${g.fill}, shadow 0 ${g.shadow[0]} ${g.shadow[1]}`]);
  }
  rows.push(['Glass.thickOverContent', st.get('Glass.thickOverContent'), tokens.thickOverContent]);
  const wub = css.find(c => c.name === 'WubbleCss').props;
  const pick = id => wub.find(p => p.id === id);
  for (const t of ['thin', 'regular', 'thick']) {
    rows.push([`Glass rims (${t})`, `rimAll ${st.get('Glass.rimAll')}, rimTop ${st.get('Glass.rimTop')}`, `all ${slintValue(pick(`wb-${t}--box-shadow-shadow-1-color`))}, top ${slintValue(pick(`wb-${t}--box-shadow-shadow-2-color`))}`]);
  }
  rows.push(['Glass.specular / specularStop', `${st.get('Glass.specular')} / ${st.get('Glass.specularStop')}`, 'no specular band: top light w .035 to 0 at 50% (Glass.top-light)']);
  for (const [k, e] of Object.entries(tokens.motion.ease)) {
    const cur = st.get(`Motion.ease${k[0].toUpperCase()}${k.slice(1)}`);
    if (cur && !same(e, cur)) rows.push([`Motion.ease${k}`, cur, e.join(', ')]);
  }
  for (const [name, [k, c]] of Object.entries(physics.K)) {
    const ck = st.get(`Motion.${name}K`), cc = st.get(`Motion.${name}C`);
    if (num(ck) !== k || num(cc) !== c) rows.push([`Motion.${name}K/C`, `${ck}/${cc}`, `${k}/${c}`]);
  }
  if (st.size) {
    lines.push('## ui/tokens.slint (the port copy) vs gen/tokens.slint', '', '| Token | wubble | design |', '|---|---|---|');
    for (const r of rows) lines.push(`| ${r.join(' | ')} |`);
  } else lines.push('## ui/tokens.slint', '', 'Deleted (L2): every Slint token comes from gen/tokens.slint.');
  // Rust constants
  const rc = rustConsts(REPO);
  const gen = new Map();
  for (const [n, v] of Object.entries(genTables)) flat(n, v, gen);
  const rrows = [], matched = [];
  for (const [k, v] of gen) {
    let cur = rc.get(k);
    if (!cur) continue;
    // a reference to another constant (`crate::motion::K::follow`) or a sum of literals
    const ref = /^(?:[\w:]*::)?(\w+::\w+)$/.exec(cur.value);
    if (ref && rc.has(ref[1])) cur = { ...cur, value: rc.get(ref[1]).value };
    else if (/^[\d.\s+*-]+$/.test(cur.value)) cur = { ...cur, value: String(Function(`return ${cur.value}`)()) };
    if (same(v, cur.value)) matched.push(k); else rrows.push([k, `${cur.value} (${cur.file})`, JSON.stringify(v)]);
  }
  lines.push('', '## wubble-core constants vs kit_gen.rs (same names)', '', `Equal: ${matched.join(', ') || 'none'}.`, '');
  if (rrows.length) { lines.push('| Constant | wubble | design |', '|---|---|---|'); for (const r of rrows) lines.push(`| ${r.join(' | ')} |`); } else lines.push('No difference among the constants with the same name.');
  const absent = [...gen.keys()].filter(k => !rc.has(k));
  lines.push('', `Generated constants with no same-named copy in wubble-core (new to wubble, or held under another`, `name; the lane that ports the machine checks them): ${absent.length}.`, '', absent.map(k => `\`${k}\``).join(', '));
  lines.push('', '## ui/app-tokens.slint `Shell` (now kit/derived.slint)', '',
    'Every `Shell` property is a wubble value. Those citing a design rule (`.wb-*`, `.st-*`, `.tp-*`) are',
    'to become references to `gen/css.slint` or `gen/tokens.slint`; the rest (Ultra Reactivate overlays,',
    'wubble-only surfaces) are re-expressed from design tokens per `wubble-only.md`. The kit lock\'s',
    'baseline counts each remaining literal in derived.slint.');
  const shellPath = ['crates/wubble-ui/ui/kit/derived.slint', 'crates/wubble-ui/ui/app-tokens.slint'].map(p => resolve(REPO, p)).find(p => { try { readFileSync(p); return true; } catch { return false; } });
  const genValues = new Set();
  for (const c of css) for (const p of c.props) genValues.add(slintValue(p).toLowerCase());
  const tok = files['crates/wubble-ui/ui/kit/gen/tokens.slint'];
  for (const m of tok.matchAll(/: ([^;{}]+);/g)) genValues.add(m[1].trim().toLowerCase());
  const norm = v => v.replace(/\s+/g, ' ').replace(/rgba\((\d+), (\d+), (\d+), 1\)/, (_, r, g, b) => `#${[r, g, b].map(x => Number(x).toString(16).padStart(2, '0')).join('')}`).toLowerCase();
  const design = [], wubble = [];
  for (const m of readFileSync(shellPath, 'utf8').matchAll(/out property <(length|duration|color|float|int)> ([\w-]+): ([^;]+);/g)) {
    const v = norm(m[3].trim());
    if (!/^[-\d.#r]/.test(v)) continue; // an expression of other tokens
    (genValues.has(v) || genValues.has(v.replace(/^0\./, '0.')) ? design : wubble).push(`\`${m[2]}\` ${m[3].trim()}`);
  }
  lines.push('', `Shell literals whose value occurs in the generated kit (to become a reference): ${design.length}.`, '', design.join(', '), '',
    `Shell literals with no equal value anywhere in the design (wubble values: derive or get the owner's sign-off): ${wubble.length}.`, '', wubble.join(', '));
  return `${lines.join('\n')}\n`;
}
