// translates: core/gl/glsl.ts:68-91
// Wave lens, translated line for line from WAVE in core/gl/glsl.ts (after Ultra Reactivate's
// distortion-waves renderer: its waveLens + waveBlurMask + waveBand). A wavefront at signed
// distance d from each pixel lifts a Gaussian lens around the front, narrower ahead (d > 0) than
// behind, pushing samples along the front's normal. The push also drives a 5-tap spectral split
// and a progressive blur that reaches further ahead than behind. A soft colour band leads the
// front (smootherstep) and trails off behind it.
// A wave is vec4(x, y, age 0..1, reach px). Energy falls as (1 − age)², the radius eases out.
// Prepended to every shader by render.rs (after common.wgsl and film.wgsl).

// @source slint-wave-wgsl
struct WaveHit { disp: vec2<f32>, band: f32, blur: f32, };

fn waveAt(p: vec2<f32>, wv: vec4<f32>, amp: f32, width: f32) -> WaveHit {
  var h: WaveHit; h.disp = vec2<f32>(0.0); h.band = 0.0; h.blur = 0.0;
  let k = clamp(wv.z, 0.0, 1.0);
  let rad = mix(2.0, wv.w, 1.0 - pow(1.0 - k, 2.2));
  let raw = p - wv.xy; let r = max(length(raw), 1e-3);
  let n = raw / r;
  // the front isn't a perfect circle: two slow lobes, like the original's bend
  let a = atan2(raw.y, raw.x);
  let bend = 0.62 * sin(2.0 * a + k * 6.0) + 0.38 * sin(3.0 * a - k * 9.0 + 1.7);
  let d = r - rad + bend * width * 0.18 * (1.0 - k);
  let energy = (1.0 - k) * (1.0 - k);
  let u = d / (width * select(1.5, 0.85, d > 0.0));
  let lift = exp(-u * u);
  h.disp = n * energy * amp * lift;
  let bu = d / select(width * 0.9, width * 1.6, d > 0.0);
  h.blur = energy * exp(-bu * bu);
  if (d >= 0.0) { let q = clamp(1.0 - d / (width * 1.1), 0.0, 1.0); h.band = q * q * q * (q * (q * 6.0 - 15.0) + 10.0); }
  else { h.band = pow(1.0 - clamp(-d / (width * 2.4), 0.0, 1.0), 2.3); }
  h.band *= energy;
  return h;
}

// GLSL `const vec3 SPECTRUM[5]`. A module `const` array can't be indexed by a loop variable in
// every WGSL implementation, so it's a private var (same values, dynamically indexable).
var<private> SPECTRUM: array<vec3<f32>, 5> = array<vec3<f32>, 5>(vec3<f32>(0.571, 0.0, 0.0), vec3<f32>(0.343, 0.206, 0.0), vec3<f32>(0.086, 0.588, 0.086), vec3<f32>(0.0, 0.206, 0.343), vec3<f32>(0.0, 0.0, 0.571));
