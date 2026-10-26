// Instanced rectangles. One unit quad (4 corners, drawn as a triangle strip) is stretched
// to each instance's rect, so a whole chart's bars and lines are a single draw call.

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
    // Animation clock, see ease.wgsl.
    anim: vec4<f32>,
}

@group(0) @binding(0) var<uniform> globals: Globals;

struct Vertex {
    // Per vertex: which corner of the unit quad, (0,0) top-left to (1,1) bottom-right.
    @location(0) corner: vec2<f32>,
    // Per instance:
    @location(1) rect: vec4<f32>,  // x, y, width, height in device pixels, origin top-left
    @location(2) color: vec4<f32>,
    // 1 while the pointer is over this rectangle: it is lightened a little.
    @location(3) hover: f32,
    // Where the rectangle starts when it is animating (`rect` and `color` are where it ends),
    // and how many milliseconds into the transition it starts moving. At rest from == to.
    @location(4) from_rect: vec4<f32>,
    @location(5) from_color: vec4<f32>,
    @location(6) delay: f32,
}

struct Varying {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
}

@vertex
fn vs_main(v: Vertex) -> Varying {
    let t = anim_t(v.delay);
    // `a (1 - t) + b t` is exactly `b` at t = 1, which `mix` is not guaranteed to be.
    let rect_xy = v.from_rect.xy * (1.0 - t) + v.rect.xy * t;
    let rect_wh = max(v.from_rect.zw * (1.0 - t) + v.rect.zw * t, vec2<f32>(0.0));
    let color = clamp(v.from_color * (1.0 - t) + v.color * t, vec4<f32>(0.0), vec4<f32>(1.0));
    let px = rect_xy + v.corner * rect_wh;
    // Orthographic projection: pixels (y down) to clip space (y up).
    let clip = vec2<f32>(
        px.x / globals.viewport.x * 2.0 - 1.0,
        1.0 - px.y / globals.viewport.y * 2.0,
    );
    let rgb = mix(color.rgb, vec3<f32>(1.0), 0.12 * v.hover);
    return Varying(vec4<f32>(clip, 0.0, 1.0), vec4<f32>(rgb, color.a));
}

@fragment
fn fs_main(in: Varying) -> @location(0) vec4<f32> {
    return in.color;
}
