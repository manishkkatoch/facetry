use facetry_core::Theme;

use crate::Error;

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

impl BackendPreference {
    fn backends(self) -> wgpu::Backends {
        match self {
            Self::Auto => wgpu::Backends::PRIMARY | wgpu::Backends::GL,
            Self::WebGpu => wgpu::Backends::BROWSER_WEBGPU,
            Self::WebGl2 => wgpu::Backends::GL,
        }
    }
}

/// Owns the GPU objects (instance, adapter info, device, queue) and the window surface.
#[derive(Debug)]
pub struct Renderer {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    backend: wgpu::Backend,
    theme: Theme,
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
        theme: Theme,
    ) -> Result<Self, Error> {
        let unavailable =
            |what: &str, e: &dyn std::fmt::Display| Error::GpuUnavailable(format!("{what}: {e}"));

        let mut desc = wgpu::InstanceDescriptor::new_without_display_handle();
        desc.backends = preference.backends();
        // `Instance::new` is sync and cannot tell whether `navigator.gpu` really yields an
        // adapter, so for the web this probes first and drops WebGPU if it doesn't.
        let instance = wgpu::util::new_instance_with_webgpu_detection(desc).await;

        let surface = instance
            .create_surface(target)
            .map_err(|e| unavailable("creating surface", &e))?;
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                compatible_surface: Some(&surface),
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

        let mut config = surface.get_default_config(&adapter, 1, 1).ok_or_else(|| {
            Error::GpuUnavailable("surface is incompatible with the adapter".into())
        })?;
        // Prefer an sRGB format so the GPU gamma-encodes our linear colours for us.
        let caps = surface.get_capabilities(&adapter);
        if let Some(format) = caps.formats.iter().copied().find(|f| f.is_srgb()) {
            config.format = format;
        }

        let mut renderer = Self {
            surface,
            device,
            queue,
            config,
            backend: adapter.get_info().backend,
            theme,
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
        theme: Theme,
    ) -> Result<Self, Error> {
        let (width, height) = (canvas.width(), canvas.height());
        Self::new(
            wgpu::SurfaceTarget::Canvas(canvas),
            width,
            height,
            preference,
            theme,
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
    }

    /// Draws one frame: clears to the theme background.
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

        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("clear"),
            });
        // Clear values are written to the target as-is: linear for sRGB formats (the GPU
        // encodes), but already gamma-encoded for plain formats such as `Rgba8Unorm`.
        let [r, g, b, a] = if self.config.format.is_srgb() {
            self.theme.background.to_array()
        } else {
            self.theme.background.to_srgb()
        };
        // Dropping the pass ends it; clearing needs no draw calls.
        drop(encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("clear pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: f64::from(r),
                        g: f64::from(g),
                        b: f64::from(b),
                        a: f64::from(a),
                    }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        }));
        self.queue.submit([encoder.finish()]);
        self.queue.present(frame);
        Ok(())
    }
}
