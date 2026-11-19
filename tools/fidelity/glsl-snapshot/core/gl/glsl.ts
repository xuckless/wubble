/** GLSL chunks shared by the Wubble shaders. */

/**
 * Thin-film interference for a soap film (n = 1.33) in air.
 * Reflectance of a film of thickness `d` nm, at refracted-angle cosine `cosT`, for each RGB channel.
 * The first surface adds a half-wave phase shift, so R ∝ sin²(2π n d cosT / λ).
 * Each channel averages three wavelengths across its band so colours stay smooth, the way
 * real white light does, instead of the hard bands a single wavelength gives.
 * Films under ~30 nm reflect almost nothing: the "black film" just before a bubble pops.
 */
// @source thin-film
export const FILM = /* glsl */ `
vec3 thinFilm(float d, float cosT) {
  float opd = 2.0 * 1.33 * d * cosT;
  const vec3 l0 = vec3(680.0, 560.0, 470.0);
  const vec3 l1 = vec3(620.0, 530.0, 445.0);
  const vec3 l2 = vec3(590.0, 500.0, 420.0);
  vec3 r = sin(3.14159265 * opd / l0); r *= r;
  vec3 g = sin(3.14159265 * opd / l1); g *= g;
  vec3 b = sin(3.14159265 * opd / l2); b *= b;
  vec3 c = (r + g + b) / 3.0;
  // perceptual weighting: soap film reads brighter in green and blue than a flat average
  return c * vec3(1.0, 0.96, 1.08);
}
float cosRefracted(float cosI) {
  float s = sqrt(max(0.0, 1.0 - cosI * cosI)) / 1.33;
  return sqrt(max(0.0, 1.0 - s * s));
}`;

// @source noise
export const NOISE = /* glsl */ `
vec3 hash3(vec3 p) {
  p = vec3(dot(p, vec3(127.1, 311.7, 74.7)), dot(p, vec3(269.5, 183.3, 246.1)), dot(p, vec3(113.5, 271.9, 124.6)));
  return -1.0 + 2.0 * fract(sin(p) * 43758.5453123);
}
float noise3(vec3 p) {
  vec3 i = floor(p), f = fract(p);
  vec3 u = f * f * (3.0 - 2.0 * f);
  return mix(mix(mix(dot(hash3(i + vec3(0,0,0)), f - vec3(0,0,0)), dot(hash3(i + vec3(1,0,0)), f - vec3(1,0,0)), u.x),
                 mix(dot(hash3(i + vec3(0,1,0)), f - vec3(0,1,0)), dot(hash3(i + vec3(1,1,0)), f - vec3(1,1,0)), u.x), u.y),
             mix(mix(dot(hash3(i + vec3(0,0,1)), f - vec3(0,0,1)), dot(hash3(i + vec3(1,0,1)), f - vec3(1,0,1)), u.x),
                 mix(dot(hash3(i + vec3(0,1,1)), f - vec3(0,1,1)), dot(hash3(i + vec3(1,1,1)), f - vec3(1,1,1)), u.x), u.y), u.z);
}
float fbm(vec3 p) {
  float a = 0.5, s = 0.0;
  for (int i = 0; i < 4; i++) { s += a * noise3(p); p = p * 2.03 + vec3(1.7, 9.2, 3.1); a *= 0.5; }
  return s;
}
float hash1(vec2 p) { return fract(sin(dot(p, vec2(12.9898, 78.233))) * 43758.5453); }`;

export const SDF = /* glsl */ `
float sdRound(vec2 p, vec2 b, float r) {
  vec2 q = abs(p) - b + r;
  return length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - r;
}`;

/**
 * Wave lens, after Ultra Reactivate's distortion-waves renderer (its waveLens + waveBlurMask + waveBand):
 * a wavefront at signed distance d from each pixel lifts a Gaussian lens around the front, narrower
 * ahead (d > 0) than behind, pushing samples along the front's normal. The push also drives a
 * 5-tap spectral split (chromatic aberration) and a progressive blur that reaches further ahead
 * than behind. A soft colour band leads the front (smootherstep) and trails off behind it.
 *
 * A wave is vec4(x, y, age 0..1, reach px). Energy falls as (1 − age)², the radius eases out.
 * WAVE_PARAMS are tuned per surface by the caller through `uWaveAmp` (px of push at full energy).
 */
// @source wave-lens
export const WAVE = /* glsl */ `
struct WaveHit { vec2 disp; float band; float blur; };
WaveHit waveAt(vec2 p, vec4 wv, float amp, float width) {
  WaveHit h; h.disp = vec2(0.0); h.band = 0.0; h.blur = 0.0;
  float k = clamp(wv.z, 0.0, 1.0);
  float rad = mix(2.0, wv.w, 1.0 - pow(1.0 - k, 2.2));
  vec2 raw = p - wv.xy; float r = max(length(raw), 1e-3);
  vec2 n = raw / r;
  // the front isn't a perfect circle: two slow lobes, like the original's bend
  float a = atan(raw.y, raw.x);
  float bend = 0.62 * sin(2.0 * a + k * 6.0) + 0.38 * sin(3.0 * a - k * 9.0 + 1.7);
  float d = r - rad + bend * width * 0.18 * (1.0 - k);
  float energy = (1.0 - k) * (1.0 - k);
  float u = d / (width * (d > 0.0 ? 0.85 : 1.5));
  float lift = exp(-u * u);
  h.disp = n * energy * amp * lift;
  float bu = d / (d > 0.0 ? width * 1.6 : width * 0.9);
  h.blur = energy * exp(-bu * bu);
  if (d >= 0.0) { float q = clamp(1.0 - d / (width * 1.1), 0.0, 1.0); h.band = q * q * q * (q * (q * 6.0 - 15.0) + 10.0); }
  else h.band = pow(1.0 - clamp(-d / (width * 2.4), 0.0, 1.0), 2.3);
  h.band *= energy;
  return h;
}
const vec3 SPECTRUM[5] = vec3[5](vec3(0.571, 0.0, 0.0), vec3(0.343, 0.206, 0.0), vec3(0.086, 0.588, 0.086), vec3(0.0, 0.206, 0.343), vec3(0.0, 0.0, 0.571));`;


/**
 * Border beam and voice glow, after Ultra Reactivate's border-beam and voice-glow (CSS there,
 * ported into the glass shader here so they share the glass's geometry and run anywhere a shader runs).
 * `perimeterAt` is the distortion-waves renderer's arc-length parameterisation of a rounded rect.
 */
// @source beam-glow
export const BEAM = /* glsl */ `
vec3 sheen(float u) { // the brand film, cyclic: Sky → Lilac → Blush → Mint → Sky
  const vec3 c0 = vec3(0.486, 0.784, 1.0), c1 = vec3(0.722, 0.651, 1.0), c2 = vec3(1.0, 0.62, 0.847), c3 = vec3(0.541, 0.949, 0.816);
  float x = fract(u) * 4.0; int i = int(x); float m = fract(x); m = m * m * (3.0 - 2.0 * m);
  vec3 a = i == 0 ? c0 : i == 1 ? c1 : i == 2 ? c2 : c3;
  vec3 b = i == 0 ? c1 : i == 1 ? c2 : i == 2 ? c3 : c0;
  return mix(a, b, m);
}
// arc length along the rim of a rounded rect (clockwise from top centre) and the total perimeter
vec2 perimeterAt(vec2 p, vec2 halfSize, float r) {
  float ex = max(halfSize.x - r, 0.0), ey = max(halfSize.y - r, 0.0);
  float arc = 0.5 * 3.14159265 * r, P = 4.0 * (ex + ey + arc);
  float ax = abs(p.x), ay = abs(p.y);
  if (ax > ex && ay > ey) {
    vec2 v = p - vec2(sign(p.x) * ex, sign(p.y) * ey);
    float a, base;
    if (p.x >= 0.0 && p.y < 0.0) { a = atan(v.x, -v.y); base = ex; }
    else if (p.x >= 0.0) { a = atan(v.y, v.x); base = ex + arc + 2.0 * ey; }
    else if (p.y >= 0.0) { a = atan(-v.x, v.y); base = 3.0 * ex + 2.0 * arc + 2.0 * ey; }
    else { a = atan(-v.y, -v.x); base = 3.0 * ex + 3.0 * arc + 4.0 * ey; }
    return vec2(base + r * clamp(a, 0.0, 1.5707963), P);
  }
  bool sideHit = ax > ex ? true : (ay > ey ? false : (halfSize.x - ax) < (halfSize.y - ay));
  if (sideHit) return vec2(p.x >= 0.0 ? ex + arc + (p.y + ey) : 3.0 * ex + 3.0 * arc + 2.0 * ey + (ey - p.y), P);
  return vec2(p.y < 0.0 ? (p.x >= 0.0 ? p.x : P + p.x) : ex + 2.0 * arc + 2.0 * ey + (ex - p.x), P);
}
// voice lobes: (x offset, width, height, band) for a ~350 px reference width, fanning from the bottom centre
const vec4 LOBES[7] = vec4[7](vec4(0.0, 74.0, 46.0, 0.0), vec4(-36.0, 54.0, 40.0, 1.0), vec4(36.0, 54.0, 40.0, 1.0),
  vec4(-72.0, 48.0, 32.0, 2.0), vec4(72.0, 48.0, 32.0, 2.0), vec4(-108.0, 42.0, 26.0, 1.0), vec4(108.0, 42.0, 26.0, 1.0));
`;