// The thinking orb's painter (orb.rs): wubble's own, no design shader. The design paints its frame
// on a 2D canvas (engine/core.ts `paintFrame`): each line stroked (butt caps), then each dot filled
// as an arc, in order, source-over, the colour `rgba(r,g,b,a)`. Here every mark is one instanced
// quad over its bounds; the fragment counts how many of 4×4 sample points per pixel the mark
// covers (the canvas's antialiased coverage: Skia's path AA samples a 4×4 grid per pixel,
// SUPERSAMPLE_SHIFT 2) and blends premultiplied, in instance order, into a float target. A second
// pass divides the alpha back out: Slint's Skia renderer imports a wgpu texture as unpremultiplied.
// Prepend: the scene prelude (vs_fullscreen).

struct OrbU {
  view: vec2<f32>,  // the target, physical px
  scale: f32,       // physical px per frame unit (CSS px of the tuned size)
  pad: f32,
};

@group(0) @binding(0) var<uniform> uOrb: OrbU;

struct MarkIn {
  // a dot: (x, y, r, 0); a line: (x1, y1, x2, y2), frame units
  @location(0) geo: vec4<f32>,
  // x: 0 a dot, 1 a line; y: the line's width, frame units
  @location(1) kind: vec4<f32>,
  // straight colour: r, g, b 0..255, alpha 0..1
  @location(2) ink: vec4<f32>,
};

struct MarkOut {
  @builtin(position) pos: vec4<f32>,
  @location(0) px: vec2<f32>,
  @location(1) @interpolate(flat) geo: vec4<f32>,
  @location(2) @interpolate(flat) kind: vec4<f32>,
  @location(3) @interpolate(flat) ink: vec4<f32>,
};

@vertex
fn vs_mark(@builtin(vertex_index) vi: u32, m: MarkIn) -> MarkOut {
  let s = uOrb.scale;
  var lo: vec2<f32>;
  var hi: vec2<f32>;
  if (m.kind.x < 0.5) {
    let r = m.geo.z * s + 1.0;
    lo = m.geo.xy * s - vec2<f32>(r);
    hi = m.geo.xy * s + vec2<f32>(r);
  } else {
    let h = m.kind.y * s * 0.5 + 1.0;
    lo = min(m.geo.xy, m.geo.zw) * s - vec2<f32>(h);
    hi = max(m.geo.xy, m.geo.zw) * s + vec2<f32>(h);
  }
  let c = vec2<f32>(f32(vi & 1u), f32((vi >> 1u) & 1u));
  let p = mix(lo, hi, c);
  var o: MarkOut;
  o.pos = vec4<f32>(p.x / uOrb.view.x * 2.0 - 1.0, 1.0 - p.y / uOrb.view.y * 2.0, 0.0, 1.0);
  o.px = p;
  o.geo = vec4<f32>(m.geo.xy * s, m.geo.zw * s);
  o.kind = vec4<f32>(m.kind.x, m.kind.y * s, 0.0, 0.0);
  o.ink = m.ink;
  return o;
}

fn covered(p: vec2<f32>, geo: vec4<f32>, kind: vec4<f32>) -> bool {
  if (kind.x < 0.5) {
    let d = p - geo.xy;
    return dot(d, d) <= geo.z * geo.z;
  }
  // a segment with butt caps: within half the width of the line, between its ends
  let a = geo.xy;
  let ab = geo.zw - a;
  let len2 = max(dot(ab, ab), 1e-12);
  let t = dot(p - a, ab) / len2;
  if (t < 0.0 || t > 1.0) {
    return false;
  }
  let q = a + ab * t - p;
  return dot(q, q) <= kind.y * kind.y * 0.25;
}

@fragment
fn fs_mark(i: MarkOut) -> @location(0) vec4<f32> {
  // the pixel's centre is i.px; its 4×4 sample points
  let base = floor(i.px);
  var n = 0.0;
  for (var sy = 0; sy < 4; sy++) {
    for (var sx = 0; sx < 4; sx++) {
      let p = base + (vec2<f32>(f32(sx), f32(sy)) + 0.5) / 4.0;
      if (covered(p, i.geo, i.kind)) {
        n += 1.0;
      }
    }
  }
  if (n == 0.0) {
    discard;
  }
  let a = i.ink.a * n / 16.0;
  return vec4<f32>(i.ink.rgb / 255.0 * a, a);
}

@group(0) @binding(1) var tAccum: texture_2d<f32>;

@fragment
fn fs_resolve(i: VsOut) -> @location(0) vec4<f32> {
  let c = textureLoad(tAccum, vec2<i32>(i.pos.xy), 0);
  if (c.a <= 0.0) {
    return vec4<f32>(0.0);
  }
  return vec4<f32>(c.rgb / c.a, c.a);
}
