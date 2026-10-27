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
    // Animation clock, see ease.wgsl.
    anim: vec4<f32>,
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
    // Where the bar starts when it is animating (the fields above are where it ends), and how
    // many milliseconds into the transition it starts moving. At rest from == to.
    @location(6) from_min: vec3<f32>,
    @location(7) from_max: vec3<f32>,
    @location(8) from_color: vec4<f32>,
    @location(9) delay: f32,
}

struct Varying {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec3<f32>,
}

@vertex
fn vs_main(v: Vertex) -> Varying {
    let t = anim_t(v.delay);
    // `a (1 - t) + b t` is exactly `b` at t = 1, which `mix` is not guaranteed to be.
    let lo = v.from_min * (1.0 - t) + v.box_min * t;
    // A bar never turns inside out, even when an overshooting easing pushes t past 1.
    let hi = max(v.from_max * (1.0 - t) + v.box_max * t, lo);
    // A bar that is moving and has no height (or width or depth) yet, as it starts to grow or
    // after it shrank, is not drawn: squashed flat it would still show its top and bottom faces
    // as a plate. (A bar at rest is always drawn, even for the value 0.)
    let moving = any(v.from_min != v.box_min) || any(v.from_max != v.box_max);
    if moving && any(hi <= lo) {
        return Varying(vec4<f32>(2.0, 2.0, 2.0, 1.0), vec3<f32>(0.0));
    }
    let color = clamp(v.from_color * (1.0 - t) + v.color * t, vec4<f32>(0.0), vec4<f32>(1.0));
    let world = mix(lo, hi, v.corner);
    // Normals stay valid because the box is only scaled along its own axes, never rotated.
    let ambient = globals.light.w;
    let lambert = max(dot(v.normal, globals.light.xyz), 0.0);
    let shade = ambient + (1.0 - ambient) * lambert;
    let lit = mix(color.rgb * shade, vec3<f32>(1.0), 0.12 * v.hover);
    return Varying(globals.view_proj * vec4<f32>(world, 1.0), lit);
}

@fragment
fn fs_main(in: Varying) -> @location(0) vec4<f32> {
    return vec4<f32>(in.color, 1.0);
}
