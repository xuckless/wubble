// translates: core/gl/util.ts:3-19
// Vertex stages shared by the full-screen passes (fluid, composite, blur, blit).
// Translated from VERT_FS in core/gl/util.ts.
//
// @source slint-uv-convention
// Texture-coordinate convention (the one place WebGL and wgpu really differ):
// - Both APIs put texture coordinate v = 0 at the FIRST row in memory.
// - WebGL rasterises clip y = −1 into framebuffer row 0; wgpu rasterises clip y = +1 into row 0.
// So in WebGL `vUv = aPos * 0.5 + 0.5` is the texture coordinate of the texel a fragment writes.
// Here we emit `vUv = (x * 0.5 + 0.5, 0.5 − y * 0.5)`, which is again exactly the texture
// coordinate of the texel being written. Every offscreen target (velocity, dye, pressure, bg,
// blur pyramid) therefore keeps the WebGL row order: row 0 is the BOTTOM of the screen, field
// y points up, and all the GLSL maths (T = +texel.y, splat y = 1 − y/h, `uv.y = 1.0 − uv.y`
// when sampling bg from top-left px) carries over unchanged. Only two places flip:
//   1. blit.wgsl samples bg with v = 1 − vUv.y when writing the top-down screen/Slint texture;
//   2. the W distance field is uploaded with its rows bottom-first, exactly like the TS.
// Quads (glass, bubble, pop) are positioned in top-left px with clip = (x, −y), which is the
// same formula in both APIs, because both have clip y pointing up.

struct VsOut {
  @builtin(position) pos: vec4<f32>,
  @location(0) vUv: vec2<f32>,
};

// One full-screen triangle-strip quad (4 vertices, no vertex buffer).
fn stripCorner(i: u32) -> vec2<f32> {
  var c = array<vec2<f32>, 4>(vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(-1.0, 1.0), vec2<f32>(1.0, 1.0)); // wubble change: util.ts quad()'s buffer, inline
  return c[i];
}

@vertex
fn vs_fullscreen(@builtin(vertex_index) vi: u32) -> VsOut {
  let aPos = stripCorner(vi);
  var o: VsOut;
  o.pos = vec4<f32>(aPos, 0.0, 1.0);
  o.vUv = vec2<f32>(aPos.x * 0.5 + 0.5, 0.5 - aPos.y * 0.5);
  return o;
}

// @source slint-quad-vs-wgsl
// VERT_QUAD in core/gl/util.ts: a quad in top-left logical px. aPos is −1..1, expanded to
// uQuad (x, y, w, h). Each quad shader calls this from its own @vertex with its own uniforms.
struct QuadOut {
  @builtin(position) pos: vec4<f32>,
  @location(0) vPx: vec2<f32>,
};

fn quadVs(vi: u32, uQuad: vec4<f32>, uView: vec2<f32>) -> QuadOut {
  let aPos = stripCorner(vi);
  let unit = aPos * 0.5 + 0.5;
  var o: QuadOut;
  o.vPx = uQuad.xy + unit * uQuad.zw;
  let clip = o.vPx / uView * 2.0 - 1.0;
  o.pos = vec4<f32>(clip.x, -clip.y, 0.0, 1.0);
  return o;
}
