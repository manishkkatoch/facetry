//! Small GPU-facing pieces shared by the pipelines: the uniform block, the unit quad, and
//! instance buffers that grow on demand.

use bytemuck::{Pod, Zeroable};

/// Uniforms shared by the 2D shaders (matches `Globals` in the WGSL). 48 bytes: uniform
/// blocks must be a multiple of 16.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct Globals {
    /// Render target size in pixels.
    pub viewport: [f32; 2],
    /// Atlas size in pixels.
    pub atlas: [f32; 2],
    pub px_range: f32,
    pub _pad: [f32; 3],
    /// The animation clock the shaders mix `from` and `to` states by: elapsed milliseconds,
    /// the duration, the easing's number (see `Easing::id`) and a spare. A mark's own start
    /// delay is an instance attribute. See `shaders/ease.wgsl`.
    pub anim: [f32; 4],
}

/// The clock of a scene that is not animating: every mark's progress is exactly 1, so what
/// the shader draws is the `to` state to the last bit.
pub const ANIM_IDLE: [f32; 4] = [1.0, 1.0, 0.0, 0.0];

/// The easing and clock functions the animated shaders share (WGSL has no `#include`, so they
/// are prepended). They read `globals.anim`, which each shader's `Globals` declares.
pub fn with_ease(shader: &str) -> String {
    format!("{}\n{shader}", include_str!("shaders/ease.wgsl"))
}

/// Uniforms shared by the 3D shaders (matches `Globals` in `box.wgsl`, `line3d.wgsl` and
/// `text3d.wgsl`). 160 bytes, a multiple of 16 as WebGL2 needs. Rewritten every frame from
/// the camera, which is all that changes when the user orbits.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct Globals3 {
    pub view_proj: [[f32; 4]; 4],
    /// Camera position (xyz).
    pub eye: [f32; 4],
    /// `x`: the camera's azimuth in degrees (`0..360`), which lines and labels test their
    /// `azimuths` range against; `y` its elevation in degrees, `z` its distance, `w` 1 for the
    /// perspective lens and 0 for the orthographic one.
    pub view: [f32; 4],
    /// Direction *to* the light (xyz, unit length) and the ambient fraction (w).
    pub light: [f32; 4],
    /// Render target size in pixels.
    pub viewport: [f32; 2],
    /// Atlas size in pixels.
    pub atlas: [f32; 2],
    pub px_range: f32,
    pub _pad: [f32; 3],
    /// The animation clock, as in [`Globals`].
    pub anim: [f32; 4],
}

/// Depth buffer format: 24 bits is plenty for one chart and is supported everywhere,
/// including WebGL2.
pub const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth24Plus;

/// An item's azimuth range as the shaders read it: `[from, to)` in degrees, wrapping when
/// `from > to`. No range means always: `[0, 360)`, which every azimuth is inside.
pub fn show_range(range: Option<facetry_core::scene3d::AzimuthRange>) -> [f32; 2] {
    range.map_or([0.0, 360.0], |r| [r.from, r.to])
}

/// Corners of the unit quad as a triangle strip: (0,0) top-left, (1,1) bottom-right.
pub const QUAD: [[f32; 2]; 4] = [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0]];

/// The vertex buffer layout for the quad corner (shader location 0).
pub const QUAD_LAYOUT: wgpu::VertexBufferLayout<'static> = wgpu::VertexBufferLayout {
    array_stride: 8,
    step_mode: wgpu::VertexStepMode::Vertex,
    attributes: &wgpu::vertex_attr_array![0 => Float32x2],
};

/// A GPU buffer of `T` instances that is rewritten whole each time the scene changes,
/// reallocated only when it must grow.
#[derive(Debug)]
pub struct InstanceBuffer {
    buffer: Option<wgpu::Buffer>,
    capacity: usize,
    count: u32,
    label: &'static str,
}

impl InstanceBuffer {
    pub fn new(label: &'static str) -> Self {
        Self {
            buffer: None,
            capacity: 0,
            count: 0,
            label,
        }
    }

    /// Replaces the contents with `items`.
    pub fn set<T: Pod>(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, items: &[T]) {
        self.count = items.len() as u32;
        if items.is_empty() {
            return;
        }
        if self.buffer.is_none() || items.len() > self.capacity {
            self.capacity = items.len().next_power_of_two();
            self.buffer = Some(device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(self.label),
                size: (self.capacity * std::mem::size_of::<T>()) as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }));
        }
        if let Some(buffer) = &self.buffer {
            queue.write_buffer(buffer, 0, bytemuck::cast_slice(items));
        }
    }

    /// Overwrites a few bytes inside the buffer (`offset` from its start), for tiny updates
    /// such as one instance's hover flag. Does nothing while the buffer is empty.
    pub fn patch(&self, queue: &wgpu::Queue, offset: u64, bytes: &[u8]) {
        if let Some(buffer) = &self.buffer {
            queue.write_buffer(buffer, offset, bytes);
        }
    }

    /// The buffer and instance count, or `None` when there is nothing to draw.
    pub fn bound(&self) -> Option<(&wgpu::Buffer, u32)> {
        match (&self.buffer, self.count) {
            (_, 0) | (None, _) => None,
            (Some(b), n) => Some((b, n)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn globals_match_the_wgsl_block_size() {
        assert_eq!(std::mem::size_of::<Globals>(), 48);
        assert_eq!(std::mem::size_of::<Globals3>(), 160);
    }

    #[test]
    fn no_range_means_always() {
        use facetry_core::scene3d::AzimuthRange;
        assert_eq!(show_range(None), [0.0, 360.0]);
        assert_eq!(
            show_range(Some(AzimuthRange::new(90.0, 180.0))),
            [90.0, 180.0]
        );
    }
}
