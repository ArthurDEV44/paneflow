// US-008: WGSL shaders for GPU-accelerated terminal rendering
//
// Two render passes:
//   1. Background quads — solid-color rectangles per cell
//   2. Glyph quads — textured rectangles sampling the R8Unorm glyph atlas

// ─── Uniforms ────────────────────────────────────────────────────────────────

struct Uniforms {
    viewport_size: vec2<f32>,
}

@group(0) @binding(0)
var<uniform> uniforms: Uniforms;

// ─── Background pass ─────────────────────────────────────────────────────────

struct BgInstance {
    @location(0) cell_pos: vec2<f32>,
    @location(1) cell_size: vec2<f32>,
    @location(2) bg_color: vec4<f32>,
}

struct BgVsOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
}

@vertex
fn vs_bg(@builtin(vertex_index) vertex_index: u32, instance: BgInstance) -> BgVsOutput {
    // Generate 2 triangles (6 vertices) for a quad
    var positions = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(0.0, 1.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(1.0, 1.0),
        vec2<f32>(0.0, 1.0),
    );

    let unit = positions[vertex_index];
    let pixel_pos = instance.cell_pos + unit * instance.cell_size;

    // Pixel coords → NDC: x maps [0, width] → [-1, 1], y maps [0, height] → [1, -1]
    let ndc = vec2<f32>(
        pixel_pos.x / uniforms.viewport_size.x * 2.0 - 1.0,
        1.0 - pixel_pos.y / uniforms.viewport_size.y * 2.0,
    );

    var out: BgVsOutput;
    out.position = vec4<f32>(ndc, 0.0, 1.0);
    out.color = instance.bg_color;
    return out;
}

@fragment
fn fs_bg(in: BgVsOutput) -> @location(0) vec4<f32> {
    return in.color;
}

// ─── Glyph pass ──────────────────────────────────────────────────────────────

@group(0) @binding(1)
var atlas_texture: texture_2d<f32>;

@group(0) @binding(2)
var atlas_sampler: sampler;

struct GlyphInstance {
    @location(0) cell_pos: vec2<f32>,
    @location(1) glyph_size: vec2<f32>,
    @location(2) glyph_offset: vec2<f32>,
    @location(3) uv_min: vec2<f32>,
    @location(4) uv_max: vec2<f32>,
    @location(5) fg_color: vec4<f32>,
}

struct GlyphVsOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) fg_color: vec4<f32>,
}

@vertex
fn vs_glyph(@builtin(vertex_index) vertex_index: u32, instance: GlyphInstance) -> GlyphVsOutput {
    var positions = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(0.0, 1.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(1.0, 1.0),
        vec2<f32>(0.0, 1.0),
    );

    let unit = positions[vertex_index];
    let pixel_pos = instance.cell_pos + instance.glyph_offset + unit * instance.glyph_size;

    let ndc = vec2<f32>(
        pixel_pos.x / uniforms.viewport_size.x * 2.0 - 1.0,
        1.0 - pixel_pos.y / uniforms.viewport_size.y * 2.0,
    );

    let uv = mix(instance.uv_min, instance.uv_max, unit);

    var out: GlyphVsOutput;
    out.position = vec4<f32>(ndc, 0.0, 1.0);
    out.uv = uv;
    out.fg_color = instance.fg_color;
    return out;
}

@fragment
fn fs_glyph(in: GlyphVsOutput) -> @location(0) vec4<f32> {
    let alpha = textureSample(atlas_texture, atlas_sampler, in.uv).r;
    return vec4<f32>(in.fg_color.rgb, in.fg_color.a * alpha);
}
