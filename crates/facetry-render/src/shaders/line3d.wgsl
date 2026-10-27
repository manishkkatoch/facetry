// Gridlines, axis edges and ticks in world space. Each line is one instance of the unit quad,
// stretched between its two projected end points and `width` pixels across, so lines have
// the same crisp pixel width at any angle and distance (hardware lines are 1px at best, and
// WebGL2 cannot do more).
//
// Each line carries an azimuth range and is drawn only while the camera is inside it. That is
// how back walls and the axes (which sit on different edges from different viewpoints) follow
// the camera without the scene being rebuilt: all variants are in the buffer, the vertex
// shader throws away the ones that do not apply.

struct Globals {
    view_proj: mat4x4<f32>,
    eye: vec4<f32>,
    // x: camera azimuth in degrees, 0..360 (lines and text show only in their range).
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

struct Vertex {
    // Per vertex: x picks the start (0) or end (1), y the side of the line.
    @location(0) corner: vec2<f32>,
    // Per instance:
    @location(1) a: vec4<f32>,       // xyz start, w width in pixels
    @location(2) b: vec4<f32>,       // xyz end, w: 1 for square caps, 0 for flat ends
    @location(3) color: vec4<f32>,
    @location(4) show: vec2<f32>,    // azimuth range [from, to) in which to draw
}

struct Varying {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
}

// `show` is the half-open azimuth range [from, to) in degrees; from > to wraps through 360.
fn in_range(show: vec2<f32>) -> bool {
    let az = globals.view.x;
    if show.x <= show.y {
        return az >= show.x && az < show.y;
    }
    return az >= show.x || az < show.y;
}

@vertex
fn vs_main(v: Vertex) -> Varying {
    // A position outside the clip volume: the whole quad is discarded.
    if !in_range(v.show) {
        return Varying(vec4<f32>(2.0, 2.0, 2.0, 1.0), v.color);
    }
    let ca = globals.view_proj * vec4<f32>(v.a.xyz, 1.0);
    let cb = globals.view_proj * vec4<f32>(v.b.xyz, 1.0);
    // Direction of the line on screen, in pixels (ndc spans 2 units over the viewport).
    let delta = (cb.xy / cb.w - ca.xy / ca.w) * globals.viewport * 0.5;
    let len = length(delta);
    var dir = vec2<f32>(1.0, 0.0);
    if len > 0.0001 {
        dir = delta / len;
    }
    let normal = vec2<f32>(-dir.y, dir.x);
    let half_width = v.a.w * 0.5;
    // Square caps: stretch by half a width at both ends so lines that meet at a corner
    // leave no notch. Data lines turn this off (b.w = 0): their joins are round discs.
    let along = (v.corner.x * 2.0 - 1.0) * half_width * v.b.w;
    let across = (v.corner.y * 2.0 - 1.0) * half_width;
    let offset_px = dir * along + normal * across;
    // Interpolating in clip space keeps the depth perspective-correct. Pixels to ndc is
    // 2 / viewport, and ndc to clip multiplies by w.
    let clip = mix(ca, cb, v.corner.x);
    let xy = clip.xy + offset_px * 2.0 / globals.viewport * clip.w;
    return Varying(vec4<f32>(xy, clip.z, clip.w), v.color);
}

@fragment
fn fs_main(in: Varying) -> @location(0) vec4<f32> {
    return in.color;
}
