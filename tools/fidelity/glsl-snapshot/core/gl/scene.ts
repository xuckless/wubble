/**
 * The Wubble scene: one WebGL2 canvas that draws, back to front,
 *   1. the fluid backdrop (stable fluids carrying Space-coloured dye, with tap ripples),
 *   2. refractive glass surfaces (rounded rects that lens and split the backdrop behind them),
 *   3. thin-film bubbles (the soft-body logo bubble and the little ones),
 *   4. pops (the film tearing open, droplets flying).
 * All public coordinates are CSS px from the canvas's top-left corner.
 * Like the distortion-waves renderer in Ultra Reactivate, glass samples blurred,
 * downsampled copies of the backdrop, which are rebuilt on the GPU each frame at low resolution.
 */
import { Fluid, type FluidOptions } from './fluid';
import { BEAM, FILM, NOISE, SDF, WAVE } from './glsl';
import { VERT_FS, VERT_QUAD, bindTex, formats, freeTarget, hex, program, quad, target, type Program, type Target } from './util';

export const RING_N = 24;

/** A distortion wave travelling through a surface: CSS px, seconds. */
export interface Wave { x: number; y: number; age: number; life: number; reach: number }

export interface GlassSurface {
  x: number; y: number; w: number; h: number; r: number;
  /** 0 thin, 1 regular, 2 thick */
  tier: number;
  /** 0..1, the touched film rim */
  film: number;
  /** 0..1, how far the surface is shown (fades and scales are done by the owner) */
  alpha?: number;
  /** RGB 0..1 tint mixed into the glass at 10% (the Space colour on sidebar glass) */
  tint?: [number, number, number];
  shadow?: number;
  /** waves running through the glass (presses) */
  waves?: Wave[];
  /** a beam of light running round the rim: head and tail length as fractions of the perimeter */
  beam?: { head: number; length: number; strength: number; hue: number };
  /** voice glow: three band levels 0..1, the flow phase, and an overall amount */
  voice?: { low: number; mid: number; high: number; phase: number; amount: number };
  /** 1 = full glass; 0 = effects only (beam, glow, waves' light) for surfaces drawn over live DOM */
  material?: number;
}

export interface BubbleState {
  x: number; y: number; r: number;
  /** RING_N radial offsets in px; omit for a perfect circle */
  ring?: Float32Array;
  /** 0 clear soap bubble, 1 the logo's dark-glass body */
  body?: number;
  alpha?: number;
  /** the logo W: inverse 2×2 transform (column-major), offset in viewBox units, and px per viewBox unit */
  w?: { inv: [number, number, number, number]; off: [number, number]; scale: number };
  /** extra film swirl, added by touches */
  swirl?: number;
  seed?: number;
  /** yaw, pitch in radians: where the bubble's face turns */
  face?: [number, number];
  /** waves running through the body (pokes, flings) */
  waves?: Wave[];
}

export interface PopState { x: number; y: number; r: number; age: number; life: number; angle: number; seed: number }

export interface SceneOptions {
  /** three dye colours */
  palette?: string[];
  ground?: string;
  fluid?: FluidOptions;
  /** emitters keep the liquid drifting on its own */
  ambient?: number;
  /** pointer pushes the liquid */
  pointerForce?: number;
  /** how strongly the liquid shows over the ground, 0..1 (0.3 = muted, the default) */
  intensity?: number;
  /** transparent canvas with bubbles and pops only, laid over DOM content */
  overlay?: boolean;
}

// @source backdrop
const COMPOSITE = `#version 300 es
precision highp float;
in vec2 vUv; out vec4 o;
uniform sampler2D uDye; uniform vec3 uGround; uniform vec2 uView; uniform float uTime, uMute;
uniform vec4 uRip[8]; uniform int uRipN;
${FILM}
${NOISE}
${WAVE}
void main() {
  vec2 px = vec2(vUv.x, 1.0 - vUv.y) * uView;
  // tap ripples: the distortion-wave lens pushes the liquid outward, splits it spectrally at the front
  vec2 disp = vec2(0.0); float band = 0.0;
  for (int i = 0; i < 8; i++) {
    if (i >= uRipN) break;
    WaveHit h = waveAt(px, uRip[i], 14.0, 34.0);
    disp += h.disp; band = max(band, h.band);
  }
  vec2 uv = (px - disp) / uView; uv.y = 1.0 - uv.y;
  vec2 sp = disp / uView * 0.45; sp.y = -sp.y;
  vec3 dye = max(vec3(texture(uDye, uv + sp).r, texture(uDye, uv).g, texture(uDye, uv - sp).b), 0.0);
  vec3 lit = 1.0 - exp(-dye * 0.9);
  float l = dot(lit, vec3(0.2126, 0.7152, 0.0722));
  lit = max(mix(vec3(l), lit, 0.85), 0.0);
  vec3 col = uGround + lit * uMute;
  col += thinFilm(300.0 + 120.0 * band, 0.9) * band * 0.05;
  // ±0.5/255 noise only to break 8-bit banding on the dark ground
  col += (hash1(px + fract(uTime)) - 0.5) / 255.0;
  o = vec4(col, 1.0);
}`;

// @source blur
const BLUR = `#version 300 es
precision highp float;
in vec2 vUv; out vec4 o;
uniform sampler2D uTex; uniform vec2 uStep; uniform float uSigma;
void main() {
  float r = ceil(uSigma * 2.5);
  vec4 s = vec4(0.0); float wsum = 0.0;
  for (float i = -12.0; i <= 12.0; i += 1.0) {
    if (abs(i) > r) continue;
    float w = exp(-i * i / (2.0 * uSigma * uSigma));
    s += texture(uTex, vUv + uStep * i) * w; wsum += w;
  }
  o = s / wsum;
}`;

const BLIT = `#version 300 es
precision highp float;
in vec2 vUv; out vec4 o;
uniform sampler2D uTex;
void main() { o = texture(uTex, vUv); }`;

// @source glass-shader
const GLASS = `#version 300 es
precision highp float;
in vec2 vPx; out vec4 o;
uniform vec2 uView; uniform vec4 uRect; uniform float uR, uTier, uFilm, uAlpha, uShadow, uTime, uDpr;
uniform vec2 uLight; uniform vec3 uTint;
uniform sampler2D uBg, uB1, uB2, uB3;
uniform vec4 uWave[4]; uniform int uWaveN;
uniform vec4 uBeam; uniform vec4 uVoice; uniform float uVoiceAmt, uMaterial;
${SDF}
${FILM}
${WAVE}
${BEAM}
float sq(float x) { return x * x; }
// Frosted: the glass reads the two deepest blur levels (≈20 and ≈44 CSS px sigma), never the sharp backdrop.
// thin 0 → 40% toward the deepest, regular 1 → 75%, thick 2 → all of it; a wave's blur pushes deeper still.
vec3 bg(vec2 px, float tier) {
  vec2 uv = px / uView; uv.y = 1.0 - uv.y;
  vec3 a = texture(uB2, uv).rgb, b = texture(uB3, uv).rgb;
  return mix(a, b, clamp(0.4 + tier * 0.3, 0.0, 1.0));
}
const vec3 LUMA = vec3(0.2126, 0.7152, 0.0722);
// @source glass-frost
// Coloured frost: what's behind keeps its hue but loses its detail and most of its contrast. The light is
// pulled toward a milky floor (so a dark ground still reads as a pane, not a hole), and the colour that is
// left is lifted (×2.7) so the pane carries the tint of what's behind it.
const vec3 FROST = vec3(0.115, 0.85, 2.7); // milk floor, how much of the original light survives, colour lift
vec3 frost(vec3 b, float tier) {
  float L = dot(b, LUMA);
  vec3 ch = b - L;
  float floorL = FROST.x + 0.012 * tier;             // thicker glass is a touch milkier
  return vec3(floorL + L * FROST.y) + ch * FROST.z;
}
void main() {
  vec2 half_ = uRect.zw * 0.5, c = uRect.xy + half_, p = vPx - c;
  float r = min(uR, min(half_.x, half_.y));
  float d = sdRound(p, half_, r);
  float aa = 0.75 / uDpr;
  // soft drop shadow, grows with tier
  float sBlur = mix(24.0, 60.0, uTier * 0.5), sOff = mix(10.0, 24.0, uTier * 0.5);
  float ds = sdRound(p - vec2(0.0, sOff * 0.5), half_ - 2.0, r);
  float shadow = (1.0 - smoothstep(-sBlur * 0.3, sBlur, ds)) * mix(0.22, 0.38, uTier * 0.5) * uShadow; // soft: frost floats, it isn't a slab
  float inside = 1.0 - smoothstep(-aa, aa, d);

  // ── border beam: a head running round the rim with a smootherstep tail; stroke, inner glow, bloom
  vec3 fx = vec3(0.0); float fxA = 0.0; vec3 bloomC = vec3(0.0); float bloomA = 0.0;
  if (uBeam.z > 0.001) {
    vec2 per = perimeterAt(p, half_, r);
    float s = per.x / per.y;
    float behind = fract(uBeam.x - s);
    float ahead = fract(s - uBeam.x);
    float along = behind < uBeam.y ? pow(1.0 - behind / uBeam.y, 2.2) : 0.0;
    along = max(along, 1.0 - smoothstep(0.0, 0.012, ahead)); // a crisp head
    float stroke = exp(-sq((d + 0.8) / 1.0));
    float inner = d < 0.0 ? exp(d / 10.0) : 0.0;
    float bloom = d > 0.0 ? exp(-d / 8.0) : 0.0;
    vec3 bc = sheen(s * 1.5 + uBeam.w);
    float k = along * uBeam.z;
    fx += bc * (stroke * 1.3 + inner * 0.32) * k; fxA = max(fxA, (stroke * 0.9 + inner * 0.25) * k);
    bloomC += bc * bloom * 0.55 * k; bloomA += bloom * 0.4 * k;
  }
  // ── voice glow: seven soft lobes fanning up from the bottom centre, each following a band
  if (uVoiceAmt > 0.001) {
    float sc = uRect.z / 350.0;
    vec2 base = vec2(0.0, half_.y);
    float levels[3] = float[3](uVoice.x, uVoice.y, uVoice.z);
    float spread = 1.0 + 0.35 * uVoice.y;
    vec3 vc = vec3(0.0); float va = 0.0;
    for (int i = 0; i < 7; i++) {
      vec4 L = LOBES[i];
      float lv = levels[int(L.w)];
      float x = L.x * sc * spread + sin(uVoice.w + float(i)) * 6.0 * sc;
      vec2 e = (p - base - vec2(x, 0.0)) / (vec2(L.y, L.z) * sc * (0.55 + 0.9 * lv));
      float g = exp(-dot(e, e) * 1.4) * (0.25 + lv);
      vc += sheen(float(i) * 0.13 + uVoice.w * 0.05) * g; va += g;
    }
    float rimGlow = exp(-sq((d + 0.8) / 1.2)) * (0.3 + uVoice.x + uVoice.y) * 0.5;
    fx += (vc * 0.6 + sheen(atan(p.y, p.x) / 6.2831853 + uVoice.w * 0.04) * rimGlow) * uVoiceAmt * step(d, 0.5);
    fxA = max(fxA, clamp((va * 0.45 + rimGlow) * uVoiceAmt, 0.0, 1.0) * step(d, 0.5));
    float vb = d > 0.0 ? exp(-d / 10.0) * smoothstep(-half_.y, half_.y, p.y) : 0.0;
    bloomC += vc * vb * 0.4 * uVoiceAmt; bloomA += min(1.0, va) * vb * 0.3 * uVoiceAmt;
  }

  if (inside <= 0.0) {
    float sa = shadow * uMaterial;
    o = vec4(bloomC, clamp(bloomA, 0.0, 1.0)) * uAlpha + vec4(0.0, 0.0, 0.0, sa * uAlpha) * (1.0 - clamp(bloomA, 0.0, 1.0));
    return;
  }
  if (uMaterial < 0.5) {
    // effects only, over live content
    float a0 = inside * uAlpha;
    o = vec4(fx * a0, clamp(fxA, 0.0, 1.0) * a0);
    return;
  }

  // normal from the SDF gradient
  vec2 e = vec2(0.5, 0.0);
  vec2 n = normalize(vec2(sdRound(p + e.xy, half_, r) - sdRound(p - e.xy, half_, r), sdRound(p + e.yx, half_, r) - sdRound(p - e.yx, half_, r)) + 1e-5);
  // frosted glass doesn't lens: the rim only bends the light a little, so the edge stays soft
  float bezel = clamp(min(half_.x, half_.y) * 0.5, 6.0, 22.0);
  float t = clamp(1.0 + d / bezel, 0.0, 1.0);
  vec2 off = -n * t * t * bezel * 0.35;
  // a press sends a distortion wave through the glass: lens push, extra blur, spectral split
  vec2 disp = vec2(0.0); float wBand = 0.0, wBlur = 0.0;
  for (int i = 0; i < 4; i++) {
    if (i >= uWaveN) break;
    WaveHit h = waveAt(vPx, uWave[i], 9.0, 26.0);
    disp += h.disp; wBand = max(wBand, h.band); wBlur = max(wBlur, h.blur);
  }
  float tierB = min(2.0, uTier + wBlur * 1.2);
  // the wave still splits the light spectrally as it passes (only the wave: resting frost has no dispersion)
  vec3 col;
  col.r = bg(vPx + off - disp * 0.85, tierB).r;
  col.g = bg(vPx + off - disp * 1.00, tierB).g;
  col.b = bg(vPx + off - disp * 1.18, tierB).b;
  col = frost(col, uTier);
  // the Space colour, lightly: the colour should mostly come from what's behind
  col = mix(col, uTint * 0.55 + 0.1, 0.07 * step(0.001, dot(uTint, uTint)));
  // the wave's leading band catches a little film colour
  col += (thinFilm(320.0 + 140.0 * wBand, 0.85) * 0.12 + 0.03) * wBand;
  // pointer light: a broad, soft brightening in the frost (no specular band, no hotspot)
  vec2 lp = uLight - c;
  float glow = exp(-dot(p - lp, p - lp) / (2.0 * sq(max(uRect.z, uRect.w) * 0.45)));
  col += glow * 0.03;
  // the edge: a faint, soft light just inside, a little brighter facing the pointer. No hairline.
  float rim = exp(d / 1.6) * step(d, 0.0);
  float facing = max(0.0, dot(n, normalize(lp - p + 1e-4)));
  col += rim * (0.035 + 0.05 * facing * facing);
  // ±0.5/255 grain against 8-bit banding in the smooth frost
  col += (fract(sin(dot(vPx, vec2(12.9898, 78.233))) * 43758.5453) - 0.5) / 255.0;
  // film rim: the brand sheen as a soft 1.5 px ring that drifts slowly round the edge, only when touched.
  // It matches the CSS .wb-film rim (a conic Sky → Lilac → Blush → Mint ring at 80%), not interference
  // colours: high-order film bands read as anodised metal on small controls.
  if (uFilm > 0.001) {
    vec2 per = perimeterAt(p, half_, r);
    float s = per.x / per.y;
    vec3 rc = sheen(s + 0.58 - uTime * 0.06);
    float ring = 1.0 - smoothstep(1.0, 2.0, -d);           // 1.5 px band, antialiased inward
    float glowIn = exp(d / 5.0) * 0.12;                        // a breath of colour just inside
    col = mix(col, rc, ring * 0.8 * uFilm);
    col += rc * glowIn * uFilm;
  }
  col += fx;
  float a = inside * uAlpha;
  o = vec4(col * a, a) + vec4(0.0, 0.0, 0.0, shadow * uAlpha) * (1.0 - a);
}`;

// @source bubble-shader
const BUBBLE = `#version 300 es
precision highp float;
in vec2 vPx; out vec4 o;
uniform vec2 uView, uC, uLight; uniform float uR, uRing[${RING_N}], uBody, uAlpha, uTime, uSwirl, uSeed, uDpr, uHasW, uOverlay;
uniform sampler2D uBg, uW; uniform mat2 uWInv; uniform vec2 uWOff; uniform float uWScale;
uniform sampler2D uB1, uB2;
uniform vec4 uWave[4]; uniform int uWaveN;
uniform vec2 uFace; // yaw, pitch (radians): the bubble turns its face toward the cursor
${FILM}
// rotate a direction by yaw (about y) then pitch (about x), and back. Screen space: x right, y down, z toward the viewer.
vec3 faceRot(vec3 v, vec2 a) {
  float cy = cos(a.x), sy = sin(a.x); v = vec3(v.x * cy + v.z * sy, v.y, -v.x * sy + v.z * cy);
  float cp = cos(a.y), sp = sin(a.y); return vec3(v.x, v.y * cp + v.z * sp, -v.y * sp + v.z * cp);
}
vec3 faceRotInv(vec3 v, vec2 a) {
  float cp = cos(a.y), sp = sin(a.y); v = vec3(v.x, v.y * cp - v.z * sp, v.y * sp + v.z * cp);
  float cy = cos(a.x), sy = sin(a.x); return vec3(v.x * cy - v.z * sy, v.y, v.x * sy + v.z * cy);
}
${NOISE}
float ringAt(float ang) {
  float f = (ang / 6.2831853 + 0.5) * ${RING_N}.0;
  int i1 = int(floor(f)); float t = fract(f);
  int i0 = (i1 + ${RING_N - 1}) % ${RING_N}, i2 = (i1 + 1) % ${RING_N}, i3 = (i1 + 2) % ${RING_N};
  i1 = i1 % ${RING_N};
  float p0 = uRing[i0], p1 = uRing[i1], p2 = uRing[i2], p3 = uRing[i3];
  // Catmull-Rom through the ring samples
  return 0.5 * ((2.0 * p1) + (-p0 + p2) * t + (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3) * t * t + (-p0 + 3.0 * p1 - 3.0 * p2 + p3) * t * t * t);
}
${WAVE}
// the backdrop at a blur level 0..1 (sharp → the blur pyramid), as the original's stacked copies
vec3 bgLevel(vec2 px, float tb) {
  vec2 uv = px / uView; uv.y = 1.0 - uv.y;
  vec3 c = textureLod(uBg, uv, 0.0).rgb;
  if (tb > 0.0) c = mix(c, textureLod(uB1, uv, 0.0).rgb, smoothstep(0.0, 0.5, tb));
  if (tb > 0.5) c = mix(c, textureLod(uB2, uv, 0.0).rgb, smoothstep(0.5, 1.0, tb));
  return c;
}
// 5-tap spectral split along the push, weighted so the taps sum back to white
vec3 spectral(vec2 px, vec2 split, float tb) {
  if (dot(split, split) < 0.0625) return bgLevel(px, tb);
  vec3 o = vec3(0.0);
  for (int i = 0; i < 5; i++) o += bgLevel(px + split * (float(i) / 2.0 - 1.0), tb) * SPECTRUM[i];
  return o;
}
vec3 bgAt(vec2 px) { return bgLevel(px, 0.0); }
void main() {
  vec2 p = vPx - uC;
  float ang = atan(p.y, p.x);
  float R = uR + ringAt(ang);
  float len = length(p);
  float d = len - R;
  float aa = 0.9 / uDpr;
  float inside = 1.0 - smoothstep(-aa, aa, d);
  if (inside <= 0.0) { o = vec4(0.0); return; }
  vec2 q = p / R; float rho = min(length(q), 1.0);
  float nz = sqrt(max(0.0, 1.0 - rho * rho));
  vec3 n = normalize(vec3(q, nz));
  float cosI = nz, cosT = cosRefracted(cosI);

  // film thickness: first-order only (≈200–560 nm), so colours stay soft and pastel, never banded.
  // Swirled slowly by noise and drained by gravity: a little thicker (warmer) toward the bottom.
  float t = uTime * 0.025 + uSeed;
  vec3 sp = vec3(n.xy * 1.1, t);
  float ca = cos(uSwirl + t * 0.5), sa = sin(uSwirl + t * 0.5);
  sp.xy = mat2(ca, -sa, sa, ca) * sp.xy;
  float warp = fbm(sp + vec3(fbm(sp * 1.3 + 4.0), fbm(sp * 1.3 + 9.0), 0.0) * 0.8);
  float drain = 0.5 + 0.5 * q.y;
  float thick = 250.0 + 150.0 * drain + 170.0 * warp;
  vec3 film = thinFilm(thick, cosT);
  film = mix(vec3(dot(film, vec3(0.3333))), film, 0.72) * 0.9 + 0.08; // calm the saturation
  float fres = pow(1.0 - cosI, 3.0);

  // body: a clear lens for little bubbles, smooth dark glass for the logo
  // pokes send a distortion wave through the body: it lenses, splits and blurs what the bubble refracts
  WaveHit wv; wv.disp = vec2(0.0); wv.band = 0.0; wv.blur = 0.0;
  for (int i = 0; i < 4; i++) {
    if (i >= uWaveN) break;
    WaveHit h = waveAt(vPx, uWave[i], R * 0.12, R * 0.22);
    wv.disp += h.disp; wv.band = max(wv.band, h.band); wv.blur = max(wv.blur, h.blur);
  }
  vec2 lensOff = -q * (1.0 - nz) * R * 0.38;
  vec3 refr = spectral(vPx + lensOff - wv.disp, wv.disp * 0.5, wv.blur * 0.8);
  vec3 glassTop = vec3(0.150, 0.165, 0.215), glassBot = vec3(0.035, 0.039, 0.055);
  // the lit side of the glass faces the light (the pointer)
  vec2 ld = (uLight - uC) / (uR * 1.2);
  vec2 ldir = length(ld) > 0.05 ? normalize(ld) : vec2(-0.3, -0.95);
  float lit = dot(q, ldir) * min(1.0, 0.45 + length(ld) * 0.4);
  vec3 bodyCol = mix(glassBot, glassTop, smoothstep(-0.9, 0.9, lit));
  bodyCol += refr * 0.12;
  vec3 col = mix(refr, bodyCol, uBody);
  // film shows only where the surface turns away (Fresnel), never as a wash across the face
  col += film * fres * mix(0.75, 0.55, uBody);
  // light carried through the glass gathers at the lower rim, tinted by the film
  col += film * smoothstep(0.6, 1.0, rho) * max(0.0, q.y) * 0.22 * uBody;
  // a hairline rim and a faint crescent reflection along the top
  float rimLine = exp(-(d + 1.1) * (d + 1.1));
  col = mix(col, film * 1.1 + 0.1, rimLine * 0.55);
  float crescent = smoothstep(0.78, 0.97, rho) * smoothstep(0.1, 0.8, dot(q, ldir));
  col += crescent * 0.16;
  float rimBand = rimLine;
  // the front of the wave catches the light: a quiet film-coloured band
  col += (film * 0.5 + 0.06) * wv.band * 0.35;

  // the W: a glass tube in the brand sheen (Sky → Lilac → Blush → Mint), lit softly
  float wA = 0.0;
  if (uHasW > 0.5) {
    // the W lives on an inner sphere (radius 0.9 R). Un-rotate the visible point to find where it sits on the face.
    const float KR = 0.9;
    vec2 qi = (q - wv.disp * 0.55 / R) / KR; float ri = dot(qi, qi);
    vec3 si = vec3(qi, sqrt(max(0.0, 1.0 - ri)));
    vec3 s0 = faceRotInv(si, uFace);
    float facing = smoothstep(0.05, 0.25, s0.z) * step(ri, 1.0);
    vec2 v = (s0.xy * KR * R) / uWScale + vec2(60.0);
    vec2 vw = uWInv * (v - uWOff) + vec2(60.0, 65.0);
    vec2 wuv = vec2(vw.x / 120.0, 1.0 - vw.y / 120.0);
    if (wuv.x > 0.0 && wuv.x < 1.0 && wuv.y > 0.0 && wuv.y < 1.0) {
      // distance to the W's centre line, in viewBox units; the tube radius is 5 (stroke width 10)
      const float TUBE = 5.0;
      float dist = texture(uW, wuv).a * 16.0;
      float e = 2.0 / 384.0;
      vec2 grad = vec2(texture(uW, wuv + vec2(e, 0.0)).a - texture(uW, wuv - vec2(e, 0.0)).a,
                       -(texture(uW, wuv + vec2(0.0, e)).a - texture(uW, wuv - vec2(0.0, e)).a));
      vec2 gdir = length(grad) > 1e-6 ? normalize(grad) : vec2(0.0);
      float px = 1.0 / uWScale; // one screen pixel in viewBox units
      wA = (1.0 - smoothstep(TUBE - px * 0.6, TUBE + px * 0.6, dist)) * facing;
      float rr = clamp(dist / TUBE, 0.0, 1.0);
      float h = sqrt(max(0.0, 1.0 - rr * rr)); // round tube profile
      vec3 wn = faceRot(normalize(vec3(gdir * rr, h + 0.001)), uFace);
      float g = clamp((vw.x - 28.0) / 64.0 * 0.75 + (vw.y - 46.0) / 40.0 * 0.25 + 0.06 * sin(uTime * 0.25 + uSwirl * 0.4), 0.0, 1.0);
      vec3 sky = vec3(0.486, 0.784, 1.0), lilac = vec3(0.722, 0.651, 1.0), blush = vec3(1.0, 0.62, 0.847), mint = vec3(0.541, 0.949, 0.816);
      vec3 wc = g < 0.36 ? mix(sky, lilac, g / 0.36) : g < 0.68 ? mix(lilac, blush, (g - 0.36) / 0.32) : mix(blush, mint, (g - 0.68) / 0.32);
      // soft key light from the top left, a little darker on the shadow side of the tube
      float shade = 0.74 + 0.30 * max(0.0, dot(wn, normalize(vec3(-0.35, -0.6, 0.72))));
      wc *= shade;
      vec3 L = normalize(vec3((uLight - uC) / (uR * 1.3), 1.2));
      float wspec = pow(max(dot(wn, normalize(L + vec3(0.0, 0.0, 1.0))), 0.0), 36.0);
      wc += wspec * 0.3;
      col = mix(col, wc, wA);
    }
  }

  // specular: one soft reflection of the light (the pointer), and a small sharp one beside it
  vec3 L = normalize(vec3((uLight - uC) / (uR * 0.85), 0.75));
  vec3 H = normalize(L + vec3(0.0, 0.0, 1.0));
  float sp1 = pow(max(dot(n, H), 0.0), 34.0) * 0.42;
  vec3 L2 = normalize(vec3(L.xy * 1.12 + vec2(0.14, -0.05), L.z));
  float sp2 = pow(max(dot(n, normalize(L2 + vec3(0.0, 0.0, 1.0))), 0.0), 900.0) * 0.55;
  col += (sp1 + sp2) * (1.0 - wA * 0.8);
  col = min(col, vec3(1.0));

  float a = inside * uAlpha;
  if (uOverlay > 0.5) {
    // over live DOM there is no backdrop copy: draw only what the film adds (reflection), premultiplied
    vec3 add = film * fres * 0.9 + sp1 + sp2 + crescent * 0.16;
    add = mix(add, film * 1.1 + 0.1, rimBand * 0.55);
    float al = clamp(0.03 + fres * 0.7 + sp1 + sp2 + rimBand * 0.5, 0.0, 1.0) * a;
    o = vec4(add * a, al);
    return;
  }
  // the bubble carries its own refracted copy of the backdrop, so it can cover fully
  o = vec4(col * a, a);
}`;

// @source pop-shader
const POP = `#version 300 es
precision highp float;
in vec2 vPx; out vec4 o;
uniform vec2 uC; uniform float uR, uK, uAng, uSeed, uDpr;
${FILM}
${NOISE}
void main() {
  vec2 p = vPx - uC; float d = length(p); float ang = atan(p.y, p.x);
  float k = uK;
  vec4 acc = vec4(0.0);
  // the tear: film remaining away from the tear point, snapping back to the far side
  float open = k * 3.4;
  float da = abs(mod(ang - uAng + 3.14159, 6.28318) - 3.14159);
  float keep = smoothstep(open - 0.25, open, da);
  float rr = uR * (1.0 + 0.12 * k);
  float w = max(0.6, 2.6 * (1.0 - k));
  float ring = exp(-(d - rr) * (d - rr) / (w * w)) * keep * (1.0 - smoothstep(0.55, 1.0, k));
  vec3 fc = thinFilm(300.0 + 300.0 * sin(ang * 3.0 + uSeed), 0.8) * 1.4 + 0.1;
  acc += vec4(fc, 1.0) * ring;
  // droplets thrown out along the tear
  for (int i = 0; i < 9; i++) {
    float fi = float(i);
    float a = uAng + (hash1(vec2(fi, uSeed)) - 0.5) * 3.8;
    float sp = 0.6 + hash1(vec2(uSeed, fi)) * 0.9;
    float dist = uR * (1.0 + k * 1.6 * sp);
    vec2 c = vec2(cos(a), sin(a)) * dist + vec2(0.0, k * k * uR * 0.5);
    float s = (1.2 + 1.8 * hash1(vec2(fi * 3.1, uSeed))) * (1.0 - k);
    float dd = length(p - c);
    float m = 1.0 - smoothstep(s - 0.6 / uDpr, s + 0.6 / uDpr, dd);
    acc += vec4(vec3(0.95), 1.0) * m * (1.0 - k);
  }
  acc.a = clamp(acc.a, 0.0, 1.0);
  o = vec4(acc.rgb * acc.a, acc.a);
}`;

// @source w-texture
/**
 * The logo W as a distance field: each texel stores the distance (viewBox units, 0–16 → 0–255) to the
 * W's centre line, measured against its quadratic Béziers (each sampled as a 48-segment polyline). The shader turns it into a round tube
 * with an analytic profile and antialiases the edge per pixel, so it stays smooth at any size.
 */
function wTexture(gl: WebGL2RenderingContext) {
  const S = 384, MAXD = 16;
  // centre line: M32 48 Q38 82 46 82 Q53 82 60 60 Q67 82 74 82 Q82 82 88 48
  const quads = [[32, 48, 38, 82, 46, 82], [46, 82, 53, 82, 60, 60], [60, 60, 67, 82, 74, 82], [74, 82, 82, 82, 88, 48]];
  const pts: number[] = [];
  for (const [x0, y0, cx, cy, x1, y1] of quads) {
    for (let i = 0; i <= 48; i++) {
      const t = i / 48, u = 1 - t;
      pts.push(u * u * x0 + 2 * u * t * cx + t * t * x1, u * u * y0 + 2 * u * t * cy + t * t * y1);
    }
  }
  const data = new Uint8Array(S * S * 4);
  for (let j = 0; j < S; j++) {
    const y = (1 - (j + 0.5) / S) * 120; // row 0 is the bottom (v = 0), matching wuv.y = 1 − y/120
    for (let i = 0; i < S; i++) {
      const x = ((i + 0.5) / S) * 120;
      let best = 1e9;
      for (let k = 0; k + 3 < pts.length; k += 2) {
        const ax = pts[k], ay = pts[k + 1], bx = pts[k + 2], by = pts[k + 3];
        const dx = bx - ax, dy = by - ay, l2 = dx * dx + dy * dy || 1;
        const t = Math.max(0, Math.min(1, ((x - ax) * dx + (y - ay) * dy) / l2));
        const ex = x - ax - dx * t, ey = y - ay - dy * t, d2 = ex * ex + ey * ey;
        if (d2 < best) best = d2;
      }
      data[(j * S + i) * 4 + 3] = Math.min(255, Math.round((Math.sqrt(best) / MAXD) * 255));
    }
  }
  const tex = gl.createTexture()!;
  gl.bindTexture(gl.TEXTURE_2D, tex);
  gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, S, S, 0, gl.RGBA, gl.UNSIGNED_BYTE, data);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
  return tex;
}

interface Ripple { x: number; y: number; age: number; life: number; reach: number }
interface Emitter { phase: number; speed: number; ax: number; ay: number; fx: number; fy: number; color: number }

export class Scene {
  readonly canvas: HTMLCanvasElement;
  readonly gl: WebGL2RenderingContext;
  private q: ReturnType<typeof quad>;
  private fluid: Fluid;
  private P: Record<'comp' | 'blur' | 'blit' | 'glass' | 'bubble' | 'pop', Program>;
  private bg!: Target; private b1!: Target; private b1t!: Target; private b2!: Target; private b2t!: Target; private b3!: Target; private b3t!: Target;
  private wTex: WebGLTexture;
  private byte = formats;
  w = 0; h = 0; dpr = 1;
  private fluidAspect = 0;
  time = 0;
  glass: GlassSurface[] = [];
  bubbles: BubbleState[] = [];
  pops: PopState[] = [];
  private ripples: Ripple[] = [];
  light = { x: 0, y: 0 };
  private lightTarget = { x: 0, y: 0 };
  private pointer: { x: number; y: number; px: number; py: number; has: boolean } = { x: 0, y: 0, px: 0, py: 0, has: false };
  private palette: [number, number, number][];
  private paletteTarget: [number, number, number][];
  private ground: [number, number, number];
  private emitters: Emitter[];
  opts: Required<Omit<SceneOptions, 'fluid' | 'palette' | 'ground'>>;

  constructor(canvas: HTMLCanvasElement, o: SceneOptions = {}) {
    this.canvas = canvas;
    const gl = canvas.getContext('webgl2', { alpha: !!o.overlay, antialias: false, premultipliedAlpha: true, powerPreference: 'low-power' });
    if (!gl) throw new Error('WebGL2 unavailable');
    this.gl = gl;
    this.q = quad(gl);
    const draw = () => gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4);
    this.fluid = new Fluid(gl, draw, o.fluid);
    this.P = {
      comp: program(gl, VERT_FS, COMPOSITE), blur: program(gl, VERT_FS, BLUR), blit: program(gl, VERT_FS, BLIT),
      glass: program(gl, VERT_QUAD, GLASS), bubble: program(gl, VERT_QUAD, BUBBLE), pop: program(gl, VERT_QUAD, POP),
    };
    this.wTex = wTexture(gl);
    const pal = (o.palette ?? ['#2F5FC4', '#A84886', '#237E69']).map(hex);
    this.palette = pal.map(c => [...c] as [number, number, number]);
    this.paletteTarget = pal;
    this.ground = hex(o.ground ?? '#08090D');
    this.opts = { ambient: o.ambient ?? 1, pointerForce: o.pointerForce ?? 1, overlay: !!o.overlay, intensity: o.intensity ?? 0.3 };
    this.emitters = [0, 1, 2, 3, 4].map(i => ({
      phase: i * 1.7, speed: 0.018 + i * 0.005, ax: 0.32 + 0.06 * i, ay: 0.28 + 0.04 * (4 - i), fx: 1 + (i % 2), fy: 2 - (i % 2) * 1, color: i % 3,
    }));
  }

  setPalette(colors: string[]) { this.paletteTarget = colors.map(hex); }

  resize(w: number, h: number, dpr = Math.min(devicePixelRatio, 2)) {
    if (w === this.w && h === this.h && dpr === this.dpr) return;
    this.w = w; this.h = h; this.dpr = dpr;
    this.canvas.width = Math.max(1, Math.round(w * dpr));
    this.canvas.height = Math.max(1, Math.round(h * dpr));
    const gl = this.gl, f = this.byte(gl).float;
    [this.bg, this.b1, this.b1t, this.b2, this.b2t, this.b3, this.b3t].forEach(t => freeTarget(gl, t));
    const hw = Math.max(1, Math.round(w / 2)), hh = Math.max(1, Math.round(h / 2));
    this.bg = target(gl, hw, hh, f);
    this.b1 = target(gl, Math.ceil(hw / 2), Math.ceil(hh / 2), f); this.b1t = target(gl, this.b1.w, this.b1.h, f);
    this.b2 = target(gl, Math.ceil(hw / 4), Math.ceil(hh / 4), f); this.b2t = target(gl, this.b2.w, this.b2.h, f);
    this.b3 = target(gl, Math.ceil(hw / 8), Math.ceil(hh / 8), f); this.b3t = target(gl, this.b3.w, this.b3.h, f);
    if (this.opts.overlay) return;
    // the liquid survives resizes: it's sampled by uv, so only a big change of shape reallocates it
    const aspect = w / Math.max(1, h);
    if (this.fluidAspect && Math.abs(Math.log(aspect / this.fluidAspect)) < 0.35) return;
    const fresh = !this.fluidAspect;
    this.fluidAspect = aspect;
    this.fluid.resize(w, h);
    if (!fresh) { for (let i = 0; i < 6; i++) { const c = this.palette[i % 3]; this.fluid.splat(0.15 + Math.random() * 0.7, 0.15 + Math.random() * 0.7, 0, 0, [c[0] * 0.9, c[1] * 0.9, c[2] * 0.9], 0.03); } return; }
    // seed the liquid so the first frame isn't empty
    for (let i = 0; i < 9; i++) {
      const c = this.palette[i % 3];
      this.fluid.splat(0.15 + Math.random() * 0.7, 0.15 + Math.random() * 0.7, (Math.random() - 0.5) * 80, (Math.random() - 0.5) * 80, [c[0] * 0.9, c[1] * 0.9, c[2] * 0.9], 0.02);
    }
  }

  /** Pointer in CSS px. Pushes the liquid along its motion and moves the light. */
  move(x: number, y: number) {
    const p = this.pointer;
    if (!p.has) { p.px = x; p.py = y; }
    p.x = x; p.y = y; p.has = true;
    this.lightTarget = { x, y };
  }
  leave() { this.pointer.has = false; }

  /** A tap: a wave ring through the backdrop and an outward push in the liquid. */
  ripple(x: number, y: number, reach = 260, strength = 1) {
    this.ripples.push({ x, y, age: 0, life: 1.15, reach });
    if (this.ripples.length > 8) this.ripples.shift();
    if (this.w && !this.opts.overlay) this.fluid.ripple(x / this.w, 1 - y / this.h, 1100 * strength, 0.01);
  }

  /** Push dye and velocity in directly (e.g. a window dragged through the liquid). */
  stir(x: number, y: number, dx: number, dy: number, dye = 0) {
    if (!this.w || this.opts.overlay) return;
    const c = this.palette[Math.floor(Math.random() * 3)];
    this.fluid.splat(x / this.w, 1 - y / this.h, dx, -dy, dye ? [c[0] * dye, c[1] * dye, c[2] * dye] : null, 0.004);
  }

  pop(x: number, y: number, r: number, angle = -Math.PI / 2) {
    this.pops.push({ x, y, r, age: 0, life: 0.36, angle, seed: Math.random() * 100 });
    if (this.w && !this.opts.overlay) this.fluid.ripple(x / this.w, 1 - y / this.h, 500, Math.max(0.002, (r / this.w) * 0.6));
  }

  frame(dt: number) {
    const gl = this.gl;
    if (!this.w) return;
    this.time += dt;
    this.lastDt = dt;
    this.ageWaves(dt);
    const t = this.time;
    // palette crossfade (Space switch): ~600 ms
    const k = 1 - Math.exp(-dt * 5);
    this.palette.forEach((c, i) => { const g = this.paletteTarget[i % this.paletteTarget.length]; for (let j = 0; j < 3; j++) c[j] += (g[j] - c[j]) * k; });
    // light lags the pointer slightly, like a reflection
    const lk = 1 - Math.exp(-dt * 18);
    this.light.x += (this.lightTarget.x - this.light.x) * lk;
    this.light.y += (this.lightTarget.y - this.light.y) * lk;

    gl.bindVertexArray(this.q.vao);
    gl.disable(gl.BLEND);
    if (this.opts.overlay) {
      gl.bindFramebuffer(gl.FRAMEBUFFER, null);
      gl.viewport(0, 0, this.canvas.width, this.canvas.height);
      gl.clearColor(0, 0, 0, 0);
      gl.clear(gl.COLOR_BUFFER_BIT);
      if (this.glass.length) this.drawGlass(t);
      this.drawBubbles(t);
      return;
    }
// @source emitters
    // ambient emitters: slow Lissajous paths, each pushing a little dye along its tangent
    if (this.opts.ambient > 0) {
      for (const e of this.emitters) {
        e.phase += dt * e.speed;
        const x = 0.5 + e.ax * Math.sin(e.phase * e.fx * 2.1), y = 0.5 + e.ay * Math.sin(e.phase * e.fy * 1.7 + 1.3);
        const dx = e.ax * Math.cos(e.phase * e.fx * 2.1) * e.fx * 2.1, dy = e.ay * Math.cos(e.phase * e.fy * 1.7 + 1.3) * e.fy * 1.7;
        const c = this.palette[e.color], a = 0.5 * dt * this.opts.ambient;
        this.fluid.splat(x, y, dx * 14 * this.opts.ambient, dy * 14 * this.opts.ambient, [c[0] * a, c[1] * a, c[2] * a], 0.012);
      }
    }
    const p = this.pointer;
    if (p.has && this.opts.pointerForce > 0) {
      const dx = p.x - p.px, dy = p.y - p.py;
      if (dx * dx + dy * dy > 0.25) this.fluid.splat(p.x / this.w, 1 - p.y / this.h, dx * 4 * this.opts.pointerForce, -dy * 4 * this.opts.pointerForce, null, 0.0016);
      p.px = p.x; p.py = p.y;
    }
    this.fluid.step(dt);

    // 1. composite the backdrop at half resolution
    const C = this.P.comp;
    gl.useProgram(C.p);
    bindTex(gl, 0, this.fluid.dye.read.tex, C.u.uDye);
    gl.uniform3f(C.u.uGround, ...this.ground);
    gl.uniform1f(C.u.uMute, this.opts.intensity);
    gl.uniform2f(C.u.uView, this.w, this.h);
    gl.uniform1f(C.u.uTime, t);
    this.ripples = this.ripples.filter(r => (r.age += dt) < r.life);
    const rip = new Float32Array(32);
    this.ripples.forEach((r, i) => rip.set([r.x, r.y, r.age / r.life, r.reach], i * 4));
    gl.uniform4fv(C.u.uRip, rip);
    gl.uniform1i(C.u.uRipN, this.ripples.length);
    this.to(this.bg); this.draw();

    // blur pyramid for the glass
    const blur = (src: Target, tmp: Target, out: Target, sigma: number) => {
      const B = this.P.blur;
      gl.useProgram(B.p);
      gl.uniform1f(B.u.uSigma, sigma);
      bindTex(gl, 0, src.tex, B.u.uTex);
      gl.uniform2f(B.u.uStep, 1 / src.w, 0);
      this.to(tmp); this.draw();
      bindTex(gl, 0, tmp.tex, B.u.uTex);
      gl.uniform2f(B.u.uStep, 0, 1 / tmp.h);
      this.to(out); this.draw();
    };
    blur(this.bg, this.b1t, this.b1, 3.5);
    blur(this.b1, this.b2t, this.b2, 4.5);
    blur(this.b2, this.b3t, this.b3, 5.0); // the frost: ≈44 CSS px sigma in all

    // 2. backdrop to the screen
    gl.bindFramebuffer(gl.FRAMEBUFFER, null);
    gl.viewport(0, 0, this.canvas.width, this.canvas.height);
    gl.useProgram(this.P.blit.p);
    bindTex(gl, 0, this.bg.tex, this.P.blit.u.uTex);
    this.draw();

    gl.enable(gl.BLEND);
    gl.blendFunc(gl.ONE, gl.ONE_MINUS_SRC_ALPHA);

    this.drawGlass(t);
    this.drawBubbles(t);
    gl.disable(gl.BLEND);
  }

  private drawGlass(t: number) {
    const gl = this.gl;
    gl.enable(gl.BLEND);
    gl.blendFunc(gl.ONE, gl.ONE_MINUS_SRC_ALPHA);
    // 3. glass
    const G = this.P.glass;
    gl.useProgram(G.p);
    gl.uniform2f(G.u.uView, this.w, this.h);
    gl.uniform1f(G.u.uTime, t);
    gl.uniform1f(G.u.uDpr, this.dpr);
    gl.uniform2f(G.u.uLight, this.light.x, this.light.y);
    bindTex(gl, 0, this.bg.tex, G.u.uBg); bindTex(gl, 1, this.b1.tex, G.u.uB1); bindTex(gl, 2, this.b2.tex, G.u.uB2); bindTex(gl, 3, this.b3.tex, G.u.uB3);
    for (const s of this.glass) {
      const a = s.alpha ?? 1;
      if (a <= 0.001 || s.w < 1 || s.h < 1) continue;
      const pad = 70;
      gl.uniform4f(G.u.uQuad, s.x - pad, s.y - pad, s.w + pad * 2, s.h + pad * 2);
      gl.uniform4f(G.u.uRect, s.x, s.y, s.w, s.h);
      this.waveUniforms(G, s.waves);
      const bm = s.beam, vo = s.voice;
      gl.uniform4f(G.u.uBeam, bm?.head ?? 0, bm?.length ?? 0.25, bm?.strength ?? 0, bm?.hue ?? 0);
      gl.uniform4f(G.u.uVoice, vo?.low ?? 0, vo?.mid ?? 0, vo?.high ?? 0, vo?.phase ?? 0);
      gl.uniform1f(G.u.uVoiceAmt, vo?.amount ?? 0);
      gl.uniform1f(G.u.uMaterial, s.material ?? 1);
      gl.uniform1f(G.u.uR, s.r);
      gl.uniform1f(G.u.uTier, s.tier);
      gl.uniform1f(G.u.uFilm, s.film);
      gl.uniform1f(G.u.uAlpha, a);
      gl.uniform1f(G.u.uShadow, s.shadow ?? 1);
      gl.uniform3f(G.u.uTint, ...(s.tint ?? [0, 0, 0]));
      this.draw();
    }

  }

  private drawBubbles(t: number) {
    const gl = this.gl;
    gl.enable(gl.BLEND);
    gl.blendFunc(gl.ONE, gl.ONE_MINUS_SRC_ALPHA);
    // 4. bubbles
    const Bu = this.P.bubble;
    gl.useProgram(Bu.p);
    gl.uniform2f(Bu.u.uView, this.w, this.h);
    gl.uniform1f(Bu.u.uTime, t);
    gl.uniform1f(Bu.u.uDpr, this.dpr);
    gl.uniform2f(Bu.u.uLight, this.light.x, this.light.y);
    bindTex(gl, 0, this.bg.tex, Bu.u.uBg);
    bindTex(gl, 1, this.wTex, Bu.u.uW);
    bindTex(gl, 2, this.b1 ? this.b1.tex : this.wTex, Bu.u.uB1);
    bindTex(gl, 3, this.b2 ? this.b2.tex : this.wTex, Bu.u.uB2);
    const zero = new Float32Array(RING_N);
    for (const b of this.bubbles) {
      const a = b.alpha ?? 1;
      if (a <= 0.001 || b.r < 0.5) continue;
      const ext = b.r * 1.7 + 4;
      gl.uniform4f(Bu.u.uQuad, b.x - ext, b.y - ext, ext * 2, ext * 2);
      gl.uniform2f(Bu.u.uC, b.x, b.y);
      gl.uniform1f(Bu.u.uR, b.r);
      gl.uniform1fv(Bu.u.uRing, b.ring ?? zero);
      gl.uniform1f(Bu.u.uBody, b.body ?? 0);
      gl.uniform1f(Bu.u.uAlpha, a);
      gl.uniform1f(Bu.u.uSwirl, b.swirl ?? 0);
      gl.uniform1f(Bu.u.uSeed, b.seed ?? 0);
      gl.uniform1f(Bu.u.uHasW, b.w ? 1 : 0);
      gl.uniform2f(Bu.u.uFace, b.face?.[0] ?? 0, b.face?.[1] ?? 0);
      this.waveUniforms(Bu, b.waves);
      gl.uniform1f(Bu.u.uOverlay, this.opts.overlay ? 1 : 0);
      if (b.w) {
        gl.uniformMatrix2fv(Bu.u.uWInv, false, b.w.inv);
        gl.uniform2f(Bu.u.uWOff, ...b.w.off);
        gl.uniform1f(Bu.u.uWScale, b.w.scale);
      }
      this.draw();
    }

    // 5. pops
    const Po = this.P.pop;
    gl.useProgram(Po.p);
    gl.uniform2f(Po.u.uView, this.w, this.h);
    gl.uniform1f(Po.u.uDpr, this.dpr);
    this.pops = this.pops.filter(o => (o.age += this.lastDt) < o.life);
    for (const o of this.pops) {
      const ext = o.r * 3 + 10;
      gl.uniform4f(Po.u.uQuad, o.x - ext, o.y - ext, ext * 2, ext * 2);
      gl.uniform2f(Po.u.uC, o.x, o.y);
      gl.uniform1f(Po.u.uR, o.r);
      gl.uniform1f(Po.u.uK, o.age / o.life);
      gl.uniform1f(Po.u.uAng, o.angle);
      gl.uniform1f(Po.u.uSeed, o.seed);
      this.draw();
    }
  }

  private lastDt = 0.016;
  private waveBuf = new Float32Array(16);
  private waveUniforms(P: Program, waves: Wave[] | undefined) {
    const n = Math.min(4, waves?.length ?? 0);
    for (let i = 0; i < n; i++) { const w = waves![waves!.length - n + i]; this.waveBuf.set([w.x, w.y, w.age / w.life, w.reach], i * 4); }
    this.gl.uniform4fv(P.u.uWave, this.waveBuf);
    this.gl.uniform1i(P.u.uWaveN, n);
  }

  // @source waves
  /** Send a distortion wave through a glass surface or a bubble, from (x, y) in CSS px. */
  wave(target: { waves?: Wave[] }, x: number, y: number, reach: number, life = 0.9) {
    (target.waves ??= []).push({ x, y, age: 0, life, reach });
    if (target.waves.length > 4) target.waves.shift();
  }
  private ageWaves(dt: number) {
    for (const o of [...this.glass, ...this.bubbles]) if (o.waves?.length) o.waves = o.waves.filter(w => (w.age += dt) < w.life);
  }
  private to(t: Target) { this.gl.bindFramebuffer(this.gl.FRAMEBUFFER, t.fbo); this.gl.viewport(0, 0, t.w, t.h); }
  private draw() { this.gl.drawArrays(this.gl.TRIANGLE_STRIP, 0, 4); }

  dispose() {
    const gl = this.gl;
    this.fluid.dispose();
    [this.bg, this.b1, this.b1t, this.b2, this.b2t, this.b3, this.b3t].forEach(t => freeTarget(gl, t));
    for (const k in this.P) gl.deleteProgram(this.P[k as keyof typeof this.P].p);
    gl.deleteTexture(this.wTex);
    gl.deleteBuffer(this.q.buf);
    gl.deleteVertexArray(this.q.vao);
    gl.getExtension('WEBGL_lose_context')?.loseContext();
  }
}
