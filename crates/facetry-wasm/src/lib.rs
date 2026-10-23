//! JS-facing API. Everything exported here becomes part of the npm package's wasm module;
//! the idiomatic TypeScript wrapper lives in `packages/facetry`.

use facetry_core::spec::Dimension;
use facetry_core::{
    ChartSpec, Color, Issue, Theme, build_scene_3d_with_theme, build_scene_with_theme, parse_spec,
    validate,
};
use facetry_font::FontMetrics;
use facetry_render::{BackendPreference, Camera, Projection, Renderer};
use wasm_bindgen::prelude::*;
use web_sys::HtmlCanvasElement;

#[wasm_bindgen(typescript_custom_section)]
const TS_TYPES: &str = r#"
export interface ChartOptions {
    /** GPU API to use. `"auto"` (default) picks WebGPU if available, else WebGL2. */
    backend?: "auto" | "webgpu" | "webgl2";
    /** Background colour override as `#rrggbb` or `#rrggbbaa` (sRGB). */
    clearColor?: string;
}

/** The 3D camera right now (see `Chart.camera`). Angles are degrees; `distance` is in plot-box radii. */
export interface CameraState {
    type: "perspective" | "orthographic";
    azimuth: number;
    elevation: number;
    distance: number;
}

/** What `Chart.setCamera` accepts: any subset. Values are clamped to the supported range. */
export type CameraUpdate = Partial<CameraState>;
"#;

#[wasm_bindgen]
extern "C" {
    /// Plain JS object; fields are read by hand to avoid pulling in serde.
    #[wasm_bindgen(typescript_type = "ChartOptions")]
    pub type ChartOptions;
}

#[wasm_bindgen(start)]
fn start() {
    console_error_panic_hook::set_once();
}

/// An `Error` named `FacetryError` carrying a machine-readable `code` property
/// (`"gpu-unavailable"`, `"invalid-options"`, `"invalid-spec"`, `"render-failed"`).
fn js_error(code: &str, message: &str) -> JsValue {
    let error = js_sys::Error::new(message);
    error.set_name("FacetryError");
    // Setting a plain property on a fresh object cannot fail.
    let _ = js_sys::Reflect::set(&error, &"code".into(), &code.into());
    error.into()
}

fn invalid_options(message: impl AsRef<str>) -> JsValue {
    js_error("invalid-options", message.as_ref())
}

/// The `invalid-spec` error: its `issues` property lists every problem as `{ path, message }`.
fn invalid_spec(issues: &[Issue]) -> JsValue {
    let summary = match issues {
        [only] => format!("invalid chart spec: {} ({})", only.message, only.path),
        many => format!(
            "invalid chart spec: {} issues, first: {} ({})",
            many.len(),
            many[0].message,
            many[0].path
        ),
    };
    let error = js_error("invalid-spec", &summary);
    if let Ok(list) = serde_wasm_bindgen::to_value(issues) {
        let _ = js_sys::Reflect::set(&error, &"issues".into(), &list);
    }
    error
}

fn parse_backend(s: &str) -> Option<BackendPreference> {
    match s {
        "auto" => Some(BackendPreference::Auto),
        "webgpu" => Some(BackendPreference::WebGpu),
        "webgl2" => Some(BackendPreference::WebGl2),
        _ => None,
    }
}

/// Reads an optional string property; `undefined`/`null` count as absent.
fn string_option(options: &JsValue, key: &str) -> Result<Option<String>, JsValue> {
    let value = js_sys::Reflect::get(options, &key.into())?;
    if value.is_undefined() || value.is_null() {
        return Ok(None);
    }
    value
        .as_string()
        .map(Some)
        .ok_or_else(|| invalid_options(format!("`{key}` must be a string")))
}

/// Parses and validates a spec passed from JS (a plain object, read via serde).
fn read_spec(value: JsValue) -> Result<ChartSpec, JsValue> {
    let spec =
        parse_spec(serde_wasm_bindgen::Deserializer::from(value)).map_err(|i| invalid_spec(&i))?;
    let issues = validate(&spec);
    if issues.is_empty() {
        Ok(spec)
    } else {
        Err(invalid_spec(&issues))
    }
}

/// A GPU-backed chart bound to one `<canvas>`.
#[wasm_bindgen]
#[derive(Debug)]
pub struct Chart {
    renderer: Renderer,
    spec: ChartSpec,
    theme: Theme,
    /// 3D only. Lives here, not in the scene, so orbiting never rebuilds the scene: the
    /// renderer just gets the new camera and the next frame uses it.
    camera: Camera,
    /// Device pixels per CSS pixel; scales fonts, margins and line widths in the layout.
    dpr: f32,
}

#[wasm_bindgen]
impl Chart {
    /// Creates a chart on `canvas`, using the canvas's current `width`/`height` as the
    /// drawing-buffer size and `dpr` as its device pixel ratio.
    ///
    /// Rejects with a `FacetryError` (see `code`) on failure; an invalid spec is
    /// `"invalid-spec"` with an `issues` list, and nothing is drawn.
    pub async fn create(
        canvas: HtmlCanvasElement,
        #[wasm_bindgen(unchecked_param_type = "ChartSpec")] spec: JsValue,
        options: Option<ChartOptions>,
        dpr: f64,
    ) -> Result<Chart, JsValue> {
        let spec = read_spec(spec)?;
        let mut preference = BackendPreference::Auto;
        let mut theme = Theme::default();
        if let Some(options) = options.as_deref() {
            if let Some(name) = string_option(options, "backend")? {
                preference = parse_backend(&name).ok_or_else(|| {
                    invalid_options(format!(
                        "`backend` must be \"auto\", \"webgpu\" or \"webgl2\", got {name:?}"
                    ))
                })?;
            }
            if let Some(hex) = string_option(options, "clearColor")? {
                let color = Color::from_hex(&hex).map_err(|e| invalid_options(e.to_string()))?;
                theme = Theme::with_background(color);
            }
        }

        let renderer = renderer_for(canvas, preference)
            .await
            .map_err(|e| js_error(e.code(), &e.to_string()))?;
        let mut chart = Chart {
            renderer,
            camera: Camera::from_spec(spec.camera.as_ref()),
            spec,
            theme,
            dpr: dpr as f32,
        };
        chart.relayout();
        Ok(chart)
    }

    /// Replaces the spec and re-lays out. On an invalid spec this throws `invalid-spec` and
    /// the previous spec stays in effect. Call `render` afterwards.
    ///
    /// The camera is kept (the user may have orbited) unless the new spec's `camera` differs
    /// from the old one's, in which case it restarts from the spec.
    pub fn update(
        &mut self,
        #[wasm_bindgen(unchecked_param_type = "ChartSpec")] spec: JsValue,
    ) -> Result<(), JsValue> {
        let spec = read_spec(spec)?;
        if spec.camera != self.spec.camera {
            self.camera = Camera::from_spec(spec.camera.as_ref());
        }
        self.spec = spec;
        self.relayout();
        Ok(())
    }

    /// Resizes the drawing buffer (physical pixels) and re-runs layout for the new size:
    /// margins, tick counts and label thinning all depend on it.
    pub fn resize(&mut self, width: u32, height: u32, dpr: f64) {
        self.dpr = dpr as f32;
        self.renderer.resize(width, height);
        self.relayout();
    }

    /// Whether this chart is 3D with orbiting enabled (`interaction.orbit`, default on).
    /// The TypeScript wrapper only attaches pointer handlers when this is true.
    #[wasm_bindgen(getter, js_name = orbitEnabled)]
    pub fn orbit_enabled(&self) -> bool {
        self.spec.orbit_enabled()
    }

    /// Turns the camera by a pointer drag of `(dx, dy)` pixels. Call `render` afterwards.
    pub fn orbit(&mut self, dx_px: f64, dy_px: f64) {
        if self.is_3d() {
            self.camera.orbit(dx_px as f32, dy_px as f32);
            self.renderer.set_camera(self.camera);
        }
    }

    /// Zooms by a wheel `delta` (positive: out, negative: in; a typical notch is about 100).
    pub fn zoom(&mut self, delta: f64) {
        if self.is_3d() {
            self.camera.zoom(delta as f32);
            self.renderer.set_camera(self.camera);
        }
    }

    /// Back to the spec's camera (or the defaults).
    #[wasm_bindgen(js_name = resetCamera)]
    pub fn reset_camera(&mut self) {
        if self.is_3d() {
            self.camera = Camera::from_spec(self.spec.camera.as_ref());
            self.renderer.set_camera(self.camera);
        }
    }

    /// Sets any of `type`, `azimuth`, `elevation`, `distance`; values are clamped. Throws
    /// `invalid-options` if a field has the wrong type.
    #[wasm_bindgen(js_name = setCamera)]
    pub fn set_camera(
        &mut self,
        #[wasm_bindgen(unchecked_param_type = "CameraUpdate")] update: JsValue,
    ) -> Result<(), JsValue> {
        let number = |key: &str| -> Result<Option<f32>, JsValue> {
            let v = js_sys::Reflect::get(&update, &key.into())?;
            if v.is_undefined() || v.is_null() {
                return Ok(None);
            }
            v.as_f64()
                .map(|n| Some(n as f32))
                .ok_or_else(|| invalid_options(format!("camera `{key}` must be a number")))
        };
        let (azimuth, elevation, distance) = (
            number("azimuth")?,
            number("elevation")?,
            number("distance")?,
        );
        let projection = match string_option(&update, "type")?.as_deref() {
            None => None,
            Some("perspective") => Some(Projection::Perspective),
            Some("orthographic") => Some(Projection::Orthographic),
            Some(other) => {
                return Err(invalid_options(format!(
                    "camera `type` must be \"perspective\" or \"orthographic\", got {other:?}"
                )));
            }
        };
        if self.is_3d() {
            if let Some(p) = projection {
                self.camera.projection = p;
            }
            self.camera.set(azimuth, elevation, distance);
            self.renderer.set_camera(self.camera);
        }
        Ok(())
    }

    /// The camera now: `{ type, azimuth, elevation, distance }` (the spec's or default
    /// values for a chart that was never orbited).
    #[wasm_bindgen(getter, unchecked_return_type = "CameraState")]
    pub fn camera(&self) -> JsValue {
        let c = &self.camera;
        let object = js_sys::Object::new();
        let set = |key: &str, value: JsValue| {
            // Setting a plain property on a fresh object cannot fail.
            let _ = js_sys::Reflect::set(&object, &key.into(), &value);
        };
        // The camera is stored as f32; widening 2.7f32 to f64 gives 2.700000047..., so round
        // to a precision far below anything visible.
        let round = |v: f32| (f64::from(v) * 1e4).round() / 1e4;
        set("type", c.projection.name().into());
        set("azimuth", round(c.azimuth).into());
        set("elevation", round(c.elevation).into());
        set("distance", round(c.distance).into());
        object.into()
    }

    /// Draws one frame.
    pub fn render(&mut self) -> Result<(), JsValue> {
        self.renderer
            .render()
            .map_err(|e| js_error(e.code(), &e.to_string()))
    }

    /// The backend in use: `"webgpu"` or `"webgl2"`.
    #[wasm_bindgen(getter)]
    pub fn backend(&self) -> String {
        self.renderer.backend().to_owned()
    }

    /// Multisampling in use: 4, or 1 if the device could not do 4x MSAA.
    #[wasm_bindgen(getter, js_name = msaaSamples)]
    pub fn msaa_samples(&self) -> u32 {
        self.renderer.msaa_samples()
    }
}

impl Chart {
    fn is_3d(&self) -> bool {
        self.spec.dimension == Dimension::ThreeD
    }

    /// spec + size -> scene -> GPU buffers.
    fn relayout(&mut self) {
        if self.is_3d() {
            let scene = build_scene_3d_with_theme(
                &self.spec,
                self.renderer.size(),
                self.dpr,
                FontMetrics::inter(),
                &self.theme,
            );
            self.renderer.set_scene_3d(&scene);
            self.renderer.set_camera(self.camera);
            return;
        }
        let scene = build_scene_with_theme(
            &self.spec,
            self.renderer.size(),
            self.dpr,
            FontMetrics::inter(),
            &self.theme,
        );
        self.renderer.set_scene(&scene);
    }
}

#[cfg(target_arch = "wasm32")]
async fn renderer_for(
    canvas: HtmlCanvasElement,
    preference: BackendPreference,
) -> Result<Renderer, facetry_render::Error> {
    Renderer::from_canvas(canvas, preference).await
}

/// Canvas surfaces only exist in browsers; this keeps the crate buildable (and testable)
/// natively, which is what `cargo test --workspace` does.
#[cfg(not(target_arch = "wasm32"))]
async fn renderer_for(
    _canvas: HtmlCanvasElement,
    _preference: BackendPreference,
) -> Result<Renderer, facetry_render::Error> {
    Err(facetry_render::Error::GpuUnavailable(
        "canvas rendering requires wasm32".into(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_names() {
        assert_eq!(parse_backend("auto"), Some(BackendPreference::Auto));
        assert_eq!(parse_backend("webgpu"), Some(BackendPreference::WebGpu));
        assert_eq!(parse_backend("webgl2"), Some(BackendPreference::WebGl2));
        assert_eq!(parse_backend("vulkan"), None);
    }
}
