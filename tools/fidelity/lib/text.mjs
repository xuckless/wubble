// A CSS rule's font as a text role: family (the `--wb-font-*` token's key), size (px), weight and
// line height, each taken from the rule's declarations (`font-family`, `font-size`,
// `font-weight`, `line-height`, or the `font` shorthand) or kept from `base` (what it inherits).

/** `decls`: `[{ prop, value }]`; `base`: `{ family, size, weight, lh }`. */
export function fontOf(decls, base) {
  const f = { ...base };
  for (const d of decls) {
    const v = d.value.trim();
    const fam = /var\(--wb-font-(\w+)\)/.exec(v);
    if (d.prop === 'font-family' && fam) f.family = fam[1];
    else if (d.prop === 'font-size' && /^[\d.]+px$/.test(v)) f.size = parseFloat(v);
    else if (d.prop === 'font-weight' && /^\d+$/.test(v)) f.weight = Number(v);
    else if (d.prop === 'line-height' && /^[\d.]+$/.test(v)) f.lh = Number(v);
    else if (d.prop === 'font' && v !== 'inherit') {
      // `[weight] size[/line-height] family`
      const m = /^(?:(\d{3})\s+)?([\d.]+)px(?:\/([\d.]+))?\s+(.+)$/.exec(v);
      if (!m) continue;
      if (m[1]) f.weight = Number(m[1]);
      f.size = parseFloat(m[2]);
      if (m[3]) f.lh = Number(m[3]);
      if (fam) f.family = fam[1];
    }
  }
  return f;
}
