// translates: core/gl/scene.ts:441-474
// Copied from the-wallpaper-project src/wubble/slint/shaders/pop.wgsl @598f814 (owner's code).
// The pop: the film tears open from the touch point, the rim snaps back, droplets fly out and
// fade. Translated line for line from POP in core/gl/scene.ts. Prepend: common.wgsl, film.wgsl, wave.wgsl, beam.wgsl.
// Blend: premultiplied. Quad: centre ± (r·3 + 10) px. uK = age / life (life 0.36 s).

// @source slint-pop-wgsl
struct PopU {
  uQuad: vec4<f32>,
  uView: vec2<f32>,
  uC: vec2<f32>,
  uR: f32,
  uK: f32,
  uAng: f32,
  uSeed: f32,
  uDpr: f32,
  _p0: f32, _p1: f32, _p2: f32, // 64 bytes
};

@group(0) @binding(0) var<uniform> U: PopU;

@vertex
fn vs_pop(@builtin(vertex_index) vi: u32) -> QuadOut { return quadVs(vi, U.uQuad, U.uView); }

@fragment
fn fs_pop(fin: QuadOut) -> @location(0) vec4<f32> {
  let p = fin.vPx - U.uC; let d = length(p); let ang = atan2(p.y, p.x);
  let k = U.uK;
  var acc = vec4<f32>(0.0);
  // the tear: film remaining away from the tear point, snapping back to the far side
  let open = k * 3.4;
  let da = abs(glsl_mod(ang - U.uAng + 3.14159, 6.28318) - 3.14159);
  let keep = smoothstep(open - 0.25, open, da);
  let rr = U.uR * (1.0 + 0.12 * k);
  let w = max(0.6, 2.6 * (1.0 - k));
  let ring = exp(-sq(d - rr) / (w * w)) * keep * (1.0 - smoothstep(0.55, 1.0, k));
  let fc = thinFilm(300.0 + 300.0 * sin(ang * 3.0 + U.uSeed), 0.8) * 1.4 + 0.1;
  acc += vec4<f32>(fc, 1.0) * ring;
  // droplets thrown out along the tear
  for (var i = 0; i < 9; i++) {
    let fi = f32(i);
    let a = U.uAng + (hash1(vec2<f32>(fi, U.uSeed)) - 0.5) * 3.8;
    let sp = 0.6 + hash1(vec2<f32>(U.uSeed, fi)) * 0.9;
    let dist = U.uR * (1.0 + k * 1.6 * sp);
    let c = vec2<f32>(cos(a), sin(a)) * dist + vec2<f32>(0.0, k * k * U.uR * 0.5);
    let s = (1.2 + 1.8 * hash1(vec2<f32>(fi * 3.1, U.uSeed))) * (1.0 - k);
    let dd = length(p - c);
    let m = 1.0 - smoothstep(s - 0.6 / U.uDpr, s + 0.6 / U.uDpr, dd);
    acc += vec4<f32>(vec3<f32>(0.95), 1.0) * m * (1.0 - k);
  }
  acc.a = clamp(acc.a, 0.0, 1.0);
  return vec4<f32>(acc.rgb * acc.a, acc.a);
}
