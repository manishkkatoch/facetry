// MSDF text. Each glyph is one instance of the unit quad, placed by an origin and two edge
// vectors (so rotated text needs no extra code), textured from the atlas.

struct Globals {
    viewport: vec2<f32>,
    // Atlas size in pixels.
    atlas: vec2<f32>,
    // Total distance range encoded in the atlas, in atlas pixels.
    px_range: f32,
    // Pads the block to 32 bytes: WebGL2 needs uniform blocks to be a multiple of 16.
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
    @location(1) origin: vec2<f32>,  // pixel position of the quad's top-left (in glyph space)
    @location(2) axis_x: vec2<f32>,  // pixel vector along the glyph's width
    @location(3) axis_y: vec2<f32>,  // pixel vector along the glyph's height, pointing down
    @location(4) uv: vec4<f32>,      // atlas rect: top-left u, v, bottom-right u, v
    @location(5) color: vec4<f32>,
}

struct Varying {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
}

@vertex
fn vs_main(v: Vertex) -> Varying {
    let px = v.origin + v.corner.x * v.axis_x + v.corner.y * v.axis_y;
    let clip = vec2<f32>(
        px.x / globals.viewport.x * 2.0 - 1.0,
        1.0 - px.y / globals.viewport.y * 2.0,
    );
    let uv = mix(v.uv.xy, v.uv.zw, v.corner);
    return Varying(vec4<f32>(clip, 0.0, 1.0), uv, v.color);
}

fn median(r: f32, g: f32, b: f32) -> f32 {
    return max(min(r, g), min(max(r, g), b));
}

@fragment
fn fs_main(in: Varying) -> @location(0) vec4<f32> {
    let msd = textureSample(atlas_texture, atlas_sampler, in.uv).rgb;
    // The median of the three channels is the signed distance with sharp corners intact:
    // 0.5 is the glyph outline, above is inside, below is outside.
    let sd = median(msd.r, msd.g, msd.b) - 0.5;
    // How many *screen* pixels the encoded distance range spans at this size. The atlas is
    // magnified or shrunk on screen; `fwidth(uv)` says by how much. The floor of 1 keeps tiny
    // text from collapsing to nothing.
    let unit_range = vec2<f32>(globals.px_range) / globals.atlas;
    let screen_tex_size = vec2<f32>(1.0) / fwidth(in.uv);
    let screen_px_range = max(0.5 * dot(unit_range, screen_tex_size), 1.0);
    // One screen pixel of distance maps to one unit of opacity: a crisp, antialiased edge.
    let opacity = clamp(sd * screen_px_range + 0.5, 0.0, 1.0);
    return vec4<f32>(in.color.rgb, in.color.a * opacity);
}
