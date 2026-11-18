// Dual-Kawase (dual-filter) blur with a folded saturate, fragment stages only.
// The vertex stage is `FULLSCREEN_VS_WGSL` in import.rs (`vs_main`,
// `VertexOutput.tex_coords`), prepended at pipeline creation.
//
// fs_down and fs_up: ported from KWin `src/plugins/blur/shaders/downsample.frag`
// and `upsample.frag` (KDE/kwin, GPL-2.0-or-later; algorithm: Bjorge, ARM,
// SIGGRAPH 2015), through the WGSL port in backdrop-blur-wgpu 0.2.0
// `src/shaders/downsample.wgsl` / `upsample.wgsl` (abdu-benayad/backdrop-blur,
// MIT OR Apache-2.0). Changed from that port: KWin's `offset` scalar is back,
// and fs_up applies the W3C feColorMatrix saturate rows from `params`.
//
// `halfpixel = 0.5 / size(sampled texture)` (KWin's convention). Sampling an
// sRGB texture decodes to linear, so blur and saturate run in linear light.

struct Params {
    halfpixel: vec2<f32>,
    offset: f32,
    _pad: f32,
    saturate: array<vec4<f32>, 3>,
};

@group(0) @binding(0)
var src: texture_2d<f32>;
@group(0) @binding(1)
var samp: sampler;
@group(0) @binding(2)
var<uniform> params: Params;

fn tap(uv: vec2<f32>) -> vec4<f32> {
    return textureSampleLevel(src, samp, uv, 0.0);
}

// 5 taps: centre x4 plus 4 diagonals, / 8.
@fragment
fn fs_down(in: VertexOutput) -> @location(0) vec4<f32> {
    let uv = in.tex_coords;
    let hp = params.halfpixel * params.offset;
    var sum = tap(uv) * 4.0;
    sum += tap(uv - hp);
    sum += tap(uv + hp);
    sum += tap(uv + vec2<f32>(hp.x, -hp.y));
    sum += tap(uv - vec2<f32>(hp.x, -hp.y));
    return sum / 8.0;
}

// One bilinear tap: a 2x2 box at half size. HalfRes's cheap first level.
@fragment
fn fs_copy(in: VertexOutput) -> @location(0) vec4<f32> {
    return tap(in.tex_coords);
}

// 8 taps: 4 cardinals x1 at 2 half-texels, 4 diagonals x2 at 1, / 12;
// then the saturate matrix (identity on intermediate passes).
@fragment
fn fs_up(in: VertexOutput) -> @location(0) vec4<f32> {
    let uv = in.tex_coords;
    let hp = params.halfpixel;
    let o = params.offset;
    var sum = tap(uv + vec2<f32>(-hp.x * 2.0, 0.0) * o);
    sum += tap(uv + vec2<f32>(-hp.x, hp.y) * o) * 2.0;
    sum += tap(uv + vec2<f32>(0.0, hp.y * 2.0) * o);
    sum += tap(uv + vec2<f32>(hp.x, hp.y) * o) * 2.0;
    sum += tap(uv + vec2<f32>(hp.x * 2.0, 0.0) * o);
    sum += tap(uv + vec2<f32>(hp.x, -hp.y) * o) * 2.0;
    sum += tap(uv + vec2<f32>(0.0, -hp.y * 2.0) * o);
    sum += tap(uv + vec2<f32>(-hp.x, -hp.y) * o) * 2.0;
    let c = sum / 12.0;
    let rgb = vec3<f32>(
        dot(params.saturate[0].xyz, c.rgb),
        dot(params.saturate[1].xyz, c.rgb),
        dot(params.saturate[2].xyz, c.rgb),
    );
    return vec4<f32>(clamp(rgb, vec3<f32>(0.0), vec3<f32>(1.0)), c.a);
}
