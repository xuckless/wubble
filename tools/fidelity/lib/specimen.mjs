// Per-control specimens: `specimens.json` is a compact table (control, page, root, elements,
// states, sequences, source anchor) that `expand` turns into ordinary scenes, so capture, cache,
// scorer and report need no special case. Pure, except `sourceIndex` (reads the design).
import { readFileSync, readdirSync } from 'node:fs';
import { resolve, relative } from 'node:path';

/** The scene name of a specimen state or sequence. */
export const sceneName = (control, state) => `spec-${control}--${state}`;
/** The id lanes and the index use: `control/state`. */
export const specimenId = (control, state) => `${control}/${state}`;

const subst = (s, root) => (typeof s === 'string' ? s.replaceAll('{root}', root) : s);

/**
 * The scenes of a specimen table. Each state is a still: its frame is the page's `stage` for the
 * control, the PNG is clipped to the union of `root` and the state's `clip` selectors plus `pad`
 * (default 8 px, room for focus rings and shadows). Each sequence is a frame burst on the 1/120 s
 * clock clipped the same way (measured before its first frame, so a popup's sequence clips to a
 * selector that already exists, usually the stage), with a curve of its `track` elements.
 */
export function expand(table) {
  const scenes = [];
  for (const c of table.controls) {
    const page = table.pages[c.page];
    if (!page) throw new Error(`specimen ${c.control}: unknown page ${c.page}`);
    const elements = Object.fromEntries(Object.entries(c.elements ?? {}).map(([k, v]) => [k, subst(v, c.root)]));
    const common = (extra, kind) => {
      const clip = [c.root, ...(extra.clip ?? c.clip ?? [])].map(s => subst(s, c.root));
      const scene = {
        name: sceneName(c.control, extra.state ?? `seq-${extra.name}`),
        url: page.url,
        surface: `Specimens: ${c.group ?? c.control}`,
        gl: c.gl ?? page.gl ?? 'swiftshader',
        frame: subst(c.stage ?? page.stage, c.root),
        clip,
        pad: extra.pad ?? c.pad ?? 8,
        steps: [...(page.steps ?? []), ...(c.before ?? []), ...(extra.steps ?? [])].map(s => JSON.parse(subst(JSON.stringify(s), c.root))),
        specimen: { id: specimenId(c.control, extra.state ?? `seq-${extra.name}`), control: c.control, state: extra.state ?? null, sequence: extra.name ?? null, kind, root: c.root, elements, source: c.source ?? null },
      };
      for (const k of ['viewport', 'ready', 'frozen', 'settle']) if ((c[k] ?? page[k]) !== undefined) scene[k] = c[k] ?? page[k];
      const after = extra.after ?? c.after ?? page.after;
      if (after !== undefined) scene.after = after;
      if (extra.gate !== undefined) scene.gate = extra.gate;
      if (extra.why) scene.why = extra.why;
      // a controls lane scores its own kit gallery scene against this state
      if (extra.wubble) scene.wubble = extra.wubble;
      return scene;
    };
    for (const st of c.states ?? []) scenes.push(common(st, 'still'));
    for (const sq of c.sequences ?? []) {
      for (const n of sq.track ?? []) if (!(n in elements)) throw new Error(`specimen ${c.control}: sequence ${sq.name} tracks unknown element ${n}`);
      const s = common(sq, 'sequence');
      s.frames = sq.frames ?? 96;
      s.track = sq.track ?? Object.keys(elements);
      scenes.push(s);
    }
  }
  const names = new Set();
  for (const s of scenes) { if (names.has(s.name)) throw new Error(`specimen scene ${s.name} twice`); names.add(s.name); }
  return scenes;
}

/** The spec with its specimen table's scenes appended (`scenes.json` `"specimens": "file"`). */
export function withSpecimens(spec, readTable) {
  if (!spec.specimens) return spec;
  return { ...spec, scenes: [...spec.scenes, ...expand(readTable(spec.specimens))] };
}

/**
 * Where each `@source` anchor and file pattern lives in the design: `anchor` -> `pages/x.tsx:N`
 * for every `// @source <anchor>` line under `src/wubble`; `resolveSource` also takes
 * `path#text` (the first line of `path` containing `text`).
 */
export function sourceIndex(designRoot) {
  const base = resolve(designRoot, 'src/wubble');
  const files = [];
  const walk = d => { for (const e of readdirSync(d, { withFileTypes: true })) { if (e.isDirectory()) { if (!['slint', 'node_modules', 'sites'].includes(e.name)) walk(resolve(d, e.name)); } else if (/\.(tsx?|css)$/.test(e.name)) files.push(resolve(d, e.name)); } };
  walk(base);
  const texts = new Map(files.map(f => [relative(base, f), readFileSync(f, 'utf8')]));
  return { texts, anchors: anchorsOf(texts) };
}

/** `// @source <id>` lines of each file text: id -> `file:line`. Pure. */
export function anchorsOf(texts) {
  const anchors = new Map();
  for (const [file, text] of texts) text.split('\n').forEach((ln, i) => { const m = /\/\/ @source ([\w-]+)/.exec(ln); if (m && !anchors.has(m[1])) anchors.set(m[1], `${file}:${i + 1}`); });
  return anchors;
}

/** `file:line` of a specimen's `source` (an anchor id or `path#text`), or null. Pure. */
export function resolveSource({ texts, anchors }, source) {
  if (!source) return null;
  const hash = source.indexOf('#');
  if (hash < 0) return anchors.get(source) ?? null;
  const file = source.slice(0, hash), needle = source.slice(hash + 1);
  const text = texts.get(file);
  if (!text) return null;
  const i = text.split('\n').findIndex(l => l.includes(needle));
  return i < 0 ? null : `${file}:${i + 1}`;
}
