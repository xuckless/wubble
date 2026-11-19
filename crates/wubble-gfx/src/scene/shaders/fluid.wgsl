// translates: core/gl/fluid.ts:9-108
// Stable fluids (Stam 1999), translated line for line from core/gl/fluid.ts.
// Semi-Lagrangian advection, vorticity confinement, divergence, Jacobi pressure, projection.
// Carries RGB dye. Splat coordinates are 0..1 with y up (see common.wgsl: the textures keep the
// WebGL row order, so every `vUv` below means exactly what it means in the GLSL).
//
// Prepend: common.wgsl (vs_fullscreen, VsOut), film.wgsl (sq), wave.wgsl, beam.wgsl. One fragment entry point per GLSL program.
// All passes share one uniform struct `FluidU` (binding 0) holding every GLSL uniform under its
// GLSL name; each pass reads only its own fields. Textures keep their GLSL names and get one
// binding each, so a pipeline with an auto layout only binds what that pass uses:
//   uVel 2 · uSrc 3 · uCurl 4 · uP 5 · uDiv 6.
// Formats: every target is RGBA16Float (velocity is signed; RGBA8 would clamp it to 0..1).
// div and curl use NEAREST in the TS; sampling here happens at texel centres ± one texel, so a
// single linear sampler gives the same values.

struct FluidU {
  uTexel: vec2<f32>,   // 1 / sim size
  uPoint: vec2<f32>,   // splat centre, 0..1, y up
  uValue: vec3<f32>,   // splat value (velocity xy, or dye rgb; ring: x = strength)
  uDt: f32,
  uDiss: f32,
  uAspect: f32,
  uRadius: f32,
  uCurlAmt: f32,
  uK: f32,             // scale factor (pressure warm start)
  uRing: i32,          // 1 = ripple ring
  _pad: vec2<f32>,
};

@group(0) @binding(0) var<uniform> U: FluidU;
@group(0) @binding(1) var samp: sampler;          // linear, clamp-to-edge
@group(0) @binding(2) var uVel: texture_2d<f32>;
@group(0) @binding(3) var uSrc: texture_2d<f32>;
@group(0) @binding(4) var uCurl: texture_2d<f32>;
@group(0) @binding(5) var uP: texture_2d<f32>;
@group(0) @binding(6) var uDiv: texture_2d<f32>;

fn tex(t: texture_2d<f32>, uv: vec2<f32>) -> vec4<f32> { return textureSampleLevel(t, samp, uv, 0.0); } // wubble change: level 0 explicitly (textureSampleLevel)

// @source slint-fluid-wgsl
@fragment
fn fs_advect(fin: VsOut) -> @location(0) vec4<f32> {
  let vUv = fin.vUv;
  let coord = vUv - U.uDt * tex(uVel, vUv).xy * U.uTexel;
  return tex(uSrc, coord) / (1.0 + U.uDiss * U.uDt);
}

// @source slint-fluid-splat-wgsl
@fragment
fn fs_splat(fin: VsOut) -> @location(0) vec4<f32> {
  let vUv = fin.vUv;
  var p = vUv - U.uPoint; p.x *= U.uAspect;
  let d2 = dot(p, p);
  var add: vec3<f32>;
  if (U.uRing == 1) {
    // outward push: a ripple. Velocity points away from the centre, strongest on a thin ring.
    let d = sqrt(d2);
    let ring = exp(-sq(d - U.uRadius * 3.0) / (U.uRadius * U.uRadius * 0.6));
    var dir: vec2<f32>; if (d > 1e-5) { dir = p / d; } else { dir = vec2<f32>(0.0); }
    dir.x /= U.uAspect;
    add = vec3<f32>(dir * U.uValue.x * ring, 0.0);
  } else {
    add = U.uValue * exp(-d2 / U.uRadius);
  }
  return vec4<f32>(tex(uSrc, vUv).xyz + add, 1.0);
}

@fragment
fn fs_curl(fin: VsOut) -> @location(0) vec4<f32> {
  let vUv = fin.vUv;
  let L = tex(uVel, vUv - vec2<f32>(U.uTexel.x, 0.0)).y;
  let R = tex(uVel, vUv + vec2<f32>(U.uTexel.x, 0.0)).y;
  let T = tex(uVel, vUv + vec2<f32>(0.0, U.uTexel.y)).x;
  let B = tex(uVel, vUv - vec2<f32>(0.0, U.uTexel.y)).x;
  return vec4<f32>(0.5 * (R - L - T + B), 0.0, 0.0, 1.0);
}

// @source slint-fluid-vorticity-wgsl
@fragment
fn fs_vorticity(fin: VsOut) -> @location(0) vec4<f32> {
  let vUv = fin.vUv;
  let L = tex(uCurl, vUv - vec2<f32>(U.uTexel.x, 0.0)).x;
  let R = tex(uCurl, vUv + vec2<f32>(U.uTexel.x, 0.0)).x;
  let T = tex(uCurl, vUv + vec2<f32>(0.0, U.uTexel.y)).x;
  let B = tex(uCurl, vUv - vec2<f32>(0.0, U.uTexel.y)).x;
  let C = tex(uCurl, vUv).x;
  var f = 0.5 * vec2<f32>(abs(T) - abs(B), abs(R) - abs(L));
  f /= length(f) + 1e-4;
  f *= U.uCurlAmt * C; f.y *= -1.0;
  let v = tex(uVel, vUv).xy + f * U.uDt;
  return vec4<f32>(clamp(v, vec2<f32>(-1000.0), vec2<f32>(1000.0)), 0.0, 1.0);
}

@fragment
fn fs_divergence(fin: VsOut) -> @location(0) vec4<f32> {
  let vUv = fin.vUv;
  var L = tex(uVel, vUv - vec2<f32>(U.uTexel.x, 0.0)).x;
  var R = tex(uVel, vUv + vec2<f32>(U.uTexel.x, 0.0)).x;
  var T = tex(uVel, vUv + vec2<f32>(0.0, U.uTexel.y)).y;
  var B = tex(uVel, vUv - vec2<f32>(0.0, U.uTexel.y)).y;
  let C = tex(uVel, vUv).xy;
  // free-slip walls: reflect at the edges
  if (vUv.x - U.uTexel.x < 0.0) { L = -C.x; }
  if (vUv.x + U.uTexel.x > 1.0) { R = -C.x; }
  if (vUv.y + U.uTexel.y > 1.0) { T = -C.y; }
  if (vUv.y - U.uTexel.y < 0.0) { B = -C.y; }
  return vec4<f32>(0.5 * (R - L + T - B), 0.0, 0.0, 1.0);
}

// @source slint-fluid-pressure-wgsl
@fragment
fn fs_pressure(fin: VsOut) -> @location(0) vec4<f32> {
  let vUv = fin.vUv;
  let L = tex(uP, vUv - vec2<f32>(U.uTexel.x, 0.0)).x;
  let R = tex(uP, vUv + vec2<f32>(U.uTexel.x, 0.0)).x;
  let T = tex(uP, vUv + vec2<f32>(0.0, U.uTexel.y)).x;
  let B = tex(uP, vUv - vec2<f32>(0.0, U.uTexel.y)).x;
  return vec4<f32>((L + R + B + T - tex(uDiv, vUv).x) * 0.25, 0.0, 0.0, 1.0);
}

@fragment
fn fs_gradient(fin: VsOut) -> @location(0) vec4<f32> {
  let vUv = fin.vUv;
  let L = tex(uP, vUv - vec2<f32>(U.uTexel.x, 0.0)).x;
  let R = tex(uP, vUv + vec2<f32>(U.uTexel.x, 0.0)).x;
  let T = tex(uP, vUv + vec2<f32>(0.0, U.uTexel.y)).x;
  let B = tex(uP, vUv - vec2<f32>(0.0, U.uTexel.y)).x;
  return vec4<f32>(tex(uVel, vUv).xy - vec2<f32>(R - L, T - B), 0.0, 1.0);
}

@fragment
fn fs_scale(fin: VsOut) -> @location(0) vec4<f32> {
  return tex(uSrc, fin.vUv) * U.uK;
}
