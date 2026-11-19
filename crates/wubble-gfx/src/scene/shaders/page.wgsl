// translates: wubble-only
// wubble's own passes for the page in the scene (pages.rs); the design's web gets the same from
// `backdrop-filter`, so there is no GLSL to translate. Written for wubble.
//   fs_page: one pane's latest frame, a rounded card at its place, into the page-inclusive blur
//            source (render.rs `P.bg`, bottom-first like `bg`: see common.wgsl). The frame holds
//            display-encoded bytes (import.rs), the values `bg` holds, so it is sampled raw.
//   fs_resolve: an overlay layer's premultiplied glass to the straight alpha Skia expects of a
//            wgpu image (`AlphaType::Unpremul`), texel for texel.
// Prepend: common.wgsl, film.wgsl, wave.wgsl, beam.wgsl (render.rs always prepends all four).

struct PageU {
  uQuad: vec4<f32>, // the card: x, y, w, h in top-left logical px
  uView: vec2<f32>,
  uUv: vec2<f32>,   // the shown part of the frame: clip size / texture size
  uR: f32,
  _p0: f32, _p1: f32, _p2: f32,
};

@group(0) @binding(0) var<uniform> U: PageU;
@group(0) @binding(1) var samp: sampler;
@group(0) @binding(2) var uTex: texture_2d<f32>;

@vertex
fn vs_page(@builtin(vertex_index) vi: u32) -> QuadOut {
  var o = quadVs(vi, U.uQuad, U.uView);
  o.pos.y = -o.pos.y; // the target is bottom-first: screen y down is clip y up
  return o;
}

@fragment
fn fs_page(fin: QuadOut) -> @location(0) vec4<f32> {
  let half_ = U.uQuad.zw * 0.5;
  let p = fin.vPx - U.uQuad.xy - half_;
  let d = sdRound(p, half_, min(U.uR, min(half_.x, half_.y)));
  let a = 1.0 - smoothstep(-1.0, 1.0, d);
  let uv = (fin.vPx - U.uQuad.xy) / U.uQuad.zw * U.uUv;
  let c = textureSampleLevel(uTex, samp, uv, 0.0).rgb;
  return vec4<f32>(c * a, a);
}

@fragment
fn fs_resolve(fin: VsOut) -> @location(0) vec4<f32> {
  let c = textureSampleLevel(uTex, samp, fin.vUv, 0.0); // same size and row order: texel centres
  return vec4<f32>(c.rgb / max(c.a, 1e-4), c.a);
}
