//! MSDF text: turning strings into glyph quads (pure, unit-tested) and drawing them.

use bytemuck::{Pod, Zeroable};
use facetry_core::scene::{Anchor, Rotation, Text};
use facetry_font::FontMetrics;

use crate::Error;
use crate::gpu::{InstanceBuffer, QUAD_LAYOUT};
use crate::rect::uniform_entry;

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

/// Decodes the embedded atlas PNG (8-bit RGB) to RGBA8 for upload; WebGPU has no RGB8 format.
fn decode_atlas() -> Result<(u32, u32, Vec<u8>), Error> {
    let fail = |e: &dyn std::fmt::Display| Error::RenderFailed(format!("decoding font atlas: {e}"));
    let mut reader = png::Decoder::new(std::io::Cursor::new(facetry_font::ATLAS_PNG))
        .read_info()
        .map_err(|e| fail(&e))?;
    let mut buf = vec![
        0;
        reader
            .output_buffer_size()
            .ok_or_else(|| fail(&"too large"))?
    ];
    let info = reader.next_frame(&mut buf).map_err(|e| fail(&e))?;
    if info.color_type != png::ColorType::Rgb || info.bit_depth != png::BitDepth::Eight {
        return Err(fail(&"expected an 8-bit RGB PNG"));
    }
    let rgba = buf[..info.buffer_size()]
        .chunks_exact(3)
        .flat_map(|p| [p[0], p[1], p[2], 255])
        .collect();
    Ok((info.width, info.height, rgba))
}

#[derive(Debug)]
pub struct TextPipeline {
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    instances: InstanceBuffer,
}

impl TextPipeline {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
        samples: u32,
        globals: &wgpu::Buffer,
    ) -> Result<Self, Error> {
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

        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("text shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/text.wgsl").into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("text bind group layout"),
            entries: &[
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
            ],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("text bind group"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: globals.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("text pipeline layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
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
        Ok(Self {
            pipeline,
            bind_group,
            instances: InstanceBuffer::new("glyph instances"),
        })
    }

    pub fn set_instances(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        items: &[GlyphInstance],
    ) {
        self.instances.set(device, queue, items);
    }

    pub fn draw<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>, quad: &'a wgpu::Buffer) {
        let Some((buffer, count)) = self.instances.bound() else {
            return;
        };
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.set_vertex_buffer(0, quad.slice(..));
        pass.set_vertex_buffer(1, buffer.slice(..));
        pass.draw(0..4, 0..count);
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
}
