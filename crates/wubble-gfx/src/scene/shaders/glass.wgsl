// translates: core/gl/scene.ts:129-274
// Coloured frosted glass, translated line for line from GLASS in core/gl/scene.ts (598f814).
// Copied from the-wallpaper-project src/wubble/slint/shaders/glass.wgsl (owner's code), which is
// line for line with the web; one wubble branch, `uMaterial` 2, is marked below.
// Rounded rects over the deepest blur levels: the light pulled toward a milky floor, the colour lifted,
// a soft pointer light and a faint inner edge (no lens, no specular band, no hairline), plus the touched
// sheen rim, distortion waves from presses, a border beam and the voice glow (with bloom outside the rect).
// `uMaterial` 0 draws the effects only (beam, glow) for surfaces over live content;
// 2 (wubble's, not the design's) is the flat fill of `Quality::Flat`.
// This pass is what Slint itself cannot do (no backdrop blur): Slint draws the content on top.
// Prepend: common.wgsl, film.wgsl, wave.wgsl, beam.wgsl. Blend: premultiplied.

// @source slint-glass-wgsl
struct GlassU {
  uQuad: vec4<f32>,  // drawn quad: rect padded by 70 px for the shadow and bloom
  uRect: vec4<f32>,  // x, y, w, h in top-left logical px
  uView: vec2<f32>,
  uLight: vec2<f32>,
  uTint: vec3<f32>,  // 0,0,0 = no tint
  uR: f32,
  uTier: f32,        // 0 thin, 1 regular, 2 thick
  uFilm: f32,
  uAlpha: f32,
  uShadow: f32,
  uTime: f32,
  uDpr: f32,
  uWaveN: i32,
  uVoiceAmt: f32,
  uMaterial: f32,    // 1 full glass, 0 effects only, 2 flat fill (wubble)
  _p0: f32, _p1: f32, _p2: f32,
  uBeam: vec4<f32>,  // head, length (fractions of the perimeter), strength, hue
  uVoice: vec4<f32>, // low, mid, high, phase
  uWave: array<vec4<f32>, 4>,
};

@group(0) @binding(0) var<uniform> U: GlassU;
@group(0) @binding(1) var samp: sampler;
// uBg is declared in the GLSL but never read there either; only the pyramid levels are used.
@group(0) @binding(3) var uB2: texture_2d<f32>;  // render.rs binds b2 (⅛ res) here
@group(0) @binding(4) var uB3: texture_2d<f32>;  // and b3 (1/16 res), the frost

@vertex
fn vs_glass(@builtin(vertex_index) vi: u32) -> QuadOut { return quadVs(vi, U.uQuad, U.uView); }

// Frosted: the two deepest blur levels only. thin 0 → 40% toward the deepest, regular 1 → 75%, thick 2 → all.
fn bg(px: vec2<f32>, tier: f32) -> vec3<f32> {
  var uv = px / U.uView; uv.y = 1.0 - uv.y;
  let a = textureSampleLevel(uB2, samp, uv, 0.0).rgb; // wubble change: level 0 explicitly (textureSampleLevel)
  let b = textureSampleLevel(uB3, samp, uv, 0.0).rgb; // wubble change: level 0 explicitly
  return mix(a, b, clamp(0.4 + tier * 0.3, 0.0, 1.0));
}
const LUMA = vec3<f32>(0.2126, 0.7152, 0.0722);
// @source slint-glass-frost
// Coloured frost: hue kept, detail and most contrast lost; light pulled toward a milky floor, colour lifted.
const FROST = vec3<f32>(0.115, 0.85, 2.7); // milk floor, how much of the original light survives, colour lift
fn frost(b: vec3<f32>, tier: f32) -> vec3<f32> {
  let L = dot(b, LUMA);
  let ch = b - vec3<f32>(L);
  let floorL = FROST.x + 0.012 * tier;
  return vec3<f32>(floorL + L * FROST.y) + ch * FROST.z;
}

@fragment
fn fs_glass(fin: QuadOut) -> @location(0) vec4<f32> {
  let vPx = fin.vPx;
  let half_ = U.uRect.zw * 0.5; let c = U.uRect.xy + half_; let p = vPx - c;
  let r = min(U.uR, min(half_.x, half_.y));
  let d = sdRound(p, half_, r);
  let aa = 0.75 / U.uDpr;
  // soft drop shadow, grows with tier
  let sBlur = mix(24.0, 60.0, U.uTier * 0.5); let sOff = mix(10.0, 24.0, U.uTier * 0.5);
  let ds = sdRound(p - vec2<f32>(0.0, sOff * 0.5), half_ - 2.0, r);
  let shadow = (1.0 - smoothstep(-sBlur * 0.3, sBlur, ds)) * mix(0.22, 0.38, U.uTier * 0.5) * U.uShadow;
  let inside = 1.0 - smoothstep(-aa, aa, d);

  // @source slint-glass-beam-wgsl
  // ── border beam: a head running round the rim with a smootherstep tail; stroke, inner glow, bloom
  var fx = vec3<f32>(0.0); var fxA = 0.0; var bloomC = vec3<f32>(0.0); var bloomA = 0.0;
  if (U.uBeam.z > 0.001) {
    let per = perimeterAt(p, half_, r);
    let s = per.x / per.y;
    let behind = fract(U.uBeam.x - s);
    let ahead = fract(s - U.uBeam.x);
    var along: f32; if (behind < U.uBeam.y) { along = pow(1.0 - behind / U.uBeam.y, 2.2); } else { along = 0.0; }
    along = max(along, 1.0 - smoothstep(0.0, 0.012, ahead)); // a crisp head
    let stroke = exp(-sq((d + 0.8) / 1.0)); 
    var inner: f32; if (d < 0.0) { inner = exp(d / 10.0); } else { inner = 0.0; }
    var bloom: f32; if (d > 0.0) { bloom = exp(-d / 8.0); } else { bloom = 0.0; }
    let bc = sheen(s * 1.5 + U.uBeam.w);
    let k = along * U.uBeam.z;
    fx += bc * (stroke * 1.3 + inner * 0.32) * k; fxA = max(fxA, (stroke * 0.9 + inner * 0.25) * k);
    bloomC += bc * bloom * 0.55 * k; bloomA += bloom * 0.4 * k;
  }
  // @source slint-glass-voice-wgsl
  // ── voice glow: seven soft lobes fanning up from the bottom centre, each following a band
  if (U.uVoiceAmt > 0.001) {
    let sc = U.uRect.z / 350.0;
    let base = vec2<f32>(0.0, half_.y);
    var levels: array<f32, 3> = array<f32, 3>(U.uVoice.x, U.uVoice.y, U.uVoice.z);
    let spread = 1.0 + 0.35 * U.uVoice.y;
    var vc = vec3<f32>(0.0); var va = 0.0;
    for (var i = 0; i < 7; i++) {
      let L = LOBES[i];
      let lv = levels[i32(L.w)];
      let x = L.x * sc * spread + sin(U.uVoice.w + f32(i)) * 6.0 * sc;
      let e = (p - base - vec2<f32>(x, 0.0)) / (vec2<f32>(L.y, L.z) * sc * (0.55 + 0.9 * lv));
      let g = exp(-dot(e, e) * 1.4) * (0.25 + lv);
      vc += sheen(f32(i) * 0.13 + U.uVoice.w * 0.05) * g; va += g;
    }
    let rimGlow = exp(-sq((d + 0.8) / 1.2)) * (0.3 + U.uVoice.x + U.uVoice.y) * 0.5;
    fx += (vc * 0.6 + sheen(atan2(p.y, p.x) / 6.2831853 + U.uVoice.w * 0.04) * rimGlow) * U.uVoiceAmt * step(d, 0.5);
    fxA = max(fxA, clamp((va * 0.45 + rimGlow) * U.uVoiceAmt, 0.0, 1.0) * step(d, 0.5));
    let vb = select(0.0, exp(-d / 10.0) * smoothstep(-half_.y, half_.y, p.y), d > 0.0);
    bloomC += vc * vb * 0.4 * U.uVoiceAmt; bloomA += min(1.0, va) * vb * 0.3 * U.uVoiceAmt;
  }

  if (inside <= 0.0) {
    let sa = shadow * min(U.uMaterial, 1.0); // wubble change: the flat fill (2) casts glass's shadow
    return vec4<f32>(bloomC, clamp(bloomA, 0.0, 1.0)) * U.uAlpha + vec4<f32>(0.0, 0.0, 0.0, sa * U.uAlpha) * (1.0 - clamp(bloomA, 0.0, 1.0));
  }
  // wubble change: `uMaterial` 2 is the flat fill of the lowest quality level, not in the design: the
  // frost of the backdrop at the panel's centre, the tint and the soft inner edge, plus the effects;
  // no rim bend, waves, grain or pointer light.
  if (U.uMaterial > 1.5) { // wubble change: the flat fill
    var flat_ = frost(bg(c, U.uTier), U.uTier);
    flat_ = mix(flat_, U.uTint * 0.55 + 0.1, 0.07 * step(0.001, dot(U.uTint, U.uTint)));
    flat_ += exp(d / 1.6) * step(d, 0.0) * 0.035;
    flat_ += fx;
    let aF = inside * U.uAlpha;
    return vec4<f32>(flat_ * aF, aF) + vec4<f32>(0.0, 0.0, 0.0, shadow * U.uAlpha) * (1.0 - aF);
  }
  if (U.uMaterial < 0.5) {
    // effects only, over live content
    let a0 = inside * U.uAlpha;
    return vec4<f32>(fx * a0, clamp(fxA, 0.0, 1.0) * a0);
  }

  // normal from the SDF gradient
  let e = vec2<f32>(0.5, 0.0);
  let n = normalize(vec2<f32>(sdRound(p + e.xy, half_, r) - sdRound(p - e.xy, half_, r), sdRound(p + e.yx, half_, r) - sdRound(p - e.yx, half_, r)) + 1e-5);
  // frosted glass doesn't lens: the rim only bends the light a little, so the edge stays soft
  let bezel = clamp(min(half_.x, half_.y) * 0.5, 6.0, 22.0);
  let t = clamp(1.0 + d / bezel, 0.0, 1.0);
  let off = -n * t * t * bezel * 0.35;
  // @source slint-glass-wave-wgsl
  // a press sends a distortion wave through the glass: lens push, extra blur, spectral split
  var disp = vec2<f32>(0.0); var wBand = 0.0; var wBlur = 0.0;
  for (var i = 0; i < 4; i++) {
    if (i >= U.uWaveN) { break; }
    let h = waveAt(vPx, U.uWave[i], 9.0, 26.0);
    disp += h.disp; wBand = max(wBand, h.band); wBlur = max(wBlur, h.blur);
  }
  let tierB = min(2.0, U.uTier + wBlur * 1.2);
  // the wave still splits the light spectrally as it passes (only the wave: resting frost has no dispersion)
  var col: vec3<f32>;
  col.x = bg(vPx + off - disp * 0.85, tierB).r;
  col.y = bg(vPx + off - disp * 1.00, tierB).g;
  col.z = bg(vPx + off - disp * 1.18, tierB).b;
  col = frost(col, U.uTier);
  // the Space colour, lightly: the colour should mostly come from what's behind
  col = mix(col, U.uTint * 0.55 + 0.1, 0.07 * step(0.001, dot(U.uTint, U.uTint)));
  // the wave's leading band catches a little film colour
  col += (thinFilm(320.0 + 140.0 * wBand, 0.85) * 0.12 + 0.03) * wBand;
  // pointer light: a broad, soft brightening in the frost (no specular band, no hotspot)
  let lp = U.uLight - c;
  let glow = exp(-dot(p - lp, p - lp) / (2.0 * sq(max(U.uRect.z, U.uRect.w) * 0.45)));
  col += glow * 0.03;
  // the edge: a faint, soft light just inside, a little brighter facing the pointer. No hairline.
  let rim = exp(d / 1.6) * step(d, 0.0);
  let facing = max(0.0, dot(n, normalize(lp - p + 1e-4)));
  col += rim * (0.035 + 0.05 * facing * facing);
  // ±0.5/255 grain against 8-bit banding in the smooth frost
  col += (fract(sin(dot(vPx, vec2<f32>(12.9898, 78.233))) * 43758.5453) - 0.5) / 255.0;
  // film rim: the brand sheen as a soft 1.5 px ring that drifts slowly round the edge, only when touched.
  // It matches the CSS .wb-film rim (a conic Sky → Lilac → Blush → Mint ring at 80%), not interference
  // colours: high-order film bands read as anodised metal on small controls.
  if (U.uFilm > 0.001) {
    let per = perimeterAt(p, half_, r);
    let s = per.x / per.y;
    let rc = sheen(s + 0.58 - U.uTime * 0.06);
    let ring = 1.0 - smoothstep(1.0, 2.0, -d);           // 1.5 px band, antialiased inward
    let glowIn = exp(d / 5.0) * 0.12;                      // a breath of colour just inside
    col = mix(col, rc, ring * 0.8 * U.uFilm);
    col += rc * glowIn * U.uFilm;
  }
  col += fx;
  let a = inside * U.uAlpha;
  return vec4<f32>(col * a, a) + vec4<f32>(0.0, 0.0, 0.0, shadow * U.uAlpha) * (1.0 - a);
}
