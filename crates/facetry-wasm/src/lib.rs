//! JS-facing API. Everything exported here becomes part of the npm package's wasm module;
//! the idiomatic TypeScript wrapper lives in `packages/facetry`.

use facetry_core::{Color, Theme};
use facetry_render::{BackendPreference, Renderer};
use wasm_bindgen::prelude::*;
use web_sys::HtmlCanvasElement;

#[wasm_bindgen(typescript_custom_section)]
const TS_TYPES: &str = r#"
export interface ChartOptions {
    /** GPU API to use. `"auto"` (default) picks WebGPU if available, else WebGL2. */
    backend?: "auto" | "webgpu" | "webgl2";
    /** Background as `#rrggbb` or `#rrggbbaa` (sRGB). */
    clearColor?: string;
}
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
/// (`"gpu-unavailable"`, `"invalid-options"`, `"render-failed"`).
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

/// A GPU-backed chart bound to one `<canvas>`.
#[wasm_bindgen]
#[derive(Debug)]
pub struct Chart {
    renderer: Renderer,
}

#[wasm_bindgen]
impl Chart {
    /// Creates a chart on `canvas`, using the canvas's current `width`/`height` as the
    /// initial backing-store size. Rejects with a `FacetryError` (see `code`) on failure.
    pub async fn create(
        canvas: HtmlCanvasElement,
        options: Option<ChartOptions>,
    ) -> Result<Chart, JsValue> {
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

        let renderer = renderer_for(canvas, preference, theme)
            .await
            .map_err(|e| js_error(e.code(), &e.to_string()))?;
        Ok(Chart { renderer })
    }

    /// Resizes the drawing buffer, in physical pixels.
    pub fn resize(&mut self, width: u32, height: u32) {
        self.renderer.resize(width, height);
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
}

#[cfg(target_arch = "wasm32")]
async fn renderer_for(
    canvas: HtmlCanvasElement,
    preference: BackendPreference,
    theme: Theme,
) -> Result<Renderer, facetry_render::Error> {
    Renderer::from_canvas(canvas, preference, theme).await
}

/// Canvas surfaces only exist in browsers; this keeps the crate buildable (and testable)
/// natively, which is what `cargo test --workspace` does.
#[cfg(not(target_arch = "wasm32"))]
async fn renderer_for(
    _canvas: HtmlCanvasElement,
    _preference: BackendPreference,
    _theme: Theme,
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
