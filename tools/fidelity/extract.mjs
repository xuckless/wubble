#!/usr/bin/env node
// The kit generator: reads the design's web sources (tokens.ts, physics.ts, the CSS, the icon
// tables, the motion constant tables) and writes the generated kit files. Nobody edits those by
// hand: change the design, then run this.
//
//   node tools/fidelity/extract.mjs                 write the generated files
//   node tools/fidelity/extract.mjs --check         fail (exit 1) when a generated file is stale
//                                                   against the clone (or its commit moved)
// Besides the kit files it copies the web GL sources verbatim into tools/fidelity/glsl-snapshot/
// (and the thinking orbs' geometry into tools/fidelity/ts-snapshot/)
// (with the commit), which the shader-constants test reads.
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
import { CSS_EASE_OUT, easeTable } from './lib/ease.mjs';
import { fontOf } from './lib/text.mjs';
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
  // Ultra Reactivate's library page (src/ultra-reactivate): the thinking orbs' study
  ['LibraryCss', '../ultra-reactivate/library.css'],
];
const CUSTOM_COLOURS = new Set(['LibraryCss']);

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
      // a keyframe step between the ends: where it falls in the run (`60%` -> .6)
      const step = rule.at.startsWith('@keyframes') && /^([\d.]+)%$/.exec(rule.selector.trim());
      if (step && rule.decls.length && !['0', '100'].includes(step[1]) && !seen.has(`${base}--at`)) {
        seen.add(`${base}--at`);
        props.push({ id: `${base}--at`, t: 'float', v: Number(step[1]) / 100, line: rule.decls[0].line, src: `${rule.at} ${rule.selector}` });
      }
      // a width query's breakpoint, once per query, so the kit switches where the design does
      const q = /^@(?:container|media)\b.*\(max-width:\s*([\d.]+)px\)/.exec(rule.at);
      const qid = q && `${slug(rule.at.replace(/^@\w+/, ''))}--query-max-width`;
      if (q && rule.decls.length && !seen.has(qid)) {
        seen.add(qid);
        props.push({ id: qid, t: 'length', v: Number(q[1]), line: rule.decls[0].line, src: `${rule.at} (its breakpoint)` });
      }
      for (const d of rule.decls) {
        // Ultra Reactivate's page keeps its colours in custom properties (`.al { --al-stage:
        // var(--ur-surface, #16181b) }`): a hex colour (or a var() with a hex fallback) is read
        // as `<rule>--var<name>`; the design's own stylesheets name theirs in tokens.ts
        if (CUSTOM_COLOURS.has(name) && d.prop.startsWith('--')) {
          const hex = /^(?:var\(--[\w-]+,\s*)?(#[0-9a-fA-F]{3,8})\)?$/.exec(d.value.trim());
          const typed = hex && readDecl('color', hex[1], R);
          if (typed) {
            use(file, d.line);
            for (const [suffix, v] of typed) props.push({ id: `${base}--var${d.prop}${suffix}`, ...v, line: d.line, src: `${rule.selector} { ${d.prop}: ${d.value} }` });
          }
          continue;
        }
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
  out.push('export struct GlassTierStyle { blur: length, saturate: float, brightness: float, fill: color, shadow-y: length, shadow-blur: length, shadow-color: color, rim-all: color, rim-top: color }');
  out.push('export global Glass {');
  for (const [k, g] of Object.entries(t.glass)) {
    const sel = `wb-${k}--box-shadow`;
    const fields = [
      `blur: ${slintValue(L(g.blur))}`, `saturate: ${fmtNum(g.saturate)}`, `brightness: ${fmtNum(g.brightness)}`,
      `fill: ${slintValue(color(g.fill))}`, `shadow-y: ${slintValue(L(g.shadow[0]))}`, `shadow-blur: ${slintValue(L(g.shadow[1]))}`,
      `shadow-color: ${slintValue(W(`${sel}-shadow-3-color`))}`, `rim-all: ${slintValue(W(`${sel}-shadow-1-color`))}`, `rim-top: ${slintValue(W(`${sel}-shadow-2-color`))}`,
    ];
    out.push(`    out property <GlassTierStyle> ${k}: { ${fields.join(', ')} }; // ${cite('core/tokens.ts', tl(`  ${k}: {`))}, core/wubble.css:${W(`${sel}-shadow-3-color`).line}`);
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
  // CSS `ease-out` per keyframe interval (wobble): Slint has no per-key easing, so the kit
  // interpolates this table (x = i / 32) over each interval's progress.
  const easeOut = easeTable(CSS_EASE_OUT, 32).map(v => fmtNum(Number(v.toFixed(6))));
  out.push(`    // CSS ease-out, cubic-bezier(${CSS_EASE_OUT.join(', ')}), sampled at x = i / 32 (the curve keyframe intervals take)`);
  out.push(`    out property <[float]> ease-out-css: [${easeOut.join(', ')}];`);
  out.push('}', '');
  out.push('/// A keyframe: progress `t` (0..1), scale x/y and rotation (deg); 0 and 1 are at rest.');
  out.push('export struct KeyStep { t: float, sx: float, sy: float, rot: float }');
  out.push('export global Keyframes {');
  for (const [n, file, kf, dur] of [['wobble', 'core/wubble.css', 'wb-wob-a', 'wb-wob-a-gt-svg--animation-duration'], ['squash', 'core/wubble.css', 'wb-sq-a', null], ['button-squash', 'pages/touch.css', 'tp-bq-a', null]]) {
    const k = keyframes(css, file, kf);
    const p = css.find(c => c.file === file).props.find(p => p.id.startsWith(`${kf}-k-`));
    out.push(`    out property <[KeyStep]> ${n}: [${k.map(([t, sx, sy, r]) => `{ t: ${fmtNum(t)}, sx: ${fmtNum(sx)}, sy: ${fmtNum(sy)}, rot: ${fmtNum(r)} }`).join(', ')}]; // ${file}:${p.line} @keyframes ${kf}`);
    if (dur) {
      const d = css.find(c => c.file === file).props.find(p => p.id === dur);
      out.push(prop(`${n}-ms`, d, `${file}:${d.line}`));
    }
  }
  out.push('}', '');
  // The touch kit's menu (pages/touch.tsx `MENU`) and the settings Select's inline numbers
  // (pages/settings.tsx `Select`: width, trigger gap, the view margin it keeps when it opens up).
  const menu = table('pages/touch.tsx', 'MENU');
  const mat = `pages/touch.tsx:${menu.first}-${menu.last}`;
  out.push('// pages/touch.tsx `MENU` and pages/settings.tsx `Select` (the kit menu, controls/menu.slint;', '// the search capsule\'s icons, controls/search-capsule.slint)', 'export global TouchMenu {');
  out.push(prop('first', D(menu.value.first), mat), prop('stagger', D(menu.value.stagger), mat), prop('gap', L(menu.value.gap), mat), prop('blur', L(menu.value.blur), mat), prop('confirm', D(menu.value.confirmMs), mat));
  const sel = read('pages/settings.tsx');
  const selLine = needle => { const l = lineOf('pages/settings.tsx', needle); use('pages/settings.tsx', l); return `pages/settings.tsx:${l}`; };
  const sw = /GAP = (\d+), W = (\d+);/.exec(sel), se = /view\.bottom - (\d+)/.exec(sel);
  if (!sw || !se) throw new Error('settings.tsx Select: GAP/W or the view margin not found');
  out.push(prop('select-gap', L(Number(sw[1])), selLine('GAP = ')), prop('select-width', L(Number(sw[2])), selLine('GAP = ')), prop('select-edge', L(Number(se[1])), selLine('view.bottom - ')));
  // the icon sizes and the chevron's stroke the menu, the Select and the settings search draw
  // inline (`size={n}` on the JSX), by file, a needle on the line and the pattern
  const inline = [
    ['row-icon', 'pages/touch.tsx', 'name={r.icon} size=', /size=\{(\d+)\}/],
    ['sub-icon', 'pages/touch.tsx', 'name="chevronRight" size=', /size=\{(\d+)\}/],
    ['done-icon', 'pages/touch.tsx', 'class="tp-menu-done"><Icon', /size=\{(\d+)\}/],
    ['chevron-icon', 'pages/touch.tsx', 'class={`tp-chev', /width="(\d+)"/],
    ['chevron-stroke', 'pages/touch.tsx', 'class={`tp-chev', /stroke-width="([\d.]+)"/],
    ['check-icon', 'pages/settings.tsx', 'class="st-mcheck">', /size=\{(\d+)\}/],
    ['search-icon', 'pages/settings.tsx', 'StrokeIcon name="search" size=', /size=\{(\d+)\}/],
    ['clear-icon', 'pages/settings.tsx', '<Icon name="close" size={14}', /size=\{(\d+)\}/],
  ];
  for (const [id, file, needle, re] of inline) {
    const l = lineOf(file, needle);
    const m = l && re.exec(read(file).split('\n')[l - 1]);
    if (!m) throw new Error(`${file}: ${id} (${needle}) not found`);
    use(file, l);
    out.push(prop(id, id === 'chevron-stroke' ? F(Number(m[1])) : L(Number(m[1])), `${file}:${l}`));
  }
  out.push('}', '');
  out.push(...tsxSlint());
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
/** Ultra Reactivate's thinking orbs, relative to src/wubble (every citation's root). */
const ORBS = '../ultra-reactivate/vendor/thinking-orbs';
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
/**
 * Values the controls take from TSX markup and timers rather than a constant table: an `<svg>`
 * or `<Icon size>` attribute, a `setTimeout` length. Each is found by its line's text and read
 * by a regex; a design change that moves or drops one fails here instead of going stale.
 */
function tsxValue(file, needle, re) {
  const line = lineOf(file, needle);
  if (!line) throw new Error(`${file}: no line with ${needle}`);
  const m = re.exec(read(file).split('\n')[line - 1]);
  if (!m) throw new Error(`${file}:${line}: ${re} does not match`);
  use(file, line);
  return { v: Number(m[1]), at: `${file}:${line}` };
}
const TSX = () => {
  const checks = table('pages/touch.tsx', 'CHECKS');
  const dock = table('pages/touch.tsx', 'DOCK');
  const tick = 'pathLength={20}';
  return {
    // the check box's tick (touch.tsx Checks; settings.tsx's row check box is the same markup)
    'check-tick-delay': { t: 'duration', v: checks.value.tickDelay, at: `pages/touch.tsx:${checks.first} CHECKS.tickDelay` },
    'check-tick-size': { t: 'length', ...tsxValue('pages/touch.tsx', tick, /<svg width="([\d.]+)"/) },
    'check-tick-stroke': { t: 'float', ...tsxValue('pages/touch.tsx', tick, /stroke-width="([\d.]+)"/) },
    'check-tick-length': { t: 'float', ...tsxValue('pages/touch.tsx', tick, /pathLength=\{([\d.]+)\}/) },
    // the tick's path (`PATH.check`, two segments of slope 1, so its length grows linearly with
    // x): where it starts and ends across, in grid units
    'check-tick-x0': { t: 'float', ...tsxValue('pages/touch-kit.tsx', '  check: ', /check: 'm([\d.]+) /) },
    'check-tick-x1': { t: 'float', ...tsxValue('pages/touch-kit.tsx', '  check: ', /L([\d.]+) [\d.]+'/) },
    // the fill's `clip-path: circle(75%)` (not read by the CSS reader): its radius over the box
    'check-fill-radius': { t: 'float', ...tsxValue('pages/touch.css', '.tp-ck.on .tp-ck-fill', /circle\(([\d.]+)%/), scale: 0.01 },
    // the dock tooltip's glide (`--tp-glide`)
    'tip-glide': { t: 'duration', v: dock.value.glideMs, at: `pages/touch.tsx:${dock.first}-${dock.last} DOCK.glideMs` },
    // HoldButton's done label icon, ConfirmButton's done icon and how long it shows
    'hold-done-icon': { t: 'length', ...tsxValue('pages/settings.tsx', 'class="st-hold-done"', /size=\{([\d.]+)\}/) },
    'confirm-done-icon': { t: 'length', ...tsxValue('pages/settings.tsx', 'class="st-ok"><Icon name="check"', /size=\{([\d.]+)\}/) },
    'confirm-done-ms': { t: 'duration', ...tsxValue('pages/settings.tsx', "setSt('idle')", /setSt\('idle'\), (\d+)\)/) },
    // the command bar (command.tsx): the rows' entrance lead before the stagger, the most rows,
    // the head's search icon; where it grows from (chrome.tsx `onOpen`): the pill's left + 40,
    // the New tab row's left + 20
    'cmd-row-lead': { t: 'duration', ...tsxValue('browser/command.tsx', 'animationDelay:', /\$\{(\d+) \+ i \* CMD\.stagger\}ms/) },
    'cmd-rows': { t: 'float', ...tsxValue('browser/command.tsx', 'const CMD = ', /rows: (\d+)/) },
    'cmd-head-icon': { t: 'length', ...tsxValue('browser/command.tsx', '<Icon name="search" size=', /size=\{(\d+)\}/) },
    'address-origin-dx': { t: 'length', ...tsxValue('browser/chrome.tsx', 'onOpen({ x: r.left + ', /r\.left \+ (\d+)/) },
    'new-tab-origin-dx': { t: 'length', ...tsxValue('browser/chrome.tsx', "openCommand('new')({ x: r.left + ", /r\.left \+ (\d+)/) },
  };
};
function tsxSlint() {
  const out = ['/// Control values the tables above do not hold: TSX markup and timers, and a CSS value the', '/// reader leaves unread (each line cites where).', 'export global Tsx {'];
  for (const [k, { t, v, at, scale = 1 }] of Object.entries(TSX())) out.push(prop(k, { t, v: v * scale }, at));
  out.push('}');
  return out;
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
    if (mod === 'settings') {
      // ConfirmButton has no table: its done label shows for a setTimeout's length
      const done = TSX()['confirm-done-ms'];
      out.push(`    /// ConfirmButton (${done.at})`, '    pub mod CONFIRM {', `        pub const doneMs: f64 = ${rnum(done.v)};`, '    }');
    }
    out.push('}', '');
  }
  // the Spaces' accents and their three dye colours, as 8-bit RGB
  const sl = lineOf('core/tokens.ts', 'export const spaces');
  const rgb = h => { const n = parseInt(/^#([0-9A-Fa-f]{6})$/.exec(h)[1], 16); return `[${n >> 16}, ${(n >> 8) & 255}, ${n & 255}]`; };
  use('core/tokens.ts', ...tokens.spaces.map((_, i) => sl + 1 + i));
  out.push(`/// core/tokens.ts:${sl + 1}-${sl + tokens.spaces.length} (\`spaces\`: ${tokens.spaces.map(s => s.id).join(', ')})`, '#[rustfmt::skip]', 'pub mod spaces {');
  out.push(`    pub const ACCENT: [[u8; 3]; ${tokens.spaces.length}] = [${tokens.spaces.map(s => rgb(s.accent)).join(', ')}];`);
  out.push(`    pub const DYE: [[[u8; 3]; 3]; ${tokens.spaces.length}] = [${tokens.spaces.map(s => `[${s.dye.map(rgb).join(', ')}]`).join(', ')}];`);
  // the settings swatches a Space's colour is picked from (a new Space takes the next one)
  const wl = lineOf('pages/settings.tsx', 'const SWATCHES = [');
  const swatches = [...read('pages/settings.tsx').split('\n')[wl - 1].matchAll(/color\.(\w+)/g)].map(m => m[1]);
  use('pages/settings.tsx', wl);
  out.push(`    /// pages/settings.tsx:${wl} (\`SWATCHES\`)`);
  out.push(`    pub const SWATCHES: [[u8; 3]; ${swatches.length}] = [${swatches.map(n => rgb(tokens.color[n])).join(', ')}];`);
  out.push(`    pub const SWATCH_NAMES: [&str; ${swatches.length}] = [${swatches.map(n => JSON.stringify(n)).join(', ')}];`);
  out.push('}', '');
  out.push('/// core/tokens.ts', '#[rustfmt::skip]', 'pub mod tokens {');
  for (const k of ['radius', 'space', 'chrome', 'motion', 'type']) rmod(k, tokens[k], `core/tokens.ts:${lineOf('core/tokens.ts', `export const ${k}`)}`, 4, out);
  // the colours as 8-bit RGB, and each glass tier's backdrop filter and fill (saturate,
  // brightness, fill r g b 0-255 and alpha): the scene's glass over a page draws the CSS tier
  const cl = lineOf('core/tokens.ts', 'export const color');
  out.push(`    /// core/tokens.ts:${cl} (\`color\`)`, '    pub mod color {');
  for (const [k, v] of Object.entries(tokens.color)) {
    const ln = lineOf('core/tokens.ts', `${k}: '`);
    use('core/tokens.ts', ln);
    out.push(`        pub const ${rid(k)}: [u8; 3] = ${rgb(v)}; // core/tokens.ts:${ln}`);
  }
  out.push('    }');
  out.push('    /// core/tokens.ts `glass`: [saturate, brightness, fill r, g, b (0-255), fill alpha]', '    pub mod glass {');
  for (const [k, g] of Object.entries(tokens.glass)) {
    const ln = lineOf('core/tokens.ts', `  ${k}: {`);
    use('core/tokens.ts', ln);
    const f = /^rgba\((\d+),\s*(\d+),\s*(\d+),\s*([\d.]+)\)$/.exec(g.fill);
    if (!f) throw new Error(`tokens.ts glass.${k}: fill ${g.fill} is not rgba()`);
    const v = [g.saturate, g.brightness, ...f.slice(1).map(Number)];
    out.push(`        pub const ${rid(k)}: [f64; 6] = [${v.map(rnum).join(', ')}]; // core/tokens.ts:${ln}`);
  }
  out.push('    }');
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
  orbRust(out);
  rustLiterals(out);
  return out.join('\n');
}

/**
 * Ultra Reactivate's thinking orbs (the owner's library, src/ultra-reactivate beside src/wubble):
 * the state → mode map, the tuned presets, the base profiles, the morph's timing and the
 * labels, as slices of pairs, so wubble_core::orb walks them as the TS walks its objects (a key
 * the design adds reaches the port without a hand edit).
 */
function orbRust(out) {
  const at = t => `${t.file}:${t.first}${t.last > t.first ? `-${t.last}` : ''}`;
  const pairs = o => `&[${Object.entries(o).map(([k, v]) => `(${JSON.stringify(k)}, ${rnum(v)})`).join(', ')}]`;
  const modes = table(`${ORBS}/presets.ts`, 'STATE_TO_MODE');
  const presets = table(`${ORBS}/presets.ts`, 'PRESETS');
  const profiles = table(`${ORBS}/engine/profiles.ts`, 'BASE_PROFILES');
  const hold = table(`${ORBS}/engine/morph.ts`, 'HOLD');
  const morph = table(`${ORBS}/engine/morph.ts`, 'MORPH');
  const labels = table(`${ORBS}/ThinkingOrb.tsx`, 'LABELS');
  out.push('', `/// ${ORBS} (presets.ts, engine/profiles.ts, engine/morph.ts, ThinkingOrb.tsx)`, '#[rustfmt::skip]', 'pub mod orb {');
  out.push(`    /// ${at(modes)}: (state, mode)`);
  out.push(`    pub const STATE_TO_MODE: &[(&str, &str)] = &[${Object.entries(modes.value).map(([k, v]) => `(${JSON.stringify(k)}, ${JSON.stringify(v)})`).join(', ')}];`);
  out.push(`    /// ${at(presets)}: (mode, size, speed, count, size multiplier, extra opts)`, '    pub const PRESETS: &[(&str, u32, f64, f64, f64, &[(&str, f64)])] = &[');
  for (const [mode, sizes] of Object.entries(presets.value))
    for (const [size, p] of Object.entries(sizes))
      out.push(`        (${JSON.stringify(mode)}, ${size}, ${rnum(p.speed)}, ${rnum(p.count)}, ${rnum(p.size)}, ${pairs(p.extra ?? {})}),`);
  out.push('    ];', `    /// ${at(profiles)}: (mode, opts)`, '    pub const BASE_PROFILES: &[(&str, &[(&str, f64)])] = &[');
  for (const [mode, o] of Object.entries(profiles.value)) out.push(`        (${JSON.stringify(mode)}, ${pairs(o)}),`);
  out.push('    ];');
  out.push(`    pub const HOLD: f64 = ${rnum(hold.value)}; // ${at(hold)}`, `    pub const MORPH: f64 = ${rnum(morph.value)}; // ${at(morph)}`);
  out.push(`    /// ${at(labels)}: (state, accessible label)`);
  out.push(`    pub const LABELS: &[(&str, &str)] = &[${Object.entries(labels.value).map(([k, v]) => `(${JSON.stringify(k)}, ${JSON.stringify(v)})`).join(', ')}];`);
  out.push('}');
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

// ── Text roles ─────────────────────────────────────────────────────────────────────────────
// Every rule of the shell's stylesheets that sets a font (family, size, weight or the `font`
// shorthand) as one role: family, size, weight and line height, each taken from the rule or
// inherited from `.wb` (core/wubble.css: the ui face, 14 px, 400, 1.45). Plus the type scale of
// tokens.ts, and the roles the markup composes from two rules (an element with a class), cited.
const TEXT_CSS = ['core/wubble.css', 'browser/browser.css', 'pages/settings.css', 'pages/touch.css', 'pages/start.css'];
// [role, file:line of the markup, [file, selector] composed in order]
const TEXT_COMPOSED = [
  ['st-title', 'pages/settings.tsx:557', [['pages/settings.css', '.st-pin h1'], ['core/wubble.css', '.wb-display']]],
  ['st-title-narrow', 'pages/settings.tsx:557', [['pages/settings.css', '.st-pin h1'], ['pages/settings.css', '@container (max-width: 759px) .st-pin h1'], ['core/wubble.css', '.wb-display']]],
];
function textSlint() {
  const rules = new Map();
  for (const file of TEXT_CSS) for (const r of parseCss(read(file))) {
    if (r.at && !r.at.startsWith('@media') && !r.at.startsWith('@container')) continue;
    rules.set(`${file} ${r.at ? `${r.at} ` : ''}${r.selector}`, r);
  }
  const wb = rules.get('core/wubble.css .wb');
  const base = fontOf(wb.decls, { family: 'ui', size: 14, weight: 400, lh: 1.45 });
  const out = [
    '// The design\'s text roles: the type scale of core/tokens.ts and every rule of the shell\'s',
    '// stylesheets that sets a font, with what it inherits from `.wb` (core/wubble.css). KitText',
    '// (kit/text.slint) draws a role; nothing else names a font.',
    '',
    'import { Fonts } from "tokens.slint";',
    '',
    '/// family, size, weight and line height (a factor of the size; Slint text has none, so it is',
    '/// the role\'s box height for layout).',
    'export struct TextRole { family: string, size: length, weight: int, line-height: float }',
    'export global Role {',
  ];
  const role = (id, f, cite) => out.push(`    out property <TextRole> ${id}: { family: Fonts.${f.family}, size: ${fmtNum(f.size)}px, weight: ${f.weight}, line-height: ${fmtNum(f.lh)} }; // ${cite}`);
  const tl = lineOf('core/tokens.ts', 'export const type') + 1;
  for (const [k, [sz, w, lh]] of Object.entries(tokens.type)) role(k, { family: k === 'display' ? 'display' : 'ui', size: sz, weight: w, lh }, `core/tokens.ts:${tl}`);
  const seen = new Set(Object.keys(tokens.type));
  // an element that also carries `.wb-mono` / `.wb-display` (the selector's last part names it)
  const classed = (sel, f) => {
    const last = sel.trim().split(/\s+/).pop();
    for (const c of ['wb-mono', 'wb-display']) if (last.includes(`.${c}`)) f = fontOf(rules.get(`core/wubble.css .${c}`).decls, f);
    return f;
  };
  for (const [key, r] of rules) {
    const file = key.slice(0, key.indexOf(' '));
    const fonts = r.decls.filter(d => /^font(-family|-size|-weight)?$/.test(d.prop) && d.value.trim() !== 'inherit');
    if (!fonts.length || r.selector === '.wb') continue;
    const first = r.selector.split(',')[0].trim();
    let id = (r.at ? `${slug(r.at.replace(/^@\w+/, ''))}-` : '') + slug(first);
    if (!/^[a-z]/.test(id)) id = `r-${id}`;
    if (seen.has(id)) continue;
    // a media/container rule restyles the element its plain rule styles
    const plain = r.at ? rules.get(`${file} ${r.selector}`) : null;
    const f = fontOf(r.decls, classed(first, plain ? fontOf(plain.decls, base) : base));
    seen.add(id);
    use(file, fonts[0].line);
    role(id, f, `${file.split('/').pop()}:${fonts[0].line}`);
  }
  for (const [id, markup, parts] of TEXT_COMPOSED) {
    let f = base;
    for (const [file, sel] of parts) f = fontOf(rules.get(`${file} ${sel}`).decls, f);
    role(id, f, `${markup} (${parts.map(([file, sel]) => `${file.split('/').pop()} ${sel}`).join(' + ')})`);
  }
  out.push('}');
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
controlsBExtras(css);

// ── Controls kit B extras (lane ui2b-slide) ────────────────────────────────────────────────
// Declarations the CSS reader cannot type (a `calc`, a percentage origin, a `min()`, a shadow
// with a `var()` colour) but the slider needs: each is read from its rule's text and the run
// throws when the rule changed, so a design edit cannot leave a stale value behind. They join
// TouchCss (and leave its "not read" list).
function controlsBExtras(css) {
  const file = 'pages/touch.css', touch = css.find(c => c.file === file);
  const lines = read(file).split('\n');
  const pick = (sel, prop, re) => {
    // the rule's line, or a continuation line of it, that sets `prop`
    const start = lines.findIndex(l => l.startsWith(`${sel} {`));
    let i = -1;
    for (let k = start; start >= 0 && k < lines.length; k++) {
      if (new RegExp(`(^|[\\s;{])${prop}:`).test(lines[k])) { i = k; break; }
      if (lines[k].includes('}')) break;
    }
    if (i < 0) throw new Error(`extract: controls B: ${file} has no ${sel} { ${prop} }`);
    const m = re.exec(lines[i].slice(lines[i].indexOf(`${prop}:`)));
    if (!m) throw new Error(`extract: controls B: ${file}:${i + 1} ${sel} { ${prop} } changed`);
    use(file, i + 1);
    touch.unparsed = touch.unparsed.filter(u => !(u.line === i + 1 && u.prop === prop));
    return { v: Number(m[1]), line: i + 1, src: `${sel} { ${m[0]} }` };
  };
  const add = (id, t, scale, p) => touch.props.push({ id, t, v: p.v * scale, line: p.line, src: p.src });
  // the keyboard focus ring round the knob: `0 0 0 3px var(--tp-accent)`
  add('tp-sl-knob-focus--ring', 'length', 1, pick('.tp-sl:focus-visible .tp-sl-knob', 'box-shadow', /^box-shadow:\s*0 0 0 ([\d.]+)px var\(--tp-accent\)/));
  // the bubble sits `calc(50% + 21px)` above the track's foot, leaning about 50% 130%
  add('tp-sl-bub--bottom-offset', 'length', 1, pick('.tp-sl-bub', 'bottom', /^bottom:\s*calc\(50% \+ ([\d.]+)px\)/));
  add('tp-sl-bub--origin-y', 'float', 0.01, pick('.tp-sl-bub', 'transform-origin', /^transform-origin:\s*50% ([\d.]+)%/));
  // its label grows from 50% 140%
  add('tp-sl-bub-b--origin-y', 'float', 0.01, pick('.tp-sl-bub b', 'transform-origin', /^transform-origin:\s*50% ([\d.]+)%/));
  // `width: min(300px, 100%)`
  add('tp-sl-wrap--max-width', 'length', 1, pick('.tp-sl-wrap', 'width', /^width:\s*min\(([\d.]+)px, 100%\)/));
}
/** Literals inside the TSX loops (not in a constant table) the controls B machines need. */
function controlsBRust() {
  const lit = (file, needle, re, what) => {
    const ln = lineOf(file, needle);
    const m = ln && re.exec(read(file).split('\n')[ln - 1]);
    if (!m) throw new Error(`extract: controls B: ${file} no longer has ${what}`);
    use(file, ln);
    return [Number(m[1]), `${file}:${ln}`];
  };
  const [slop, slopAt] = lit('pages/touch-kit.tsx', 'Math.abs(x - s.x0) > ', /Math\.abs\(x - s\.x0\) > ([\d.]+)/, 'the Segmented drag slop');
  const keyNeedle = 'seam.saved = clamp(seam.saved + dir * ';
  const [step, stepAt] = lit('browser/chrome.tsx', keyNeedle, /seam\.saved \+ dir \* ([\d.]+)/, 'the sidebar seam key step');
  const [kick] = lit('browser/chrome.tsx', keyNeedle, /seam\.bend\.v \+= dir \* ([\d.]+)/, 'the sidebar seam key kick');
  return [
    '/// Literals of the controls\' loops and key handlers (not in a table): controls kit B.',
    '#[rustfmt::skip]',
    'pub mod controls_b {',
    `    pub const SEG_SLOP: f64 = ${rnum(slop)}; // ${slopAt}`,
    `    pub const SEAM_KEY_STEP: f64 = ${rnum(step)}; // ${stepAt}`,
    `    pub const SEAM_KEY_KICK: f64 = ${rnum(kick)}; // ${stepAt}`,
    '}',
  ].join('\n');
}
// The web GL sources, verbatim (same line numbers as the clone), for the shader-constants test
// (crates/wubble-gfx/tests/shader_constants.rs), which must run without the clone.
const GL = ['core/gl/scene.ts', 'core/gl/glsl.ts', 'core/gl/fluid.ts', 'core/gl/util.ts'];
// The thinking orbs' geometry, verbatim, for wubble-core's translation test
// (crates/wubble-core/tests/orb_constants.rs), which must run without the clone. Stored under
// their path below src/ (the `../` of the citation dropped).
const ORB_ENGINE = [...['core', 'orbits', 'lattice', 'web', 'braid', 'ribbon', 'morph', 'profiles'].map(f => `${ORBS}/engine/${f}.ts`), `${ORBS}/presets.ts`];
const files = {
  'crates/wubble-ui/ui/kit/gen/tokens.slint': withHeader(tokensSlint(css)),
  'crates/wubble-ui/ui/kit/gen/css.slint': withHeader(cssSlint(css)),
  'crates/wubble-ui/ui/kit/gen/icons.slint': withHeader(iconsSlint()),
  'crates/wubble-ui/ui/kit/gen/text.slint': withHeader(textSlint()),
  'crates/wubble-ui/ui/kit/gen/literals.slint': withHeader(literalsSlint()),
  'crates/wubble-core/src/kit_gen.rs': withHeader(`${rustGen(css)}\n\n${controlsBRust()}`),
  'tools/fidelity/glsl-snapshot/COMMIT': `${commit}\n`,
  ...Object.fromEntries(GL.map(f => [`tools/fidelity/glsl-snapshot/${f}`, read(f)])),
  'tools/fidelity/ts-snapshot/COMMIT': `${commit}\n`,
  ...Object.fromEntries(ORB_ENGINE.map(f => [`tools/fidelity/ts-snapshot/${f.replace(/^\.\.\//, '')}`, read(f)])),
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
