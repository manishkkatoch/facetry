// Lit triangle meshes: extruded pie slices, and a diagram's nodes, arrows and cards. The same
// lighting as the bars (`box.wgsl`): one directional light plus ambient, with a flat or smooth
// normal per vertex, so slices read as solid; glossy surfaces add a specular highlight, worked
// out per pixel so it can slide across a flat face. A bar is an axis-aligned box that a static unit cube can be stretched into;
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
    // Specular strength: 0 for matte surfaces (slices), up to 1 for a glossy diagram's slabs.
    @location(4) gloss: f32,
}

struct Varying {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) world: vec3<f32>,
    @location(3) gloss: f32,
}

@vertex
fn vs_main(v: Vertex) -> Varying {
    let ambient = globals.light.w;
    let lambert = max(dot(normalize(v.normal), globals.light.xyz), 0.0);
    let shade = ambient + (1.0 - ambient) * lambert;
    let lit = mix(v.color.rgb * shade, vec3<f32>(1.0), 0.12 * v.hover);
    return Varying(globals.view_proj * vec4<f32>(v.pos, 1.0), lit, v.normal, v.pos, v.gloss);
}

@fragment
fn fs_main(in: Varying) -> @location(0) vec4<f32> {
    if in.gloss <= 0.0 {
        return vec4<f32>(in.color, 1.0);
    }
    // Sheen: the reflection of a bright sky, as strong as the (interpolated) gloss says; the
    // mesh makes it a gradient across each top face.
    let c = mix(in.color, vec3<f32>(1.0), 0.3 * in.gloss);
    // And a specular highlight from a light that follows the camera, high behind the scene.
    // The direction to the eye: from the point in perspective, the view direction (the same
    // everywhere) for the orthographic lens (view.w = 0).
    let az = radians(globals.view.x);
    let el = radians(globals.view.y);
    let forward = vec3<f32>(sin(az) * cos(el), sin(el), cos(az) * cos(el));
    var to_eye = forward;
    if globals.view.w > 0.5 {
        to_eye = normalize(globals.eye.xyz - in.world);
    }
    let key = normalize(vec3<f32>(-forward.x, 1.2, -forward.z));
    let n = normalize(in.normal);
    let h = normalize(key + to_eye);
    let spec = pow(max(dot(n, h), 0.0), 60.0) * 0.35 * in.gloss;
    return vec4<f32>(c + vec3<f32>(spec), 1.0);
}
