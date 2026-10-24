// Lit triangle meshes: extruded pie slices. The same lighting as the bars (`box.wgsl`):
// one directional light plus ambient, with a flat or smooth normal per vertex, so slices read
// as solid. A bar is an axis-aligned box that a static unit cube can be stretched into;
// a slice has curved walls, so its vertices come from the CPU (see `arc.rs`).

struct Globals {
    view_proj: mat4x4<f32>,
    // Camera position.
    eye: vec4<f32>,
    // x: camera azimuth in degrees, 0..360 (lines and text show only in their range).
    view: vec4<f32>,
    // xyz: unit vector pointing toward the light. w: ambient fraction.
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
    @location(1) normal: vec3<f32>,
    @location(2) color: vec4<f32>,
    // 1 while the pointer is over this slice: it is lightened a little.
    @location(3) hover: f32,
}

struct Varying {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec3<f32>,
}

@vertex
fn vs_main(v: Vertex) -> Varying {
    let ambient = globals.light.w;
    let lambert = max(dot(normalize(v.normal), globals.light.xyz), 0.0);
    let shade = ambient + (1.0 - ambient) * lambert;
    let lit = mix(v.color.rgb * shade, vec3<f32>(1.0), 0.12 * v.hover);
    return Varying(globals.view_proj * vec4<f32>(v.pos, 1.0), lit);
}

@fragment
fn fs_main(in: Varying) -> @location(0) vec4<f32> {
    return vec4<f32>(in.color, 1.0);
}
