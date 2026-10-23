// Bars as instanced boxes. A static 36-vertex unit cube (each vertex carries its face's flat
// normal) is stretched to every instance's min/max corners. Lambert shading with one
// directional light plus ambient gives every face a different brightness, which is what
// makes the boxes read as solid.

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
    // Per vertex: which corner of the unit cube, (0,0,0) to (1,1,1), and the face normal.
    @location(0) corner: vec3<f32>,
    @location(1) normal: vec3<f32>,
    // Per instance:
    @location(2) box_min: vec3<f32>,
    @location(3) box_max: vec3<f32>,
    @location(4) color: vec4<f32>,
    // 1 while the pointer is over this bar: it is lightened a little.
    @location(5) hover: f32,
}

struct Varying {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec3<f32>,
}

@vertex
fn vs_main(v: Vertex) -> Varying {
    let world = mix(v.box_min, v.box_max, v.corner);
    // Normals stay valid because the box is only scaled along its own axes, never rotated.
    let ambient = globals.light.w;
    let lambert = max(dot(v.normal, globals.light.xyz), 0.0);
    let shade = ambient + (1.0 - ambient) * lambert;
    let lit = mix(v.color.rgb * shade, vec3<f32>(1.0), 0.12 * v.hover);
    return Varying(globals.view_proj * vec4<f32>(world, 1.0), lit);
}

@fragment
fn fs_main(in: Varying) -> @location(0) vec4<f32> {
    return vec4<f32>(in.color, 1.0);
}
