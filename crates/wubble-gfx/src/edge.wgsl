// wubble: a seam string's bowed edge as a displacement table (wubble_core::bow::EdgeTable, packed
// by lib.rs `EdgeU::pack`), shared by the sidebar glass (scene glass.wgsl) and the page's edge
// (mask.wgsl), so both follow the same string. Prepended to those two shaders only.

const EDGE_VEC4S: u32 = 64u; // lib.rs EDGE_SAMPLES / 4 (the uniform mirror test checks it)

struct EdgeU {
  tab: vec4<f32>,                     // step, y0 (logical px), samples, unused
  off: array<vec4<f32>, EDGE_VEC4S>,  // the samples, 4 to a vec4
};
@group(0) @binding(5) var<uniform> EDGES: array<EdgeU, 2>;

// Edge `k`'s displacement at logical height y (bow::EdgeTable::at): linear, 0 outside.
fn edgeAt(k: u32, y: f32) -> f32 {
  let e = &EDGES[k];
  let tab = (*e).tab;
  if (tab.z < 2.0 || tab.x <= 0.0) { return 0.0; }
  let u = (y - tab.y) / tab.x;
  if (u < 0.0 || u > tab.z - 1.0) { return 0.0; }
  let i = min(u32(floor(u)), u32(tab.z) - 2u);
  let j = i + 1u;
  return mix((*e).off[i / 4u][i % 4u], (*e).off[j / 4u][j % 4u], u - f32(i));
}
