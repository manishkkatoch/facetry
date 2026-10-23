use facetry_core::scene::Scene;
use facetry_core::scene3d::Scene3d;
use facetry_core::{Color, Theme};
use facetry_font::FontMetrics;
use glam::Vec3;
use wgpu::util::DeviceExt as _;

use crate::Error;
use crate::camera::Camera;
use crate::gpu::{self, DEPTH_FORMAT, Globals, Globals3};
use crate::labels::{LabelPlan, SLOTS};
use crate::rect::{self, RectPipeline};
use crate::solid::{self, BoxPipeline, LinePipeline};
use crate::text::{self, Atlas, Glyph3Instance, GlyphInstance, Text3dPipeline, TextPipeline};

/// Antialiasing quality we ask for; dropped to 1 if the device cannot do it.
const MSAA_SAMPLES: u32 = 4;

/// Which GPU API the caller wants.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BackendPreference {
    /// WebGPU when the browser really has it, otherwise WebGL2 (or the native API).
    #[default]
    Auto,
    /// WebGPU only; fails with [`Error::GpuUnavailable`] if absent.
    WebGpu,
    /// The WebGL2 fallback only (the GL backend).
    WebGl2,
}

/// Whether this build contains the browser WebGPU backend. Always true off the web.
const HAS_WEBGPU: bool = cfg!(any(not(target_arch = "wasm32"), feature = "webgpu"));
/// Whether this build contains the WebGL2 (GL) backend. Always true off the web.
const HAS_WEBGL: bool = cfg!(any(not(target_arch = "wasm32"), feature = "webgl"));

impl BackendPreference {
    /// The wgpu backends to try. A backend this build was compiled without is an error, not
    /// a silent fallback: the JS loader chooses the module, so asking one for the other's
    /// backend is a bug worth reporting. `Auto` means "whatever this build has".
    fn backends(self) -> Result<wgpu::Backends, Error> {
        let missing = |what: &str, has: &str| {
            Error::GpuUnavailable(format!(
                "this build of facetry does not include the {what} backend (it contains {has} only)"
            ))
        };
        match self {
            Self::Auto => {
                if cfg!(target_arch = "wasm32") {
                    let mut backends = wgpu::Backends::empty();
                    if HAS_WEBGPU {
                        backends |= wgpu::Backends::BROWSER_WEBGPU;
                    }
                    if HAS_WEBGL {
                        backends |= wgpu::Backends::GL;
                    }
                    Ok(backends)
                } else {
                    Ok(wgpu::Backends::PRIMARY | wgpu::Backends::GL)
                }
            }
            Self::WebGpu if HAS_WEBGPU => Ok(wgpu::Backends::BROWSER_WEBGPU),
            Self::WebGpu => Err(missing("WebGPU", "WebGL2")),
            Self::WebGl2 if HAS_WEBGL => Ok(wgpu::Backends::GL),
            Self::WebGl2 => Err(missing("WebGL2", "WebGPU")),
        }
    }
}

/// Direction *toward* the light in world space: from above, a little to the right and the
/// front, so at the default camera the top is brightest, the front mid and the right side
/// darker (the left and back faces get ambient only).
const LIGHT_TO: [f32; 3] = [0.55, 0.8, 0.45];
/// Share of a face's colour it keeps with no light at all.
const AMBIENT: f32 = 0.5;
/// The pipelines and buffers only 3D charts need, created the first time a 3D scene arrives
/// so 2D charts do not pay for them.
#[derive(Debug)]
struct Solid {
    globals: wgpu::Buffer,
    /// Per-label "show it, nudge it" table the text shader reads (see `labels.rs`).
    label_states: wgpu::Buffer,
    boxes: BoxPipeline,
    lines: LinePipeline,
    text: Text3dPipeline,
}

/// Owns the GPU objects (instance, adapter info, device, queue) and the window surface,
/// and draws a [`Scene`] with them.
#[derive(Debug)]
pub struct Renderer {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    backend: wgpu::Backend,
    /// Clear colour; set from each scene's background.
    background: Color,
    /// 4 when multisampling works, else 1.
    samples: u32,
    /// The multisampled colour target, resolved into the surface each frame. `None` when
    /// `samples == 1` or the surface has no area.
    msaa_view: Option<wgpu::TextureView>,
    /// Depth buffer for the 3D pass (same sample count as the colour target); only exists
    /// while a 3D scene is shown.
    depth_view: Option<wgpu::TextureView>,
    globals: wgpu::Buffer,
    atlas: Atlas,
    solid: Option<Solid>,
    /// The plot box's size when a 3D scene is active (`None`: draw the 2D scene).
    box_size: Option<[f32; 3]>,
    camera: Camera,
    /// The 3D axis labels that are shown or hidden per frame, and the camera and viewport the
    /// label table was last computed for (so a redraw that changed neither skips the work).
    label_plan: Option<LabelPlan>,
    label_key: Option<(Camera, (u32, u32))>,
    quad: wgpu::Buffer,
    rects: RectPipeline,
    text: TextPipeline,
    /// Largest texture side the device accepts; larger resizes are clamped.
    max_dimension: u32,
    /// `false` while the surface has zero area (hidden canvas); we cannot configure
    /// or draw to a 0x0 surface, so we wait for the next non-zero `resize`.
    drawable: bool,
}

impl Renderer {
    /// Creates a renderer drawing to `target` (on the web: `SurfaceTarget::Canvas`).
    ///
    /// Async because browsers hand out adapters and devices via promises.
    pub async fn new(
        target: impl Into<wgpu::SurfaceTarget<'static>>,
        width: u32,
        height: u32,
        preference: BackendPreference,
    ) -> Result<Self, Error> {
        let unavailable =
            |what: &str, e: &dyn std::fmt::Display| Error::GpuUnavailable(format!("{what}: {e}"));

        let mut desc = wgpu::InstanceDescriptor::new_without_display_handle();
        desc.backends = preference.backends()?;
        let has_gl = desc.backends.contains(wgpu::Backends::GL);
        let mut target = Some(target.into());
        // `Instance::new` is sync and cannot tell whether `navigator.gpu` really yields an
        // adapter, so for the web this probes first and drops WebGPU if it doesn't.
        let instance = wgpu::util::new_instance_with_webgpu_detection(desc).await;

        // Creating the surface calls `canvas.getContext(...)`, which locks the canvas to that
        // context type for good. WebGL needs the surface to find its adapter, but WebGPU does
        // not, so a WebGPU-only module asks for adapter and device first: if they fail, the
        // canvas is still untouched and the caller can retry with the WebGL module.
        let surface_first = !cfg!(target_arch = "wasm32") || has_gl;
        let mut surface = None;
        if surface_first {
            surface = Some(
                instance
                    .create_surface(target.take().expect("target is used once"))
                    .map_err(|e| unavailable("creating surface", &e))?,
            );
        }
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                compatible_surface: surface.as_ref(),
                ..Default::default()
            })
            .await
            .map_err(|e| unavailable("requesting adapter", &e))?;

        // WebGL2 exposes far fewer limits than WebGPU, and wgpu refuses a device that asks
        // for more than the adapter has. Start from the WebGL2-safe baseline, then raise only
        // the texture-size limit to what the adapter offers so large canvases (HiDPI) work.
        let required_limits =
            wgpu::Limits::downlevel_webgl2_defaults().using_resolution(adapter.limits());
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("facetry device"),
                required_limits,
                ..Default::default()
            })
            .await
            .map_err(|e| unavailable("requesting device", &e))?;

        let surface = match surface {
            Some(surface) => surface,
            None => instance
                .create_surface(target.take().expect("target is used once"))
                .map_err(|e| unavailable("creating surface", &e))?,
        };
        let mut config = surface.get_default_config(&adapter, 1, 1).ok_or_else(|| {
            Error::GpuUnavailable("surface is incompatible with the adapter".into())
        })?;
        // Prefer an sRGB format so the GPU gamma-encodes our linear colours for us.
        let caps = surface.get_capabilities(&adapter);
        if let Some(format) = caps.formats.iter().copied().find(|f| f.is_srgb()) {
            config.format = format;
        }

        // 4x MSAA smooths bar and text-quad edges. Every WebGPU device supports it for the
        // surface formats we pick; WebGL2 usually does, so we ask rather than assume.
        let samples = if adapter
            .get_texture_format_features(config.format)
            .flags
            .sample_count_supported(MSAA_SAMPLES)
        {
            MSAA_SAMPLES
        } else {
            1
        };
        let font = FontMetrics::inter();
        let globals = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("globals"),
            size: std::mem::size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(
            &globals,
            0,
            bytemuck::bytes_of(&Globals {
                viewport: [1.0, 1.0],
                atlas: [font.atlas.width as f32, font.atlas.height as f32],
                px_range: font.atlas.px_range,
                _pad: [0.0; 3],
            }),
        );
        let quad = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("unit quad"),
            contents: bytemuck::cast_slice(&gpu::QUAD),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let rects = RectPipeline::new(&device, config.format, samples, &globals);
        let atlas = Atlas::new(&device, &queue)?;
        let text = TextPipeline::new(&device, &atlas, config.format, samples, &globals);

        let mut renderer = Self {
            surface,
            device,
            queue,
            config,
            backend: adapter.get_info().backend,
            background: Theme::default().background,
            samples,
            msaa_view: None,
            depth_view: None,
            globals,
            atlas,
            solid: None,
            box_size: None,
            camera: Camera::default(),
            label_plan: None,
            label_key: None,
            quad,
            rects,
            text,
            max_dimension: adapter.limits().max_texture_dimension_2d,
            drawable: false,
        };
        renderer.resize(width, height);
        Ok(renderer)
    }

    /// Creates a renderer drawing to an HTML canvas, using its current `width`/`height`.
    #[cfg(target_arch = "wasm32")]
    pub async fn from_canvas(
        canvas: web_sys::HtmlCanvasElement,
        preference: BackendPreference,
    ) -> Result<Self, Error> {
        let (width, height) = (canvas.width(), canvas.height());
        Self::new(
            wgpu::SurfaceTarget::Canvas(canvas),
            width,
            height,
            preference,
        )
        .await
    }

    /// `"webgpu"`, `"webgl2"`, or the native API name (`"vulkan"`, `"metal"`, ...).
    pub fn backend(&self) -> &'static str {
        match self.backend {
            wgpu::Backend::BrowserWebGpu => "webgpu",
            // On the web the GL backend is always WebGL2.
            wgpu::Backend::Gl if cfg!(target_arch = "wasm32") => "webgl2",
            other => other.to_str(),
        }
    }

    /// Resizes the backing store, in physical pixels. Zero is allowed and pauses drawing.
    pub fn resize(&mut self, width_px: u32, height_px: u32) {
        let width = width_px.min(self.max_dimension);
        let height = height_px.min(self.max_dimension);
        self.drawable = width > 0 && height > 0;
        if self.drawable {
            self.config.width = width;
            self.config.height = height;
            self.surface.configure(&self.device, &self.config);
        }
        self.rebuild_targets();
    }

    /// (Re)creates the textures that depend on the surface size: the multisampled colour
    /// target, and the depth buffer while a 3D scene is active.
    fn rebuild_targets(&mut self) {
        let target = |label, format, samples| {
            self.device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d {
                        width: self.config.width,
                        height: self.config.height,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: samples,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                    view_formats: &[],
                })
                .create_view(&wgpu::TextureViewDescriptor::default())
        };
        self.msaa_view = (self.drawable && self.samples > 1)
            .then(|| target("msaa colour target", self.config.format, self.samples));
        self.depth_view = (self.drawable && self.box_size.is_some())
            .then(|| target("depth target", DEPTH_FORMAT, self.samples));
    }

    /// Samples per pixel actually in use: 4 (MSAA) or 1 when the device cannot multisample.
    pub fn msaa_samples(&self) -> u32 {
        self.samples
    }

    /// Size of the drawing buffer in pixels (after clamping to the device limit).
    pub fn size(&self) -> (u32, u32) {
        (self.config.width, self.config.height)
    }

    /// Converts a colour to what the render target expects: linear for sRGB formats (the GPU
    /// encodes on write), already gamma-encoded for plain ones such as `Rgba8Unorm`.
    fn encode_color(&self, color: Color) -> [f32; 4] {
        encode_color(self.config.format, color)
    }

    /// Uploads a scene: the next `render` draws it. Cheap enough to call on every resize or
    /// data change (one buffer write per list).
    pub fn set_scene(&mut self, scene: &Scene) {
        if self.box_size.take().is_some() {
            self.rebuild_targets(); // drops the depth buffer
        }
        self.background = scene.background;
        let rects = rect::instances(scene, |c| self.encode_color(c));
        self.rects.set_instances(&self.device, &self.queue, &rects);

        let font = FontMetrics::inter();
        let mut glyphs: Vec<GlyphInstance> = Vec::new();
        for t in &scene.texts {
            text::layout_glyphs(t, self.encode_color(t.color), font, &mut glyphs);
        }
        self.text.set_instances(&self.device, &self.queue, &glyphs);
    }

    /// Uploads a 3D scene: the next `render` draws it from the current camera. Orbiting does
    /// not call this; only a new spec or a resize does.
    pub fn set_scene_3d(&mut self, scene: &Scene3d) {
        self.background = scene.background;
        let (device, format, samples) = (&self.device, self.config.format, self.samples);
        let atlas = &self.atlas;
        let solid = self.solid.get_or_insert_with(|| {
            let globals = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("globals 3d"),
                size: std::mem::size_of::<Globals3>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let label_states = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("label states"),
                size: (SLOTS * std::mem::size_of::<[f32; 4]>()) as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            Solid {
                boxes: BoxPipeline::new(device, format, samples, &globals),
                lines: LinePipeline::new(device, format, samples, &globals),
                text: Text3dPipeline::new(device, atlas, format, samples, &globals, &label_states),
                globals,
                label_states,
            }
        });
        let encode = |c| encode_color(self.config.format, c);
        solid.boxes.set_instances(
            &self.device,
            &self.queue,
            &solid::box_instances(scene, encode),
        );
        solid.lines.set_instances(
            &self.device,
            &self.queue,
            &solid::line_instances(scene, encode),
        );
        let font = FontMetrics::inter();
        let (plan, slots) = LabelPlan::new(scene, font);
        let mut glyphs: Vec<Glyph3Instance> = Vec::new();
        for (t, &slot) in scene.texts.iter().zip(&slots) {
            text::layout_glyphs_3d(t, encode(t.color), slot, font, &mut glyphs);
        }
        // Outlines first (every label's, so no outline covers another label's letters), then
        // the letters themselves.
        let mut instances = text::halo_instances(&glyphs, encode(scene.background), scene.halo_px);
        instances.extend(glyphs);
        solid
            .text
            .set_instances(&self.device, &self.queue, &instances);
        self.label_plan = Some(plan);
        self.label_key = None;
        let entering = self.box_size.is_none();
        self.box_size = Some(scene.size);
        if entering {
            self.rebuild_targets(); // creates the depth buffer
        }
    }

    /// Sets the camera for the next `render`. Cheap: it only changes a uniform.
    pub fn set_camera(&mut self, camera: Camera) {
        self.camera = camera;
    }

    /// The globals for a frame drawn into a target of `target` pixels, from the camera.
    fn globals_3d(&self, size: [f32; 3], target: (u32, u32)) -> Globals3 {
        let (centre, radius) = Camera::plot_frame(size);
        let (w, h) = (target.0 as f32, target.1 as f32);
        let eye = self.camera.eye(centre, radius);
        let font = FontMetrics::inter();
        Globals3 {
            view_proj: self.camera.plot_view_proj(size, (w, h)).to_cols_array_2d(),
            eye: eye.extend(1.0).to_array(),
            view: [
                self.camera.azimuth,
                self.camera.elevation,
                self.camera.distance,
                1.0,
            ],
            light: Vec3::from(LIGHT_TO).normalize().extend(AMBIENT).to_array(),
            viewport: [w, h],
            atlas: [font.atlas.width as f32, font.atlas.height as f32],
            px_range: font.atlas.px_range,
            _pad: [0.0; 3],
        }
    }

    /// Writes everything the shaders read per frame for a target of `target` pixels: the
    /// globals, and in 3D the label table (skipped when the camera and size are the ones it
    /// was last computed for).
    fn write_frame_uniforms(&mut self, target: (u32, u32)) {
        let font = FontMetrics::inter();
        if let (Some(size), Some(solid)) = (self.box_size, &self.solid) {
            self.queue.write_buffer(
                &solid.globals,
                0,
                bytemuck::bytes_of(&self.globals_3d(size, target)),
            );
            let key = Some((self.camera, target));
            if self.label_key != key {
                if let Some(plan) = &self.label_plan {
                    let viewport = (target.0 as f32, target.1 as f32);
                    let table = plan.states(
                        self.camera.plot_view_proj(size, viewport),
                        [viewport.0, viewport.1],
                        self.camera.azimuth,
                    );
                    self.queue
                        .write_buffer(&solid.label_states, 0, bytemuck::cast_slice(&table));
                }
                self.label_key = key;
            }
        }
        self.queue.write_buffer(
            &self.globals,
            0,
            bytemuck::bytes_of(&Globals {
                viewport: [target.0 as f32, target.1 as f32],
                atlas: [font.atlas.width as f32, font.atlas.height as f32],
                px_range: font.atlas.px_range,
                _pad: [0.0; 3],
            }),
        );
    }

    /// Records the chart pass. 2D: the background, then gridlines, bars, axis rules and text.
    /// 3D: bars (depth-tested), then lines (depth-tested against the bars), then text on top.
    ///
    /// Draws into `color`; with MSAA that is the multisampled target and `resolve` is where
    /// it is averaged down to.
    fn record_pass(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        color: &wgpu::TextureView,
        resolve: Option<&wgpu::TextureView>,
        depth: Option<&wgpu::TextureView>,
        clear: [f32; 4],
    ) {
        let [r, g, b, a] = clear;
        // The multisampled contents are thrown away once resolved.
        let store = if resolve.is_some() {
            wgpu::StoreOp::Discard
        } else {
            wgpu::StoreOp::Store
        };
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("chart pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: color,
                depth_slice: None,
                resolve_target: resolve,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: f64::from(r),
                        g: f64::from(g),
                        b: f64::from(b),
                        a: f64::from(a),
                    }),
                    store,
                },
            })],
            depth_stencil_attachment: depth.map(|view| wgpu::RenderPassDepthStencilAttachment {
                view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Discard,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        match (&self.solid, self.box_size) {
            (Some(solid), Some(_)) => {
                solid.boxes.draw(&mut pass);
                solid.lines.draw(&mut pass, &self.quad);
                // Labels are drawn last and ignore depth: they sit just outside the box,
                // and a label hidden behind a bar is worse than one drawn over it.
                solid.text.draw(&mut pass, &self.quad);
            }
            // No depth test in 2D: draw order alone decides what is on top.
            _ => {
                self.rects.draw(&mut pass, &self.quad);
                self.text.draw(&mut pass, &self.quad);
            }
        }
    }

    /// Draws one frame to the surface.
    pub fn render(&mut self) -> Result<(), Error> {
        if !self.drawable {
            return Ok(());
        }
        use wgpu::CurrentSurfaceTexture as Frame;
        let frame = match self.surface.get_current_texture() {
            Frame::Success(frame) => frame,
            // Still presentable; reconfigure after presenting so the next frame is optimal.
            Frame::Suboptimal(frame) => {
                self.surface.configure(&self.device, &self.config);
                frame
            }
            // Transient or stale: reconfigure and let the caller's next render retry.
            Frame::Outdated | Frame::Lost => {
                self.surface.configure(&self.device, &self.config);
                return Ok(());
            }
            Frame::Timeout | Frame::Occluded => return Ok(()),
            Frame::Validation => {
                return Err(Error::RenderFailed("surface validation error".into()));
            }
        };

        let frame_view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        self.write_frame_uniforms((self.config.width, self.config.height));
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("frame"),
            });
        // With MSAA we draw into the multisampled texture and resolve (average the samples)
        // into the surface.
        let (view, resolve) = match &self.msaa_view {
            Some(msaa) => (msaa, Some(&frame_view)),
            None => (&frame_view, None),
        };
        let clear = self.encode_color(self.background);
        self.record_pass(&mut encoder, view, resolve, self.depth_view.as_ref(), clear);
        self.queue.submit([encoder.finish()]);
        self.queue.present(frame);
        Ok(())
    }
}

/// Converts a colour to what a render target of `format` expects: linear for sRGB formats
/// (the GPU encodes on write), already gamma-encoded for plain ones such as `Rgba8Unorm`.
fn encode_color(format: wgpu::TextureFormat, color: Color) -> [f32; 4] {
    if format.is_srgb() {
        color.to_array()
    } else {
        color.to_srgb()
    }
}
