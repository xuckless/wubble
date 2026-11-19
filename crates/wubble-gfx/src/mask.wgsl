// The page with its bowed edges (mask.rs): the pane's last frame, filled and stretched as the
// shell draws it at rest (SplitPane) with the stale frame cross-fading over it, cut by a rounded
// rect whose left and right edges follow two edge tables (wubble_core::bow), with the design's
// page colour where an edge bulges past the frame. Straight alpha out: Slint's Skia renderer imports a wgpu texture as unpremultiplied.
// The curve is the design's (`seam.ts clip`, `split.ts stringRect`); the edge as a cut is the
// page's `clipPath` in `browser/chrome.tsx`. Prepend: the scene prelude (vs_fullscreen, sdRound)
// and edge.wgsl (edgeAt).

struct MaskU {
  pane: vec2<f32>,    // the pane, physical px
  margin: f32,        // physical px either side of it in the output
  scale: f32,         // physical px per logical px (the edge tables are logical)
  stretch: vec2<f32>, // the frame's extra stretch per axis (0 at rest)
  clipUv: vec2<f32>,  // the part of the source shown, as a fraction of it
  fill: vec4<f32>,    // where an edge bulges past the frame (display-encoded)
  radius: f32,        // corner radius, physical px
  prevOpacity: f32,   // the stale frame fading out over the source (0: none)
  prevClipUv: vec2<f32>,
};

@group(0) @binding(0) var<uniform> M: MaskU;
@group(0) @binding(1) var samp: sampler;
@group(0) @binding(2) var src: texture_2d<f32>;
@group(0) @binding(3) var prevSrc: texture_2d<f32>;
// binding 5: EDGES (edge.wgsl), 0 the left edge, 1 the right

@fragment
fn fs_mask(fin: VsOut) -> @location(0) vec4<f32> {
  // pane-local physical px of this texel's centre
  let p = vec2<f32>(fin.pos.x - M.margin, fin.pos.y);
  let yl = p.y / M.scale;
  let xl = edgeAt(0u, yl) * M.scale;
  let xr = M.pane.x + edgeAt(1u, yl) * M.scale;
  let half_ = vec2<f32>(max(0.0, (xr - xl) * 0.5), M.pane.y * 0.5);
  let c = vec2<f32>((xl + xr) * 0.5, M.pane.y * 0.5);
  let d = sdRound(p - c, half_, min(M.radius, min(half_.x, half_.y)));
  let cover = clamp(0.5 - d, 0.0, 1.0);
  // the frame fills the pane times the stretch, anchored top left (SplitPane at rest)
  let f = p / (M.pane * (vec2<f32>(1.0) + M.stretch));
  var col = M.fill;
  var old = M.fill;
  if (f.x >= 0.0 && f.x < 1.0 && f.y >= 0.0 && f.y < 1.0) {
    col = textureSampleLevel(src, samp, f * M.clipUv, 0.0);
    old = textureSampleLevel(prevSrc, samp, f * M.prevClipUv, 0.0);
  }
  // the stale frame over the new one, as SplitPane's pane-prev at rest
  col = mix(col, old, M.prevOpacity);
  return vec4<f32>(col.rgb, cover * col.a);
}
