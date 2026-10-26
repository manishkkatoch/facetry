//! Instanced rectangles: bars, gridlines, axis lines and ticks.

use bytemuck::{Pod, Zeroable};
use facetry_core::scene::{RuleKind, Scene};
use facetry_core::transition::Plan;

use crate::gpu::{InstanceBuffer, QUAD_LAYOUT, with_ease};

/// One rectangle: `[x, y, width, height]` in device pixels, an RGBA colour, and a hover flag
/// (1 brightens the rectangle; it is rewritten alone when the pointer moves between bars).
///
/// For animation it also carries where it starts (`from_*`) and how long after the transition
/// begins it starts moving; the vertex shader mixes `from` into `rect`/`color` by the eased
/// progress. A rectangle at rest has `from == to`.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable)]
pub struct RectInstance {
    pub rect: [f32; 4],
    pub color: [f32; 4],
    pub hover: f32,
    pub from_rect: [f32; 4],
    pub from_color: [f32; 4],
    pub delay: f32,
}

impl RectInstance {
    /// A rectangle that stays where it is.
    pub fn still(b: facetry_core::scene::Bounds, color: [f32; 4]) -> Self {
        let rect = [b.x, b.y, b.w, b.h];
        Self {
            rect,
            color,
            hover: 0.0,
            from_rect: rect,
            from_color: color,
            delay: 0.0,
        }
    }

    /// A rectangle that moves from `from` to `to`, starting `delay_ms` into the transition.
    pub fn moving(
        from: (facetry_core::scene::Bounds, [f32; 4]),
        to: (facetry_core::scene::Bounds, [f32; 4]),
        delay_ms: f32,
    ) -> Self {
        let mut r = Self::still(to.0, to.1);
        r.from_rect = [from.0.x, from.0.y, from.0.w, from.0.h];
        r.from_color = from.1;
        r.delay = delay_ms;
        r
    }
}

/// Byte offset of [`RectInstance::hover`] inside an instance.
pub const HOVER_OFFSET: usize = std::mem::offset_of!(RectInstance, hover);

/// Everything that is a box, back to front. `encode` turns a scene colour into what the
/// render target expects (see `Renderer::encode_color`).
pub fn instances(
    scene: &Scene,
    encode: impl Fn(facetry_core::Color) -> [f32; 4],
) -> Vec<RectInstance> {
    from_boxes(scene.boxes_in_draw_order(), encode)
}

/// Rectangle instances for any list of `(bounds, colour)` (a 3D chart's legend swatches).
pub fn from_boxes(
    boxes: impl Iterator<Item = (facetry_core::scene::Bounds, facetry_core::Color)>,
    encode: impl Fn(facetry_core::Color) -> [f32; 4],
) -> Vec<RectInstance> {
    boxes
        .map(|(b, color)| RectInstance::still(b, encode(color)))
        .collect()
}

/// The rectangles of a transition, back to front like [`instances`]: gridlines, bars (in the
/// plan's order, where bar `i` of the target is `plan.rect_slot[i]`), then the other rules.
///
/// Returns the instances and the number of gridlines and of bars among them.
pub fn pair_instances(
    plan: &Plan,
    encode: impl Fn(facetry_core::Color) -> [f32; 4],
) -> (Vec<RectInstance>, usize, usize) {
    let rule = |p: &facetry_core::transition::Pair<facetry_core::scene::Rule>| {
        RectInstance::moving(
            (p.from.bounds, encode(p.from.color)),
            (p.to.bounds, encode(p.to.color)),
            p.delay_ms,
        )
    };
    let grid = |p: &&facetry_core::transition::Pair<facetry_core::scene::Rule>| {
        p.to.kind == RuleKind::Grid
    };
    let mut out: Vec<RectInstance> = plan.rules.iter().filter(grid).map(rule).collect();
    let grids = out.len();
    out.extend(plan.rects.iter().map(|p| {
        RectInstance::moving(
            (p.from.bounds, encode(p.from.color)),
            (p.to.bounds, encode(p.to.color)),
            p.delay_ms,
        )
    }));
    let bars = plan.rects.len();
    out.extend(plan.rules.iter().filter(|p| !grid(p)).map(rule));
    (out, grids, bars)
}

#[derive(Debug)]
pub struct RectPipeline {
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    instances: InstanceBuffer,
}

impl RectPipeline {
    pub fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        samples: u32,
        globals: &wgpu::Buffer,
    ) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("rect shader"),
            source: wgpu::ShaderSource::Wgsl(with_ease(include_str!("shaders/rect.wgsl")).into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("rect bind group layout"),
            entries: &[uniform_entry(0)],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("rect bind group"),
            layout: &layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: globals.as_entire_binding(),
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("rect pipeline layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let instance_attrs = wgpu::vertex_attr_array![
            1 => Float32x4, 2 => Float32x4, 3 => Float32, 4 => Float32x4, 5 => Float32x4, 6 => Float32
        ];
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("rect pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[
                    Some(QUAD_LAYOUT),
                    Some(wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<RectInstance>() as u64,
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
            instances: InstanceBuffer::new("rect instances"),
        }
    }

    pub fn set_instances(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        items: &[RectInstance],
    ) {
        self.instances.set(device, queue, items);
    }

    /// Sets the hover flag of instance `index` (an index into the slice given to
    /// `set_instances`).
    pub fn set_hover(&self, queue: &wgpu::Queue, index: usize, on: bool) {
        let offset = index * std::mem::size_of::<RectInstance>() + HOVER_OFFSET;
        self.instances.patch(
            queue,
            offset as u64,
            bytemuck::bytes_of(&if on { 1.0_f32 } else { 0.0 }),
        );
    }

    /// Draws instances `range` (clamped to what exists): the 2D pass draws the rectangles in
    /// two parts, with the line layers between them.
    pub fn draw_range<'a>(
        &'a self,
        pass: &mut wgpu::RenderPass<'a>,
        quad: &'a wgpu::Buffer,
        range: std::ops::Range<u32>,
    ) {
        let Some((buffer, count)) = self.instances.bound() else {
            return;
        };
        let range = range.start.min(count)..range.end.min(count);
        if range.is_empty() {
            return;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.set_vertex_buffer(0, quad.slice(..));
        pass.set_vertex_buffer(1, buffer.slice(..));
        pass.draw(0..4, range);
    }
}

/// A uniform buffer binding visible to both shader stages.
pub fn uniform_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}
