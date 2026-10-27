//! MSDF text: turning strings into glyph quads (pure, unit-tested) and drawing them.

use bytemuck::{Pod, Zeroable};
use facetry_core::scene::{Anchor, Rotation, Text};
use facetry_core::scene3d::{Text3, VAlign};
use facetry_font::FontMetrics;

use crate::Error;
use crate::gpu::{InstanceBuffer, QUAD_LAYOUT, show_range};
use crate::rect::uniform_entry;
use crate::solid::{PipelineDesc, pipeline};

/// One glyph quad. The quad's corners are `origin`, `origin + axis_x`, `origin + axis_y` and
/// both: two edge vectors instead of a width and height, so rotated text is just different
/// vectors. `uv` is the atlas rect as `[u0, v0, u1, v1]`.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable)]
pub struct GlyphInstance {
    pub origin: [f32; 2],
    pub axis_x: [f32; 2],
    pub axis_y: [f32; 2],
    pub uv: [f32; 4],
    pub color: [f32; 4],
}

/// Appends one instance per visible glyph of `text`.
///
/// Layout is a simple left-to-right pen walk: each glyph's quad sits at the pen plus its
/// plane bounds (in em, y up, scaled by the font size), then the pen advances. The whole
/// run is shifted back by the anchor (none / half / full width). There is no kerning in M1.
///
/// Rotation works in a local frame with `x` along the reading direction and `y` pointing
/// "down" through the glyph; `dir_x`/`dir_y` map that frame to the screen.
pub fn layout_glyphs(
    text: &Text,
    color: [f32; 4],
    font: &FontMetrics,
    out: &mut Vec<GlyphInstance>,
) {
    let size = text.size_px;
    let width = font.text_width(&text.text, size);
    let mut pen = match text.anchor {
        Anchor::Start => 0.0,
        Anchor::Middle => -width / 2.0,
        Anchor::End => -width,
    };
    let (dir_x, dir_y) = match text.rotation {
        Rotation::None => ([1.0, 0.0], [0.0, 1.0]),
        // Reads upward; glyph tops point left, so "down" through a glyph points right.
        Rotation::Minus90 => ([0.0, -1.0], [1.0, 0.0]),
    };
    let to_screen = |lx: f32, ly: f32| {
        [
            text.x + dir_x[0] * lx + dir_y[0] * ly,
            text.y + dir_x[1] * lx + dir_y[1] * ly,
        ]
    };
    let atlas = &font.atlas;
    for ch in text.text.chars() {
        let glyph = font.glyph(ch);
        if let (Some([l, b, r, t]), Some([ax, ay, aw, ah])) = (glyph.plane, glyph.atlas) {
            // Plane y points up from the baseline; local y points down, hence the flip.
            let (x0, x1) = (pen + l * size, pen + r * size);
            let (y0, y1) = (-t * size, -b * size);
            out.push(GlyphInstance {
                origin: to_screen(x0, y0),
                axis_x: [dir_x[0] * (x1 - x0), dir_x[1] * (x1 - x0)],
                axis_y: [dir_y[0] * (y1 - y0), dir_y[1] * (y1 - y0)],
                uv: [
                    ax as f32 / atlas.width as f32,
                    ay as f32 / atlas.height as f32,
                    (ax + aw) as f32 / atlas.width as f32,
                    (ay + ah) as f32 / atlas.height as f32,
                ],
                color,
            });
        }
        pen += glyph.advance * size;
    }
}

/// Expands the embedded raw RGB atlas to RGBA8 for upload; WebGPU has no RGB8 format.
fn decode_atlas() -> Result<(u32, u32, Vec<u8>), Error> {
    let atlas = &facetry_font::FontMetrics::inter().atlas;
    let rgb = facetry_font::ATLAS_RGB;
    if rgb.len() != atlas.width as usize * atlas.height as usize * 3 {
        return Err(Error::RenderFailed(
            "font atlas size does not match its metrics".into(),
        ));
    }
    let rgba = rgb
        .chunks_exact(3)
        .flat_map(|p| [p[0], p[1], p[2], 255])
        .collect();
    Ok((atlas.width, atlas.height, rgba))
}

/// The glyph atlas on the GPU, shared by the 2D and 3D text pipelines.
#[derive(Debug)]
pub struct Atlas {
    view: wgpu::TextureView,
    sampler: wgpu::Sampler,
}

impl Atlas {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Result<Self, Error> {
        let (width, height, pixels) = decode_atlas()?;
        let size = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };
        // Plain `Rgba8Unorm`, not sRGB: the channels are distances, not colours, and must
        // not be gamma-decoded when sampled.
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("font atlas"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            texture.as_image_copy(),
            &pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width * 4),
                rows_per_image: Some(height),
            },
            size,
        );
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        // Bilinear filtering is what makes the distance field smooth when scaled.
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("font atlas sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Ok(Self { view, sampler })
    }

    /// The bind group layout and group shared by both text pipelines: globals, atlas, sampler,
    /// and optionally (binding 3) a second uniform buffer, which the 3D pipeline uses for its
    /// label-state table.
    fn bind(
        &self,
        device: &wgpu::Device,
        name: &str,
        globals: &wgpu::Buffer,
        extra: Option<&wgpu::Buffer>,
    ) -> (wgpu::PipelineLayout, wgpu::BindGroup) {
        let mut layout_entries = vec![
            uniform_entry(0),
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
        ];
        let mut entries = vec![
            wgpu::BindGroupEntry {
                binding: 0,
                resource: globals.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(&self.view),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::Sampler(&self.sampler),
            },
        ];
        if let Some(buffer) = extra {
            layout_entries.push(uniform_entry(3));
            entries.push(wgpu::BindGroupEntry {
                binding: 3,
                resource: buffer.as_entire_binding(),
            });
        }
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some(&format!("{name} bind group layout")),
            entries: &layout_entries,
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(&format!("{name} bind group")),
            layout: &layout,
            entries: &entries,
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some(&format!("{name} pipeline layout")),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        (pipeline_layout, bind_group)
    }
}

#[derive(Debug)]
pub struct TextPipeline {
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    instances: InstanceBuffer,
    /// A few glyphs drawn after the rest and replaced on their own (the hovered bar's label),
    /// so hovering does not rewrite the whole text buffer.
    overlay: InstanceBuffer,
}

impl TextPipeline {
    pub fn new(
        device: &wgpu::Device,
        atlas: &Atlas,
        format: wgpu::TextureFormat,
        samples: u32,
        globals: &wgpu::Buffer,
    ) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("text shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/text.wgsl").into()),
        });
        let (pipeline_layout, bind_group) = atlas.bind(device, "text", globals, None);
        let instance_attrs = wgpu::vertex_attr_array![
            1 => Float32x2, 2 => Float32x2, 3 => Float32x2, 4 => Float32x4, 5 => Float32x4
        ];
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("text pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[
                    Some(QUAD_LAYOUT),
                    Some(wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<GlyphInstance>() as u64,
                        step_mode: wgpu::VertexStepMode::Instance,
                        attributes: &instance_attrs,
                    }),
                ],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState {
                count: samples,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        Self {
            pipeline,
            bind_group,
            instances: InstanceBuffer::new("glyph instances"),
            overlay: InstanceBuffer::new("glyph overlay"),
        }
    }

    pub fn set_instances(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        items: &[GlyphInstance],
    ) {
        self.instances.set(device, queue, items);
    }

    /// Replaces the overlay glyphs (empty: none).
    pub fn set_overlay(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        items: &[GlyphInstance],
    ) {
        self.overlay.set(device, queue, items);
    }

    /// Draws the text, then the overlay. `overlay: false` skips the overlay (snapshots draw
    /// no hover state).
    pub fn draw<'a>(
        &'a self,
        pass: &mut wgpu::RenderPass<'a>,
        quad: &'a wgpu::Buffer,
        overlay: bool,
    ) {
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.set_vertex_buffer(0, quad.slice(..));
        let layers = [Some(&self.instances), overlay.then_some(&self.overlay)];
        for (buffer, count) in layers.into_iter().flatten().filter_map(|b| b.bound()) {
            pass.set_vertex_buffer(1, buffer.slice(..));
            pass.draw(0..4, 0..count);
        }
    }
}

/// One glyph of a world-anchored label: a [`GlyphInstance`]'s quad, relative to where the
/// label's `anchor` lands on screen, plus the azimuth range it is shown in (see `line3d.wgsl`)
/// and the slot whose entry in the label-state table decides whether it shows and where it is
/// nudged (see `labels.rs`; [`NO_SLOT`] for labels that always show).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable)]
pub struct Glyph3Instance {
    pub anchor: [f32; 3],
    pub origin: [f32; 2],
    pub axis_x: [f32; 2],
    pub axis_y: [f32; 2],
    pub uv: [f32; 4],
    pub color: [f32; 4],
    pub show: [f32; 2],
    pub slot: f32,
    /// Printed text (see `Text3::flat`): world units per pixel of the quad, which then lies on
    /// the horizontal plane through `anchor`. 0 for text that faces the viewer.
    pub flat: f32,
    /// Printed upright (see `Text3::upright`): the horizontal reading direction `[x, z]` of
    /// the vertical plane the quad stands in; zero for text printed on the floor plane.
    pub upright: [f32; 2],
}

/// The `slot` of a label that is not managed per frame.
pub const NO_SLOT: f32 = -1.0;

/// Lays out a 3D label with the 2D glyph code: the text is placed around the pixel origin
/// `(0, 0)` (shifted by `offset_px` and the vertical alignment), and the shader later moves
/// that origin to the projected anchor.
pub fn layout_glyphs_3d(
    text: &Text3,
    color: [f32; 4],
    slot: f32,
    font: &FontMetrics,
    out: &mut Vec<Glyph3Instance>,
) {
    // The 2D layout puts the baseline at y; shift it so the requested point of the text's
    // height lands on the anchor (screen y points down, so the baseline is *below* the top).
    let shift = match text.valign {
        VAlign::Top => font.cap_height * text.size_px,
        VAlign::Middle => font.cap_height * text.size_px / 2.0,
        VAlign::Baseline => 0.0,
    };
    let flat = Text {
        key: String::new(),
        text: text.text.clone(),
        x: text.offset_px[0],
        y: text.offset_px[1] + shift,
        size_px: text.size_px,
        color: text.color,
        anchor: text.align,
        rotation: Rotation::None,
    };
    let show = show_range(text.azimuths);
    let mut glyphs = Vec::new();
    layout_glyphs(&flat, color, font, &mut glyphs);
    out.extend(glyphs.into_iter().map(|g| Glyph3Instance {
        anchor: text.anchor,
        origin: g.origin,
        axis_x: g.axis_x,
        axis_y: g.axis_y,
        uv: g.uv,
        color: g.color,
        show,
        slot,
        flat: text.flat.unwrap_or(0.0),
        upright: text.upright.unwrap_or([0.0; 2]),
    }));
}

/// Copies of `glyphs` shifted `radius` pixels in eight directions and painted `color`: drawn
/// before the glyphs themselves they form an outline. (The MSDF atlas encodes too little
/// distance range for the shader to grow glyphs at this size, so we draw copies instead.)
pub fn halo_instances(
    glyphs: &[Glyph3Instance],
    color: [f32; 4],
    radius: f32,
) -> Vec<Glyph3Instance> {
    const DIRECTIONS: [[f32; 2]; 8] = [
        [1.0, 0.0],
        [-1.0, 0.0],
        [0.0, 1.0],
        [0.0, -1.0],
        [0.7, 0.7],
        [-0.7, 0.7],
        [0.7, -0.7],
        [-0.7, -0.7],
    ];
    let mut out = Vec::with_capacity(glyphs.len() * DIRECTIONS.len());
    for g in glyphs {
        for d in DIRECTIONS {
            out.push(Glyph3Instance {
                origin: [g.origin[0] + d[0] * radius, g.origin[1] + d[1] * radius],
                color,
                ..*g
            });
        }
    }
    out
}

#[derive(Debug)]
pub struct Text3dPipeline {
    pipeline: wgpu::RenderPipeline,
    /// The same shader with depth testing, for printed text (hidden behind what stands in
    /// front of the surface it is printed on).
    printed_pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    instances: InstanceBuffer,
    printed: InstanceBuffer,
    /// Like [`TextPipeline`]'s overlay: the hovered bar's label.
    overlay: InstanceBuffer,
}

impl Text3dPipeline {
    pub fn new(
        device: &wgpu::Device,
        atlas: &Atlas,
        format: wgpu::TextureFormat,
        samples: u32,
        globals: &wgpu::Buffer,
        label_states: &wgpu::Buffer,
    ) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("text3d shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/text3d.wgsl").into()),
        });
        let (layout, bind_group) = atlas.bind(device, "text3d", globals, Some(label_states));
        let instance_attrs = wgpu::vertex_attr_array![
            1 => Float32x3, 2 => Float32x2, 3 => Float32x2, 4 => Float32x2, 5 => Float32x4,
            6 => Float32x4, 7 => Float32x2, 8 => Float32, 9 => Float32, 10 => Float32x2
        ];
        let buffers = [
            Some(QUAD_LAYOUT),
            Some(wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<Glyph3Instance>() as u64,
                step_mode: wgpu::VertexStepMode::Instance,
                attributes: &instance_attrs,
            }),
        ];
        let make = |label, depth_compare| {
            pipeline(
                device,
                &PipelineDesc {
                    label,
                    module: &module,
                    layout: &layout,
                    buffers: &buffers,
                    topology: wgpu::PrimitiveTopology::TriangleStrip,
                    cull_mode: None,
                    depth_write: false,
                    depth_compare,
                    blend: wgpu::BlendState::ALPHA_BLENDING,
                },
                format,
                samples,
            )
        };
        Self {
            // Labels ignore depth: they sit outside the box and must stay readable even when a
            // bar is in the way (see `Renderer::render`).
            pipeline: make("text3d pipeline", wgpu::CompareFunction::Always),
            printed_pipeline: make("printed text pipeline", wgpu::CompareFunction::LessEqual),
            bind_group,
            instances: InstanceBuffer::new("glyph3 instances"),
            printed: InstanceBuffer::new("printed glyph instances"),
            overlay: InstanceBuffer::new("glyph3 overlay"),
        }
    }

    pub fn set_instances(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        items: &[Glyph3Instance],
    ) {
        self.instances.set(device, queue, items);
    }

    /// Replaces the overlay glyphs (empty: none).
    pub fn set_overlay(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        items: &[Glyph3Instance],
    ) {
        self.overlay.set(device, queue, items);
    }

    /// Replaces the printed glyphs (`flat` > 0).
    pub fn set_printed(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        items: &[Glyph3Instance],
    ) {
        self.printed.set(device, queue, items);
    }

    /// Draws the printed text, depth-tested against the solids drawn before it.
    pub fn draw_printed<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>, quad: &'a wgpu::Buffer) {
        let Some((buffer, count)) = self.printed.bound() else {
            return;
        };
        pass.set_pipeline(&self.printed_pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.set_vertex_buffer(0, quad.slice(..));
        pass.set_vertex_buffer(1, buffer.slice(..));
        pass.draw(0..4, 0..count);
    }

    /// Draws the text, then the overlay; `overlay: false` skips it (snapshots).
    pub fn draw<'a>(
        &'a self,
        pass: &mut wgpu::RenderPass<'a>,
        quad: &'a wgpu::Buffer,
        overlay: bool,
    ) {
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.set_vertex_buffer(0, quad.slice(..));
        let layers = [Some(&self.instances), overlay.then_some(&self.overlay)];
        for (buffer, count) in layers.into_iter().flatten().filter_map(|b| b.bound()) {
            pass.set_vertex_buffer(1, buffer.slice(..));
            pass.draw(0..4, 0..count);
        }
    }
}

#[cfg(test)]
mod tests {
    use facetry_core::Color;

    use super::*;

    fn text(s: &str, anchor: Anchor, rotation: Rotation) -> Text {
        Text {
            key: "t".into(),
            text: s.into(),
            x: 100.0,
            y: 200.0,
            size_px: 20.0,
            color: Color::linear(0.0, 0.0, 0.0, 1.0),
            anchor,
            rotation,
        }
    }

    fn glyphs(t: &Text) -> Vec<GlyphInstance> {
        let mut out = Vec::new();
        layout_glyphs(t, [0.0; 4], FontMetrics::inter(), &mut out);
        out
    }

    #[test]
    fn atlas_decodes_to_the_size_the_metrics_claim() {
        let (w, h, rgba) = decode_atlas().unwrap();
        let m = FontMetrics::inter();
        assert_eq!((w, h), (m.atlas.width, m.atlas.height));
        assert_eq!(rgba.len(), (w * h * 4) as usize);
    }

    #[test]
    fn spaces_make_no_quads() {
        assert_eq!(glyphs(&text("a b", Anchor::Start, Rotation::None)).len(), 2);
        assert!(glyphs(&text("   ", Anchor::Start, Rotation::None)).is_empty());
    }

    #[test]
    fn anchors_shift_the_run_by_its_width() {
        let width = FontMetrics::inter().text_width("Hello", 20.0);
        let first = |a| glyphs(&text("Hello", a, Rotation::None))[0].origin[0];
        let start = first(Anchor::Start);
        assert!((first(Anchor::Middle) - (start - width / 2.0)).abs() < 1e-3);
        assert!((first(Anchor::End) - (start - width)).abs() < 1e-3);
    }

    #[test]
    fn glyphs_sit_on_the_baseline() {
        // "x" has no descender: its quad's bottom (plus the small distance-field padding)
        // is at the baseline y = 200, and it extends upward.
        let g = glyphs(&text("x", Anchor::Start, Rotation::None))[0];
        let bottom = g.origin[1] + g.axis_y[1];
        assert!((bottom - 200.0).abs() < 0.2 * 20.0, "bottom {bottom}");
        assert!(g.origin[1] < 200.0);
        assert_eq!(g.axis_x[1], 0.0);
        assert!(g.axis_x[0] > 0.0 && g.axis_y[1] > 0.0);
    }

    #[test]
    fn rotated_text_reads_upward_with_tops_to_the_left() {
        let t = text("ab", Anchor::Start, Rotation::Minus90);
        let g = glyphs(&t);
        // Reading direction is up the screen: later glyphs have smaller y.
        assert!(g[1].origin[1] < g[0].origin[1]);
        // Width vector points up, height vector points right.
        assert!(g[0].axis_x[1] < 0.0 && g[0].axis_x[0] == 0.0);
        assert!(g[0].axis_y[0] > 0.0 && g[0].axis_y[1] == 0.0);
        // Glyph tops point left, so the quad's left edge is above the baseline x = 100.
        assert!(g[0].origin[0] < 100.0);
    }

    #[test]
    fn uv_rects_are_inside_the_atlas() {
        for g in glyphs(&text(
            "Revenue 0123456789 (k$)",
            Anchor::Start,
            Rotation::None,
        )) {
            assert!(g.uv.iter().all(|&v| (0.0..=1.0).contains(&v)));
            assert!(g.uv[0] < g.uv[2] && g.uv[1] < g.uv[3]);
        }
    }

    fn text3(valign: VAlign, align: Anchor) -> Text3 {
        Text3 {
            key: "t".into(),
            text: "Hx".into(),
            anchor: [1.0, 2.0, 3.0],
            offset_px: [10.0, 20.0],
            size_px: 20.0,
            color: Color::linear(0.0, 0.0, 0.0, 1.0),
            align,
            valign,
            azimuths: None,
            role: Default::default(),
            halo: true,
            flat: None,
            upright: None,
        }
    }

    fn glyphs3(t: &Text3) -> Vec<Glyph3Instance> {
        let mut out = Vec::new();
        layout_glyphs_3d(t, [0.0; 4], NO_SLOT, FontMetrics::inter(), &mut out);
        out
    }

    #[test]
    fn world_labels_share_one_anchor_and_hang_around_the_offset() {
        let g = glyphs3(&text3(VAlign::Baseline, Anchor::Start));
        assert_eq!(g.len(), 2);
        assert!(g.iter().all(|g| g.anchor == [1.0, 2.0, 3.0]));
        // Start-aligned: the first glyph begins just right of the x offset.
        assert!((g[0].origin[0] - 10.0).abs() < 0.2 * 20.0);
        // Baseline-aligned: the glyph's bottom is at the y offset.
        let bottom = g[0].origin[1] + g[0].axis_y[1];
        assert!((bottom - 20.0).abs() < 0.2 * 20.0, "{bottom}");
    }

    #[test]
    fn vertical_alignment_moves_the_baseline_by_the_cap_height() {
        let cap = FontMetrics::inter().cap_height * 20.0;
        let y = |v| glyphs3(&text3(v, Anchor::Start))[0].origin[1];
        assert!((y(VAlign::Top) - y(VAlign::Baseline) - cap).abs() < 1e-3);
        assert!((y(VAlign::Middle) - y(VAlign::Baseline) - cap / 2.0).abs() < 1e-3);
    }

    #[test]
    fn halos_are_painted_copies_around_each_glyph() {
        let g = glyphs3(&text3(VAlign::Middle, Anchor::Middle));
        let halo = halo_instances(&g, [1.0; 4], 2.0);
        assert_eq!(halo.len(), g.len() * 8);
        assert!(halo.iter().all(|h| h.color == [1.0; 4]));
        // Every copy is within the radius of the glyph it came from.
        for (i, h) in halo.iter().enumerate() {
            let src = g[i / 8];
            let d = ((h.origin[0] - src.origin[0]).powi(2) + (h.origin[1] - src.origin[1]).powi(2))
                .sqrt();
            assert!(d > 0.0 && d <= 2.0 + 1e-3, "{d}");
            assert_eq!((h.anchor, h.axis_x, h.uv), (src.anchor, src.axis_x, src.uv));
        }
    }

    #[test]
    fn world_labels_default_to_every_azimuth() {
        let g = glyphs3(&text3(VAlign::Middle, Anchor::Middle))[0];
        assert_eq!(g.show, [0.0, 360.0]);
    }
}
