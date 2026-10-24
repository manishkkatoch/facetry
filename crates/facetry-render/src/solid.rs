//! The 3D pipelines for geometry: instanced boxes (bars) and thick world-space lines.

use std::ops::Range;

use bytemuck::{Pod, Zeroable};
use facetry_core::Color;
use facetry_core::scene3d::Scene3d;

use crate::arc::Vertex3;
use crate::gpu::{DEPTH_FORMAT, InstanceBuffer, QUAD_LAYOUT, show_range};
use crate::rect::uniform_entry;

/// One bar: opposite corners, a colour and a hover flag (1 lightens it; rewritten alone when
/// the pointer moves between bars).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable)]
pub struct BoxInstance {
    pub min: [f32; 3],
    pub max: [f32; 3],
    pub color: [f32; 4],
    pub hover: f32,
}

/// Byte offset of [`BoxInstance::hover`] inside an instance.
pub const BOX_HOVER_OFFSET: usize = std::mem::offset_of!(BoxInstance, hover);

/// One line: endpoints, width in pixels, colour, and the azimuth range it is drawn in.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable)]
pub struct LineInstance {
    pub a: [f32; 3],
    pub width_px: f32,
    pub b: [f32; 3],
    pub _pad: f32,
    pub color: [f32; 4],
    pub show: [f32; 2],
}

pub fn box_instances(scene: &Scene3d, encode: impl Fn(Color) -> [f32; 4]) -> Vec<BoxInstance> {
    scene
        .boxes
        .iter()
        .map(|b| BoxInstance {
            min: b.min,
            max: b.max,
            color: encode(b.color),
            hover: 0.0,
        })
        .collect()
}

/// Grid first, then axis lines and ticks, as the scene lists them. (Order hardly matters:
/// lines are depth-tested against the bars and never overlap each other.)
pub fn line_instances(scene: &Scene3d, encode: impl Fn(Color) -> [f32; 4]) -> Vec<LineInstance> {
    scene
        .lines
        .iter()
        .map(|l| LineInstance {
            a: l.a,
            width_px: l.width_px,
            b: l.b,
            _pad: 0.0,
            color: encode(l.color),
            show: show_range(l.azimuths),
        })
        .collect()
}

/// A vertex of the shared unit cube.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable)]
struct CubeVertex {
    corner: [f32; 3],
    normal: [f32; 3],
}

/// The six faces as `(normal, origin, u, v)` in unit-cube coordinates. `u x v == normal`, so
/// walking origin -> origin+u -> origin+u+v goes counter-clockwise seen from outside, which
/// is what the pipeline treats as the front.
type Vec3 = [f32; 3];
const FACES: [(Vec3, Vec3, Vec3, Vec3); 6] = [
    (
        [1.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [0.0, 1.0, 0.0],
        [0.0, 0.0, 1.0],
    ),
    (
        [-1.0, 0.0, 0.0],
        [0.0, 0.0, 0.0],
        [0.0, 0.0, 1.0],
        [0.0, 1.0, 0.0],
    ),
    (
        [0.0, 1.0, 0.0],
        [0.0, 1.0, 0.0],
        [0.0, 0.0, 1.0],
        [1.0, 0.0, 0.0],
    ),
    (
        [0.0, -1.0, 0.0],
        [0.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0],
    ),
    (
        [0.0, 0.0, 1.0],
        [0.0, 0.0, 1.0],
        [1.0, 0.0, 0.0],
        [0.0, 1.0, 0.0],
    ),
    (
        [0.0, 0.0, -1.0],
        [0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0],
        [1.0, 0.0, 0.0],
    ),
];

/// 36 vertices: 6 faces x 2 triangles. Not indexed, because a vertex shared between faces
/// would need one normal per face anyway ("flat shading").
fn cube() -> Vec<CubeVertex> {
    let add = |a: [f32; 3], b: [f32; 3]| [a[0] + b[0], a[1] + b[1], a[2] + b[2]];
    let mut out = Vec::with_capacity(36);
    for (normal, origin, u, v) in FACES {
        let p = [
            origin,
            add(origin, u),
            add(add(origin, u), v),
            add(origin, v),
        ];
        for i in [0, 1, 2, 0, 2, 3] {
            out.push(CubeVertex {
                corner: p[i],
                normal,
            });
        }
    }
    out
}

/// What differs between the 3D pipelines; `pipeline` fills in the rest.
#[derive(Debug)]
pub struct PipelineDesc<'a> {
    pub label: &'a str,
    pub module: &'a wgpu::ShaderModule,
    pub layout: &'a wgpu::PipelineLayout,
    pub buffers: &'a [Option<wgpu::VertexBufferLayout<'a>>],
    pub topology: wgpu::PrimitiveTopology,
    pub cull_mode: Option<wgpu::Face>,
    pub depth_write: bool,
    pub depth_compare: wgpu::CompareFunction,
    pub blend: wgpu::BlendState,
}

pub fn pipeline(
    device: &wgpu::Device,
    desc: &PipelineDesc,
    format: wgpu::TextureFormat,
    samples: u32,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(desc.label),
        layout: Some(desc.layout),
        vertex: wgpu::VertexState {
            module: desc.module,
            entry_point: Some("vs_main"),
            compilation_options: Default::default(),
            buffers: desc.buffers,
        },
        primitive: wgpu::PrimitiveState {
            topology: desc.topology,
            cull_mode: desc.cull_mode,
            ..Default::default()
        },
        depth_stencil: Some(depth_state(desc.depth_write, desc.depth_compare)),
        multisample: wgpu::MultisampleState {
            count: samples,
            mask: !0,
            alpha_to_coverage_enabled: false,
        },
        fragment: Some(wgpu::FragmentState {
            module: desc.module,
            entry_point: Some("fs_main"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: Some(desc.blend),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

/// Every pipeline drawn in the 3D pass must declare the pass's depth format, even the ones
/// (text) that ignore depth: they pass `(false, Always)`.
pub fn depth_state(write: bool, compare: wgpu::CompareFunction) -> wgpu::DepthStencilState {
    wgpu::DepthStencilState {
        format: DEPTH_FORMAT,
        depth_write_enabled: Some(write),
        depth_compare: Some(compare),
        stencil: Default::default(),
        bias: Default::default(),
    }
}

#[derive(Debug)]
pub struct BoxPipeline {
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    cube: wgpu::Buffer,
    instances: InstanceBuffer,
}

impl BoxPipeline {
    pub fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        samples: u32,
        globals: &wgpu::Buffer,
    ) -> Self {
        use wgpu::util::DeviceExt as _;
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("box shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/box.wgsl").into()),
        });
        let (layout, bind_group) = globals_only(device, "box", globals);
        let cube = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("unit cube"),
            contents: bytemuck::cast_slice(&cube()),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let cube_attrs = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3];
        let instance_attrs =
            wgpu::vertex_attr_array![2 => Float32x3, 3 => Float32x3, 4 => Float32x4, 5 => Float32];
        let pipeline = pipeline(
            device,
            &PipelineDesc {
                label: "box pipeline",
                module: &module,
                layout: &layout,
                buffers: &[
                    Some(wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<CubeVertex>() as u64,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &cube_attrs,
                    }),
                    Some(wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<BoxInstance>() as u64,
                        step_mode: wgpu::VertexStepMode::Instance,
                        attributes: &instance_attrs,
                    }),
                ],
                topology: wgpu::PrimitiveTopology::TriangleList,
                // Faces turned away from the camera are never seen (the box is closed), so
                // skipping them halves the fragment work; the depth test would hide them
                // anyway.
                cull_mode: Some(wgpu::Face::Back),
                depth_write: true,
                depth_compare: wgpu::CompareFunction::Less,
                blend: wgpu::BlendState::REPLACE,
            },
            format,
            samples,
        );
        Self {
            pipeline,
            bind_group,
            cube,
            instances: InstanceBuffer::new("box instances"),
        }
    }

    pub fn set_instances(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        items: &[BoxInstance],
    ) {
        self.instances.set(device, queue, items);
    }

    /// Sets the hover flag of instance `index` (an index into the slice given to
    /// `set_instances`).
    pub fn set_hover(&self, queue: &wgpu::Queue, index: usize, on: bool) {
        let offset = index * std::mem::size_of::<BoxInstance>() + BOX_HOVER_OFFSET;
        self.instances.patch(
            queue,
            offset as u64,
            bytemuck::bytes_of(&if on { 1.0_f32 } else { 0.0 }),
        );
    }

    pub fn draw<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>) {
        let Some((buffer, count)) = self.instances.bound() else {
            return;
        };
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.set_vertex_buffer(0, self.cube.slice(..));
        pass.set_vertex_buffer(1, buffer.slice(..));
        pass.draw(0..36, 0..count);
    }
}

/// The slices of an extruded pie: one lit mesh, drawn with depth testing like the bars. The
/// vertices are kept on the CPU too, so hovering a slice can rewrite just its range.
#[derive(Debug)]
pub struct SlicePipeline {
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    vertices: Vec<Vertex3>,
    /// Each slice's vertices within `vertices`, in slice order.
    ranges: Vec<Range<usize>>,
    buffer: InstanceBuffer,
}

impl SlicePipeline {
    pub fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        samples: u32,
        globals: &wgpu::Buffer,
    ) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("mesh3d shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/mesh3d.wgsl").into()),
        });
        let (layout, bind_group) = globals_only(device, "mesh3d", globals);
        let attrs = wgpu::vertex_attr_array![
            0 => Float32x3, 1 => Float32x3, 2 => Float32x4, 3 => Float32
        ];
        let pipeline = pipeline(
            device,
            &PipelineDesc {
                label: "mesh3d pipeline",
                module: &module,
                layout: &layout,
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Vertex3>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &attrs,
                })],
                topology: wgpu::PrimitiveTopology::TriangleList,
                // Both sides are drawn (the depth test hides what is behind): a slice is a
                // closed solid except for its floor, which is never seen.
                cull_mode: None,
                depth_write: true,
                depth_compare: wgpu::CompareFunction::Less,
                blend: wgpu::BlendState::REPLACE,
            },
            format,
            samples,
        );
        Self {
            pipeline,
            bind_group,
            vertices: Vec::new(),
            ranges: Vec::new(),
            buffer: InstanceBuffer::new("slice vertices"),
        }
    }

    pub fn set(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        scene: &Scene3d,
        encode: impl Fn(Color) -> [f32; 4],
    ) {
        self.vertices.clear();
        self.ranges.clear();
        for s in &scene.slices {
            let start = self.vertices.len();
            crate::arc::slice(s, encode(s.color), &mut self.vertices);
            self.ranges.push(start..self.vertices.len());
        }
        self.buffer.set(device, queue, &self.vertices);
    }

    /// Lights slice `index` up (or not): rewrites the hover flag of its vertices only.
    pub fn set_hover(&mut self, queue: &wgpu::Queue, index: usize, on: bool) {
        let Some(range) = self.ranges.get(index).cloned() else {
            return;
        };
        for v in &mut self.vertices[range.clone()] {
            v.hover = if on { 1.0 } else { 0.0 };
        }
        let stride = std::mem::size_of::<Vertex3>();
        self.buffer.patch(
            queue,
            (range.start * stride) as u64,
            bytemuck::cast_slice(&self.vertices[range]),
        );
    }

    pub fn draw<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>) {
        let Some((buffer, count)) = self.buffer.bound() else {
            return;
        };
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.set_vertex_buffer(0, buffer.slice(..));
        pass.draw(0..count, 0..1);
    }
}

#[derive(Debug)]
pub struct LinePipeline {
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    instances: InstanceBuffer,
}

impl LinePipeline {
    pub fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        samples: u32,
        globals: &wgpu::Buffer,
    ) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("line shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/line3d.wgsl").into()),
        });
        let (layout, bind_group) = globals_only(device, "line", globals);
        let instance_attrs = wgpu::vertex_attr_array![
            1 => Float32x4, 2 => Float32x4, 3 => Float32x4, 4 => Float32x2
        ];
        let pipeline = pipeline(
            device,
            &PipelineDesc {
                label: "line pipeline",
                module: &module,
                layout: &layout,
                buffers: &[
                    Some(QUAD_LAYOUT),
                    Some(wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<LineInstance>() as u64,
                        step_mode: wgpu::VertexStepMode::Instance,
                        attributes: &instance_attrs,
                    }),
                ],
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                cull_mode: None,
                // Tested against the bars (a line behind a bar is hidden) but not written,
                // so lines never hide each other or the bars.
                depth_write: false,
                depth_compare: wgpu::CompareFunction::LessEqual,
                blend: wgpu::BlendState::ALPHA_BLENDING,
            },
            format,
            samples,
        );
        Self {
            pipeline,
            bind_group,
            instances: InstanceBuffer::new("line instances"),
        }
    }

    pub fn set_instances(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        items: &[LineInstance],
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

/// A bind group layout with just the globals uniform, and the group built from it.
pub fn globals_only(
    device: &wgpu::Device,
    name: &str,
    globals: &wgpu::Buffer,
) -> (wgpu::PipelineLayout, wgpu::BindGroup) {
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some(&format!("{name} bind group layout")),
        entries: &[uniform_entry(0)],
    });
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some(&format!("{name} bind group")),
        layout: &layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: globals.as_entire_binding(),
        }],
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some(&format!("{name} pipeline layout")),
        bind_group_layouts: &[Some(&layout)],
        immediate_size: 0,
    });
    (pipeline_layout, bind_group)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
        [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
    }

    fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
        [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ]
    }

    #[test]
    fn every_cube_triangle_winds_counter_clockwise_seen_from_outside() {
        let vertices = cube();
        assert_eq!(vertices.len(), 36);
        for tri in vertices.chunks(3) {
            let n = cross(
                sub(tri[1].corner, tri[0].corner),
                sub(tri[2].corner, tri[0].corner),
            );
            // Same direction as the declared normal, and the same for all three corners.
            let dot = n[0] * tri[0].normal[0] + n[1] * tri[0].normal[1] + n[2] * tri[0].normal[2];
            assert!(dot > 0.0, "{tri:?} faces inward");
            assert!(tri.iter().all(|v| v.normal == tri[0].normal));
        }
    }

    #[test]
    fn each_face_lies_on_the_cube_surface() {
        for v in cube() {
            let axis = v.normal.iter().position(|&n| n != 0.0).unwrap();
            let expected = if v.normal[axis] > 0.0 { 1.0 } else { 0.0 };
            assert_eq!(v.corner[axis], expected, "{v:?}");
        }
    }

    #[test]
    fn instance_structs_match_their_vertex_layouts() {
        assert_eq!(std::mem::size_of::<BoxInstance>(), 44);
        assert_eq!(BOX_HOVER_OFFSET, 40);
        assert_eq!(std::mem::size_of::<LineInstance>(), 56);
    }
}
