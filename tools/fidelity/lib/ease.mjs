// CSS cubic-bezier easing: y at x for control points (x1, y1, x2, y2), the curve the CSS
// timing functions (`ease-out` = cubic-bezier(0, 0, .58, 1)) apply per keyframe interval.
// Solved by bisection on x (monotone for 0 <= x1, x2 <= 1), exact to 1e-9.

export function cubicBezier([x1, y1, x2, y2], x) {
  const at = (a, b, t) => 3 * a * t * (1 - t) ** 2 + 3 * b * t * t * (1 - t) + t ** 3;
  if (x <= 0) return 0;
  if (x >= 1) return 1;
  let lo = 0, hi = 1, t = x;
  for (let i = 0; i < 60; i++) {
    t = (lo + hi) / 2;
    if (at(x1, x2, t) < x) lo = t; else hi = t;
  }
  return at(y1, y2, t);
}

/** CSS `ease-out` (cubic-bezier(0, 0, .58, 1)) sampled at `n + 1` even steps of x. */
export const CSS_EASE_OUT = [0, 0, 0.58, 1];
export function easeTable(points, n) {
  return Array.from({ length: n + 1 }, (_, i) => cubicBezier(points, i / n));
}
