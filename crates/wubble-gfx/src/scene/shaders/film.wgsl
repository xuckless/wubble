// Shared WGSL chunks, translated line for line from core/gl/glsl.ts (FILM, NOISE, SDF).
// WGSL has no #include: render.rs prepends common.wgsl, this file, wave.wgsl and beam.wgsl
// to each shader source with `concat!(include_str!(..))`.
//
// GLSL → WGSL notes that matter here:
// - GLSL `mod(x, y)` is x − y·floor(x/y); WGSL `%` truncates. Use `glsl_mod`.
// - `pow(x, 2.0)` with x < 0 is undefined in both GLSL and WGSL (NaN on Metal/D3D). Where the
//   GLSL squares a signed value (with x*x, or with pow in the beam/voice rim), the WGSL uses `sq(x)`.
// - Texture reads use textureSampleLevel(.., 0.0) everywhere: WGSL's textureSample needs
//   uniform control flow, and these shaders sample inside loops and branches.

// @source slint-thin-film-wgsl
// Thin-film interference for a soap film (n = 1.33) in air.
// Reflectance of a film of thickness `d` nm at refracted-angle cosine `cosT`, per RGB channel.
// R ∝ sin²(2π n d cosT / λ); each channel averages three wavelengths across its band.
fn thinFilm(d: f32, cosT: f32) -> vec3<f32> {
  let opd = 2.0 * 1.33 * d * cosT;
  let l0 = vec3<f32>(680.0, 560.0, 470.0);
  let l1 = vec3<f32>(620.0, 530.0, 445.0);
  let l2 = vec3<f32>(590.0, 500.0, 420.0);
  var r = sin(3.14159265 * opd / l0); r *= r;
  var g = sin(3.14159265 * opd / l1); g *= g;
  var b = sin(3.14159265 * opd / l2); b *= b;
  let c = (r + g + b) / 3.0;
  // perceptual weighting: soap film reads brighter in green and blue than a flat average
  return c * vec3<f32>(1.0, 0.96, 1.08);
}
fn cosRefracted(cosI: f32) -> f32 {
  let s = sqrt(max(0.0, 1.0 - cosI * cosI)) / 1.33;
  return sqrt(max(0.0, 1.0 - s * s));
}

// @source slint-noise-wgsl
fn hash3(p0: vec3<f32>) -> vec3<f32> {
  let p = vec3<f32>(dot(p0, vec3<f32>(127.1, 311.7, 74.7)), dot(p0, vec3<f32>(269.5, 183.3, 246.1)), dot(p0, vec3<f32>(113.5, 271.9, 124.6)));
  return -1.0 + 2.0 * fract(sin(p) * 43758.5453123);
}
fn noise3(p: vec3<f32>) -> f32 {
  let i = floor(p);
  let f = fract(p);
  let u = f * f * (3.0 - 2.0 * f);
  return mix(mix(mix(dot(hash3(i + vec3<f32>(0.0, 0.0, 0.0)), f - vec3<f32>(0.0, 0.0, 0.0)), dot(hash3(i + vec3<f32>(1.0, 0.0, 0.0)), f - vec3<f32>(1.0, 0.0, 0.0)), u.x),
                 mix(dot(hash3(i + vec3<f32>(0.0, 1.0, 0.0)), f - vec3<f32>(0.0, 1.0, 0.0)), dot(hash3(i + vec3<f32>(1.0, 1.0, 0.0)), f - vec3<f32>(1.0, 1.0, 0.0)), u.x), u.y),
             mix(mix(dot(hash3(i + vec3<f32>(0.0, 0.0, 1.0)), f - vec3<f32>(0.0, 0.0, 1.0)), dot(hash3(i + vec3<f32>(1.0, 0.0, 1.0)), f - vec3<f32>(1.0, 0.0, 1.0)), u.x),
                 mix(dot(hash3(i + vec3<f32>(0.0, 1.0, 1.0)), f - vec3<f32>(0.0, 1.0, 1.0)), dot(hash3(i + vec3<f32>(1.0, 1.0, 1.0)), f - vec3<f32>(1.0, 1.0, 1.0)), u.x), u.y), u.z);
}
fn fbm(p0: vec3<f32>) -> f32 {
  var p = p0;
  var a = 0.5;
  var s = 0.0;
  for (var i = 0; i < 4; i++) { s += a * noise3(p); p = p * 2.03 + vec3<f32>(1.7, 9.2, 3.1); a *= 0.5; }
  return s;
}
fn hash1(p: vec2<f32>) -> f32 { return fract(sin(dot(p, vec2<f32>(12.9898, 78.233))) * 43758.5453); }

// @source slint-sdf-wgsl
fn sdRound(p: vec2<f32>, b: vec2<f32>, r: f32) -> f32 {
  let q = abs(p) - b + r;
  return length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0) - r;
}

// helpers with no GLSL counterpart (see notes at the top)
fn sq(x: f32) -> f32 { return x * x; }
fn glsl_mod(x: f32, y: f32) -> f32 { return x - y * floor(x / y); }
