// translates: core/gl/scene.ts:108-121
// Separable Gaussian for the glass blur pyramid, translated from BLUR in core/gl/scene.ts.
// Run twice per level: uStep = (1/src.w, 0) into the temp target, then (0, 1/tmp.h) into the
// level. bg (half res) → b1 (quarter, σ 3.5) → b2 (eighth, σ 4.5). Prepend: common.wgsl, film.wgsl, wave.wgsl, beam.wgsl (render.rs always prepends all four).

// @source slint-blur-wgsl
struct BlurU {
  uStep: vec2<f32>,
  uSigma: f32,
  _pad: f32,
};

@group(0) @binding(0) var<uniform> U: BlurU;
@group(0) @binding(1) var samp: sampler;
@group(0) @binding(2) var uTex: texture_2d<f32>;

@fragment
fn fs_blur(fin: VsOut) -> @location(0) vec4<f32> {
  let r = ceil(U.uSigma * 2.5);
  var s = vec4<f32>(0.0); var wsum = 0.0;
  for (var i = -12.0; i <= 12.0; i += 1.0) {
    if (abs(i) > r) { continue; }
    let w = exp(-i * i / (2.0 * U.uSigma * U.uSigma));
    s += textureSampleLevel(uTex, samp, fin.vUv + U.uStep * i, 0.0) * w; wsum += w;
  }
  return s / wsum;
}
