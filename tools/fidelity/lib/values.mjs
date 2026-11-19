// CSS values to typed Slint values. Every helper returns `null` for what it does not read, so
// the caller lists it as unparsed instead of guessing.
import { splitTop } from './css.mjs';

/** A typed value: `{ t: 'length'|'duration'|'color'|'float'|'easing'|'string'|'bool', v }`. */
const T = (t, v) => ({ t, v });

export function makeReader({ palette, ease, radius, space }) {
  const num = s => (/^-?(\d+\.?\d*|\.\d+)$/.test(s) ? Number(s) : null);
  function length(s) {
    if (s === '0') return T('length', 0);
    let m = /^(-?(?:\d+\.?\d*|\.\d+))px$/.exec(s);
    if (m) return T('length', Number(m[1]));
    m = /^var\(--wb-r-(\w+)\)$/.exec(s);
    if (m && radius[m[1]] !== undefined) return T('length', radius[m[1]]);
    m = /^var\(--wb-s-(\w+)\)$/.exec(s);
    if (m && space[m[1]] !== undefined) return T('length', space[m[1]]);
    return null;
  }
  function duration(s) {
    const m = /^((?:\d+\.?\d*|\.\d+))(ms|s)$/.exec(s);
    if (!m) return null;
    return T('duration', Math.round(Number(m[1]) * (m[2] === 's' ? 1000 : 1) * 1000) / 1000);
  }
  function rgba(r, g, b, a) {
    return T('color', { r: Math.round(r), g: Math.round(g), b: Math.round(b), a });
  }
  function color(s) {
    if (s === 'transparent') return rgba(0, 0, 0, 0);
    let m = /^#([0-9a-fA-F]{3,8})$/.exec(s);
    if (m) {
      let h = m[1];
      if (h.length <= 4) h = [...h].map(c => c + c).join('');
      const n = parseInt(h.slice(0, 6), 16);
      const a = h.length === 8 ? parseInt(h.slice(6), 16) / 255 : 1;
      return rgba(n >> 16, (n >> 8) & 255, n & 255, Math.round(a * 1000) / 1000);
    }
    m = /^rgba?\((.*)\)$/.exec(s);
    if (m) {
      const [rgb, alpha] = m[1].includes('/') ? m[1].split('/') : [m[1], null];
      const parts = rgb.split(/[\s,]+/).filter(Boolean);
      let a = alpha !== null ? alpha.trim() : parts[3] ?? '1';
      a = a.endsWith('%') ? Number(a.slice(0, -1)) / 100 : Number(a);
      if (parts.length < 3 || parts.slice(0, 3).some(p => num(p) === null) || Number.isNaN(a)) return null;
      return rgba(Number(parts[0]), Number(parts[1]), Number(parts[2]), a);
    }
    m = /^var\(--wb-(\w+)\)$/.exec(s);
    if (m && palette[m[1]]) return color(palette[m[1]]);
    return null;
  }
  function easing(s) {
    if (['linear', 'ease', 'ease-in', 'ease-out', 'ease-in-out'].includes(s)) return T('easing', s);
    let m = /^var\(--wb-ease-([\w-]+)\)$/.exec(s);
    if (m) {
      const key = m[1].replace(/-(\w)/g, (_, c) => c.toUpperCase());
      if (ease[key]) return T('easing', `cubic-bezier(${ease[key].join(', ')})`);
    }
    m = /^cubic-bezier\(([^)]*)\)$/.exec(s);
    if (m) return T('easing', `cubic-bezier(${m[1].split(',').map(x => Number(x.trim())).join(', ')})`);
    return null;
  }
  function percent(s) {
    const m = /^(-?(?:\d+\.?\d*|\.\d+))%$/.exec(s);
    return m ? T('float', Number(m[1]) / 100) : null;
  }
  const scalar = s => length(s) ?? duration(s) ?? color(s) ?? easing(s) ?? (num(s) !== null ? T('float', num(s)) : null) ?? percent(s);
  return { length, duration, color, easing, percent, scalar, num };
}

const SIDES = ['top', 'right', 'bottom', 'left'];
const CORNERS = ['tl', 'tr', 'br', 'bl'];

/**
 * One declaration to `[[suffix, typed]]` (suffix '' is the property itself), or `null` when it
 * is not read. `R` is a reader from `makeReader`.
 */
export function readDecl(prop, value, R) {
  const out = [];
  const put = (suffix, v) => { if (v) out.push([suffix, v]); return v; };
  const words = splitTop(value, ' ');
  const fns = s => [...s.matchAll(/([\w-]+)\(([^()]*(?:\([^()]*\)[^()]*)*)\)/g)].map(m => [m[1], m[2]]);
  const box = names => {
    const v = words.map(w => R.length(w) ?? R.percent(w));
    if (v.some(x => !x) || v.length > 4) return null;
    if (v.every(x => x.v === v[0].v && x.t === v[0].t)) return put('', v[0]) && out;
    const [a, b = a, c = a, d = b] = v;
    [a, b, c, d].forEach((x, i) => put(`-${names[i]}`, x));
    return out;
  };
  switch (prop) {
    case 'padding': case 'margin': case 'inset': return box(SIDES);
    case 'border-radius': return box(CORNERS);
    case 'gap': {
      const v = words.map(w => R.length(w));
      if (v.some(x => !x)) return null;
      if (v.length === 1 || v[0].v === v[1].v) return [['', v[0]]];
      return [['-row', v[0]], ['-column', v[1]]];
    }
    case 'box-shadow': {
      if (value === 'none') return [['-none', { t: 'bool', v: true }]];
      const layers = splitTop(value, ',');
      for (const [n, layer] of layers.entries()) {
        const parts = splitTop(layer, ' ');
        const inset = parts[0] === 'inset';
        const rest = inset ? parts.slice(1) : parts;
        const col = R.color(rest[rest.length - 1]);
        const lens = rest.slice(0, -1).map(w => R.length(w));
        if (!col || lens.some(x => !x) || lens.length < 2) return null;
        const p = `-shadow-${n + 1}`;
        put(`${p}-inset`, { t: 'bool', v: inset });
        put(`${p}-x`, lens[0]); put(`${p}-y`, lens[1]);
        put(`${p}-blur`, lens[2] ?? { t: 'length', v: 0 });
        put(`${p}-spread`, lens[3] ?? { t: 'length', v: 0 });
        put(`${p}-color`, col);
      }
      return out;
    }
    case 'transition': {
      for (const layer of splitTop(value, ',')) {
        const parts = splitTop(layer, ' ');
        const name = parts.find(w => /^[a-z-]+$/.test(w) && !R.easing(w)) ?? 'all';
        const durs = parts.map(R.duration).filter(Boolean);
        const ease = parts.map(R.easing).find(Boolean);
        if (!durs.length) return null;
        put(`-${name}-duration`, durs[0]);
        if (durs[1]) put(`-${name}-delay`, durs[1]);
        if (ease) put(`-${name}-easing`, ease);
      }
      return out;
    }
    case 'animation': {
      const parts = splitTop(splitTop(value, ',')[0], ' ');
      const durs = parts.map(R.duration).filter(Boolean);
      const ease = parts.map(R.easing).find(Boolean);
      const name = parts.find(w => /^[a-z][\w-]*$/.test(w) && !R.easing(w) && !['forwards', 'backwards', 'both', 'infinite', 'alternate', 'none'].includes(w));
      if (!durs.length) return null;
      if (name) put('-name', { t: 'string', v: name });
      put('-duration', durs[0]);
      if (durs[1]) put('-delay', durs[1]);
      if (ease) put('-easing', ease);
      if (parts.includes('infinite')) put('-infinite', { t: 'bool', v: true });
      return out;
    }
    case 'transform': case 'filter': case 'backdrop-filter': case '-webkit-backdrop-filter': {
      if (value === 'none') return [['-none', { t: 'bool', v: true }]];
      const pre = prop === 'transform' ? '' : prop.endsWith('backdrop-filter') ? '-backdrop' : '';
      for (const [f, args] of fns(value)) {
        const a = splitTop(args, ',').flatMap(x => splitTop(x, ' '));
        const name = `${pre}-${f.toLowerCase()}`;
        if (f === 'scale' || f === 'scaleX' || f === 'scaleY') {
          const v = a.map(x => R.num(x));
          if (v.some(x => x === null)) return null;
          if (f === 'scale' && v.length === 2) { put(`${name}-x`, { t: 'float', v: v[0] }); put(`${name}-y`, { t: 'float', v: v[1] }); }
          else put(name, { t: 'float', v: v[0] });
        } else if (/^translate/.test(f) || f === 'blur') {
          const v = a.map(x => R.length(x) ?? R.percent(x));
          if (v.some(x => !x)) return null;
          if (v.length === 2) { put(`${name}-x`, v[0]); put(`${name}-y`, v[1]); } else put(name, v[0]);
        } else if (/^rotate/.test(f)) {
          const m = /^(-?(?:\d+\.?\d*|\.\d+))(deg|turn)$/.exec(a[0] ?? '');
          if (!m) return null;
          put(name, { t: 'float', v: Number(m[1]) * (m[2] === 'turn' ? 360 : 1) });
        } else if (['saturate', 'brightness', 'opacity', 'contrast'].includes(f)) {
          const v = R.percent(a[0]) ?? (R.num(a[0]) !== null ? { t: 'float', v: R.num(a[0]) } : null);
          if (!v) return null;
          put(name, v);
        } else return null;
      }
      return out.length ? out : null;
    }
    case 'scale': {
      const v = words.map(w => R.num(w));
      if (v.some(x => x === null)) return null;
      if (v.length === 2) return [['-x', { t: 'float', v: v[0] }], ['-y', { t: 'float', v: v[1] }]];
      return [['', { t: 'float', v: v[0] }]];
    }
    case 'border': case 'outline': case 'border-top': case 'border-bottom': case 'border-left': case 'border-right': {
      if (value === 'none' || value === '0') return [['-none', { t: 'bool', v: true }]];
      const len = words.map(R.length).find(Boolean);
      const col = words.map(R.color).find(Boolean);
      if (!len && !col) return null;
      if (len) put('-width', len);
      if (col) put('-color', col);
      return out;
    }
    case 'background': case 'background-image': {
      const c = R.color(value);
      if (c) return [['', c]];
      const g = /^(linear|radial|conic)-gradient\((.*)\)$/.exec(value);
      if (!g) return null;
      const parts = splitTop(g[2], ',');
      let n = 0;
      for (const p of parts) {
        const w = splitTop(p, ' ');
        const col = R.color(w[0]);
        if (!col) {
          const ang = /^(-?\d+\.?\d*)deg$/.exec(w[0]);
          if (ang) put('-angle', { t: 'float', v: Number(ang[1]) });
          continue;
        }
        n++;
        put(`-stop-${n}-color`, col);
        const pos = w.slice(1).map(x => R.percent(x) ?? R.length(x)).filter(Boolean);
        if (pos[0]) put(`-stop-${n}-at`, pos[0]);
        if (pos[1]) put(`-stop-${n}-to`, pos[1]);
      }
      return n ? out : null;
    }
    case 'grid-template-columns': case 'grid-template-rows': {
      const v = words.map(R.length);
      v.forEach((x, i) => x && put(`-${i + 1}`, x));
      return out.length ? out : null;
    }
    case 'font': return null;
    default: {
      if (words.length === 1) {
        const m = /^clamp\(([^,]+),([^,]+),([^,]+)\)$/.exec(value);
        if (m) {
          put('-min', R.length(m[1].trim()));
          put('-max', R.length(m[3].trim()));
          return out.length ? out : null;
        }
        const v = R.scalar(value);
        return v ? [['', v]] : null;
      }
      return null;
    }
  }
}
