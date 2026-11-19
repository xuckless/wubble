// translates: core/gl/scene.ts:77-105
// The living backdrop: dye → lit liquid on the dark ground, muted by `uMute` (SceneOptions
// intensity, default 0.3), with tap ripples as distortion waves. Translated line for line from
// COMPOSITE in core/gl/scene.ts. Drawn at HALF resolution into `bg` (RGBA16Float), which keeps
// the WebGL row order (row 0 = bottom; see common.wgsl).
// Prepend: common.wgsl, film.wgsl, wave.wgsl, beam.wgsl (render.rs always prepends all four).

// @source slint-composite-wgsl
struct CompositeU {
  uGround: vec3<f32>,
  uTime: f32,
  uView: vec2<f32>,       // full view size in logical px
  uRipN: i32,
  uMute: f32,
  uRip: array<vec4<f32>, 8>, // (x px, y px from top-left, age 0..1, reach px)
};

@group(0) @binding(0) var<uniform> U: CompositeU;
@group(0) @binding(1) var samp: sampler;
@group(0) @binding(2) var uDye: texture_2d<f32>;

fn dyeAt(uv: vec2<f32>) -> vec3<f32> { return textureSampleLevel(uDye, samp, uv, 0.0).rgb; } // wubble change: level 0 explicitly (textureSampleLevel)

@fragment
fn fs_composite(fin: VsOut) -> @location(0) vec4<f32> {
  let vUv = fin.vUv;
  let px = vec2<f32>(vUv.x, 1.0 - vUv.y) * U.uView;
  // tap ripples: the distortion-wave lens pushes the liquid outward, splits it spectrally at the front
  var disp = vec2<f32>(0.0); var band = 0.0;
  for (var i = 0; i < 8; i++) {
    if (i >= U.uRipN) { break; }
    let h = waveAt(px, U.uRip[i], 14.0, 34.0);
    disp += h.disp; band = max(band, h.band);
  }
  var uv = (px - disp) / U.uView; uv.y = 1.0 - uv.y;
  var sp = disp / U.uView * 0.45; sp.y = -sp.y;
  let dye = max(vec3<f32>(dyeAt(uv + sp).r, dyeAt(uv).g, dyeAt(uv - sp).b), vec3<f32>(0.0));
  var lit = 1.0 - exp(-dye * 0.9);
  let l = dot(lit, vec3<f32>(0.2126, 0.7152, 0.0722));
  lit = max(mix(vec3<f32>(l), lit, 0.85), vec3<f32>(0.0));
  var col = U.uGround + lit * U.uMute;
  col += thinFilm(300.0 + 120.0 * band, 0.9) * band * 0.05;
  // ±0.5/255 noise only to break 8-bit banding on the dark ground
  col += (hash1(px + fract(U.uTime)) - 0.5) / 255.0;
  return vec4<f32>(col, 1.0);
}
