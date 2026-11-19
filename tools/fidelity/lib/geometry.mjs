// Crop geometry: the rects a crop takes on the web capture and on wubble's shot, in each
// image's pixels.

/**
 * `[webRect, wubbleRect]` for `crop` of a scene, or null when the web element was missing.
 * `meta` is the capture's JSON (crop rects relative to the frame, CSS px); a direct crop uses
 * the same logical rect on both; a derived crop takes wubble's own logical rect. Both are
 * scaled by `dpr` and clamped to their images.
 */
export function cropRects(meta, crop, dpr, web, wubble) {
  const r = crop.name === 'window' && !crop.web ? [0, 0, meta.frame[2], meta.frame[3]] : meta.crops?.[crop.name];
  if (!r) return null;
  const scale = ([x, y, w, h]) => [x * dpr, y * dpr, w * dpr, h * dpr].map(Math.round);
  const clamp = ([x, y, w, h], img) => {
    const x0 = Math.max(0, Math.min(x, img.width)), y0 = Math.max(0, Math.min(y, img.height));
    return [x0, y0, Math.max(0, Math.min(x + w, img.width) - x0), Math.max(0, Math.min(y + h, img.height) - y0)];
  };
  return [clamp(scale(r), web), clamp(scale(crop.wubble ?? r), wubble)];
}
