// Discs that face the viewer, anchored at a world point: the round joins of 3D lines and the
// markers on their data points. One instance of the unit quad per disc, stretched to its
// diameter in pixels around the projected anchor, so a disc is the same size at any distance.
// The fragment shader draws the disc (an antialiased circle) with a rim: outside `inner` it
// is the rim colour, inside the fill colour. A joint's disc has no rim (both colours equal).

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
    // Animation clock, see ease.wgsl.
    anim: vec4<f32>,
}

@group(0) @binding(0) var<uniform> globals: Globals;

// How far (world units) a disc is pulled toward the camera, so that a marker on the top of a
// bar, or a join at the edge of one, is not half swallowed by the surface it sits on.
const TOWARD_EYE: f32 = 0.015;

struct Vertex {
    @location(0) corner: vec2<f32>,
    // Per instance:
    @location(1) pos_size: vec4<f32>,  // xyz anchor, w diameter in pixels (0: not drawn)
    @location(2) color: vec4<f32>,
    @location(3) rim: vec4<f32>,
    @location(4) inner: f32,           // diameter of the fill in pixels
}

struct Varying {
    @builtin(position) position: vec4<f32>,
    // Pixels from the disc's centre.
    @location(0) local: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) rim: vec4<f32>,
    // Outer and fill radius in pixels.
    @location(3) radii: vec2<f32>,
}

@vertex
fn vs_main(v: Vertex) -> Varying {
    let size = v.pos_size.w;
    if size <= 0.0 {
        return Varying(vec4<f32>(2.0, 2.0, 2.0, 1.0), vec2<f32>(0.0), v.color, v.rim, vec2<f32>(0.0));
    }
    let anchor = v.pos_size.xyz;
    let world = anchor + normalize(globals.eye.xyz - anchor) * TOWARD_EYE;
    let clip = globals.view_proj * vec4<f32>(world, 1.0);
    let radius = size * 0.5;
    // A pixel of margin for the antialiased edge.
    let local = (v.corner * 2.0 - 1.0) * (radius + 1.0);
    // Pixels (y down) to clip space (y up); ndc to clip multiplies by w.
    let xy = clip.xy + vec2<f32>(local.x, -local.y) * 2.0 / globals.viewport * clip.w;
    return Varying(
        vec4<f32>(xy, clip.z, clip.w),
        local,
        v.color,
        v.rim,
        vec2<f32>(radius, v.inner * 0.5),
    );
}

@fragment
fn fs_main(in: Varying) -> @location(0) vec4<f32> {
    let d = length(in.local);
    let outer = clamp(in.radii.x - d + 0.5, 0.0, 1.0);
    let fill = clamp(in.radii.y - d + 0.5, 0.0, 1.0);
    let c = mix(in.rim, in.color, fill);
    return vec4<f32>(c.rgb, c.a * outer);
}
