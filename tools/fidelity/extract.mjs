#!/usr/bin/env node
// The kit generator: reads the design's web sources (tokens.ts, physics.ts, the CSS, the icon
// tables, the motion constant tables) and writes the generated kit files. Nobody edits those by
// hand: change the design, then run this.
//
//   node tools/fidelity/extract.mjs                 write the generated files
//   node tools/fidelity/extract.mjs --check         fail (exit 1) when a generated file is stale
//                                                   against the clone (or its commit moved)
//   node tools/fidelity/extract.mjs --report FILE   also write the differences between the
//                                                   generated values and the ones wubble uses
//   node tools/fidelity/extract.mjs --coverage INV  also list the inventory's A-rows whose cited
//                                                   lines produced no generated value
//
// The clone is `$WUBBLE_DESIGN` or ~/Dev/wubble-wt/ref/the-wallpaper-project; it is only read.
import { readFileSync, writeFileSync, mkdirSync, existsSync, readdirSync } from 'node:fs';
import { resolve, dirname, relative } from 'node:path';
import { pathToFileURL } from 'node:url';
import { REPO, designRoot, designSrc, designCommit } from './lib/design.mjs';
import { parseCss } from './lib/css.mjs';
import { makeReader, readDecl } from './lib/values.mjs';
import { literal } from './lib/literals.mjs';
import { LITERALS } from './literals.mjs';

const args = process.argv.slice(2);
const flag = n => args.includes(n);
const opt = n => (args.includes(n) ? args[args.indexOf(n) + 1] : null);

const root = designRoot();
const SRC = designSrc(root);
const { commit, dirty } = designCommit(root);
const read = f => readFileSync(resolve(SRC, f), 'utf8');

// Every generated value remembers the design lines it came from (for --coverage and comments).
const used = new Map(); // file -> Set(line)
const use = (file, ...lines) => {
  if (!used.has(file)) used.set(file, new Set());
  for (const l of lines) used.get(file).add(l);
};
const lineOf = (file, needle, from = 0) => {
  const lines = read(file).split('\n');
  for (let i = from; i < lines.length; i++) if (lines[i].includes(needle)) return i + 1;
  return null;
};

// ── The design's own modules: tokens.ts imports cleanly under node's type stripping ─────────
const tokens = await import(pathToFileURL(resolve(SRC, 'core/tokens.ts')).href);

/** The balanced `{...}` or `[...]` starting at `text[at]`. */
function balanced(text, at) {
  const open = text[at], close = open === '{' ? '}' : ']';
  let depth = 0, q = null;
  for (let i = at; i < text.length; i++) {
    const c = text[i];
    if (q) { if (c === '\\') i++; else if (c === q) q = null; continue; }
    if (c === '/' && text[i + 1] === '/') { i = text.indexOf('\n', i); continue; }
    if (c === '/' && text[i + 1] === '*') { i = text.indexOf('*/', i) + 1; continue; }
    if (c === "'" || c === '"' || c === '`') q = c;
    else if (c === open) depth++;
    else if (c === close && --depth === 0) return text.slice(at, i + 1);
  }
  throw new Error('unbalanced table');
}

/** `const NAME = {...}` (or a scalar) in a design file, evaluated with the token scope. */
function table(file, name, scope = {}) {
  const text = read(file);
  const m = new RegExp(`(?:export )?const ${name}\\b[^=]*=\\s*`).exec(text);
  if (!m) throw new Error(`${name} not found in ${file}`);
  const at = m.index + m[0].length;
  const body = '{['.includes(text[at]) ? balanced(text, at) : text.slice(at, text.indexOf(';', at));
  const js = body
    .replace(/\s+as\s+const\b/g, '')
    .replace(/\s+as\s+\[[^\]]*\](\[\])?/g, '')
    .replace(/\s+satisfies\s+[^\n;]*/g, '');
  const env = { K: physics.K, BAND_K: physics.BAND_K, ...tokens, ...scope };
  const value = new Function(...Object.keys(env), `return (${js});`)(...Object.values(env));
  const first = text.slice(0, m.index).split('\n').length;
  const last = first + body.split('\n').length - 1;
  use(file, ...Array.from({ length: last - first + 1 }, (_, i) => first + i));
  return { value, file, first, last };
}

// physics.ts has a class with parameter properties (no type stripping), so its tables are read
// as text like every other file's.
const physics = {};
physics.K = (() => { const t = table('core/physics.ts', 'K', { K: {}, BAND_K: {} }); return t.value; })();
physics.BAND_K = table('core/physics.ts', 'BAND_K', { K: physics.K, BAND_K: {} }).value;

// ── CSS ────────────────────────────────────────────────────────────────────────────────────
const R = makeReader({ palette: tokens.color, ease: tokens.motion.ease, radius: tokens.radius, space: tokens.space });

const CSS = [
  ['WubbleCss', 'core/wubble.css'],
  ['BrowserCss', 'browser/browser.css'],
  ['SettingsCss', 'pages/settings.css'],
  ['TouchCss', 'pages/touch.css'],
  ['StartCss', 'pages/start.css'],
  ['RoomCss', 'pages/room.css'],
  ['FoundationsCss', 'pages/foundations.css'],
];

/** A selector (or at-rule step) as a Slint identifier part. */
function slug(s) {
  return s
    .replace(/::?/g, '-')
    .replace(/\[([^\]=]*)(?:="?([^\]"]*)"?)?\]/g, (_, a, b) => `-${a}${b ? `-${b}` : ''}`)
    .replace(/>/g, ' gt ')
    .replace(/\+/g, ' plus ')
    .replace(/~/g, ' sib ')
    .replace(/\*/g, 'all')
    .replace(/%/g, 'pct')
    .replace(/[.#]/g, ' ')
    .replace(/[^A-Za-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '')
    .toLowerCase();
}

/** Each CSS file as `{ name, file, props: [{ id, type, value, line, src }], unparsed: [...] }`. */
function cssTables() {
  return CSS.map(([name, file]) => {
    const props = [], unparsed = [], seen = new Set();
    let keywords = 0;
    for (const rule of parseCss(read(file))) {
      let base;
      if (rule.at.startsWith('@keyframes')) base = `${slug(rule.at.slice(10))}-k-${slug(rule.selector)}`;
      else if (rule.at.startsWith('@property')) base = `property-${slug(rule.at.slice(9))}`;
      else {
        const first = rule.selector.split(',')[0].trim();
        base = (rule.at ? `${slug(rule.at.replace(/^@\w+/, ''))}-` : '') + slug(first);
      }
      if (!/^[a-z]/.test(base)) base = `r-${base}`;
      for (const d of rule.decls) {
        if (d.prop.startsWith('--') || d.prop.startsWith('-webkit-') && d.prop !== '-webkit-backdrop-filter') continue;
        if (d.prop === '-webkit-backdrop-filter') continue; // the same as backdrop-filter
        const typed = readDecl(d.prop, d.value, R);
        if (!typed) {
          // a keyword (position, display, cursor...) is layout the kit writes as Slint layout
          if (/\d|#|rgb|var\(--wb-(?!font)/.test(d.value)) unparsed.push({ sel: rule.selector, at: rule.at, ...d });
          else keywords++;
          continue;
        }
        use(file, d.line);
        for (const [suffix, v] of typed) {
          let id = `${base}--${d.prop}${suffix}`;
          // Slint treats `-` and `_` alike; a later rule with the same name gets a number
          let n = 2, key = id;
          while (seen.has(key)) key = `${id}-${n++}`;
          seen.add(key);
          props.push({ id: key, ...v, line: d.line, src: `${rule.at ? `${rule.at} ` : ''}${rule.selector} { ${d.prop}: ${d.value} }` });
        }
      }
    }
    return { name, file, props, unparsed, keywords };
  });
}

// ── Slint output ───────────────────────────────────────────────────────────────────────────
const SLINT_TYPE = { length: 'length', duration: 'duration', color: 'color', float: 'float', easing: 'easing', string: 'string', bool: 'bool' };
const fmtNum = v => (Number.isInteger(v) ? String(v) : String(Number(v.toPrecision(10))));
function slintValue({ t, v }) {
  switch (t) {
    case 'length': return `${fmtNum(v)}px`;
    case 'duration': return `${fmtNum(v)}ms`;
    case 'float': return fmtNum(v);
    case 'bool': return String(v);
    case 'easing': return v;
    case 'string': return JSON.stringify(v);
    case 'color': {
      const { r, g, b, a } = v;
      if (a === 1) return `#${[r, g, b].map(x => x.toString(16).padStart(2, '0')).join('').toUpperCase()}`;
      return `rgba(${r}, ${g}, ${b}, ${fmtNum(a)})`;
    }
  }
  throw new Error(`no slint value for ${t}`);
}
const prop = (id, typed, cite) => `    out property <${SLINT_TYPE[typed.t]}> ${id}: ${slintValue(typed)};${cite ? ` // ${cite}` : ''}`;
const color = s => R.color(s);
const L = v => ({ t: 'length', v });
const F = v => ({ t: 'float', v });
const D = v => ({ t: 'duration', v });

function tokensSlint(css) {
  const t = tokens;
  const wub = css.find(c => c.name === 'WubbleCss');
  const W = id => {
    const p = wub.props.find(p => p.id === id);
    if (!p) throw new Error(`wubble.css: no ${id}`);
    return p;
  };
  const tl = n => lineOf('core/tokens.ts', n);
  const out = [];
  const cite = (file, line) => { use(file, line); return `${file}:${line}`; };
  out.push(`// The design's tokens: core/tokens.ts (colour, Spaces, radius, space, chrome, glass, type,`);
  out.push(`// motion), the glass tiers' CSS (core/wubble.css) and the spring presets (core/physics.ts).`);
  out.push(`// Names are the design's; the CSS rule tables are in css.slint, the icons in icons.slint.`);
  out.push('');
  out.push('export global Palette {');
  for (const [k, v] of Object.entries(t.color)) out.push(prop(k, color(v), cite('core/tokens.ts', lineOf('core/tokens.ts', `${k}: '`))));
  out.push('}', '');
  out.push('export struct SpaceToken { id: string, name: string, accent: color, dye: [color] }');
  out.push('export global Spaces {');
  const sl = tl('export const spaces');
  out.push(`    // ${cite('core/tokens.ts', sl)}`);
  out.push(`    out property <[SpaceToken]> spaces: [`);
  t.spaces.forEach((s, i) => out.push(`        { id: ${JSON.stringify(s.id)}, name: ${JSON.stringify(s.name)}, accent: ${slintValue(color(s.accent))}, dye: [${s.dye.map(d => slintValue(color(d))).join(', ')}] },${i === 0 ? '' : ''} // ${cite('core/tokens.ts', sl + 1 + i)}`));
  out.push('    ];', '}', '');
  out.push('export global Radius {');
  for (const [k, v] of Object.entries(t.radius)) out.push(prop(k, L(v), cite('core/tokens.ts', tl('export const radius'))));
  out.push('}', '');
  out.push('/// `space` in tokens.ts has numeric keys; Slint names need a letter: `s<key>`.');
  out.push('export global Space {');
  for (const [k, v] of Object.entries(t.space)) out.push(prop(`s${k}`, L(v), cite('core/tokens.ts', tl('export const space ='))));
  out.push('}', '');
  out.push('export global Chrome {');
  for (const [k, v] of Object.entries(t.chrome)) {
    if (typeof v === 'object') for (const [kk, vv] of Object.entries(v)) out.push(prop(`${k}-${kk}`, L(vv), cite('core/tokens.ts', tl(`${k}:`))));
    else out.push(prop(k, L(v), cite('core/tokens.ts', tl(`${k}:`) ?? tl(`, ${k}:`))));
  }
  out.push('}', '');
  out.push('/// A glass tier: tokens.ts `glass` (backdrop blur, saturate, brightness, fill, shadow) and the');
  out.push('/// tier rule in wubble.css (the shadow colour, the inset rim all round and the top light line).');
  out.push('export struct GlassTier { blur: length, saturate: float, brightness: float, fill: color, shadow-y: length, shadow-blur: length, shadow-color: color, rim-all: color, rim-top: color }');
  out.push('export global Glass {');
  for (const [k, g] of Object.entries(t.glass)) {
    const sel = `wb-${k}--box-shadow`;
    const fields = [
      `blur: ${slintValue(L(g.blur))}`, `saturate: ${fmtNum(g.saturate)}`, `brightness: ${fmtNum(g.brightness)}`,
      `fill: ${slintValue(color(g.fill))}`, `shadow-y: ${slintValue(L(g.shadow[0]))}`, `shadow-blur: ${slintValue(L(g.shadow[1]))}`,
      `shadow-color: ${slintValue(W(`${sel}-shadow-3-color`))}`, `rim-all: ${slintValue(W(`${sel}-shadow-1-color`))}`, `rim-top: ${slintValue(W(`${sel}-shadow-2-color`))}`,
    ];
    out.push(`    out property <GlassTier> ${k}: { ${fields.join(', ')} }; // ${cite('core/tokens.ts', tl(`  ${k}: {`))}, core/wubble.css:${W(`${sel}-shadow-3-color`).line}`);
  }
  out.push(prop('over-content', color(t.thickOverContent), cite('core/tokens.ts', tl('thickOverContent ='))));
  const top = W('wb-glass-after--background-stop-1-color');
  out.push(prop('top-light', top, `core/wubble.css:${top.line} (.wb-glass::after, to 0 at top-light-stop)`));
  out.push(prop('top-light-stop', W('wb-glass-after--background-stop-2-at'), `core/wubble.css:${top.line}`));
  const film = W('wb-film-before--padding');
  out.push(prop('film-width', film, `core/wubble.css:${film.line} (.wb-film::before padding)`));
  const filmOn = W('wb-film-hover-before--opacity');
  out.push(prop('film-opacity', filmOn, `core/wubble.css:${filmOn.line}`));
  out.push(prop('film-in', W('wb-film-hover-before--transition-opacity-duration'), `core/wubble.css:${filmOn.line}`));
  out.push(prop('film-out', W('wb-film-before--transition-opacity-duration'), `core/wubble.css:${W('wb-film-before--transition-opacity-duration').line}`));
  out.push(prop('film-swirl', W('wb-film-hover-before--animation-duration'), `core/wubble.css:${filmOn.line} (wb-film-swirl, 210deg to 570deg)`));
  out.push('}', '');
  out.push('export global Fonts {');
  out.push(`    // the first family of each stack, as the embedded fonts name themselves (no " Variable")`);
  for (const [k, v] of Object.entries(t.font)) {
    const fam = v.split(',')[0].replace(/['"]/g, '').replace(/ Variable$/, '').trim();
    out.push(`    out property <string> ${k}: ${JSON.stringify(fam)}; // ${cite('core/tokens.ts', tl(`  ${k}: "`))}`);
  }
  out.push('}', '');
  out.push('/// Type scale: [size px, weight, line-height].');
  out.push('export struct TypeRole { size: length, weight: int, line-height: float }');
  out.push('export global Type {');
  for (const [k, [s, w, lh]] of Object.entries(t.type)) out.push(`    out property <TypeRole> ${k}: { size: ${s}px, weight: ${w}, line-height: ${fmtNum(lh)} }; // ${cite('core/tokens.ts', tl('export const type') + 1)}`);
  out.push('}', '');
  out.push('export global Motion {');
  const ml = n => cite('core/tokens.ts', tl(`  ${n}:`));
  for (const [k, e] of Object.entries(t.motion.ease)) out.push(prop(`ease-${k.replace(/[A-Z]/g, c => `-${c.toLowerCase()}`)}`, { t: 'easing', v: `cubic-bezier(${e.join(', ')})` }, ml('ease')));
  const ein = table('core/physics.ts', 'EASE').value.in;
  out.push(prop('ease-in', R.easing(ein), `core/physics.ts:${lineOf('core/physics.ts', "in: 'cubic")} (EASE.in, CSS only)`));
  const o = t.motion.overlap;
  out.push(prop('overlap-in', D(o.in), ml('overlap')), prop('overlap-out', D(o.out), ml('overlap')), prop('overlap-blur', L(o.blur), ml('overlap')), prop('overlap-shift', L(o.shift), ml('overlap')), prop('overlap-stagger', D(o.stagger), ml('overlap')));
  out.push(prop('press-scale', F(t.motion.press.scale), ml('press')), prop('press-in', D(t.motion.press.ms), ml('press')));
  out.push(prop('press-out', W('wb-press--transition-transform-duration'), `core/wubble.css:${W('wb-press--transition-transform-duration').line} (.wb-press release)`));
  const dd = t.motion.dropdown;
  out.push(prop('dropdown-in', D(dd.in), ml('dropdown')), prop('dropdown-out', D(dd.out), ml('dropdown')), prop('dropdown-from', F(dd.from), ml('dropdown')));
  const kl = lineOf('core/physics.ts', 'export const K =');
  for (const [k, [kk, cc]] of Object.entries(physics.K)) {
    const at = cite('core/physics.ts', lineOf('core/physics.ts', `${k}: [`, kl));
    out.push(prop(`${k}-k`, F(kk), at), prop(`${k}-c`, F(cc), at));
  }
  const bl = lineOf('core/physics.ts', 'export const BAND_K');
  for (const [k, v] of Object.entries(physics.BAND_K)) out.push(prop(`band-${k}`, L(v), cite('core/physics.ts', bl)));
  out.push('}');
  return out.join('\n');
}

function cssSlint(css) {
  const out = [
    '// Every declaration of the design\'s stylesheets that reads as a typed value, one global per',
    '// file: `<selector>--<property>[-part]` (the first selector of a list; keyframe steps as',
    '// `<name>-k-<step>`; container and media queries as a prefix). Shorthands are split (`-top`,',
    '// `-shadow-1-y`, `-transform-duration`). Declarations not read are listed at the end of each',
    '// global, so nothing is lost silently.',
  ];
  for (const c of css) {
    out.push('', `// ${c.file}: ${c.props.length} values; ${c.unparsed.length} declarations with numbers not read (listed`, `// below); ${c.keywords} keyword declarations (layout: position, display, cursor...) left to Slint layout`);
    out.push(`export global ${c.name} {`);
    for (const p of c.props) out.push(prop(p.id, p, `${c.file.split('/').pop()}:${p.line}`));
    for (const u of c.unparsed) out.push(`    // not read, ${c.file.split('/').pop()}:${u.line}: ${u.at ? `${u.at} ` : ''}${u.sel} { ${u.prop}: ${u.value.replace(/\s+/g, ' ')} }`);
    out.push('}');
  }
  return out.join('\n');
}

/** `{ name: 'path' }` object literal bodies by regex (TSX can't be imported). */
function pathTable(file, startNeedle) {
  const text = read(file);
  const at = text.indexOf(startNeedle);
  if (at < 0) throw new Error(`${startNeedle} not in ${file}`);
  const body = balanced(text, text.indexOf('{', at + startNeedle.length - 1));
  const first = text.slice(0, at).split('\n').length;
  const out = [];
  body.split('\n').forEach((ln, i) => {
    for (const m of ln.matchAll(/(\w+):\s*'([^']+)'/g)) { out.push({ name: m[1], path: m[2], line: first + i }); use(file, first + i); }
  });
  return out;
}

function iconsSlint() {
  const out = [
    '// The design\'s stroke icons (24 grid, stroke 1.7 unless a call site says, round caps and',
    '// joins, no fill), its Space icons, the settings sections\' icons, every inline path of the',
    '// browser and pages, and the logo\'s geometry. Kit `Icon` draws them; nothing else does.',
    '',
    'export global IconStroke {',
  ];
  out.push(`    out property <float> width: 1.7; // browser/icons.tsx:${lineOf('browser/icons.tsx', 'stroke = 1.7')}`);
  out.push(`    out property <float> grid: 24; // browser/icons.tsx:${lineOf('browser/icons.tsx', 'viewBox="0 0 24 24"')}`);
  out.push(`    out property <length> size: 16px; // browser/icons.tsx:${lineOf('browser/icons.tsx', 'size = 16')}`);
  out.push(`    out property <length> kit-size: 20px; // pages/touch-kit.tsx:${lineOf('pages/touch-kit.tsx', 'size = 20')}`);
  use('browser/icons.tsx', lineOf('browser/icons.tsx', 'stroke = 1.7'));
  out.push('}', '');
  const P = pathTable('browser/icons.tsx', 'const P: Record<string, string> = {');
  out.push('// browser/icons.tsx `P`', 'export global BrowserIcons {');
  for (const p of P) out.push(`    out property <string> ${p.name}: ${JSON.stringify(p.path)}; // browser/icons.tsx:${p.line}`);
  out.push('}', '');
  const K = pathTable('pages/touch-kit.tsx', 'export const PATH = {');
  out.push('// pages/touch-kit.tsx `PATH` (the touch kit\'s own set; some differ from the browser\'s)', 'export global KitIcons {');
  for (const p of K) out.push(`    out property <string> ${p.name.replace(/[A-Z]/g, c => `-${c.toLowerCase()}`)}: ${JSON.stringify(p.path)}; // pages/touch-kit.tsx:${p.line}`);
  out.push('}', '');
  const sj = JSON.parse(read('slint/space-icons.json'));
  out.push('// slint/space-icons.json (browser/icons.tsx SPACE_ICONS prefers it over its fallback)', 'export global SpaceIcons {');
  for (const k of ['personal', 'work', 'study']) {
    const ln = lineOf('slint/space-icons.json', `"${k}"`);
    use('slint/space-icons.json', ln);
    out.push(`    out property <string> ${k}: ${JSON.stringify(sj[k])}; // slint/space-icons.json:${ln}`);
  }
  out.push('}', '');
  const sec = table('pages/settings.tsx', 'SECTIONS').value;
  out.push('// pages/settings.tsx `SECTIONS` (names resolved through browser/icons.tsx `P`)', 'export global SettingsIcons {');
  const pmap = Object.fromEntries(P.map(p => [p.name, p.path]));
  for (const s of sec) out.push(`    out property <string> ${s.id}: ${JSON.stringify(pmap[s.icon] ?? s.icon)}; // pages/settings.tsx:${lineOf('pages/settings.tsx', `id: '${s.id}'`)}`);
  out.push('}', '');
  // every inline path in the browser and pages: `name="M…"` on Icon, `d="M…"` on <path>
  out.push('// Inline paths: every `<Icon name="M…">` and `<path d="M…">` of browser/ and pages/, by', '// file and line.', 'export global InlinePaths {');
  for (const dir of ['browser', 'pages']) {
    for (const f of readdirSync(resolve(SRC, dir)).filter(f => f.endsWith('.tsx')).sort()) {
      const file = `${dir}/${f}`;
      read(file).split('\n').forEach((ln, i) => {
        let n = 0;
        for (const m of ln.matchAll(/\b(?:name|d)="([Mm][^"]+)"/g)) {
          n++;
          use(file, i + 1);
          out.push(`    out property <string> ${f.replace(/\.tsx$/, '')}-l${i + 1}${n > 1 ? `-${n}` : ''}: ${JSON.stringify(m[1])}; // ${file}:${i + 1}`);
        }
      });
    }
  }
  out.push('}', '');
  const logo = table('pages/found-ui.tsx', 'LOGO');
  const lg = logo.value;
  const at = `pages/found-ui.tsx:${logo.first}-${logo.last}`;
  out.push('// The mark (pages/found-ui.tsx `LOGO`), in its 120-unit viewBox.', 'export global Mark {');
  for (const k of ['body', 'w', 'w32', 'w16']) out.push(`    out property <string> ${k}: ${JSON.stringify(lg[k])}; // ${at}`);
  for (const g of ['highlight', 'dot', 'box']) for (const [k, v] of Object.entries(lg[g])) out.push(`    out property <float> ${g}-${k}: ${fmtNum(v)}; // ${at}`);
  for (const g of ['bodyDark', 'bodyLight']) lg[g].forEach((c, i) => out.push(prop(`${g.replace(/[A-Z]/g, c => `-${c.toLowerCase()}`)}-${i + 1}`, color(c), at)));
  out.push(prop('tiny-body', color(lg.tiny.body), at), prop('tiny-w', color(lg.tiny.w), at));
  lg.film.forEach(([pos, c], i) => out.push(prop(`film-${i + 1}`, color(c), at), prop(`film-${i + 1}-at`, F(pos), at)));
  // the radial gradients the mark and the app-icon plates are drawn with (inline SVG)
  for (const file of ['pages/found-ui.tsx', 'pages/found-icons.tsx']) {
    const lines = read(file).split('\n');
    lines.forEach((ln0, i) => {
      const ln = ln0.includes('<radialGradient') && !ln0.includes('</radialGradient>') ? `${ln0} ${lines[i + 1] ?? ''}` : ln0;
      const so = /offset=\{\[([^\]]+)\]\[i\]\}/.exec(ln0);
      if (so) {
        use(file, i + 1);
        so[1].split(',').forEach((v, j) => out.push(`    out property <float> body-stop-${j + 1}: ${fmtNum(Number(v))}; // ${file}:${i + 1} (stops())`));
      }
      for (const m of ln.matchAll(/<radialGradient id="([\w-]+)" cx="([\d.]+)" cy="([\d.]+)" r="([\d.]+)"/g)) {
        use(file, i + 1);
        for (const [k, v] of [['cx', m[2]], ['cy', m[3]], ['r', m[4]]]) out.push(`    out property <float> ${m[1]}-${k}: ${fmtNum(Number(v))}; // ${file}:${i + 1}`);
        [...ln.slice(m.index).matchAll(/<stop offset="([\d.]+)"/g)].forEach((st, j) => out.push(`    out property <float> ${m[1]}-stop-${j + 1}: ${fmtNum(Number(st[1]))}; // ${file}:${i + 1}`));
      }
    });
  }
  out.push('}', '');
  const plate = table('pages/found-icons.tsx', 'PLATE');
  const pat = `pages/found-icons.tsx:${plate.first}-${plate.last}`;
  out.push('// The app icons\' plates (pages/found-icons.tsx `PLATE`).', 'export global AppIconPlate {');
  const kebab = k => k.replace(/[A-Z]/g, c => `-${c.toLowerCase()}`);
  for (const [k, v] of Object.entries(plate.value)) {
    if (typeof v === 'string') out.push(prop(kebab(k), color(v), pat));
    else if (Array.isArray(v)) v.forEach((c, i) => out.push(prop(`${kebab(k)}-${i + 1}`, color(c), pat)));
    else for (const [kk, c] of Object.entries(v)) out.push(prop(`${kebab(k)}-${kk}`, color(c), pat));
  }
  out.push('}');
  return out.join('\n');
}

// ── Rust output ────────────────────────────────────────────────────────────────────────────
// The tables wubble-core's motion needs, as `pub mod NAME { pub const key: T = v; }` (the style
// of wubble_core::motion), grouped by design file.
const RUST_TABLES = [
  ['physics', 'core/physics.ts', ['BAND_K', 'K', 'DT_CAP', 'REST']],
  ['touch', 'pages/touch.tsx', ['SLIDER', 'TOGGLE_SIZE', 'TOGGLE', 'PIN', 'DL', 'DOCK', 'MENU', 'CHECKS']],
  ['touch_kit', 'pages/touch-kit.tsx', ['SQUASH', 'SEG']],
  ['settings', 'pages/settings.tsx', ['SWATCH', 'HOLD', 'NAV']],
  ['seam', 'browser/seam.ts', ['SEAM']],
  ['split', 'browser/split.ts', ['SPLIT']],
  ['tabs', 'browser/tabs.tsx', ['TABS', 'PITCH']],
  ['scroller', 'browser/scroller.tsx', ['SCROLL']],
  ['window', 'browser/desktop.tsx', ['WIN']],
  ['command', 'browser/command.tsx', ['CMD']],
  ['voice', 'core/voice.ts', ['VOICE']],
];
const KEYWORDS = new Set(['in', 'type', 'move', 'ref', 'mod', 'fn', 'use', 'loop', 'match', 'self', 'impl', 'let', 'const', 'static', 'struct', 'enum', 'trait', 'where', 'while', 'for', 'if', 'else', 'return', 'pub', 'crate', 'super', 'as', 'box', 'dyn', 'true', 'false']);
const rid = k => (/^\d/.test(k) ? `s${k}` : KEYWORDS.has(k) ? `r#${k}` : k);
const rnum = v => (Number.isInteger(v) ? `${v}.0` : String(v));
function rtype(v) {
  if (typeof v === 'number') return ['f64', rnum(v)];
  if (typeof v === 'string') return ['&str', JSON.stringify(v)];
  if (typeof v === 'boolean') return ['bool', String(v)];
  if (Array.isArray(v) && v.length) {
    const inner = v.map(rtype);
    if (inner.some(x => !x) || inner.some(x => x[0] !== inner[0][0])) return null;
    return [`[${inner[0][0]}; ${v.length}]`, `[${inner.map(x => x[1]).join(', ')}]`];
  }
  return null;
}
function rmod(name, value, at, indent, out) {
  const pad = ' '.repeat(indent);
  if (typeof value !== 'object' || Array.isArray(value)) {
    const t = rtype(value);
    if (t) out.push(`${pad}pub const ${rid(name)}: ${t[0]} = ${t[1]}; // ${at}`);
    else out.push(`${pad}// ${name}: not a plain value (${at})`);
    return;
  }
  out.push(`${pad}pub mod ${rid(name)} {`);
  for (const [k, v] of Object.entries(value)) {
    if (typeof v === 'object' && !Array.isArray(v)) rmod(k, v, at, indent + 4, out);
    else {
      const t = rtype(v);
      if (t) out.push(`${pad}    pub const ${rid(k)}: ${t[0]} = ${t[1]};`);
      else out.push(`${pad}    // ${k}: not a plain value`);
    }
  }
  out.push(`${pad}}`);
}
function keyframes(css, file, name) {
  const c = css.find(c => c.file === file);
  const steps = new Map([[0, [0, 1, 1, 0]], [1, [1, 1, 1, 0]]]);
  for (const p of c.props) {
    const m = new RegExp(`^${name}-k-(\\d+)pct--transform-(rotate|scale-x|scale-y|scale)$`).exec(p.id);
    if (!m) continue;
    const t = Number(m[1]) / 100;
    const s = steps.get(t) ?? [t, 1, 1, 0];
    if (m[2] === 'scale-x') s[1] = p.v; else if (m[2] === 'scale-y') s[2] = p.v; else if (m[2] === 'scale') { s[1] = p.v; s[2] = p.v; } else s[3] = p.v;
    steps.set(t, s);
  }
  return [...steps.values()].sort((a, b) => a[0] - b[0]);
}
const genTables = {};
function rustGen(css) {
  const out = [
    '//! The design\'s motion constants, generated from its web sources (each table cites the file and',
    '//! lines). wubble_core\'s machines use these; a hand-typed copy is a bug.',
    '#![allow(non_snake_case, non_upper_case_globals, dead_code, clippy::all)]',
    '',
  ];
  for (const [mod, file, names] of RUST_TABLES) {
    out.push(`/// ${file}`, '#[rustfmt::skip]', `pub mod ${mod} {`);
    for (const n of names) {
      const tb = table(file, n);
      genTables[n] = tb.value;
      rmod(n, tb.value, `${file}:${tb.first}${tb.last > tb.first ? `-${tb.last}` : ''}`, 4, out);
    }
    if (mod === 'physics') {
      const ln = lineOf(file, 'update(value: number, dt: number, max =');
      const m = /max = ([\d.]+), full = (\d+)/.exec(read(file).split('\n')[ln - 1]);
      use(file, ln);
      out.push(`    /// Stretch.update's defaults (${file}:${ln})`, '    pub mod STRETCH {', `        pub const max: f64 = ${rnum(Number(m[1]))};`, `        pub const full: f64 = ${rnum(Number(m[2]))};`, '    }');
    }
    out.push('}', '');
  }
  out.push('/// core/tokens.ts', '#[rustfmt::skip]', 'pub mod tokens {');
  for (const k of ['radius', 'space', 'chrome', 'motion', 'type']) rmod(k, tokens[k], `core/tokens.ts:${lineOf('core/tokens.ts', `export const ${k}`)}`, 4, out);
  out.push('}', '');
  out.push('/// Keyframe tables [t, scale x, scale y, rotate deg] from the CSS (0 and 1 at rest).', '#[rustfmt::skip]', 'pub mod keyframes {');
  for (const [n, file, kf] of [['SQUASH_CSS', 'core/wubble.css', 'wb-sq-a'], ['WOBBLE', 'core/wubble.css', 'wb-wob-a'], ['BUTTON_SQUASH', 'pages/touch.css', 'tp-bq-a']]) {
    const k = keyframes(css, file, kf);
    const p = css.find(c => c.file === file).props.find(p => p.id.startsWith(`${kf}-k-`));
    out.push(`    /// ${file}:${p.line} @keyframes ${kf}`, `    pub const ${n}: [[f64; 4]; ${k.length}] = [${k.map(s => `[${s.map(rnum).join(', ')}]`).join(', ')}];`);
  }
  const dur = (file, id) => css.find(c => c.file === file).props.find(p => p.id === id);
  const wob = dur('core/wubble.css', 'wb-wob-a-gt-svg--animation-duration');
  out.push(`    /// ${'core/wubble.css'}:${wob.line}`, `    pub const WOBBLE_MS: f64 = ${rnum(wob.v)};`);
  out.push('}');
  rustLiterals(out);
  return out.join('\n');
}

// Inline literals (literals.mjs), one module per manifest module, in manifest order.
const literalValues = LITERALS.map(([mod, name, file, needle, o = {}]) => {
  const { value, line } = literal(read(file), needle, file);
  use(file, line);
  return { mod, name, file, line, value, ...o };
});
function rustLiterals(out) {
  for (const mod of [...new Set(literalValues.map(l => l.mod))]) {
    const files = [...new Set(literalValues.filter(l => l.mod === mod).map(l => l.file))];
    out.push('', `/// Inline literals of ${files.join(', ')} (tools/fidelity/literals.mjs)`, '#[rustfmt::skip]', `pub mod ${mod} {`);
    for (const l of literalValues.filter(l => l.mod === mod)) {
      const t = l.type === 'usize' ? ['usize', String(l.value)] : rtype(l.value);
      out.push(`    pub const ${rid(l.name)}: ${t[0]} = ${t[1]}; // ${l.file}:${l.line}`);
    }
    out.push('}');
  }
}
function literalsSlint() {
  const out = ['// Inline literals of the design\'s code that its CSS tables miss (tools/fidelity/literals.mjs).'];
  const slint = literalValues.filter(l => l.slint);
  for (const g of [...new Set(slint.map(l => l.slint[0]))]) {
    out.push('', `export global ${g} {`);
    for (const l of slint.filter(l => l.slint[0] === g)) out.push(prop(l.slint[1], { t: l.slint[2], v: l.value }, `${l.file}:${l.line}`));
    out.push('}');
  }
  return out.join('\n');
}

// ── Headers, hashes, writing ───────────────────────────────────────────────────────────────
/** FNV-1a 64 of a string's UTF-8 bytes, as 16 hex digits (the lock test recomputes it). */
export function fnv1a64(text) {
  let h = 0xcbf29ce484222325n;
  for (const b of Buffer.from(text, 'utf8')) { h ^= BigInt(b); h = (h * 0x100000001b3n) & 0xffffffffffffffffn; }
  return h.toString(16).padStart(16, '0');
}
const withHeader = body => {
  const text = `${body.trimEnd()}\n`;
  return `// generated by tools/fidelity/extract.mjs from ${commit} (the-wallpaper-project src/wubble); do not edit\n// body-fnv1a64: ${fnv1a64(text)}\n${text}`;
};

const css = cssTables();
const files = {
  'crates/wubble-ui/ui/kit/gen/tokens.slint': withHeader(tokensSlint(css)),
  'crates/wubble-ui/ui/kit/gen/css.slint': withHeader(cssSlint(css)),
  'crates/wubble-ui/ui/kit/gen/icons.slint': withHeader(iconsSlint()),
  'crates/wubble-ui/ui/kit/gen/literals.slint': withHeader(literalsSlint()),
  'crates/wubble-core/src/kit_gen.rs': withHeader(rustGen(css)),
};

if (dirty) console.warn(`extract: warning: the design clone has local changes under src/wubble (${commit}+)`);
let stale = 0;
for (const [rel, text] of Object.entries(files)) {
  const path = resolve(REPO, rel);
  const now = existsSync(path) ? readFileSync(path, 'utf8') : null;
  if (flag('--check')) {
    if (now !== text) { stale++; console.error(`extract: stale: ${rel}${now ? ` (made from ${/from (\w+)/.exec(now)?.[1]})` : ' (missing)'}`); }
  } else if (now !== text) {
    mkdirSync(dirname(path), { recursive: true });
    writeFileSync(path, text);
    console.log(`extract: wrote ${rel}`);
  }
}
const total = css.reduce((n, c) => n + c.props.length, 0), unread = css.reduce((n, c) => n + c.unparsed.length, 0);
console.log(`extract: design ${commit}${dirty ? '+' : ''}; css: ${total} values, ${unread} declarations with numbers not read`);

if (opt('--coverage')) coverage(opt('--coverage'));
if (opt('--report')) {
  const { report } = await import('./lib/report.mjs');
  writeFileSync(opt('--report'), report({ tokens, physics, css, files, REPO, commit, genTables, slintValue }));
  console.log(`extract: wrote ${opt('--report')}`);
}
if (flag('--check') && stale) {
  console.error(`extract: ${stale} generated file(s) stale; run node tools/fidelity/extract.mjs`);
  process.exit(1);
}

/**
 * The inventory's A-rows whose values are not all in the generated files: every hex colour and
 * every number of the row's Value cell must occur among the generated values (numbers as
 * written, colours as `#RRGGBB` or an rgba() channel set).
 */
function coverage(inv) {
  const rows = readFileSync(inv, 'utf8').split('\n').filter(l => /^\| A-\d{3} /.test(l));
  const genText = Object.values(files).join('\n');
  const genNums = new Set([...genText.matchAll(/-?\d*\.?\d+/g)].map(m => Number(m[0])));
  // durations are generated in ms; the inventory often writes seconds
  for (const m of genText.matchAll(/(\d*\.?\d+)ms\b/g)) genNums.add(Number(m[1]) / 1000);
  const genHex = new Set([...genText.matchAll(/#([0-9A-Fa-f]{6})\b/g)].map(m => m[1].toUpperCase()));
  const missing = [];
  for (const row of rows) {
    const cells = row.split('|').map(c => c.trim());
    const value = cells[4].replace(/`/g, '');
    const lost = [];
    for (const m of value.matchAll(/#([0-9A-Fa-f]{6})\b/g)) if (!genHex.has(m[1].toUpperCase())) lost.push(`#${m[1]}`);
    const bare = value.replace(/#[0-9A-Fa-f]{3,8}\b/g, ' ').replace(/[A-Za-z]+\d+/g, ' ');
    for (const m of bare.matchAll(/(?<![\w.])-?\d*\.?\d+(?![\w])/g)) {
      const n = Number(m[0]);
      if (!genNums.has(n) && !genNums.has(-n)) lost.push(m[0]);
    }
    if (lost.length) missing.push(`${cells[1]} ${cells[3]}: ${[...new Set(lost)].join(' ')}`);
  }
  console.log(`extract: A-rows whose every value is generated: ${rows.length - missing.length}/${rows.length}`);
  for (const m of missing) console.log(`  missing: ${m}`);
}
