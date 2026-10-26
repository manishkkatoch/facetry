use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

use facetry_core::scene::{ArcMark, PointMark, Scene, Text};
use facetry_core::scene3d::{Scene3d, Text3};
use facetry_core::spec::LabelShow;
use facetry_core::transition::{Phase, Plan};
use facetry_core::transition3d::Plan3;
use facetry_core::{Color, Theme};
use facetry_font::FontMetrics;
use glam::Vec3;
use wgpu::util::DeviceExt as _;

use crate::Error;
use crate::camera::Camera;
use crate::gpu::{self, ANIM_IDLE, DEPTH_FORMAT, Globals, Globals3};
use crate::image::{self, ReadbackFormat};
use crate::labels::{LabelPlan, SLOTS};
use crate::mesh::{self, Mesh, MeshData, MeshPipeline};
use crate::pick;
use crate::rect::{self, RectPipeline};
use crate::solid::{
    self, BoxPipeline, CurtainPipeline, LinePipeline, PointPipeline, SlicePipeline,
};
use crate::text::{
    self, Atlas, Glyph3Instance, GlyphInstance, NO_SLOT, Text3dPipeline, TextPipeline,
};

/// What one `record_pass` is for: the clear colour, whether hover overlays are drawn (not in
/// snapshots), and the size of the target in pixels.
#[derive(Debug, Clone, Copy)]
struct PassSpec {
    clear: [f32; 4],
    overlay: bool,
    size: (u32, u32),
}

use crate::solid::HOVER_MARKER_SCALE;

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
    slices: SlicePipeline,
    /// Gridlines, axis edges and ticks.
    lines: LinePipeline,
    /// Line layers' series (flat-ended segments; the joins are in `points`), the translucent
    /// curtains under them, and their round joins and point markers.
    data_lines: LinePipeline,
    curtains: CurtainPipeline,
    points: PointPipeline,
    text: Text3dPipeline,
}

/// A 2D value label that shows while its bar is hovered, colour already encoded.
#[derive(Debug)]
struct HoverLabel {
    mark: usize,
    text: Text,
    color: [f32; 4],
}

#[derive(Debug)]
struct HoverLabel3 {
    mark: usize,
    text: Text3,
    color: [f32; 4],
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
    /// Hover: the bar under the pointer (an index into the scene's bars), how many bars
    /// there are, and the labels that show only while their bar is hovered.
    hover: Option<usize>,
    mark_count: usize,
    /// 2D: bars follow the gridlines in the rect buffer, so bar `i` is instance
    /// `bar_offset + i`.
    bar_offset: usize,
    /// 2D: how many of the marks are bars; then come the line points, then the pie slices.
    bar_count: usize,
    /// 2D: where bar `i` of the scene is among the bars' rect instances (`bar_offset + slot`),
    /// during a transition, when leaving bars sit between the others. Empty: bar `i` is at `i`.
    bar_slots: Vec<usize>,
    /// 2D: the rect instance after the last bar, where the 2D pass splits the rectangles
    /// (`bar_offset + bar_count`, plus the bars that are leaving during a transition).
    bar_end: usize,
    /// The animation clock the shaders read: elapsed ms, duration ms, easing number. Idle
    /// (progress exactly 1) unless a transition is on screen.
    anim: [f32; 4],
    /// 2D: the line points and pie slices, kept for what is drawn over them while hovered.
    point_marks: Vec<PointMark>,
    arc_marks: Vec<ArcMark>,
    /// 3D: how many of the marks are boxes; then come the pie slices, then the line points.
    box_count: usize,
    slice_count: usize,
    /// 3D: each line point's marker diameter and whether it is drawn at rest, to resize it
    /// when hovered.
    point_sizes: Vec<(f32, bool)>,
    /// 3D: the part of the canvas the plot is drawn in (`[x, y, w, h]`, device pixels; `None`
    /// is all of it: a legend takes a band), and whether a 2D overlay (the legend) is drawn
    /// over the finished picture in a second pass.
    plot_rect: Option<[f32; 4]>,
    has_overlay: bool,
    /// 2D triangles: pie slices, tessellated line strokes, point markers, and the hover mark.
    mesh: MeshPipeline,
    arcs: Mesh,
    lines: Mesh,
    markers: Mesh,
    hover_marker: Mesh,
    hover_labels: Vec<HoverLabel>,
    hover_labels_3d: Vec<HoverLabel3>,
    /// Colour and radius of the outline round 3D labels.
    halo: ([f32; 4], f32),
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
                anim: ANIM_IDLE,
            }),
        );
        let quad = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("unit quad"),
            contents: bytemuck::cast_slice(&gpu::QUAD),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let rects = RectPipeline::new(&device, config.format, samples, &globals);
        let mesh = MeshPipeline::new(&device, config.format, samples, &globals);
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
            hover: None,
            mark_count: 0,
            bar_offset: 0,
            bar_count: 0,
            bar_slots: Vec::new(),
            bar_end: 0,
            anim: ANIM_IDLE,
            point_marks: Vec::new(),
            arc_marks: Vec::new(),
            box_count: 0,
            slice_count: 0,
            point_sizes: Vec::new(),
            plot_rect: None,
            has_overlay: false,
            mesh,
            arcs: Mesh::new("pie slices"),
            lines: Mesh::new("line strokes"),
            markers: Mesh::new("point markers"),
            hover_marker: Mesh::new("hover marker"),
            hover_labels: Vec::new(),
            hover_labels_3d: Vec::new(),
            halo: ([1.0; 4], 1.0),
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
    /// data change (one buffer write per list). Clears any hover and any transition.
    pub fn set_scene(&mut self, scene: &Scene) {
        self.anim = ANIM_IDLE;
        self.load_scene(scene, None);
    }

    /// Uploads a transition: the next `render` draws it at time 0 (the old scene) and
    /// [`set_time`](Self::set_time) moves it along. The bars, gridlines, axes, legend and text
    /// are drawn by the GPU mixing each item's start and end state; what is tessellated on the
    /// CPU (lines, markers, pie slices) is redone by [`set_marks`](Self::set_marks) each frame.
    /// Clears any hover.
    pub fn begin_transition(&mut self, plan: &Plan) {
        self.anim = [
            0.0,
            plan.timing.duration_ms,
            plan.timing.easing.id() as f32,
            0.0,
        ];
        self.load_scene(&plan.target, Some(plan));
    }

    /// Moves the transition started by [`begin_transition`](Self::begin_transition) to
    /// `elapsed_ms`. Only a uniform changes.
    pub fn set_time(&mut self, elapsed_ms: f32) {
        self.anim[0] = elapsed_ms;
    }

    /// Re-tessellates the lines, point markers and pie slices of `marks` (a scene or
    /// [`Plan::marks_at`]): what the CPU draws while a transition runs. Everything else of the
    /// scene on screen is left alone.
    pub fn set_marks(&mut self, marks: &Scene) {
        let arcs = mesh::arc_mesh(marks, |c| self.encode_color(c));
        self.arcs.set(&self.device, &self.queue, &arcs);
        let lines = mesh::line_mesh(marks, |c| self.encode_color(c));
        self.lines.set(&self.device, &self.queue, &lines);
        let markers = mesh::point_mesh(
            marks,
            |c| self.encode_color(c),
            self.encode_color(marks.background),
        );
        self.markers.set(&self.device, &self.queue, &markers);
    }

    fn load_scene(&mut self, scene: &Scene, plan: Option<&Plan>) {
        if self.box_size.take().is_some() {
            self.rebuild_targets(); // drops the depth buffer
        }
        self.background = scene.background;
        let (rects, grids, bars) = match plan {
            Some(plan) => rect::pair_instances(plan, |c| self.encode_color(c)),
            None => {
                let rects = rect::instances(scene, |c| self.encode_color(c));
                let grids = scene
                    .rules
                    .iter()
                    .filter(|r| r.kind == facetry_core::scene::RuleKind::Grid)
                    .count();
                (rects, grids, scene.rects.len())
            }
        };
        self.rects.set_instances(&self.device, &self.queue, &rects);
        // Lines, markers and slices are tessellated here, so a transition starts them from
        // where they are at time 0 and `set_marks` redoes them every frame.
        match plan {
            Some(plan) => self.set_marks(&plan.marks_at(0.0)),
            None => self.set_marks(scene),
        }
        self.hover_marker
            .set(&self.device, &self.queue, &MeshData::default());
        self.point_marks.clone_from(&scene.points);
        self.arc_marks.clone_from(&scene.arcs);
        self.plot_rect = None;
        self.has_overlay = false;

        let font = FontMetrics::inter();
        let mut glyphs: Vec<GlyphInstance> = Vec::new();
        // Value labels: `always` ones are ordinary text, `hover` ones wait for their bar.
        self.hover_labels.clear();
        match plan {
            Some(plan) => {
                let format = self.config.format;
                let enc = move |t: &Text| encode_color(format, t.color);
                for p in &plan.texts {
                    text::layout_glyph_pair(
                        (&p.from, enc(&p.from)),
                        (&p.to, enc(&p.to)),
                        p.delay_ms,
                        font,
                        &mut glyphs,
                    );
                }
                for p in &plan.labels {
                    let (from, to) = (&p.from.text, &p.to.text);
                    match p.to.show {
                        LabelShow::Always => text::layout_glyph_pair(
                            (from, enc(from)),
                            (to, enc(to)),
                            p.delay_ms,
                            font,
                            &mut glyphs,
                        ),
                        // Hover labels are not animated; one shows at its mark's end state.
                        LabelShow::Hover if p.phase != facetry_core::transition::Phase::Exit => {
                            self.hover_labels.push(HoverLabel {
                                mark: p.to.mark,
                                text: to.clone(),
                                color: enc(to),
                            });
                        }
                        LabelShow::Hover => {}
                    }
                }
            }
            None => {
                for t in &scene.texts {
                    text::layout_glyphs(t, self.encode_color(t.color), font, &mut glyphs);
                }
                for label in &scene.labels {
                    let color = self.encode_color(label.text.color);
                    match label.show {
                        LabelShow::Always => {
                            text::layout_glyphs(&label.text, color, font, &mut glyphs);
                        }
                        LabelShow::Hover => self.hover_labels.push(HoverLabel {
                            mark: label.mark,
                            text: label.text.clone(),
                            color,
                        }),
                    }
                }
            }
        }
        self.text.set_instances(&self.device, &self.queue, &glyphs);
        self.text.set_overlay(&self.device, &self.queue, &[]);
        self.hover_labels_3d.clear();
        self.hover = None;
        self.mark_count = scene.mark_count();
        self.bar_count = scene.rects.len();
        self.bar_offset = grids;
        self.bar_end = grids + bars;
        self.bar_slots = plan.map_or_else(Vec::new, |p| p.rect_slot.clone());
    }

    /// Uploads a 3D scene: the next `render` draws it from the current camera. Orbiting does
    /// not call this; only a new spec or a resize does. Clears any hover and any transition.
    pub fn set_scene_3d(&mut self, scene: &Scene3d) {
        self.anim = ANIM_IDLE;
        self.load_scene_3d(scene, None);
    }

    /// Uploads a 3D transition: the next `render` draws it at time 0 (the old scene).
    /// [`set_time`](Self::set_time) moves the clock the GPU mixes boxes and text by;
    /// [`set_marks_3d`](Self::set_marks_3d) and [`set_view_3d`](Self::set_view_3d) are what the
    /// caller does each frame for everything else. Which axis labels show is decided from the
    /// labels of the *target* scene throughout. Clears any hover.
    pub fn begin_transition_3d(&mut self, plan: &Plan3) {
        self.anim = [
            0.0,
            plan.timing.duration_ms,
            plan.timing.easing.id() as f32,
            0.0,
        ];
        self.load_scene_3d(&plan.target, Some(plan));
    }

    /// Frames the plot box of size `size` in the part `plot` of the canvas (`None`: all of it):
    /// what changes each frame while a transition resizes the box. Only uniforms change.
    pub fn set_view_3d(&mut self, size: [f32; 3], plot: Option<[f32; 4]>) {
        if self.box_size.is_some() {
            self.box_size = Some(size);
            self.plot_rect = plot;
            self.label_key = None;
        }
    }

    /// Re-tessellates what the CPU draws in 3D (grid and axis lines, pie slices, data lines,
    /// curtains, markers) from `marks` (a scene or [`Plan3::marks_at`]), keeping the hover.
    pub fn set_marks_3d(&mut self, marks: &Scene3d) {
        let format = self.config.format;
        let encode = move |c| encode_color(format, c);
        let Some(solid) = &mut self.solid else { return };
        solid.slices.set(&self.device, &self.queue, marks, encode);
        solid.lines.set_instances(
            &self.device,
            &self.queue,
            &solid::line_instances(marks, encode),
        );
        solid.data_lines.set_instances(
            &self.device,
            &self.queue,
            &solid::polyline_instances(marks, encode),
        );
        solid.curtains.set(
            &self.device,
            &self.queue,
            &solid::curtain_vertices(marks, encode),
        );
        solid.points.set(
            &self.device,
            &self.queue,
            &solid::join_instances(marks, encode),
            &solid::marker_instances(marks, encode, encode(marks.background)),
        );
        // The meshes were rebuilt without the hover: light the hovered mark again.
        if let Some(m) = self.hover {
            light_3d(
                solid,
                &self.queue,
                (
                    self.box_count,
                    self.slice_count,
                    self.point_sizes.as_slice(),
                ),
                m,
                true,
            );
        }
    }

    fn load_scene_3d(&mut self, scene: &Scene3d, plan: Option<&Plan3>) {
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
                slices: SlicePipeline::new(device, format, samples, &globals),
                lines: LinePipeline::new(device, format, samples, &globals),
                data_lines: LinePipeline::new(device, format, samples, &globals),
                curtains: CurtainPipeline::new(device, format, samples, &globals),
                points: PointPipeline::new(device, format, samples, &globals),
                text: Text3dPipeline::new(device, atlas, format, samples, &globals, &label_states),
                globals,
                label_states,
            }
        });
        let encode = |c| encode_color(self.config.format, c);
        let boxes = match plan {
            Some(plan) => solid::box_pair_instances(plan, encode),
            None => solid::box_instances(scene, encode),
        };
        solid.boxes.set_instances(&self.device, &self.queue, &boxes);
        self.point_sizes = scene
            .points
            .iter()
            .map(|p| (p.size_px, p.visible))
            .collect();
        let font = FontMetrics::inter();
        // Which labels show is decided from the target's labels (see `begin_transition_3d`).
        let (label_plan, slots) = LabelPlan::new(scene, font);
        let mut glyphs: Vec<Glyph3Instance> = Vec::new();
        // The glyphs that get an outline (labels printed on a bar or slice do not).
        let mut outlined: Vec<Glyph3Instance> = Vec::new();
        self.hover_labels.clear();
        self.hover_labels_3d.clear();
        match plan {
            Some(plan) => {
                let by_key =
                    |texts: &[&Text3], slots: &[f32]| -> std::collections::HashMap<String, f32> {
                        texts
                            .iter()
                            .zip(slots)
                            .map(|(t, &s)| (t.key.clone(), s))
                            .collect()
                    };
                let texts: Vec<&Text3> = scene.texts.iter().collect();
                let text_slots = by_key(&texts, &slots);
                let labels: Vec<&Text3> = scene.labels.iter().map(|l| &l.text).collect();
                let label_slots = by_key(&labels, label_plan.label_slots());
                let format = self.config.format;
                let enc = move |t: &Text3| encode_color(format, t.color);
                for p in &plan.texts {
                    let slot = match p.phase {
                        Phase::Exit => NO_SLOT,
                        _ => text_slots.get(&p.to.key).copied().unwrap_or(NO_SLOT),
                    };
                    let from = glyphs.len();
                    text::layout_glyph3_pair(
                        (&p.from, enc(&p.from)),
                        (&p.to, enc(&p.to)),
                        slot,
                        p.delay_ms,
                        font,
                        &mut glyphs,
                    );
                    if p.to.halo {
                        outlined.extend_from_slice(&glyphs[from..]);
                    }
                }
                for p in &plan.labels {
                    let (a, b) = (&p.from.text, &p.to.text);
                    match p.to.show {
                        LabelShow::Always => {
                            let slot = match p.phase {
                                Phase::Exit => NO_SLOT,
                                _ => label_slots.get(&b.key).copied().unwrap_or(NO_SLOT),
                            };
                            let from = glyphs.len();
                            text::layout_glyph3_pair(
                                (a, enc(a)),
                                (b, enc(b)),
                                slot,
                                p.delay_ms,
                                font,
                                &mut glyphs,
                            );
                            if b.halo {
                                outlined.extend_from_slice(&glyphs[from..]);
                            }
                        }
                        LabelShow::Hover if p.phase != Phase::Exit => {
                            self.hover_labels_3d.push(HoverLabel3 {
                                mark: p.to.mark,
                                text: b.clone(),
                                color: enc(b),
                            });
                        }
                        LabelShow::Hover => {}
                    }
                }
            }
            None => {
                for (t, &slot) in scene.texts.iter().zip(&slots) {
                    let from = glyphs.len();
                    text::layout_glyphs_3d(t, encode(t.color), slot, font, &mut glyphs);
                    if t.halo {
                        outlined.extend_from_slice(&glyphs[from..]);
                    }
                }
                for (index, label) in scene.labels.iter().enumerate() {
                    let color = encode(label.text.color);
                    match label.show {
                        LabelShow::Always => {
                            let from = glyphs.len();
                            let slot = label_plan
                                .label_slots()
                                .get(index)
                                .copied()
                                .unwrap_or(NO_SLOT);
                            text::layout_glyphs_3d(&label.text, color, slot, font, &mut glyphs);
                            if label.text.halo {
                                outlined.extend_from_slice(&glyphs[from..]);
                            }
                        }
                        LabelShow::Hover => self.hover_labels_3d.push(HoverLabel3 {
                            mark: label.mark,
                            text: label.text.clone(),
                            color,
                        }),
                    }
                }
            }
        }
        // Outlines first (every label's, so no outline covers another label's letters), then
        // the letters themselves.
        self.halo = (encode(scene.background), scene.halo_px);
        let mut instances = text::halo_instances(&outlined, self.halo.0, self.halo.1);
        instances.extend(glyphs);
        let Some(solid) = &mut self.solid else { return };
        solid
            .text
            .set_instances(&self.device, &self.queue, &instances);
        solid.text.set_overlay(&self.device, &self.queue, &[]);
        // The legend: drawn after the 3D pass with the 2D rectangle and text pipelines (whose
        // buffers a 3D chart does not otherwise use).
        let mut legend_glyphs: Vec<GlyphInstance> = Vec::new();
        match plan {
            Some(plan) => {
                let (rects, _, _) = rect::pair_instances(&plan.overlay, encode);
                self.rects.set_instances(&self.device, &self.queue, &rects);
                for p in &plan.overlay.texts {
                    text::layout_glyph_pair(
                        (&p.from, encode(p.from.color)),
                        (&p.to, encode(p.to.color)),
                        p.delay_ms,
                        font,
                        &mut legend_glyphs,
                    );
                }
                self.has_overlay =
                    !(plan.overlay.rules.is_empty() && plan.overlay.texts.is_empty());
            }
            None => {
                let swatches = rect::from_boxes(
                    scene.overlay.rules.iter().map(|r| (r.bounds, r.color)),
                    encode,
                );
                self.rects
                    .set_instances(&self.device, &self.queue, &swatches);
                for t in &scene.overlay.texts {
                    text::layout_glyphs(t, encode(t.color), font, &mut legend_glyphs);
                }
                self.has_overlay = !scene.overlay.is_empty();
            }
        }
        self.text
            .set_instances(&self.device, &self.queue, &legend_glyphs);
        self.text.set_overlay(&self.device, &self.queue, &[]);
        self.plot_rect = plan.map_or(scene.plot, |p| p.plot_at(0.0));
        self.bar_offset = 0;
        self.bar_count = 0;
        self.bar_end = 0;
        self.bar_slots.clear();
        self.label_plan = Some(label_plan);
        self.label_key = None;
        self.hover = None;
        self.mark_count = scene.mark_count();
        self.box_count = scene.boxes.len();
        self.slice_count = scene.slices.len();
        let entering = self.box_size.is_none();
        self.box_size = Some(plan.map_or(scene.size, |p| p.size_at(0.0)));
        // Everything the CPU tessellates: the plan's start, or the scene itself.
        match plan {
            Some(plan) => self.set_marks_3d(&plan.marks_at(0.0)),
            None => self.set_marks_3d(scene),
        }
        if entering {
            self.rebuild_targets(); // creates the depth buffer
        }
    }

    /// Sets the camera for the next `render`. Cheap: it only changes a uniform.
    pub fn set_camera(&mut self, camera: Camera) {
        self.camera = camera;
    }

    /// Marks bar `mark` (an index into the scene's bars) as hovered, or none: the bar is
    /// lightened, and a `hover`-mode value label of it is shown. Changes one flag per bar and
    /// a few glyphs; nothing else is rewritten.
    pub fn set_hover(&mut self, mark: Option<usize>) {
        let mark = mark.filter(|&m| m < self.mark_count);
        if mark == self.hover {
            return;
        }
        let previous = std::mem::replace(&mut self.hover, mark);
        let font = FontMetrics::inter();
        if let (Some(solid), Some(_)) = (&mut self.solid, self.box_size) {
            // Marks 0..box_count are boxes, then come the pie slices, then the line points.
            let counts = (
                self.box_count,
                self.slice_count,
                self.point_sizes.as_slice(),
            );
            let light = |solid: &mut Solid, mark: usize, on: bool| {
                light_3d(solid, &self.queue, counts, mark, on);
            };
            if let Some(p) = previous {
                light(solid, p, false);
            }
            let mut instances = Vec::new();
            if let Some(m) = mark {
                light(solid, m, true);
                if let Some(l) = self.hover_labels_3d.iter().find(|l| l.mark == m) {
                    text::layout_glyphs_3d(&l.text, l.color, NO_SLOT, font, &mut instances);
                    if l.text.halo {
                        let halo = text::halo_instances(&instances, self.halo.0, self.halo.1);
                        instances.splice(0..0, halo);
                    }
                }
            }
            solid
                .text
                .set_overlay(&self.device, &self.queue, &instances);
        } else {
            // Marks 0..bar_count are bars (a flag in the rect buffer); then come the line
            // points and the pie slices, which get something drawn over them instead.
            if let Some(p) = previous.filter(|&p| p < self.bar_count) {
                self.rects
                    .set_hover(&self.queue, self.bar_instance(p), false);
            }
            let mut glyphs = Vec::new();
            let mut marker = MeshData::default();
            if let Some(m) = mark {
                let points = self.bar_count + self.point_marks.len();
                if m < self.bar_count {
                    self.rects
                        .set_hover(&self.queue, self.bar_instance(m), true);
                } else if m < points {
                    let p = &self.point_marks[m - self.bar_count];
                    let background = self.encode_color(self.background);
                    // Bigger than the plain marker, with the same rim.
                    mesh::push_marker(
                        &mut marker,
                        p,
                        HOVER_MARKER_SCALE,
                        background,
                        self.encode_color(p.color),
                    );
                    marker.ranges.push(0..marker.vertices.len() as u32);
                } else if let Some(a) = self.arc_marks.get(m - points) {
                    // The slice again, a little lighter, exactly on top of itself.
                    let [r, g, b, alpha] = self.encode_color(a.color);
                    let lighter = |c: f32| c + (1.0 - c) * 0.12;
                    mesh::push_arc(&mut marker, a, [lighter(r), lighter(g), lighter(b), alpha]);
                    marker.ranges.push(0..marker.vertices.len() as u32);
                }
                if let Some(l) = self.hover_labels.iter().find(|l| l.mark == m) {
                    text::layout_glyphs(&l.text, l.color, font, &mut glyphs);
                }
            }
            self.hover_marker.set(&self.device, &self.queue, &marker);
            self.text.set_overlay(&self.device, &self.queue, &glyphs);
        }
    }

    /// The rect instance of bar `i` of the scene.
    fn bar_instance(&self, i: usize) -> usize {
        self.bar_offset + self.bar_slots.get(i).copied().unwrap_or(i)
    }

    /// The mark of `scene` (a bar, or a pie slice after the bars) under pixel `(x, y)` of the
    /// surface, in the current camera: a ray through the pixel against every box and every
    /// slice's top face, nearest wins. `None` in a 2D chart.
    pub fn pick_3d(&self, scene: &Scene3d, x: f32, y: f32) -> Option<usize> {
        let size = self.box_size?;
        // The plot has its own part of the canvas (a legend takes a band).
        let [ox, oy, w, h] = self.plot_viewport((self.config.width, self.config.height));
        pick::pick_scene(
            scene,
            self.camera.plot_view_proj(size, (w, h)),
            (w, h),
            (x - ox, y - oy),
        )
    }

    /// Where a world point is on the surface right now, in pixels. `None` in a 2D chart.
    pub fn project_3d(&self, point: [f32; 3]) -> Option<[f32; 2]> {
        let size = self.box_size?;
        let [ox, oy, w, h] = self.plot_viewport((self.config.width, self.config.height));
        let vp = self.camera.plot_view_proj(size, (w, h));
        pick::project(vp, (w, h), Vec3::from(point)).map(|(x, y)| [x + ox, y + oy])
    }

    /// The largest width or height a surface or snapshot may have.
    pub fn max_dimension(&self) -> u32 {
        self.max_dimension
    }

    /// The globals for a frame drawn into a target of `target` pixels, from the camera.
    fn globals_3d(&self, size: [f32; 3], target: (u32, u32)) -> Globals3 {
        let (centre, radius) = Camera::plot_frame(size);
        // The plot is drawn in its own part of the canvas (see `plot_viewport`).
        let [_, _, w, h] = self.plot_viewport(target);
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
            anim: self.anim,
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
                    let [_, _, w, h] = self.plot_viewport(target);
                    let viewport = (w, h);
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
                anim: self.anim,
            }),
        );
    }

    /// Records the chart pass. 2D: the background, then gridlines, bars, axis rules and text.
    /// 3D: bars (depth-tested), then lines (depth-tested against the bars), then text on top.
    ///
    /// Draws into `color`; with MSAA that is the multisampled target and `resolve` is where
    /// it is averaged down to. `overlay: false` leaves out the hover label.
    fn record_pass(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        color: &wgpu::TextureView,
        resolve: Option<&wgpu::TextureView>,
        depth: Option<&wgpu::TextureView>,
        frame: PassSpec,
    ) {
        let PassSpec {
            clear,
            overlay,
            size: target,
        } = frame;
        let [r, g, b, a] = clear;
        // A 3D chart with a legend is drawn in two passes (the 3D scene with its depth
        // buffer, then the legend without one), so the first must keep its colour for the
        // second to load, and only the last resolves.
        let two_passes = self.box_size.is_some() && self.has_overlay;
        let (resolve_now, store) = match resolve {
            // The multisampled contents are thrown away once resolved.
            Some(_) if !two_passes => (resolve, wgpu::StoreOp::Discard),
            _ => (None, wgpu::StoreOp::Store),
        };
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("chart pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: color,
                depth_slice: None,
                resolve_target: resolve_now,
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
                // The plot is framed in the part of the canvas the legend leaves.
                let [x, y, w, h] = self.plot_viewport(target);
                pass.set_viewport(x, y, w, h, 0.0, 1.0);
                solid.boxes.draw(&mut pass);
                solid.slices.draw(&mut pass);
                solid.lines.draw(&mut pass, &self.quad);
                // Line layers: the curtains first (translucent, under everything of the
                // line), then the segments, then the round joins and the markers on top.
                solid.curtains.draw(&mut pass);
                solid.data_lines.draw(&mut pass, &self.quad);
                solid.points.draw(&mut pass, &self.quad);
                // Labels are drawn last and ignore depth: they sit just outside the box,
                // and a label hidden behind a bar is worse than one drawn over it.
                solid.text.draw(&mut pass, &self.quad, overlay);
            }
            // No depth test in 2D: draw order alone decides what is on top.
            _ => {
                // Back to front: gridlines and bars, then pie slices, the line layers and their markers,
                // then axis lines and ticks, the hover marker, and text on top.
                let split = self.bar_end as u32;
                self.rects.draw_range(&mut pass, &self.quad, 0..split);
                self.arcs.draw(&mut pass, &self.mesh);
                self.lines.draw(&mut pass, &self.mesh);
                self.markers.draw(&mut pass, &self.mesh);
                self.rects
                    .draw_range(&mut pass, &self.quad, split..u32::MAX);
                if overlay {
                    self.hover_marker.draw(&mut pass, &self.mesh);
                }
                self.text.draw(&mut pass, &self.quad, overlay);
            }
        }
        drop(pass);
        if two_passes {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("legend pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: color,
                    depth_slice: None,
                    resolve_target: resolve,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: if resolve.is_some() {
                            wgpu::StoreOp::Discard
                        } else {
                            wgpu::StoreOp::Store
                        },
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            self.rects.draw_range(&mut pass, &self.quad, 0..u32::MAX);
            self.text.draw(&mut pass, &self.quad, false);
        }
    }

    /// The part of a `target`-sized canvas the 3D plot is drawn in, `[x, y, w, h]`, clamped
    /// to the canvas. The whole canvas unless a legend takes a band of it.
    fn plot_viewport(&self, target: (u32, u32)) -> [f32; 4] {
        let (tw, th) = (target.0.max(1) as f32, target.1.max(1) as f32);
        let [x, y, w, h] = self.plot_rect.unwrap_or([0.0, 0.0, tw, th]);
        let (x, y) = (x.clamp(0.0, tw - 1.0), y.clamp(0.0, th - 1.0));
        [x, y, w.clamp(1.0, tw - x), h.clamp(1.0, th - y)]
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
        self.record_pass(
            &mut encoder,
            view,
            resolve,
            self.depth_view.as_ref(),
            PassSpec {
                clear,
                overlay: true,
                size: (self.config.width, self.config.height),
            },
        );
        self.queue.submit([encoder.finish()]);
        self.queue.present(frame);
        Ok(())
    }

    /// Draws the loaded scene, from the current camera, into a fresh `width` x `height`
    /// texture (its own multisampled colour, resolve and depth targets, same format as the
    /// surface so the same pipelines and colour handling apply) and starts copying it back.
    /// The surface is untouched. With `transparent` the background is clear instead of the
    /// theme's.
    ///
    /// The caller loads the scene laid out for this size first, and loads the on-screen one
    /// again afterwards; hover state is not drawn. Finish with
    /// [`poll_snapshot`](Self::poll_snapshot) until it yields the pixels.
    pub fn snapshot(
        &mut self,
        width: u32,
        height: u32,
        transparent: bool,
    ) -> Result<Snapshot, Error> {
        let fail = |m: &str| Error::RenderFailed(format!("snapshot: {m}"));
        if width == 0 || height == 0 || width.max(height) > self.max_dimension {
            return Err(fail(&format!(
                "size {width}x{height} is outside 1..={} (the device's texture limit)",
                self.max_dimension
            )));
        }
        use wgpu::TextureFormat as F;
        let (bgra, srgb) = match self.config.format {
            F::Rgba8Unorm => (false, false),
            F::Rgba8UnormSrgb => (false, true),
            F::Bgra8Unorm => (true, false),
            F::Bgra8UnormSrgb => (true, true),
            _ => return Err(fail("the surface format is not an 8-bit RGBA or BGRA one")),
        };
        let size = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };
        let texture = |label, format, samples, usage| {
            self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size,
                mip_level_count: 1,
                sample_count: samples,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            })
        };
        let format = self.config.format;
        let resolved = texture(
            "snapshot colour",
            format,
            1,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        );
        let resolved_view = resolved.create_view(&wgpu::TextureViewDescriptor::default());
        let msaa_view = (self.samples > 1).then(|| {
            texture(
                "snapshot msaa",
                format,
                self.samples,
                wgpu::TextureUsages::RENDER_ATTACHMENT,
            )
            .create_view(&wgpu::TextureViewDescriptor::default())
        });
        let depth_view = self.box_size.map(|_| {
            texture(
                "snapshot depth",
                DEPTH_FORMAT,
                self.samples,
                wgpu::TextureUsages::RENDER_ATTACHMENT,
            )
            .create_view(&wgpu::TextureViewDescriptor::default())
        });

        let padded_row = image::padded_bytes_per_row(width);
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("snapshot readback"),
            size: u64::from(padded_row) * u64::from(height),
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        self.write_frame_uniforms((width, height));
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("snapshot"),
            });
        let clear = if transparent {
            [0.0; 4]
        } else {
            self.encode_color(self.background)
        };
        let (view, resolve) = match &msaa_view {
            Some(msaa) => (msaa, Some(&resolved_view)),
            None => (&resolved_view, None),
        };
        self.record_pass(
            &mut encoder,
            view,
            resolve,
            depth_view.as_ref(),
            PassSpec {
                clear,
                overlay: false,
                size: (width, height),
            },
        );
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &resolved,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded_row),
                    rows_per_image: Some(height),
                },
            },
            size,
        );
        self.queue.submit([encoder.finish()]);
        // The next on-screen frame must rewrite the globals and label table for its own size.
        self.label_key = None;

        let state = Arc::new(AtomicU8::new(PENDING));
        let done = Arc::clone(&state);
        buffer
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                done.store(
                    if result.is_ok() { READY } else { FAILED },
                    Ordering::Release,
                );
            });
        Ok(Snapshot {
            buffer,
            width,
            height,
            padded_row,
            format: ReadbackFormat {
                bgra,
                srgb,
                premultiplied: transparent,
            },
            state,
        })
    }

    /// Checks on a snapshot: `Ok(None)` while the GPU is still copying (call again later,
    /// yielding to the event loop in between: in browsers the copy finishes there), or the
    /// tightly packed RGBA8 pixels, rows top to bottom, not premultiplied.
    pub fn poll_snapshot(&self, snapshot: &Snapshot) -> Result<Option<Vec<u8>>, Error> {
        // WebGPU resolves the mapping from the event loop; the GL backend needs this nudge.
        let _ = self.device.poll(wgpu::PollType::Poll);
        match snapshot.state.load(Ordering::Acquire) {
            PENDING => Ok(None),
            READY => {
                let pixels = {
                    let mapped = snapshot
                        .buffer
                        .slice(..)
                        .get_mapped_range()
                        .map_err(|e| Error::RenderFailed(format!("snapshot: {e}")))?;
                    image::to_rgba(
                        &mapped,
                        snapshot.width,
                        snapshot.height,
                        snapshot.padded_row,
                        snapshot.format,
                    )
                };
                snapshot.buffer.unmap();
                Ok(Some(pixels))
            }
            _ => Err(Error::RenderFailed(
                "snapshot: reading the image back from the GPU failed".into(),
            )),
        }
    }
}

const PENDING: u8 = 0;
const READY: u8 = 1;
const FAILED: u8 = 2;

/// A snapshot being copied back from the GPU; see [`Renderer::snapshot`].
#[derive(Debug)]
pub struct Snapshot {
    buffer: wgpu::Buffer,
    width: u32,
    height: u32,
    padded_row: u32,
    format: ReadbackFormat,
    state: Arc<AtomicU8>,
}

impl Snapshot {
    /// Width and height in pixels.
    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }
}

/// Lights mark `mark` of a 3D scene (a box, then a pie slice, then a line point) up or not.
/// `(boxes, slices, points)` say how many of each kind there are and, for the points, how big
/// each marker is and whether it is drawn at rest.
fn light_3d(
    solid: &mut Solid,
    queue: &wgpu::Queue,
    (boxes, slices, points): (usize, usize, &[(f32, bool)]),
    mark: usize,
    on: bool,
) {
    if mark < boxes {
        solid.boxes.set_hover(queue, mark, on);
    } else if mark < boxes + slices {
        solid.slices.set_hover(queue, mark - boxes, on);
    } else if let Some(&(size_px, visible)) = points.get(mark - boxes - slices) {
        // A point's marker grows (and appears, if it was not drawn).
        let (size, inner) = solid::marker_sizes(size_px, visible, on);
        solid
            .points
            .set_marker(queue, mark - boxes - slices, size, inner);
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
