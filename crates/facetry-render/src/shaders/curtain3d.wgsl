// The translucent curtain under a 3D line (`style.area`): plain coloured triangles in world
// space. The colour carries the opacity; blending and depth are set up by the pipeline.

struct Globals {
    view_proj: mat4x4<f32>,
    eye: vec4<f32>,
    view: vec4<f32>,
    light: vec4<f32>,
    viewport: vec2<f32>,
    atlas: vec2<f32>,
    px_range: f32,
    // Pads the block to a multiple of 16 bytes (WebGL2).
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
}

@group(0) @binding(0) var<uniform> globals: Globals;

struct Vertex {
    @location(0) pos: vec3<f32>,
    @location(1) color: vec4<f32>,
}

struct Varying {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
}

@vertex
fn vs_main(v: Vertex) -> Varying {
    return Varying(globals.view_proj * vec4<f32>(v.pos, 1.0), v.color);
}

@fragment
fn fs_main(in: Varying) -> @location(0) vec4<f32> {
    return in.color;
}
