// The page with its bowed edges (mask.rs): the pane's last frame, filled and stretched as the
// shell draws it at rest (SplitPane), cut by a rounded rect whose left and right edges follow two
// edge tables (wubble_core::bow), with the design's page colour where an edge bulges past the
// frame. Straight alpha out: femtovg treats a wgpu texture as not premultiplied.
// The curve is the design's (`seam.ts clip`, `split.ts stringRect`); the edge as a cut is the
// page's `clipPath` in `browser/chrome.tsx`. Prepend: the scene prelude (vs_fullscreen, sdRound).

struct MaskU {
  pane: vec2<f32>,    // the pane, physical px
  margin: f32,        // physical px either side of it in the output
  scale: f32,         // physical px per logical px (the tables are logical)
  stretch: vec2<f32>, // the frame's extra stretch per axis (0 at rest)
  clipUv: vec2<f32>,  // the part of the source shown, as a fraction of it
  fill: vec4<f32>,    // where an edge bulges past the frame (display-encoded)
  radius: f32,        // corner radius, physical px
  _p0: f32, _p1: f32, _p2: f32,
  lTab: vec4<f32>,    // left table: step, y0 (logical), samples, unused
  rTab: vec4<f32>,
  left: array<vec4<f32>, 64>,
  right: array<vec4<f32>, 64>,
};

@group(0) @binding(0) var<uniform> M: MaskU;
@group(0) @binding(1) var samp: sampler;
@group(0) @binding(2) var src: texture_2d<f32>;

fn leftAt(i: u32) -> f32 { return M.left[i / 4u][i % 4u]; }
fn rightAt(i: u32) -> f32 { return M.right[i / 4u][i % 4u]; }

// An edge's displacement at logical height y (bow::EdgeTable::at): linear, 0 outside.
fn edgeAt(tab: vec4<f32>, y: f32, isLeft: bool) -> f32 {
  if (tab.z < 2.0 || tab.x <= 0.0) { return 0.0; }
  let u = (y - tab.y) / tab.x;
  if (u < 0.0 || u > tab.z - 1.0) { return 0.0; }
  let i = min(u32(floor(u)), u32(tab.z) - 2u);
  let t = u - f32(i);
  var a: f32; var b: f32;
  if (isLeft) { a = leftAt(i); b = leftAt(i + 1u); } else { a = rightAt(i); b = rightAt(i + 1u); }
  return mix(a, b, t);
}

@fragment
fn fs_mask(fin: VsOut) -> @location(0) vec4<f32> {
  // pane-local physical px of this texel's centre
  let p = vec2<f32>(fin.pos.x - M.margin, fin.pos.y);
  let yl = p.y / M.scale;
  let xl = edgeAt(M.lTab, yl, true) * M.scale;
  let xr = M.pane.x + edgeAt(M.rTab, yl, false) * M.scale;
  let half_ = vec2<f32>(max(0.0, (xr - xl) * 0.5), M.pane.y * 0.5);
  let c = vec2<f32>((xl + xr) * 0.5, M.pane.y * 0.5);
  let d = sdRound(p - c, half_, min(M.radius, min(half_.x, half_.y)));
  let cover = clamp(0.5 - d, 0.0, 1.0);
  // the frame fills the pane times the stretch, anchored top left (SplitPane at rest)
  let f = p / (M.pane * (vec2<f32>(1.0) + M.stretch));
  var col = M.fill;
  if (f.x >= 0.0 && f.x < 1.0 && f.y >= 0.0 && f.y < 1.0) {
    col = textureSampleLevel(src, samp, f * M.clipUv, 0.0);
  }
  return vec4<f32>(col.rgb, cover * col.a);
}
