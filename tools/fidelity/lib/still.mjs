// The still dye on the web side: the same table wubble's browser window shows
// (crates/wubble-gfx/src/scene/still_dye.txt), mapped onto the design's desktop canvas, where the
// browser window is a rect inside a bigger canvas. vite.config.mjs inlines these functions into
// the page (so they must stay self-contained: no imports, no outer names).
//
// A blob is window-relative: (fx, fy) are fractions of the window, top-left origin; its radius is
// `Fluid.splat`'s for a canvas as tall as the window. fs_splat/SPLAT adds `value * exp(-d^2 / r)`
// with d in canvas heights, so on a canvas `H` tall the same blob in px has `r * (h / H)^2`.

/** `fx fy colour radius strength` per line; `#` starts a comment. */
export function parseStill(text) {
  return text.split('\n')
    .map(l => l.split('#')[0].trim())
    .filter(Boolean)
    .map(l => {
      const v = l.split(/\s+/).map(Number);
      if (v.length !== 5 || v.some(Number.isNaN)) throw new Error(`still_dye.txt: ${l}`);
      return { fx: v[0], fy: v[1], color: v[2], radius: v[3], strength: v[4] };
    });
}

/**
 * Where a blob splats on a canvas `W`x`H` (CSS px) holding the window at `rect` {x, y, w, h}
 * (CSS px, relative to the canvas): `Fluid.splat`'s (x, y) (0..1, y up) and radius.
 */
export function toCanvas(blob, rect, W, H) {
  return {
    x: (rect.x + blob.fx * rect.w) / W,
    y: 1 - (rect.y + blob.fy * rect.h) / H,
    radius: blob.radius * (rect.h / H) ** 2,
  };
}
