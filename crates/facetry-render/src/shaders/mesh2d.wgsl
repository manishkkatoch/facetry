// Plain coloured triangles in pixel space: tessellated lines, line markers and pie slices.
// The vertices come from the CPU (see `stroke.rs` and `arc.rs`), already in device pixels.

struct Globals {
    // Render target size in pixels; converts pixel coordinates to clip space.
    viewport: vec2<f32>,
    // Atlas size in pixels and its distance range (used by the text shader only).
    atlas: vec2<f32>,
    px_range: f32,
    // Pads the block to 32 bytes: WebGL2 needs uniform blocks to be a multiple of 16.
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
}

@group(0) @binding(0) var<uniform> globals: Globals;

struct Vertex {
    @location(0) pos: vec2<f32>,    // device pixels, origin top-left
    @location(1) color: vec4<f32>,
}

struct Varying {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
}

@vertex
fn vs_main(v: Vertex) -> Varying {
    let clip = vec2<f32>(
        v.pos.x / globals.viewport.x * 2.0 - 1.0,
        1.0 - v.pos.y / globals.viewport.y * 2.0,
    );
    return Varying(vec4<f32>(clip, 0.0, 1.0), v.color);
}

@fragment
fn fs_main(in: Varying) -> @location(0) vec4<f32> {
    return in.color;
}
