//! JS-facing API. Everything exported here becomes part of the npm package's wasm module;
//! the idiomatic TypeScript wrapper lives in `packages/facetry`.

use facetry_core::spec::{Column, Dimension};
use facetry_core::value::MAX_DEPTH;
use facetry_core::{
    ChartSpec, Color, Issue, Scene, Scene3d, Theme, Value, build_scene_3d_with_theme,
    build_scene_with_theme, parse_spec_value, set_series_visible, settle_hidden, validate,
};
use facetry_font::FontMetrics;
use facetry_render::{
    BackendPreference, CLOSE_UP_DISTANCE, Camera, MIN_DISTANCE, Projection, Renderer, Snapshot,
};
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

/**
 * The 3D camera right now (see `Chart.camera`). Angles are degrees; `distance` is in plot-box
 * radii; `pan` is how far the point looked at has been moved from the plot's centre, `[x, z]`
 * in world units on the floor (`[0, 0]` until the viewer pans).
 */
export interface CameraState {
    type: "perspective" | "orthographic";
    azimuth: number;
    elevation: number;
    distance: number;
    pan: [number, number];
}

/** What `Chart.setCamera` accepts: any subset. Values are clamped to the supported range. */
export type CameraUpdate = Partial<CameraState>;

/**
 * A mark (a bar, a line's data point, a pie slice or a diagram node), as hover and click
 * events report it.
 */
export interface MarkInfo {
    /**
     * The mark's key (`layer0/Q3`, `layer1/Q3/Pro`, or whatever `encoding.key` made of it); a
     * diagram node's id.
     */
    key: string;
    /** Index into the spec's `layers`; `-1` for a diagram node. */
    layer: number;
    /**
     * The mark's row of the layer's table, as a plain object (`{ quarter: "Q3", revenue: 140 }`).
     * A mark that stands for several rows that were summed (stacked, normalised and grouped
     * bars, pie slices) reports the first of them with the summed value in place of its y (or
     * theta) value. A diagram node: `{ id, label, shape }`.
     */
    datum: Record<string, number | string | null>;
}

/** The size, in pixels, of the image `Chart.snapshotStart` is producing. */
export interface SnapshotMeta {
    width: number;
    height: number;
}
"#;

#[wasm_bindgen]
extern "C" {
    /// Plain JS object; fields are read by hand.
    #[wasm_bindgen(typescript_type = "ChartOptions")]
    pub type ChartOptions;
}

#[wasm_bindgen(start)]
fn start() {
    console_error_panic_hook::set_once();
}

/// An `Error` named `FacetryError` carrying a machine-readable `code` property
/// (`"gpu-unavailable"`, `"invalid-options"`, `"invalid-spec"`, `"render-failed"`).
/// `gpu-unavailable` also covers asking this module for a backend it was built without.
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
    let list = js_sys::Array::new();
    for issue in issues {
        let item = js_sys::Object::new();
        let _ = js_sys::Reflect::set(&item, &"path".into(), &issue.path.as_str().into());
        let _ = js_sys::Reflect::set(&item, &"message".into(), &issue.message.as_str().into());
        list.push(&item);
    }
    let _ = js_sys::Reflect::set(&error, &"issues".into(), &list);
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

/// Copies a JS value into the plain tree the spec reader works on (`facetry_core::Value`).
///
/// JSON-like data is followed: numbers (a safe integer becomes `Int`, as JSON would have it),
/// strings, booleans, plain objects (own enumerable keys, in order) and arrays. Other
/// iterables, typed arrays for instance, count as arrays. `null` and `undefined` are both
/// `Null`. Functions, symbols and BigInts become `Value::Other`, which the reader rejects
/// at their path. Nesting past `MAX_DEPTH` (or a cycle) does too.
fn value_from_js(v: &JsValue, depth: usize) -> Value {
    // Numbers first: columns are mostly numbers and this runs once per cell.
    if let Some(n) = v.as_f64() {
        return if js_sys::Number::is_safe_integer(v) {
            Value::Int(n as i64)
        } else {
            Value::Float(n)
        };
    }
    if let Some(s) = v.as_string() {
        return Value::Str(s);
    }
    if let Some(b) = v.as_bool() {
        return Value::Bool(b);
    }
    if v.is_null() || v.is_undefined() {
        return Value::Null;
    }
    if v.is_bigint() {
        return Value::Other("bigint");
    }
    if !v.is_object() {
        return Value::Other(if v.is_function() {
            "function"
        } else {
            "symbol"
        });
    }
    if depth >= MAX_DEPTH {
        return Value::Other("a value nested too deeply");
    }
    let items = |iter: js_sys::IntoIter| {
        Value::Array(
            iter.map(|r| {
                r.map_or(Value::Other("an iterator that threw"), |x| {
                    value_from_js(&x, depth + 1)
                })
            })
            .collect(),
        )
    };
    if js_sys::Array::is_array(v) {
        return Value::Array(
            js_sys::Array::from(v)
                .iter()
                .map(|x| value_from_js(&x, depth + 1))
                .collect(),
        );
    }
    if let Ok(Some(iter)) = js_sys::try_iter(v) {
        return items(iter);
    }
    Value::Object(
        js_sys::Object::entries(v.unchecked_ref())
            .iter()
            .map(|pair| {
                let pair: js_sys::Array = pair.unchecked_into();
                let key = pair.get(0).as_string().unwrap_or_default();
                (key, value_from_js(&pair.get(1), depth + 1))
            })
            .collect(),
    )
}

/// Parses and validates a spec passed from JS (a plain object).
fn read_spec(value: JsValue) -> Result<ChartSpec, JsValue> {
    let spec = parse_spec_value(value_from_js(&value, 0)).map_err(|i| invalid_spec(&i))?;
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
    /// What is on screen, kept for picking and for putting back after a snapshot.
    scene: SceneKind,
    /// The bar under the pointer (an index into the scene's bars).
    hover: Option<usize>,
    /// A snapshot being read back from the GPU.
    pending: Option<Snapshot>,
}

/// The laid-out chart, 2D or 3D.
#[derive(Debug)]
enum SceneKind {
    TwoD(Scene),
    ThreeD(Scene3d),
}

/// Snapshots are at most this many times the CSS size: beyond that the picture is huge and
/// the extra pixels buy nothing a viewer could see.
const MAX_SNAPSHOT_SCALE: f64 = 4.0;

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
        let mut spec = read_spec(spec)?;
        settle_hidden(&mut spec, None);
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
        let camera = start_camera(&spec, renderer.size(), dpr as f32);
        let mut chart = Chart {
            renderer,
            camera,
            spec,
            theme,
            dpr: dpr as f32,
            scene: SceneKind::TwoD(Scene::empty()),
            hover: None,
            pending: None,
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
        let mut spec = read_spec(spec)?;
        // What the viewer hid stays hidden, for series that still exist, unless the new spec
        // says (`legend.hidden`).
        settle_hidden(&mut spec, self.spec.hidden_override.as_deref());
        let (size, dpr) = (self.renderer.size(), self.dpr);
        if spec.start_camera(size, dpr) != self.spec.start_camera(size, dpr) {
            self.camera = start_camera(&spec, size, dpr);
        }
        self.spec = spec;
        self.relayout();
        Ok(())
    }

    /// Hides (`visible = false`) or shows one series: its marks leave every layer that
    /// colours by that category, and stacks, shares, pies and axes are computed again without
    /// it. The legend entry stays, dimmed. Returns whether anything changed: `false` for an
    /// unknown name, a series already in that state, and for hiding the last visible series
    /// (which is refused). The camera is kept. Call `render` afterwards.
    #[wasm_bindgen(js_name = setSeriesVisible)]
    pub fn set_series_visible(&mut self, name: &str, visible: bool) -> bool {
        let changed = set_series_visible(&mut self.spec, name, visible);
        if changed {
            self.relayout();
        }
        changed
    }

    /// The names of the hidden series, in legend order.
    #[wasm_bindgen(getter, js_name = hiddenSeries, unchecked_return_type = "string[]")]
    pub fn hidden_series(&self) -> JsValue {
        self.spec
            .hidden_series()
            .iter()
            .map(|s| JsValue::from_str(s))
            .collect::<js_sys::Array>()
            .into()
    }

    /// The series whose legend entry is at `(x, y)` (CSS pixels from the canvas's top-left),
    /// or `undefined`: no legend, `legend.interactive: false`, or nothing there. The legend is
    /// a few rectangles on the CPU, so this is as cheap as `pick`.
    #[wasm_bindgen(js_name = legendPick)]
    pub fn legend_pick(&self, x: f64, y: f64) -> Option<String> {
        let (x, y) = (
            (x * f64::from(self.dpr)) as f32,
            (y * f64::from(self.dpr)) as f32,
        );
        match &self.scene {
            SceneKind::TwoD(scene) => scene.legend_pick(x, y),
            SceneKind::ThreeD(scene) => scene.legend_pick(x, y),
        }
        .map(str::to_owned)
    }

    /// Where the legend entry of `series` is: `{ x, y }`, its middle in CSS pixels from the
    /// canvas's top-left, or `undefined` (no such entry, or the legend is not interactive).
    /// For tests and for anchoring your own annotations.
    #[wasm_bindgen(js_name = legendPosition, unchecked_return_type = "{ x: number, y: number } | undefined")]
    pub fn legend_position(&self, series: &str) -> JsValue {
        let centre = match &self.scene {
            SceneKind::TwoD(scene) => scene.legend_center(series),
            SceneKind::ThreeD(scene) => scene.legend_center(series),
        };
        let Some([x, y]) = centre else {
            return JsValue::UNDEFINED;
        };
        let dpr = f64::from(self.dpr);
        let object = js_sys::Object::new();
        let _ = js_sys::Reflect::set(&object, &"x".into(), &(f64::from(x) / dpr).into());
        let _ = js_sys::Reflect::set(&object, &"y".into(), &(f64::from(y) / dpr).into());
        object.into()
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

    /// Zooms by `delta` (as `zoom`) toward the point under `(x, y)` (CSS pixels from the
    /// canvas's top-left): that point of the plane through the point looked at stays under
    /// the pointer, as in a map. Call `render` afterwards.
    #[wasm_bindgen(js_name = zoomAt)]
    pub fn zoom_at(&mut self, delta: f64, x: f64, y: f64) {
        if !self.is_3d() {
            return;
        }
        let s = f64::from(self.dpr);
        let (x, y) = ((x * s) as f32, (y * s) as f32);
        let before = self.renderer.plane_point_3d(x, y);
        self.camera.zoom(delta as f32);
        self.renderer.set_camera(self.camera);
        let after = self.renderer.plane_point_3d(x, y);
        if let (Some(b), Some(a), Some(size)) = (before, after, self.renderer.box_size()) {
            self.camera.move_pan([b[0] - a[0], b[2] - a[2]], size);
            self.renderer.set_camera(self.camera);
        }
    }

    /// Pans by a pointer drag of `(dx, dy)` CSS pixels: the floor follows the pointer. The
    /// point looked at stays inside the plot box. Call `render` afterwards.
    pub fn pan(&mut self, dx: f64, dy: f64) {
        let Some(size) = self.renderer.box_size() else {
            return;
        };
        if self.is_3d() {
            let s = f64::from(self.dpr);
            let height = self.renderer.size().1 as f32;
            self.camera
                .pan_by((dx * s) as f32, (dy * s) as f32, height, size);
            self.renderer.set_camera(self.camera);
        }
    }

    /// Back to the spec's camera (or the defaults).
    #[wasm_bindgen(js_name = resetCamera)]
    pub fn reset_camera(&mut self) {
        if self.is_3d() {
            self.camera = start_camera(&self.spec, self.renderer.size(), self.dpr);
            self.renderer.set_camera(self.camera);
        }
    }

    /// Sets any of `type`, `azimuth`, `elevation`, `distance` and `pan` (`[x, z]`); values are
    /// clamped (the pan to the plot box). Throws `invalid-options` if a field has the wrong
    /// type.
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
        let pan = js_sys::Reflect::get(&update, &"pan".into())?;
        let pan = if pan.is_undefined() || pan.is_null() {
            None
        } else {
            let array = js_sys::Array::from(&pan);
            let x = array.get(0).as_f64();
            let z = array.get(1).as_f64();
            match (js_sys::Array::is_array(&pan), x, z, array.length()) {
                (true, Some(x), Some(z), 2) => Some([x as f32, z as f32]),
                _ => return Err(invalid_options("camera `pan` must be [x, z] (two numbers)")),
            }
        };
        if self.is_3d() {
            if let Some(p) = projection {
                self.camera.projection = p;
            }
            self.camera.set(azimuth, elevation, distance);
            if let (Some([x, z]), Some(size)) = (pan, self.renderer.box_size()) {
                let current = self.camera.pan;
                self.camera.move_pan([x - current[0], z - current[1]], size);
            }
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
        let pan = js_sys::Array::of2(&round(c.pan[0]).into(), &round(c.pan[1]).into());
        set("pan", pan.into());
        object.into()
    }

    /// The bar under the pointer at `(x, y)` (CSS pixels from the canvas's top-left), as an
    /// index to give `markInfo` and `setHover`; `-1` when there is none. A point-in-rectangle
    /// test in 2D, a ray through every bar in 3D; both are microseconds.
    pub fn pick(&self, x: f64, y: f64) -> i32 {
        let (x, y) = (
            (x * f64::from(self.dpr)) as f32,
            (y * f64::from(self.dpr)) as f32,
        );
        let hit = match &self.scene {
            SceneKind::TwoD(scene) => scene.pick(x, y),
            SceneKind::ThreeD(scene) => self.renderer.pick_3d(scene, x, y),
        };
        hit.map_or(-1, |i| i as i32)
    }

    /// Highlights mark `mark` (an index from `pick`; `-1` for none) and shows its value label
    /// if the layer's `label.show` is `"hover"`. Call `render` afterwards.
    #[wasm_bindgen(js_name = setHover)]
    pub fn set_hover(&mut self, mark: i32) {
        self.hover = usize::try_from(mark).ok();
        self.renderer.set_hover(self.hover);
    }

    /// `{ key, layer, datum }` for bar `mark` (an index from `pick`), or `null`.
    #[wasm_bindgen(js_name = markInfo, unchecked_return_type = "MarkInfo | null")]
    pub fn mark_info(&self, mark: i32) -> JsValue {
        let Ok(mark) = usize::try_from(mark) else {
            return JsValue::NULL;
        };
        if let SceneKind::ThreeD(s) = &self.scene {
            let i = mark.checked_sub(s.boxes.len() + s.slices.len());
            if let Some(prism) = i.and_then(|i| s.prisms.get(i)) {
                return self.node_info(prism.node);
            }
        }
        let (key, layer, row, sum) = match &self.scene {
            SceneKind::TwoD(s) => s.mark(mark).map(|m| (m.key, m.layer, m.row, m.sum)),
            // Boxes, then pie slices, then line points.
            SceneKind::ThreeD(s) => s
                .boxes
                .get(mark)
                .map(|b| (b.key.as_str(), b.layer, b.row, b.sum))
                .or_else(|| {
                    let i = mark.checked_sub(s.boxes.len())?;
                    s.slices
                        .get(i)
                        .map(|p| (p.key.as_str(), p.layer, p.row, p.sum))
                        .or_else(|| {
                            let i = i.checked_sub(s.slices.len() + s.prisms.len())?;
                            s.points
                                .get(i)
                                .map(|p| (p.key.as_str(), p.layer, p.row, None))
                        })
                }),
        }
        .map_or((None, 0, 0, None), |(k, l, r, s)| (Some(k), l, r, s));
        let Some(key) = key else {
            return JsValue::NULL;
        };
        let object = js_sys::Object::new();
        let set = |name: &str, value: JsValue| {
            // Setting a plain property on a fresh object cannot fail.
            let _ = js_sys::Reflect::set(&object, &name.into(), &value);
        };
        set("key", key.into());
        set("layer", (layer as f64).into());
        set("datum", self.datum(layer, row, sum));
        object.into()
    }

    /// Where the middle of the mark (a bar, or a line's data point) with this key is, `{ x, y }` in CSS pixels from the
    /// canvas's top-left (in 3D: where the box's centre projects, right now), or `undefined`
    /// for an unknown key. For tests and for anchoring your own annotations.
    #[wasm_bindgen(js_name = markPosition, unchecked_return_type = "{ x: number, y: number } | undefined")]
    pub fn mark_position(&self, key: &str) -> JsValue {
        let dpr = f64::from(self.dpr);
        let device = match &self.scene {
            SceneKind::TwoD(s) => s.mark_center(key),
            SceneKind::ThreeD(s) => {
                let centre = s
                    .boxes
                    .iter()
                    .find(|b| b.key == key)
                    .map(|b| [0, 1, 2].map(|i| (b.min[i] + b.max[i]) / 2.0))
                    .or_else(|| {
                        // A slice: the middle of its top face.
                        let p = s.slices.iter().find(|p| p.key == key)?;
                        let (a0, a1) = p.span();
                        Some(p.point((a0 + a1) / 2.0, (p.inner + p.outer) / 2.0, p.height))
                    })
                    // A diagram node: the middle of its top face.
                    .or_else(|| {
                        let p = s.prisms.iter().find(|p| p.key == key)?;
                        let [x, z] = p.centre();
                        Some([x, p.top, z])
                    })
                    // A line's data point.
                    .or_else(|| s.points.iter().find(|p| p.key == key).map(|p| p.pos));
                centre.and_then(|c| self.renderer.project_3d(c))
            }
        };
        let Some([x, y]) = device else {
            return JsValue::UNDEFINED;
        };
        let object = js_sys::Object::new();
        let _ = js_sys::Reflect::set(&object, &"x".into(), &(f64::from(x) / dpr).into());
        let _ = js_sys::Reflect::set(&object, &"y".into(), &(f64::from(y) / dpr).into());
        object.into()
    }

    /// Starts a PNG snapshot: lays the chart out for `scale` device pixels per CSS pixel
    /// (capped at 4 and at the GPU's texture limit), draws it from the current camera into an
    /// offscreen texture, and starts copying that back. The canvas is untouched and keeps its
    /// state. Returns the image's `{ width, height }`; call `snapshotPoll` until it yields the
    /// pixels. `transparent` leaves the background clear instead of the theme's.
    #[wasm_bindgen(js_name = snapshotStart, unchecked_return_type = "SnapshotMeta")]
    pub fn snapshot_start(&mut self, scale: f64, transparent: bool) -> Result<JsValue, JsValue> {
        if self.pending.is_some() {
            return Err(js_error(
                "render-failed",
                "a snapshot is already being read back",
            ));
        }
        if !(scale.is_finite() && scale > 0.0) {
            return Err(invalid_options("`scale` must be a positive number"));
        }
        let dpr = f64::from(self.dpr);
        let (css_w, css_h) = {
            let (w, h) = self.renderer.size();
            (f64::from(w) / dpr, f64::from(h) / dpr)
        };
        if css_w < 1.0 || css_h < 1.0 {
            return Err(js_error(
                "render-failed",
                "the chart has no size to snapshot",
            ));
        }
        let limit = f64::from(self.renderer.max_dimension());
        let scale = scale.min(MAX_SNAPSHOT_SCALE).min(limit / css_w.max(css_h));
        let size = (
            ((css_w * scale).round().max(1.0)).min(limit) as u32,
            ((css_h * scale).round().max(1.0)).min(limit) as u32,
        );

        // Lay the same chart out for the snapshot's size (text stays crisp at any scale),
        // draw it, then put the on-screen layout back whatever happened.
        let font = FontMetrics::inter();
        match &self.spec.dimension {
            Dimension::ThreeD => {
                let scene =
                    build_scene_3d_with_theme(&self.spec, size, scale as f32, font, &self.theme);
                self.renderer.set_scene_3d(&scene);
            }
            Dimension::TwoD => {
                let scene =
                    build_scene_with_theme(&self.spec, size, scale as f32, font, &self.theme);
                self.renderer.set_scene(&scene);
            }
        }
        let started = self.renderer.snapshot(size.0, size.1, transparent);
        self.reload_scene();
        let snapshot = started.map_err(|e| js_error(e.code(), &e.to_string()))?;
        self.pending = Some(snapshot);

        let object = js_sys::Object::new();
        let _ = js_sys::Reflect::set(&object, &"width".into(), &size.0.into());
        let _ = js_sys::Reflect::set(&object, &"height".into(), &size.1.into());
        Ok(object.into())
    }

    /// `undefined` while the snapshot started by `snapshotStart` is still being copied back
    /// (try again after yielding to the event loop); then its RGBA pixels, row by row, top to
    /// bottom, not premultiplied.
    #[wasm_bindgen(js_name = snapshotPoll, unchecked_return_type = "Uint8ClampedArray | undefined")]
    pub fn snapshot_poll(&mut self) -> Result<JsValue, JsValue> {
        let Some(snapshot) = &self.pending else {
            return Err(js_error("render-failed", "no snapshot was started"));
        };
        match self.renderer.poll_snapshot(snapshot) {
            Ok(None) => Ok(JsValue::UNDEFINED),
            Ok(Some(pixels)) => {
                self.pending = None;
                Ok(js_sys::Uint8ClampedArray::from(pixels.as_slice()).into())
            }
            Err(e) => {
                self.pending = None;
                Err(js_error(e.code(), &e.to_string()))
            }
        }
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

    /// spec + size -> scene -> GPU buffers. The hover is dropped: the bars may have moved or
    /// vanished (the TypeScript wrapper tells listeners and picks again).
    fn relayout(&mut self) {
        self.scene = if self.is_3d() {
            SceneKind::ThreeD(build_scene_3d_with_theme(
                &self.spec,
                self.renderer.size(),
                self.dpr,
                FontMetrics::inter(),
                &self.theme,
            ))
        } else {
            SceneKind::TwoD(build_scene_with_theme(
                &self.spec,
                self.renderer.size(),
                self.dpr,
                FontMetrics::inter(),
                &self.theme,
            ))
        };
        self.hover = None;
        self.reload_scene();
    }

    /// Uploads the current scene (and camera) to the GPU again, keeping the hover.
    fn reload_scene(&mut self) {
        match &self.scene {
            SceneKind::ThreeD(scene) => {
                self.renderer.set_scene_3d(scene);
                self.renderer.set_camera(self.camera);
            }
            SceneKind::TwoD(scene) => self.renderer.set_scene(scene),
        }
        self.renderer.set_hover(self.hover);
    }

    /// `{ key, layer: -1, datum: { id, label, shape } }` for diagram node `index`.
    fn node_info(&self, index: usize) -> JsValue {
        let Some(node) = self.spec.diagram.as_ref().and_then(|d| d.nodes.get(index)) else {
            return JsValue::NULL;
        };
        let object = js_sys::Object::new();
        let datum = js_sys::Object::new();
        let set = |o: &js_sys::Object, name: &str, value: JsValue| {
            let _ = js_sys::Reflect::set(o, &name.into(), &value);
        };
        set(&datum, "id", node.id.as_str().into());
        set(
            &datum,
            "label",
            node.label.as_deref().unwrap_or(&node.id).into(),
        );
        set(&datum, "shape", node.shape.name().into());
        set(&object, "key", node.id.as_str().into());
        set(&object, "layer", (-1.0).into());
        set(&object, "datum", datum.into());
        object.into()
    }

    /// The row of a layer's table as a plain JS object (numbers, strings, `null` for a
    /// missing number). When the mark stands for several merged rows (stacked, normalised
    /// and grouped bars add up rows with the same x and series) `sum` is the total, and it
    /// replaces the y value: the datum is "the first row, with the summed value".
    fn datum(&self, layer: usize, row: usize, sum: Option<f64>) -> JsValue {
        let object = js_sys::Object::new();
        let spec_layer = self.spec.layers.get(layer);
        let table = spec_layer.and_then(|l| self.spec.data.get(&l.data));
        // The field a merged mark's sum stands in for: y for bars, theta for pie slices.
        let y_field = spec_layer.and_then(|l| {
            let e = &l.encoding;
            e.y.as_ref()
                .map(|y| y.field.as_str())
                .or_else(|| e.theta.as_ref().map(|t| t.field.as_str()))
        });
        for (name, column) in table.into_iter().flatten() {
            if let (Some(sum), true) = (sum, Some(name.as_str()) == y_field) {
                let _ = js_sys::Reflect::set(&object, &name.as_str().into(), &sum.into());
                continue;
            }
            let value = match column {
                Column::Numeric(v) => v
                    .get(row)
                    .filter(|x| x.is_finite())
                    .map_or(JsValue::NULL, |&x| x.into()),
                Column::Categorical { codes, dict } => codes
                    .get(row)
                    .and_then(|&c| dict.get(c as usize))
                    .map_or(JsValue::NULL, |s| s.as_str().into()),
            };
            let _ = js_sys::Reflect::set(&object, &name.as_str().into(), &value);
        }
        object.into()
    }
}

/// The camera a spec starts from: its own (with a diagram's defaults, fitted to `size`), and
/// for a diagram the right to come much closer, to read a corner of a big one.
fn start_camera(spec: &ChartSpec, size: (u32, u32), dpr: f32) -> Camera {
    let min = if spec.diagram.is_some() {
        CLOSE_UP_DISTANCE
    } else {
        MIN_DISTANCE
    };
    Camera::from_spec_with_min(spec.start_camera(size, dpr).as_ref(), min)
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
