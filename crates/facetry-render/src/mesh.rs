//! Plain triangles in pixel space: a line layer's tessellated strokes, its point markers and
//! (later) pie slices. The geometry is built on the CPU (`stroke.rs`), uploaded as one vertex
//! buffer per kind and drawn with a single trivial pipeline.

use std::ops::Range;

use bytemuck::{Pod, Zeroable};
use facetry_core::Color;
use facetry_core::scene::{PointMark, Scene};

use crate::gpu::InstanceBuffer;
use crate::rect::uniform_entry;
use crate::stroke;

/// One vertex: device-pixel position and RGBA colour (already encoded for the target).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable)]
pub struct MeshVertex {
    pub pos: [f32; 2],
    pub color: [f32; 4],
}

/// Triangles with a colour each: `push_triangles` takes bare positions (three per triangle,
/// as `stroke` produces them) and paints them.
#[derive(Debug, Default)]
pub struct MeshData {
    pub vertices: Vec<MeshVertex>,
    /// Draw calls: ranges of `vertices`, in order.
    pub ranges: Vec<Range<u32>>,
}

impl MeshData {
    fn push_triangles(&mut self, positions: &[[f32; 2]], color: [f32; 4]) {
        self.vertices
            .extend(positions.iter().map(|&pos| MeshVertex { pos, color }));
    }

    /// Ends the current draw call (if it has any vertices).
    fn close_range(&mut self, start: usize) {
        if self.vertices.len() > start {
            self.ranges.push(start as u32..self.vertices.len() as u32);
        }
    }
}

/// The strokes of every line layer: one draw call (range) per layer, in layer order.
pub fn line_mesh(scene: &Scene, encode: impl Fn(Color) -> [f32; 4]) -> MeshData {
    let mut mesh = MeshData::default();
    let mut scratch = Vec::new();
    let mut start = 0;
    let mut layer = None;
    for line in &scene.polylines {
        if layer.is_some_and(|l| l != line.layer) {
            mesh.close_range(start);
            start = mesh.vertices.len();
        }
        layer = Some(line.layer);
        for run in &line.runs {
            scratch.clear();
            stroke::polyline(run, line.width, stroke::MITER_LIMIT, &mut scratch);
            mesh.push_triangles(&scratch, encode(line.color));
        }
    }
    mesh.close_range(start);
    mesh
}

/// A marker: a disc in the background colour with a smaller one in the point's colour on
/// top, which reads as a dot with a clean rim over the line and the bars.
pub fn push_marker(mesh: &mut MeshData, p: &PointMark, scale: f32, rim: [f32; 4], color: [f32; 4]) {
    let radius = p.size / 2.0 * scale;
    let ring = (radius * 0.22).max(1.0);
    let mut tris = Vec::new();
    stroke::disc([p.x, p.y], radius, &mut tris);
    mesh.push_triangles(&tris, rim);
    tris.clear();
    stroke::disc([p.x, p.y], (radius - ring).max(radius * 0.4), &mut tris);
    mesh.push_triangles(&tris, color);
}

/// The markers of the layers that asked for them, as one draw call.
pub fn point_mesh(
    scene: &Scene,
    encode: impl Fn(Color) -> [f32; 4],
    background: [f32; 4],
) -> MeshData {
    let mut mesh = MeshData::default();
    for p in scene.points.iter().filter(|p| p.visible) {
        push_marker(&mut mesh, p, 1.0, background, encode(p.color));
    }
    mesh.close_range(0);
    mesh
}

#[derive(Debug)]
pub struct MeshPipeline {
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
}

impl MeshPipeline {
    pub fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        samples: u32,
        globals: &wgpu::Buffer,
    ) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("mesh2d shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/mesh2d.wgsl").into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("mesh2d bind group layout"),
            entries: &[uniform_entry(0)],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("mesh2d bind group"),
            layout: &layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: globals.as_entire_binding(),
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("mesh2d pipeline layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let attrs = wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x4];
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("mesh2d pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<MeshVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &attrs,
                })],
            },
            // No culling: strokes and slices are flat and may wind either way.
            primitive: wgpu::PrimitiveState::default(),
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
        }
    }
}

/// A vertex buffer of triangles and the draw calls (ranges) that split it.
#[derive(Debug)]
pub struct Mesh {
    vertices: InstanceBuffer,
    ranges: Vec<Range<u32>>,
}

impl Mesh {
    pub fn new(label: &'static str) -> Self {
        Self {
            vertices: InstanceBuffer::new(label),
            ranges: Vec::new(),
        }
    }

    pub fn set(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, data: &MeshData) {
        self.vertices.set(device, queue, &data.vertices);
        self.ranges.clone_from(&data.ranges);
    }

    /// One `draw` per range.
    pub fn draw<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>, pipeline: &'a MeshPipeline) {
        let Some((buffer, count)) = self.vertices.bound() else {
            return;
        };
        pass.set_pipeline(&pipeline.pipeline);
        pass.set_bind_group(0, &pipeline.bind_group, &[]);
        pass.set_vertex_buffer(0, buffer.slice(..));
        for range in &self.ranges {
            pass.draw(range.start..range.end.min(count), 0..1);
        }
    }
}

#[cfg(test)]
mod tests {
    use facetry_core::{build_scene, parse_spec_json};
    use facetry_font::FontMetrics;

    use super::*;

    fn scene(name: &str) -> Scene {
        let json = std::fs::read_to_string(format!(
            "{}/../../examples/specs/{name}.json",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap();
        let spec = parse_spec_json(&json).unwrap();
        build_scene(&spec, (800, 500), 1.0, FontMetrics::inter())
    }

    fn identity(c: Color) -> [f32; 4] {
        c.to_array()
    }

    #[test]
    fn one_draw_call_per_line_layer() {
        let basic = scene("line-basic"); // one layer, two series
        let mesh = line_mesh(&basic, identity);
        assert_eq!(mesh.ranges.len(), 1);
        assert_eq!(mesh.ranges[0].end as usize, mesh.vertices.len());
        assert_eq!(mesh.vertices.len() % 3, 0);
        // The combo chart has bars and one line layer: still one range.
        assert_eq!(
            line_mesh(&scene("combo-bar-line"), identity).ranges.len(),
            1
        );
        // No lines, no draw calls.
        assert!(line_mesh(&scene("bar-basic"), identity).ranges.is_empty());
    }

    #[test]
    fn only_layers_with_markers_get_them() {
        let with = point_mesh(&scene("line-basic"), identity, [1.0; 4]);
        assert!(!with.vertices.is_empty());
        assert_eq!(with.vertices.len() % 3, 0);
        // line-numeric-x has no `point` option: its points exist for hover but are not drawn.
        let without = point_mesh(&scene("line-numeric-x"), identity, [1.0; 4]);
        assert!(without.vertices.is_empty() && without.ranges.is_empty());
    }
}
