// translates: core/gl/scene.ts:277-438
// Copied from the-wallpaper-project src/wubble/slint/shaders/bubble.wgsl @598f814 (owner's code),
// which is line for line with BUBBLE in core/gl/scene.ts; the `wubble change` marks are WGSL's forced
// differences (the ring packed four to a vec4, explicit LOD, a template's numbers spelled out).
// Thin-film bubbles: the soft-body logo bubble and the little ones. Translated line for line
// from BUBBLE in core/gl/scene.ts: first-order film, Fresnel-only film, hairline rim, a crescent
// and body shading that follow the light, a brand-gradient W tube from a distance field mapped
// onto an inner sphere that turns to face the cursor (uFace), and distortion waves that lens,
// split (5-tap spectral) and blur what the body refracts.
// Prepend: common.wgsl, film.wgsl, wave.wgsl, beam.wgsl. Blend: premultiplied.
// Quad: centre ± (r·1.7 + 4) px.

// @source slint-bubble-wgsl
const RING_N: i32 = 24; // wubble change: the GLSL's `${RING_N}` template constant, spelled out

struct BubbleU {
  uQuad: vec4<f32>,
  uView: vec2<f32>,
  uC: vec2<f32>,
  uLight: vec2<f32>,
  uWOff: vec2<f32>,
  uWInv: mat2x2<f32>,          // column-major, as SoftBody writes it
  uR: f32,
  uBody: f32,
  uAlpha: f32,
  uTime: f32,
  uSwirl: f32,
  uSeed: f32,
  uDpr: f32,
  uHasW: f32,
  uOverlay: f32,
  uWScale: f32,
  uFace: vec2<f32>,            // yaw, pitch (radians): the bubble turns its face toward the cursor
  uWaveN: i32,
  _p0: f32, _p1: f32, _p2: f32,
  // GLSL `float uRing[24]`. WGSL uniform arrays have a 16-byte stride, so the 24 floats are
  // packed four to a vec4: sample i lives at uRing[i / 4][i % 4]. Same bytes as a [f32; 24].
  uRing: array<vec4<f32>, 6>, // wubble change: 24 floats packed four to a vec4 (uniform array stride)
  uWave: array<vec4<f32>, 4>,
};

@group(0) @binding(0) var<uniform> U: BubbleU;
@group(0) @binding(1) var samp: sampler;
@group(0) @binding(2) var uBg: texture_2d<f32>;
@group(0) @binding(3) var uW: texture_2d<f32>;   // W distance field, 384², RGBA8, rows bottom-first
@group(0) @binding(4) var uB1: texture_2d<f32>;
@group(0) @binding(5) var uB2: texture_2d<f32>;

@vertex
fn vs_bubble(@builtin(vertex_index) vi: u32) -> QuadOut { return quadVs(vi, U.uQuad, U.uView); }

fn ring(i: i32) -> f32 { return U.uRing[i / 4][i % 4]; } // wubble change: the packed ring's sample i

// @source slint-bubble-ring-wgsl
fn ringAt(ang: f32) -> f32 {
  let f = (ang / 6.2831853 + 0.5) * f32(RING_N); // wubble change: the GLSL line is a template (`${RING_N}.0`), its numbers spelled out
  var i1 = i32(floor(f)); let t = fract(f);
  let i0 = (i1 + RING_N - 1) % RING_N; let i2 = (i1 + 1) % RING_N; let i3 = (i1 + 2) % RING_N; // wubble change: a template line in the GLSL
  i1 = i1 % RING_N;
  let p0 = ring(i0); let p1 = ring(i1); let p2 = ring(i2); let p3 = ring(i3);
  // Catmull-Rom through the ring samples
  return 0.5 * ((2.0 * p1) + (-p0 + p2) * t + (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3) * t * t + (-p0 + 3.0 * p1 - 3.0 * p2 + p3) * t * t * t); // wubble change: none (the GLSL's numbers); marked because the template lines above, not counted on the GLSL side, shift the pairing of the `2`s
}
// @source slint-bubble-face-wgsl
// rotate a direction by yaw (about y) then pitch (about x), and back. Screen space: x right, y down, z toward the viewer.
fn faceRot(v0: vec3<f32>, a: vec2<f32>) -> vec3<f32> {
  let cy = cos(a.x); let sy = sin(a.x); let v = vec3<f32>(v0.x * cy + v0.z * sy, v0.y, -v0.x * sy + v0.z * cy);
  let cp = cos(a.y); let sp = sin(a.y); return vec3<f32>(v.x, v.y * cp + v.z * sp, -v.y * sp + v.z * cp);
}
fn faceRotInv(v0: vec3<f32>, a: vec2<f32>) -> vec3<f32> {
  let cp = cos(a.y); let sp = sin(a.y); let v = vec3<f32>(v0.x, v0.y * cp - v0.z * sp, v0.y * sp + v0.z * cp);
  let cy = cos(a.x); let sy = sin(a.x); return vec3<f32>(v.x * cy - v.z * sy, v.y, v.x * sy + v.z * cy);
}
// the backdrop at a blur level 0..1 (sharp → the blur pyramid), as the original's stacked copies
fn bgLevel(px: vec2<f32>, tb: f32) -> vec3<f32> {
  var uv = px / U.uView; uv.y = 1.0 - uv.y;
  var c = textureSampleLevel(uBg, samp, uv, 0.0).rgb;
  if (tb > 0.0) { c = mix(c, textureSampleLevel(uB1, samp, uv, 0.0).rgb, smoothstep(0.0, 0.5, tb)); }
  if (tb > 0.5) { c = mix(c, textureSampleLevel(uB2, samp, uv, 0.0).rgb, smoothstep(0.5, 1.0, tb)); }
  return c;
}
// 5-tap spectral split along the push, weighted so the taps sum back to white
fn spectral(px: vec2<f32>, split: vec2<f32>, tb: f32) -> vec3<f32> {
  if (dot(split, split) < 0.0625) { return bgLevel(px, tb); }
  var o = vec3<f32>(0.0);
  for (var i = 0; i < 5; i++) { o += bgLevel(px + split * (f32(i) / 2.0 - 1.0), tb) * SPECTRUM[i]; }
  return o;
}
fn bgAt(px: vec2<f32>) -> vec3<f32> { return bgLevel(px, 0.0); }
fn wAt(uv: vec2<f32>) -> f32 { return textureSampleLevel(uW, samp, uv, 0.0).a; } // wubble change: GLSL `texture(uW, uv).a`, here with an explicit LOD

@fragment
fn fs_bubble(fin: QuadOut) -> @location(0) vec4<f32> {
  let vPx = fin.vPx;
  let p = vPx - U.uC;
  let ang = atan2(p.y, p.x);
  let R = U.uR + ringAt(ang);
  let len = length(p);
  let d = len - R;
  let aa = 0.9 / U.uDpr;
  let inside = 1.0 - smoothstep(-aa, aa, d);
  if (inside <= 0.0) { return vec4<f32>(0.0); }
  let q = p / R; let rho = min(length(q), 1.0);
  let nz = sqrt(max(0.0, 1.0 - rho * rho));
  let n = normalize(vec3<f32>(q, nz));
  let cosI = nz; let cosT = cosRefracted(cosI);

  // film thickness: first-order only (≈200–560 nm), so colours stay soft and pastel, never banded.
  // Swirled slowly by noise and drained by gravity: a little thicker (warmer) toward the bottom.
  let t = U.uTime * 0.025 + U.uSeed;
  var sp = vec3<f32>(n.xy * 1.1, t);
  let ca = cos(U.uSwirl + t * 0.5); let sa = sin(U.uSwirl + t * 0.5);
  sp = vec3<f32>(mat2x2<f32>(ca, -sa, sa, ca) * sp.xy, sp.z);
  let warp = fbm(sp + vec3<f32>(fbm(sp * 1.3 + 4.0), fbm(sp * 1.3 + 9.0), 0.0) * 0.8);
  let drain = 0.5 + 0.5 * q.y;
  let thick = 250.0 + 150.0 * drain + 170.0 * warp;
  var film = thinFilm(thick, cosT);
  film = mix(vec3<f32>(dot(film, vec3<f32>(0.3333))), film, 0.72) * 0.9 + 0.08; // calm the saturation
  let fres = pow(1.0 - cosI, 3.0);

  // body: a clear lens for little bubbles, smooth dark glass for the logo
  // @source slint-bubble-wave-wgsl
  // pokes send a distortion wave through the body: it lenses, splits and blurs what the bubble refracts
  var wv: WaveHit; wv.disp = vec2<f32>(0.0); wv.band = 0.0; wv.blur = 0.0;
  for (var i = 0; i < 4; i++) {
    if (i >= U.uWaveN) { break; }
    let h = waveAt(vPx, U.uWave[i], R * 0.12, R * 0.22);
    wv.disp += h.disp; wv.band = max(wv.band, h.band); wv.blur = max(wv.blur, h.blur);
  }
  let lensOff = -q * (1.0 - nz) * R * 0.38;
  let refr = spectral(vPx + lensOff - wv.disp, wv.disp * 0.5, wv.blur * 0.8);
  let glassTop = vec3<f32>(0.150, 0.165, 0.215); let glassBot = vec3<f32>(0.035, 0.039, 0.055);
  // the lit side of the glass faces the light (the pointer)
  let ld = (U.uLight - U.uC) / (U.uR * 1.2);
  let ldir = select(vec2<f32>(-0.3, -0.95), normalize(ld), length(ld) > 0.05); // wubble change: WGSL `select(false, true, cond)` for the ternary, so the condition's number comes last
  let lit = dot(q, ldir) * min(1.0, 0.45 + length(ld) * 0.4);
  var bodyCol = mix(glassBot, glassTop, smoothstep(-0.9, 0.9, lit));
  bodyCol += refr * 0.12;
  var col = mix(refr, bodyCol, U.uBody);
  // film shows only where the surface turns away (Fresnel), never as a wash across the face
  col += film * fres * mix(0.75, 0.55, U.uBody);
  // light carried through the glass gathers at the lower rim, tinted by the film
  col += film * smoothstep(0.6, 1.0, rho) * max(0.0, q.y) * 0.22 * U.uBody;
  // a hairline rim and a faint crescent reflection along the top
  let rimLine = exp(-(d + 1.1) * (d + 1.1));
  col = mix(col, film * 1.1 + 0.1, rimLine * 0.55);
  let crescent = smoothstep(0.78, 0.97, rho) * smoothstep(0.1, 0.8, dot(q, ldir));
  col += crescent * 0.16;
  let rimBand = rimLine;
  // the front of the wave catches the light: a quiet film-coloured band
  col += (film * 0.5 + 0.06) * wv.band * 0.35;

  // @source slint-bubble-w-wgsl
  // the W: a glass tube in the brand sheen (Sky → Lilac → Blush → Mint), lit softly
  var wA = 0.0;
  if (U.uHasW > 0.5) {
    // the W lives on an inner sphere (radius 0.9 R). Un-rotate the visible point to find where it sits on the face.
    const KR = 0.9;
    let qi = (q - wv.disp * 0.55 / R) / KR; let ri = dot(qi, qi);
    let si = vec3<f32>(qi, sqrt(max(0.0, 1.0 - ri)));
    let s0 = faceRotInv(si, U.uFace);
    let facing = smoothstep(0.05, 0.25, s0.z) * step(ri, 1.0);
    let v = (s0.xy * KR * R) / U.uWScale + vec2<f32>(60.0);
    let vw = U.uWInv * (v - U.uWOff) + vec2<f32>(60.0, 65.0);
    let wuv = vec2<f32>(vw.x / 120.0, 1.0 - vw.y / 120.0);
    if (wuv.x > 0.0 && wuv.x < 1.0 && wuv.y > 0.0 && wuv.y < 1.0) {
      // distance to the W's centre line, in viewBox units; the tube radius is 5 (stroke width 10)
      const TUBE = 5.0;
      let dist = wAt(wuv) * 16.0;
      let e = 2.0 / 384.0;
      let grad = vec2<f32>(wAt(wuv + vec2<f32>(e, 0.0)) - wAt(wuv - vec2<f32>(e, 0.0)),
                           -(wAt(wuv + vec2<f32>(0.0, e)) - wAt(wuv - vec2<f32>(0.0, e))));
      let gdir = select(vec2<f32>(0.0), normalize(grad), length(grad) > 1e-6); // wubble change: the ternary as `select`, condition last
      let px = 1.0 / U.uWScale; // one screen pixel in viewBox units
      wA = (1.0 - smoothstep(TUBE - px * 0.6, TUBE + px * 0.6, dist)) * facing;
      let rr = clamp(dist / TUBE, 0.0, 1.0);
      let h = sqrt(max(0.0, 1.0 - rr * rr)); // round tube profile
      let wn = faceRot(normalize(vec3<f32>(gdir * rr, h + 0.001)), U.uFace);
      let g = clamp((vw.x - 28.0) / 64.0 * 0.75 + (vw.y - 46.0) / 40.0 * 0.25 + 0.06 * sin(U.uTime * 0.25 + U.uSwirl * 0.4), 0.0, 1.0);
      let sky = vec3<f32>(0.486, 0.784, 1.0); let lilac = vec3<f32>(0.722, 0.651, 1.0);
      let blush = vec3<f32>(1.0, 0.62, 0.847); let mint = vec3<f32>(0.541, 0.949, 0.816);
      var wc: vec3<f32>;
      if (g < 0.36) { wc = mix(sky, lilac, g / 0.36); } else if (g < 0.68) { wc = mix(lilac, blush, (g - 0.36) / 0.32); } else { wc = mix(blush, mint, (g - 0.68) / 0.32); }
      // soft key light from the top left, a little darker on the shadow side of the tube
      let shade = 0.74 + 0.30 * max(0.0, dot(wn, normalize(vec3<f32>(-0.35, -0.6, 0.72))));
      wc *= shade;
      let L = normalize(vec3<f32>((U.uLight - U.uC) / (U.uR * 1.3), 1.2));
      let wspec = pow(max(dot(wn, normalize(L + vec3<f32>(0.0, 0.0, 1.0))), 0.0), 36.0);
      wc += wspec * 0.3;
      col = mix(col, wc, wA);
    }
  }

  // specular: one soft reflection of the light (the pointer), and a small sharp one beside it
  let L = normalize(vec3<f32>((U.uLight - U.uC) / (U.uR * 0.85), 0.75));
  let H = normalize(L + vec3<f32>(0.0, 0.0, 1.0));
  let sp1 = pow(max(dot(n, H), 0.0), 34.0) * 0.42;
  let L2 = normalize(vec3<f32>(L.xy * 1.12 + vec2<f32>(0.14, -0.05), L.z));
  let sp2 = pow(max(dot(n, normalize(L2 + vec3<f32>(0.0, 0.0, 1.0))), 0.0), 900.0) * 0.55;
  col += (sp1 + sp2) * (1.0 - wA * 0.8);
  col = min(col, vec3<f32>(1.0));

  let a = inside * U.uAlpha;
  if (U.uOverlay > 0.5) {
    // over live content there is no backdrop copy: draw only what the film adds (reflection), premultiplied
    var add = film * fres * 0.9 + sp1 + sp2 + crescent * 0.16;
    add = mix(add, film * 1.1 + 0.1, rimBand * 0.55);
    let al = clamp(0.03 + fres * 0.7 + sp1 + sp2 + rimBand * 0.5, 0.0, 1.0) * a;
    return vec4<f32>(add * a, al);
  }
  // the bubble carries its own refracted copy of the backdrop, so it can cover fully
  return vec4<f32>(col * a, a);
}
