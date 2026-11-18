// Border beam and voice glow helpers, translated line for line from BEAM in core/gl/glsl.ts
// (after Ultra Reactivate's border-beam and voice-glow). `perimeterAt` is the distortion-waves
// renderer's arc-length parameterisation of a rounded rect. Prepended by render.rs.

// @source slint-beam-wgsl
// the brand film, cyclic: Sky → Lilac → Blush → Mint → Sky
fn sheen(u: f32) -> vec3<f32> {
  let c0 = vec3<f32>(0.486, 0.784, 1.0); let c1 = vec3<f32>(0.722, 0.651, 1.0);
  let c2 = vec3<f32>(1.0, 0.62, 0.847); let c3 = vec3<f32>(0.541, 0.949, 0.816);
  let x = fract(u) * 4.0; let i = i32(x); var m = fract(x); m = m * m * (3.0 - 2.0 * m);
  var a = c3; var b = c0;
  if (i == 0) { a = c0; b = c1; } else if (i == 1) { a = c1; b = c2; } else if (i == 2) { a = c2; b = c3; }
  return mix(a, b, m);
}

// arc length along the rim of a rounded rect (clockwise from top centre) and the total perimeter
fn perimeterAt(p: vec2<f32>, halfSize: vec2<f32>, r: f32) -> vec2<f32> {
  let ex = max(halfSize.x - r, 0.0); let ey = max(halfSize.y - r, 0.0);
  let arc = 0.5 * 3.14159265 * r; let P = 4.0 * (ex + ey + arc);
  let ax = abs(p.x); let ay = abs(p.y);
  if (ax > ex && ay > ey) {
    let v = p - vec2<f32>(sign(p.x) * ex, sign(p.y) * ey);
    var a: f32; var base: f32;
    if (p.x >= 0.0 && p.y < 0.0) { a = atan2(v.x, -v.y); base = ex; }
    else if (p.x >= 0.0) { a = atan2(v.y, v.x); base = ex + arc + 2.0 * ey; }
    else if (p.y >= 0.0) { a = atan2(-v.x, v.y); base = 3.0 * ex + 2.0 * arc + 2.0 * ey; }
    else { a = atan2(-v.y, -v.x); base = 3.0 * ex + 3.0 * arc + 4.0 * ey; }
    return vec2<f32>(base + r * clamp(a, 0.0, 1.5707963), P);
  }
  var sideHit: bool;
  if (ax > ex) { sideHit = true; } else if (ay > ey) { sideHit = false; } else { sideHit = (halfSize.x - ax) < (halfSize.y - ay); }
  if (sideHit) { return vec2<f32>(select(3.0 * ex + 3.0 * arc + 2.0 * ey + (ey - p.y), ex + arc + (p.y + ey), p.x >= 0.0), P); }
  if (p.y < 0.0) { return vec2<f32>(select(P + p.x, p.x, p.x >= 0.0), P); }
  return vec2<f32>(ex + 2.0 * arc + 2.0 * ey + (ex - p.x), P);
}

// voice lobes: (x offset, width, height, band) for a ~350 px reference width, fanning from the bottom centre
var<private> LOBES: array<vec4<f32>, 7> = array<vec4<f32>, 7>(vec4<f32>(0.0, 74.0, 46.0, 0.0), vec4<f32>(-36.0, 54.0, 40.0, 1.0), vec4<f32>(36.0, 54.0, 40.0, 1.0),
  vec4<f32>(-72.0, 48.0, 32.0, 2.0), vec4<f32>(72.0, 48.0, 32.0, 2.0), vec4<f32>(-108.0, 42.0, 26.0, 1.0), vec4<f32>(108.0, 42.0, 26.0, 1.0));
