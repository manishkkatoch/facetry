//! The 3D pipelines for geometry: instanced boxes (bars) and thick world-space lines.

use std::ops::Range;

use bytemuck::{Pod, Zeroable};
use facetry_core::Color;
use facetry_core::scene3d::Scene3d;

use crate::arc::Vertex3;
use crate::gpu::{DEPTH_FORMAT, InstanceBuffer, QUAD_LAYOUT, show_range, with_ease};
use crate::rect::uniform_entry;

/// One bar: opposite corners, a colour and a hover flag (1 lightens it; rewritten alone when
/// the pointer moves between bars). For animation it also carries where it starts (`from_*`)
/// and how long after the transition begins it starts moving; the vertex shader mixes the two.
/// A bar at rest has `from == to`.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable)]
pub struct BoxInstance {
    pub min: [f32; 3],
    pub max: [f32; 3],
    pub color: [f32; 4],
    pub hover: f32,
    pub from_min: [f32; 3],
    pub from_max: [f32; 3],
    pub from_color: [f32; 4],
    pub delay: f32,
}

impl BoxInstance {
    /// A bar that stays where it is.
    pub fn still(min: [f32; 3], max: [f32; 3], color: [f32; 4]) -> Self {
        Self {
            min,
            max,
            color,
            hover: 0.0,
            from_min: min,
            from_max: max,
            from_color: color,
            delay: 0.0,
        }
    }
}

/// The bars of a transition, new scene's first (so instance `i` is bar `i` of the target, which
/// hover relies on), then the ones that are leaving.
pub fn box_pair_instances(
    plan: &facetry_core::transition3d::Plan3,
    encode: impl Fn(Color) -> [f32; 4],
) -> Vec<BoxInstance> {
    plan.boxes
        .iter()
        .map(|p| BoxInstance {
            from_min: p.from.min,
            from_max: p.from.max,
            from_color: encode(p.from.color),
            delay: p.delay_ms,
            ..BoxInstance::still(p.to.min, p.to.max, encode(p.to.color))
        })
        .collect()
}

/// Byte offset of [`BoxInstance::hover`] inside an instance.
pub const BOX_HOVER_OFFSET: usize = std::mem::offset_of!(BoxInstance, hover);

/// One line: endpoints, width in pixels, colour, and the azimuth range it is drawn in.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable)]
pub struct LineInstance {
    pub a: [f32; 3],
    pub width_px: f32,
    /// `1.0`: the ends stick out half a width (square caps), so grid lines that meet at a
    /// corner leave no notch. `0.0`: flat ends, for data lines whose joins are round discs.
    pub b: [f32; 3],
    pub cap: f32,
    pub color: [f32; 4],
    pub show: [f32; 2],
}

pub fn box_instances(scene: &Scene3d, encode: impl Fn(Color) -> [f32; 4]) -> Vec<BoxInstance> {
    scene
        .boxes
        .iter()
        .map(|b| BoxInstance::still(b.min, b.max, encode(b.color)))
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
            cap: 1.0,
            color: encode(l.color),
            show: show_range(l.azimuths),
        })
        .collect()
}

/// Every segment of every run of every 3D polyline (a line layer's series), with flat ends:
/// the joins are the discs of [`join_instances`], which round every corner and end alike.
pub fn polyline_instances(
    scene: &Scene3d,
    encode: impl Fn(Color) -> [f32; 4],
) -> Vec<LineInstance> {
    let mut out = Vec::new();
    for line in &scene.polylines {
        let color = encode(line.color);
        for run in &line.runs {
            for pair in run.windows(2) {
                out.push(LineInstance {
                    a: pair[0],
                    width_px: line.width_px,
                    b: pair[1],
                    cap: 0.0,
                    color,
                    show: show_range(None),
                });
            }
        }
    }
    out
}

/// One billboard disc as wide as the line at every vertex of every run. Two thick segments
/// that meet at an angle leave a wedge-shaped gap on the outside of the bend (square caps only
/// fill it exactly at 90 degrees); a disc of the line's own width centred on the vertex covers
/// that wedge at any angle, in screen space like the segments, and also gives the line round
/// ends and a run of one point its dot. Same opaque colour, so overlaps do not show.
pub fn join_instances(scene: &Scene3d, encode: impl Fn(Color) -> [f32; 4]) -> Vec<PointInstance> {
    let mut out = Vec::new();
    for line in &scene.polylines {
        let color = encode(line.color);
        for p in line.runs.iter().flatten() {
            out.push(PointInstance::solid(*p, line.width_px, color));
        }
    }
    out
}

/// How much larger than its normal marker a hovered line point is drawn (the 2D value).
pub const HOVER_MARKER_SCALE: f32 = 1.5;

/// A marker's fill diameter inside its rim of background colour, for a marker `size` pixels
/// across: the rim is a fifth of the radius (at least a pixel) thick, as in 2D.
fn marker_inner(size: f32) -> f32 {
    let radius = size / 2.0;
    let ring = (radius * 0.22).max(1.0);
    2.0 * (radius - ring).max(radius * 0.4)
}

/// A disc at `size` (0 hides it) for a hovered or resting marker.
pub fn marker_sizes(size_px: f32, visible: bool, hovered: bool) -> (f32, f32) {
    let size = match (hovered, visible) {
        (true, _) => size_px * HOVER_MARKER_SCALE,
        (false, true) => size_px,
        (false, false) => 0.0,
    };
    (size, marker_inner(size))
}

/// One marker per data point, in the scene's order (so a mark index maps to an instance). A
/// point whose layer has no `point` markers has size 0 until it is hovered.
pub fn marker_instances(
    scene: &Scene3d,
    encode: impl Fn(Color) -> [f32; 4],
    rim: [f32; 4],
) -> Vec<PointInstance> {
    scene
        .points
        .iter()
        .map(|p| {
            let (size, inner) = marker_sizes(p.size_px, p.visible, false);
            PointInstance {
                pos: p.pos,
                size,
                color: encode(p.color),
                rim,
                inner,
            }
        })
        .collect()
}

/// A disc facing the viewer, anchored at a world point: position, diameter in pixels, the
/// colour of the fill (inside `inner` pixels across) and of the rim (outside it).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable)]
pub struct PointInstance {
    pub pos: [f32; 3],
    pub size: f32,
    pub color: [f32; 4],
    pub rim: [f32; 4],
    pub inner: f32,
}

/// Byte offsets of [`PointInstance::size`] and [`PointInstance::inner`], for hover patches.
pub const POINT_SIZE_OFFSET: usize = std::mem::offset_of!(PointInstance, size);
pub const POINT_INNER_OFFSET: usize = std::mem::offset_of!(PointInstance, inner);

impl PointInstance {
    /// A plain disc with no rim.
    fn solid(pos: [f32; 3], size: f32, color: [f32; 4]) -> Self {
        Self {
            pos,
            size,
            color,
            rim: color,
            inner: size,
        }
    }
}

/// A vertex of a curtain: a world position and a colour with its opacity.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable)]
pub struct CurtainVertex {
    pub pos: [f32; 3],
    pub color: [f32; 4],
}

/// The curtains (`style.area`) of the polylines that have one, after the floor shadows of the
/// scene: for every segment, a vertical quad from the line down to its baseline (two
/// triangles), translucent. Later series are
/// emitted first, so with the first series on the front row the farther curtains are blended
/// first and the nearer ones over them (right for the default camera).
pub fn curtain_vertices(scene: &Scene3d, encode: impl Fn(Color) -> [f32; 4]) -> Vec<CurtainVertex> {
    let mut out = Vec::new();
    // Soft shadows on the floor (a glossy diagram's) share the curtains' translucent pass:
    // just above the floor, so nothing standing on it is darkened.
    let floor = scene.size[1] * 1e-3;
    for s in &scene.shadows {
        crate::arc::shadow(s, encode(s.color), floor + s.lift, &mut out);
    }
    for line in scene.polylines.iter().rev() {
        let Some(base) = line.area else { continue };
        let mut color = encode(line.color);
        color[3] *= facetry_core::scene3d::AREA_ALPHA;
        for run in &line.runs {
            for pair in run.windows(2) {
                let (p, q) = (pair[0], pair[1]);
                let (pb, qb) = ([p[0], base, p[2]], [q[0], base, q[2]]);
                for pos in [p, q, qb, p, qb, pb] {
                    out.push(CurtainVertex { pos, color });
                }
            }
        }
    }
    out
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
            source: wgpu::ShaderSource::Wgsl(with_ease(include_str!("shaders/box.wgsl")).into()),
        });
        let (layout, bind_group) = globals_only(device, "box", globals);
        let cube = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("unit cube"),
            contents: bytemuck::cast_slice(&cube()),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let cube_attrs = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3];
        let instance_attrs = wgpu::vertex_attr_array![
            2 => Float32x3, 3 => Float32x3, 4 => Float32x4, 5 => Float32,
            6 => Float32x3, 7 => Float32x3, 8 => Float32x4, 9 => Float32
        ];
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

/// The slices of an extruded pie and a diagram's nodes and arrowheads: one lit mesh, drawn
/// with depth testing like the bars. The vertices are kept on the CPU too, so hovering a slice
/// or node can rewrite just its range.
#[derive(Debug)]
pub struct SlicePipeline {
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    vertices: Vec<Vertex3>,
    /// Each slice's vertices within `vertices`, in slice order, then each diagram node's (the
    /// arrowheads come last and have none: they are not marks).
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
            0 => Float32x3, 1 => Float32x3, 2 => Float32x4, 3 => Float32, 4 => Float32
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
        for p in &scene.prisms {
            let start = self.vertices.len();
            crate::arc::prism(p, encode(p.color), &mut self.vertices);
            self.ranges.push(start..self.vertices.len());
        }
        for p in &scene.decor {
            crate::arc::prism(p, encode(p.color), &mut self.vertices);
        }
        for r in &scene.ribbons {
            crate::arc::ribbon(r, encode(r.color), &mut self.vertices);
        }
        for f in &scene.fills {
            crate::arc::fill(f, encode(f.color), &mut self.vertices);
        }
        self.buffer.set(device, queue, &self.vertices);
    }

    /// Lights slice (or, after the slices, node) `index` up (or not): rewrites the hover flag of
    /// its vertices only.
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

/// Discs that face the viewer, anchored in the world: the round joins of 3D lines and their
/// point markers. Two buffers drawn in order (joins, then markers) so markers always sit on
/// top. Depth-tested against the bars but not written, like the lines.
#[derive(Debug)]
pub struct PointPipeline {
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    joins: InstanceBuffer,
    markers: InstanceBuffer,
}

impl PointPipeline {
    pub fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        samples: u32,
        globals: &wgpu::Buffer,
    ) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("point3d shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/point3d.wgsl").into()),
        });
        let (layout, bind_group) = globals_only(device, "point3d", globals);
        let instance_attrs = wgpu::vertex_attr_array![
            1 => Float32x4, 2 => Float32x4, 3 => Float32x4, 4 => Float32
        ];
        let pipeline = pipeline(
            device,
            &PipelineDesc {
                label: "point3d pipeline",
                module: &module,
                layout: &layout,
                buffers: &[
                    Some(QUAD_LAYOUT),
                    Some(wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<PointInstance>() as u64,
                        step_mode: wgpu::VertexStepMode::Instance,
                        attributes: &instance_attrs,
                    }),
                ],
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                cull_mode: None,
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
            joins: InstanceBuffer::new("line joins"),
            markers: InstanceBuffer::new("point markers 3d"),
        }
    }

    pub fn set(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        joins: &[PointInstance],
        markers: &[PointInstance],
    ) {
        self.joins.set(device, queue, joins);
        self.markers.set(device, queue, markers);
    }

    /// Resizes marker `index` (an index into the slice given to `set`) to a hovered or
    /// resting look: a diameter and the diameter of its fill.
    pub fn set_marker(&self, queue: &wgpu::Queue, index: usize, size: f32, inner: f32) {
        let base = index * std::mem::size_of::<PointInstance>();
        self.markers.patch(
            queue,
            (base + POINT_SIZE_OFFSET) as u64,
            bytemuck::bytes_of(&size),
        );
        self.markers.patch(
            queue,
            (base + POINT_INNER_OFFSET) as u64,
            bytemuck::bytes_of(&inner),
        );
    }

    pub fn draw<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>, quad: &'a wgpu::Buffer) {
        for buffer in [&self.joins, &self.markers] {
            let Some((buffer, count)) = buffer.bound() else {
                continue;
            };
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.set_vertex_buffer(0, quad.slice(..));
            pass.set_vertex_buffer(1, buffer.slice(..));
            pass.draw(0..4, 0..count);
        }
    }
}

/// The translucent curtains under 3D lines (`style.area`): plain coloured triangles in world
/// space, blended over what is already drawn, tested against the bars but not written (so
/// curtains never hide the lines and bars behind them, only tint them).
#[derive(Debug)]
pub struct CurtainPipeline {
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    vertices: InstanceBuffer,
}

impl CurtainPipeline {
    pub fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        samples: u32,
        globals: &wgpu::Buffer,
    ) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("curtain3d shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/curtain3d.wgsl").into()),
        });
        let (layout, bind_group) = globals_only(device, "curtain3d", globals);
        let attrs = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x4];
        let pipeline = pipeline(
            device,
            &PipelineDesc {
                label: "curtain3d pipeline",
                module: &module,
                layout: &layout,
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<CurtainVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &attrs,
                })],
                topology: wgpu::PrimitiveTopology::TriangleList,
                // Seen from either side.
                cull_mode: None,
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
            vertices: InstanceBuffer::new("curtain vertices"),
        }
    }

    pub fn set(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, vertices: &[CurtainVertex]) {
        self.vertices.set(device, queue, vertices);
    }

    pub fn draw<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>) {
        let Some((buffer, count)) = self.vertices.bound() else {
            return;
        };
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.set_vertex_buffer(0, buffer.slice(..));
        pass.draw(0..count, 0..1);
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

    fn lines_scene(area: bool) -> Scene3d {
        let json = include_str!("../../../examples/specs/line3d.json").replace(
            r#""strokeWidth": 3"#,
            if area {
                r#""strokeWidth": 3, "area": true"#
            } else {
                r#""strokeWidth": 3"#
            },
        );
        facetry_core::build_scene_3d(
            &facetry_core::parse_spec_json(&json).unwrap(),
            (800, 500),
            1.0,
            facetry_font::FontMetrics::inter(),
        )
    }

    fn identity(c: Color) -> [f32; 4] {
        c.to_array()
    }

    #[test]
    fn a_polyline_is_flat_ended_segments_with_a_round_disc_at_every_vertex() {
        let scene = lines_scene(false);
        // Three series of six points: five segments each, six discs each.
        let segments = polyline_instances(&scene, identity);
        assert_eq!(segments.len(), 15);
        assert!(segments.iter().all(|l| l.cap == 0.0 && l.width_px == 3.0));
        assert!(segments.iter().all(|l| l.show == [0.0, 360.0]));
        let joins = join_instances(&scene, identity);
        assert_eq!(joins.len(), 18);
        // As wide as the line, solid (no rim), at the run's own vertices.
        assert!(
            joins
                .iter()
                .all(|j| j.size == 3.0 && j.inner == 3.0 && j.rim == j.color)
        );
        assert_eq!(joins[0].pos, scene.polylines[0].runs[0][0]);
        // Grid lines keep their square caps.
        assert!(
            line_instances(&scene, identity)
                .iter()
                .all(|l| l.cap == 1.0)
        );
    }

    #[test]
    fn markers_follow_the_points_and_grow_when_hovered() {
        let scene = lines_scene(false);
        let rim = [1.0; 4];
        let markers = marker_instances(&scene, identity, rim);
        assert_eq!(markers.len(), scene.points.len());
        assert!(markers.iter().all(|m| m.size == 8.0 && m.rim == rim));
        assert!(markers.iter().all(|m| m.inner < m.size && m.inner > 0.0));
        let (rest, _) = marker_sizes(8.0, true, false);
        let (hover, inner) = marker_sizes(8.0, true, true);
        assert_eq!((rest, hover), (8.0, 12.0));
        assert!(inner < hover);
        // A point without a marker is not drawn until it is hovered.
        assert_eq!(marker_sizes(8.0, false, false).0, 0.0);
        assert_eq!(marker_sizes(8.0, false, true).0, 12.0);
    }

    #[test]
    fn curtains_are_translucent_quads_from_the_line_to_its_baseline_far_rows_first() {
        assert!(curtain_vertices(&lines_scene(false), identity).is_empty());
        let scene = lines_scene(true);
        let vertices = curtain_vertices(&scene, identity);
        // 15 segments, two triangles each.
        assert_eq!(vertices.len(), 15 * 6);
        assert!(
            vertices
                .iter()
                .all(|v| v.color[3] < 0.5 && v.color[3] > 0.0)
        );
        // Every quad's bottom vertices are on the baseline, its top ones on the line.
        for quad in vertices.chunks(6) {
            let on_base = quad.iter().filter(|v| v.pos[1] == 0.0).count();
            assert_eq!(on_base, 3, "{quad:?}");
        }
        // The last series (the farthest row) is drawn first.
        let last = scene.polylines.last().unwrap();
        assert_eq!(vertices[0].pos, last.runs[0][0]);
    }

    #[test]
    fn instance_structs_match_their_vertex_layouts() {
        assert_eq!(std::mem::size_of::<BoxInstance>(), 44 + 4 * 11);
        assert_eq!(BOX_HOVER_OFFSET, 40);
        assert_eq!(std::mem::size_of::<LineInstance>(), 56);
        assert_eq!(std::mem::size_of::<PointInstance>(), 52);
        assert_eq!((POINT_SIZE_OFFSET, POINT_INNER_OFFSET), (12, 48));
        assert_eq!(std::mem::size_of::<CurtainVertex>(), 28);
    }
}
