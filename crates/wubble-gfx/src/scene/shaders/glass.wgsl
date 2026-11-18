// Refractive glass surfaces, translated line for line from GLASS in core/gl/scene.ts.
// Rounded rects that lens the blurred backdrop toward the rim, split per channel (chromatic
// dispersion), with a specular band, a pointer glow, a 1px inner rim, the touched sheen rim,
// distortion waves from presses, a border beam and the voice glow (with bloom outside the rect).
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
@group(0) @binding(3) var uB1: texture_2d<f32>;
@group(0) @binding(4) var uB2: texture_2d<f32>;

@vertex
fn vs_glass(@builtin(vertex_index) vi: u32) -> QuadOut { return quadVs(vi, U.uQuad, U.uView); }

fn bg(px: vec2<f32>, tier: f32) -> vec3<f32> {
  var uv = px / U.uView; uv.y = 1.0 - uv.y;
  let a = textureSampleLevel(uB1, samp, uv, 0.0).rgb;
  let b = textureSampleLevel(uB2, samp, uv, 0.0).rgb;
  // thin 0 → the first blur level, regular 1 → 55% toward the second, thick 2 → the second; continuous in between
  let t = select(0.55 + (tier - 1.0) * 0.45, tier * 0.55, tier < 1.0);
  return mix(a, b, clamp(t, 0.0, 1.0));
}
fn sat(c: vec3<f32>, s: f32) -> vec3<f32> { let l = dot(c, vec3<f32>(0.2126, 0.7152, 0.0722)); return mix(vec3<f32>(l), c, s); }

@fragment
fn fs_glass(fin: QuadOut) -> @location(0) vec4<f32> {
  let vPx = fin.vPx;
  let half_ = vec2<f32>(max(0.5, U.uRect.z * 0.5), U.uRect.w * 0.5); let c = U.uRect.xy + half_; let p = vPx - c;
  let r = min(U.uR, min(half_.x, half_.y));
  let d = sdRound(p, half_, r);
  let aa = 0.75 / U.uDpr;
  // soft drop shadow, grows with tier
  let sBlur = mix(24.0, 60.0, U.uTier * 0.5); let sOff = mix(10.0, 24.0, U.uTier * 0.5);
  let ds = sdRound(p - vec2<f32>(0.0, sOff * 0.5), half_ - 2.0, r);
  let shadow = (1.0 - smoothstep(-sBlur * 0.3, sBlur, ds)) * mix(0.35, 0.55, U.uTier * 0.5) * U.uShadow;
  let inside = 1.0 - smoothstep(-aa, aa, d);

  // @source slint-glass-beam-wgsl
  // ── border beam: a head running round the rim with a smootherstep tail; stroke, inner glow, bloom
  var fx = vec3<f32>(0.0); var fxA = 0.0; var bloomC = vec3<f32>(0.0); var bloomA = 0.0;
  if (U.uBeam.z > 0.001) {
    let per = perimeterAt(p, half_, r);
    let s = per.x / per.y;
    let behind = fract(U.uBeam.x - s);
    let ahead = fract(s - U.uBeam.x);
    var along = select(0.0, pow(1.0 - behind / U.uBeam.y, 2.2), behind < U.uBeam.y);
    along = max(along, 1.0 - smoothstep(0.0, 0.012, ahead)); // a crisp head
    let stroke = exp(-sq((d + 0.8) / 1.0)); 
    let inner = select(0.0, exp(d / 10.0), d < 0.0);
    let bloom = select(0.0, exp(-d / 8.0), d > 0.0);
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
    var levels = array<f32, 3>(U.uVoice.x, U.uVoice.y, U.uVoice.z);
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
    let sa = shadow * U.uMaterial;
    return vec4<f32>(bloomC, clamp(bloomA, 0.0, 1.0)) * U.uAlpha + vec4<f32>(0.0, 0.0, 0.0, sa * U.uAlpha) * (1.0 - clamp(bloomA, 0.0, 1.0));
  }
  if (U.uMaterial < 0.5) {
    // effects only, over live content
    let a0 = inside * U.uAlpha;
    return vec4<f32>(fx * a0, clamp(fxA, 0.0, 1.0) * a0);
  }
  // wubble: `uMaterial` 2 is the flat fill of the lowest quality level, not in the design.
  // One colour per panel (the blurred backdrop at its centre under the tier fill), the 1px rim
  // and the effects: no lens, dispersion, waves, specular or pointer light.
  if (U.uMaterial > 1.5) {
    var fillF = vec4<f32>(0.11, 0.122, 0.157, 0.64);
    if (U.uTier < 0.5) { fillF = vec4<f32>(1.0, 1.0, 1.0, 0.08); } else if (U.uTier < 1.5) { fillF = vec4<f32>(0.086, 0.098, 0.129, 0.42); }
    var flat_ = mix(bg(c, U.uTier), fillF.rgb, fillF.a);
    flat_ = mix(flat_, U.uTint, 0.10 * step(0.001, dot(U.uTint, U.uTint)));
    flat_ += (1.0 - smoothstep(0.0, 1.25, -d)) * 0.1;
    flat_ += fx;
    let aF = inside * U.uAlpha;
    return vec4<f32>(flat_ * aF, aF) + vec4<f32>(0.0, 0.0, 0.0, shadow * U.uAlpha) * (1.0 - aF);
  }

  // normal from the SDF gradient
  let e = vec2<f32>(0.5, 0.0);
  let n = normalize(vec2<f32>(sdRound(p + e.xy, half_, r) - sdRound(p - e.xy, half_, r), sdRound(p + e.yx, half_, r) - sdRound(p - e.yx, half_, r)) + 1e-5);
  // convex lens bezel: circular profile, strongest at the rim
  let bezel = clamp(min(half_.x, half_.y) * 0.5, 6.0, 22.0);
  let t = clamp(1.0 + d / bezel, 0.0, 1.0);
  let lens = 1.0 - sqrt(max(0.0, 1.0 - t * t));
  let off = -n * lens * bezel * 1.1;
  // @source slint-glass-wave-wgsl
  // a press sends a distortion wave through the glass: lens push, extra blur, spectral split
  var disp = vec2<f32>(0.0); var wBand = 0.0; var wBlur = 0.0;
  for (var i = 0; i < 4; i++) {
    if (i >= U.uWaveN) { break; }
    let h = waveAt(vPx, U.uWave[i], 9.0, 26.0);
    disp += h.disp; wBand = max(wBand, h.band); wBlur = max(wBlur, h.blur);
  }
  let tierB = min(2.0, U.uTier + wBlur * 1.2);
  // chromatic dispersion: blue bends most
  var col: vec3<f32>;
  col.x = bg(vPx + off * 0.92 - disp * 0.85, tierB).r;
  col.y = bg(vPx + off * 1.00 - disp * 1.00, tierB).g;
  col.z = bg(vPx + off * 1.10 - disp * 1.18, tierB).b;
  col = sat(col, mix(1.6, 2.0, U.uTier * 0.5));
  // tint fill per tier: white 8% / #161921 42% / #1C1F28 64%
  var fill = vec4<f32>(0.11, 0.122, 0.157, 0.64);
  if (U.uTier < 0.5) { fill = vec4<f32>(1.0, 1.0, 1.0, 0.08); } else if (U.uTier < 1.5) { fill = vec4<f32>(0.086, 0.098, 0.129, 0.42); }
  col = mix(col, fill.rgb, fill.a);
  col = mix(col, U.uTint, 0.10 * step(0.001, dot(U.uTint, U.uTint)));
  // the wave's leading band catches a little film colour
  col += (thinFilm(320.0 + 140.0 * wBand, 0.85) * 0.12 + 0.03) * wBand;
  // specular: white 10% to 0 over the top 42%
  let top = clamp((p.y + half_.y) / (U.uRect.w * 0.42), 0.0, 1.0);
  col += (1.0 - top) * (1.0 - top) * 0.10;
  // pointer light sweeping across the surface
  let lp = U.uLight - c;
  let glow = exp(-dot(p - lp, p - lp) / (2.0 * pow(max(U.uRect.z, U.uRect.w) * 0.35, 2.0)));
  col += glow * 0.035;
  // 1px inner rim: 22% at the top, 7% all round, brighter where it faces the light
  let rim = 1.0 - smoothstep(0.0, 1.25, -d);
  let facing = max(0.0, dot(n, normalize(lp - p + 1e-4)));
  let rimA = 0.07 + 0.15 * max(0.0, -n.y) + 0.18 * facing * facing;
  col += rim * rimA;
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
