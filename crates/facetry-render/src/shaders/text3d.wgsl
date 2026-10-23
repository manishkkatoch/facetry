// MSDF text anchored in the world. Every glyph of a label shares one world-space anchor; the
// vertex shader projects it to the screen and lays the glyph out around that pixel, so labels
// always face the viewer and keep their pixel size however the camera moves. The fragment
// half is the same as text.wgsl (keep them in sync).
//
// Like lines, labels carry an azimuth range; see line3d.wgsl.

struct Globals {
    view_proj: mat4x4<f32>,
    eye: vec4<f32>,
    // x: camera azimuth in degrees, 0..360 (lines and text show only in their range).
    view: vec4<f32>,
    light: vec4<f32>,
    viewport: vec2<f32>,
    // Atlas size in pixels.
    atlas: vec2<f32>,
    // Total distance range encoded in the atlas, in atlas pixels.
    px_range: f32,
    // Pads the block to a multiple of 16 bytes (WebGL2).
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
}

@group(0) @binding(0) var<uniform> globals: Globals;
@group(0) @binding(1) var atlas_texture: texture_2d<f32>;
@group(0) @binding(2) var atlas_sampler: sampler;

struct Vertex {
    @location(0) corner: vec2<f32>,
    // Per instance:
    @location(1) anchor: vec3<f32>,  // world position the label hangs from
    @location(2) origin: vec2<f32>,  // pixels from the projected anchor to the glyph's top-left
    @location(3) axis_x: vec2<f32>,  // pixel vector along the glyph's width
    @location(4) axis_y: vec2<f32>,  // pixel vector along the glyph's height, pointing down
    @location(5) uv: vec4<f32>,      // atlas rect: top-left u, v, bottom-right u, v
    @location(6) color: vec4<f32>,
    @location(7) show: vec2<f32>,    // azimuth range [from, to) in which to draw
}

struct Varying {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
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
    if !in_range(v.show) {
        return Varying(vec4<f32>(2.0, 2.0, 2.0, 1.0), vec2<f32>(0.0), v.color);
    }
    let clip = globals.view_proj * vec4<f32>(v.anchor, 1.0);
    let ndc = clip.xy / clip.w;
    // Projected anchor in pixels (y down), rounded to a whole pixel so glyph edges stay
    // sharp instead of shimmering as the camera moves a sub-pixel amount.
    let anchor_px = round(vec2<f32>(
        (ndc.x * 0.5 + 0.5) * globals.viewport.x,
        (0.5 - ndc.y * 0.5) * globals.viewport.y,
    ));
    let px = anchor_px + v.origin + v.corner.x * v.axis_x + v.corner.y * v.axis_y;
    let out_clip = vec2<f32>(
        px.x / globals.viewport.x * 2.0 - 1.0,
        1.0 - px.y / globals.viewport.y * 2.0,
    );
    let uv = mix(v.uv.xy, v.uv.zw, v.corner);
    return Varying(vec4<f32>(out_clip, 0.0, 1.0), uv, v.color);
}

fn median(r: f32, g: f32, b: f32) -> f32 {
    return max(min(r, g), min(max(r, g), b));
}

@fragment
fn fs_main(in: Varying) -> @location(0) vec4<f32> {
    let msd = textureSample(atlas_texture, atlas_sampler, in.uv).rgb;
    let sd = median(msd.r, msd.g, msd.b) - 0.5;
    let unit_range = vec2<f32>(globals.px_range) / globals.atlas;
    let screen_tex_size = vec2<f32>(1.0) / fwidth(in.uv);
    let screen_px_range = max(0.5 * dot(unit_range, screen_tex_size), 1.0);
    let opacity = clamp(sd * screen_px_range + 0.5, 0.0, 1.0);
    return vec4<f32>(in.color.rgb, in.color.a * opacity);
}
