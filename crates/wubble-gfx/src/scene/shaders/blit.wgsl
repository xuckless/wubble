// translates: core/gl/scene.ts:123-127
// Backdrop to the screen, translated from BLIT in core/gl/scene.ts. Prepend: common.wgsl, film.wgsl, wave.wgsl, beam.wgsl (render.rs always prepends all four).
// This is the one full-screen pass that flips v: `bg` is stored bottom-first (WebGL order,
// see common.wgsl) and the destination (swapchain / Slint image texture) is top-first.

// @source slint-blit-wgsl
@group(0) @binding(1) var samp: sampler;
@group(0) @binding(2) var uTex: texture_2d<f32>;

@fragment
fn fs_blit(fin: VsOut) -> @location(0) vec4<f32> {
  return textureSampleLevel(uTex, samp, vec2<f32>(fin.vUv.x, 1.0 - fin.vUv.y), 0.0); // wubble change: rows flipped for the top-first target, explicit LOD 0
}
