//! JS-facing API. Everything exported here becomes part of the npm package's wasm module;
//! the idiomatic TypeScript wrapper lives in `packages/facetry`.

use facetry_core::spec::{Column, Dimension, Timing, TransitionSpec, resolve_transition};
use facetry_core::transition::Plan;
use facetry_core::transition3d::{Plan3, blank_like as blank_like_3d};
use facetry_core::value::MAX_DEPTH;
use facetry_core::{
    ChartSpec, Color, Issue, Scene, Scene3d, Theme, Value, build_scene_3d_with_theme,
    build_scene_with_theme, parse_spec_value, parse_transition_value, set_series_visible,
    settle_hidden, validate, validate_transition,
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
    /**
     * Animate the first frame too (marks enter as they would after an update). Default: `false`,
     * the chart is simply there. Ignored when the viewer prefers reduced motion.
     */
    animateIn?: boolean;
}

/**
 * What `Chart.update` and `Chart.setSeriesVisible` take besides the spec: a `transition`
 * that replaces the spec's own for this one call. `false` jumps, an object sets the duration
 * (ms), `easing` and `stagger` (ms).
 */
export interface UpdateOptions {
    transition?: TransitionSpec;
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
 * Turns `Chart.setCamera` / `Chart.resetCamera` into a tween: how long it takes (ms, default
 * 400; 0 jumps) and the speed curve (default `"cubicInOut"`). Azimuth takes the shortest way
 * round.
 */
export interface CameraTweenOptions {
    duration?: number;
    easing?: Easing;
}

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

/// Reads an optional boolean property; `undefined`/`null` count as absent.
fn bool_option(options: &JsValue, key: &str) -> Result<Option<bool>, JsValue> {
    let value = js_sys::Reflect::get(options, &key.into())?;
    if value.is_undefined() || value.is_null() {
        return Ok(None);
    }
    value
        .as_bool()
        .map(Some)
        .ok_or_else(|| invalid_options(format!("`{key}` must be a boolean")))
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

/// The timing of a camera tween from its `options` (`{ duration, easing }`): `None` for no
/// tween (no options, `false`, or a duration of 0).
fn tween_timing(options: &JsValue) -> Result<Option<Timing>, JsValue> {
    Ok(transition_option(options)?.flatten())
}

/// How an `update` call's `transition` option reads: absent (use the spec's), or what to use.
fn transition_option(value: &JsValue) -> Result<Option<Option<Timing>>, JsValue> {
    if value.is_undefined() || value.is_null() {
        return Ok(None);
    }
    let spec: TransitionSpec = parse_transition_value(value_from_js(value, 0))
        .map_err(|i| invalid_options(format!("`transition`{}: {}", i[0].path, i[0].message)))?;
    let mut issues = Vec::new();
    validate_transition(&spec, &[], &mut issues);
    if let Some(i) = issues.first() {
        return Err(invalid_options(format!(
            "`transition`{}: {}",
            i.path, i.message
        )));
    }
    Ok(Some(spec.resolve()))
}

/// What a transition is made of: the keyed start and end states of a 2D or a 3D scene, or a
/// diagram building itself in (the scene, with its schedule).
#[derive(Debug)]
enum Plans {
    TwoD(Box<Plan>),
    ThreeD(Box<Plan3>),
    Build(Box<Scene3d>),
}

impl Plans {
    /// The transition from the scene on screen to its replacement, or `None` when there is
    /// nothing to animate (the same picture, or a change between 2D and 3D).
    fn diff(from: &SceneKind, to: &SceneKind, timing: Timing) -> Option<Self> {
        match (from, to) {
            (SceneKind::TwoD(a), SceneKind::TwoD(b)) => {
                let plan = Plan::diff(a, b, timing);
                (!plan.is_static()).then(|| Self::TwoD(Box::new(plan)))
            }
            // A diagram coming onto an empty canvas (`animateIn`) builds itself in.
            (SceneKind::ThreeD(a), SceneKind::ThreeD(b))
                if b.has_diagram() && !a.has_diagram() && !b.build.is_empty() =>
            {
                Some(Self::Build(Box::new(b.clone())))
            }
            // A diagram replaced by another jumps to the new picture.
            (SceneKind::ThreeD(a), SceneKind::ThreeD(b)) if a.has_diagram() || b.has_diagram() => {
                None
            }
            (SceneKind::ThreeD(a), SceneKind::ThreeD(b)) => {
                let plan = Plan3::diff(a, b, timing);
                (!plan.is_static()).then(|| Self::ThreeD(Box::new(plan)))
            }
            _ => None,
        }
    }

    fn total_ms(&self) -> f32 {
        match self {
            Self::TwoD(p) => p.total_ms(),
            Self::ThreeD(p) => p.total_ms(),
            Self::Build(s) => s.build_ms(),
        }
    }

    /// The scene as it looks `elapsed` ms into the transition.
    fn sample(&self, elapsed: f32) -> SceneKind {
        match self {
            Self::TwoD(p) => SceneKind::TwoD(p.sample(elapsed)),
            Self::ThreeD(p) => SceneKind::ThreeD(p.sample(elapsed)),
            Self::Build(s) => SceneKind::ThreeD(s.built_at(elapsed)),
        }
    }
}

/// An empty scene of the size and background of `scene`: what a chart that animates in starts
/// from.
fn blank_scene(scene: &SceneKind) -> SceneKind {
    match scene {
        SceneKind::TwoD(s) => SceneKind::TwoD(blank_like(s)),
        SceneKind::ThreeD(s) => SceneKind::ThreeD(blank_like_3d(s)),
    }
}

/// A transition in progress. The renderer draws it from the plan's start and end states by
/// the clock; the chart keeps the clock.
#[derive(Debug)]
struct Anim {
    plan: Plans,
    timing: Timing,
    /// The `now` of the first frame, so the transition begins at time 0 whenever that frame
    /// comes. `None` until then.
    start_ms: Option<f64>,
    elapsed: f32,
    origin: Origin,
    /// The lines, markers and slices as of the last frame, kept so the next frame only rewrites
    /// their numbers (see `Plan::fill_marks`).
    marks: Marks,
}

/// What the CPU tessellates, kept from frame to frame.
#[derive(Debug, Default)]
enum Marks {
    #[default]
    None,
    TwoD(Box<Scene>),
    ThreeD(Box<Scene3d>),
}

/// Where a transition started, which decides whether a resize can lay it out again.
#[derive(Debug)]
enum Origin {
    /// A settled chart of this spec: laid out again at the new size when the canvas resizes.
    Spec(Box<ChartSpec>),
    /// A settled chart of the current spec, but with this set of hidden series (what a legend
    /// toggle starts from; saves copying the whole spec for it).
    Hidden(Option<Vec<String>>),
    /// Nothing (`animateIn`): everything enters.
    Blank,
    /// The middle of another transition: there is no spec to lay out, so a resize ends it.
    Mid,
}

/// An empty scene of the size and background of `scene`: what a chart that animates in
/// starts from.
fn blank_like(scene: &Scene) -> Scene {
    let mut blank = Scene::empty();
    (blank.width, blank.height, blank.background) = (scene.width, scene.height, scene.background);
    blank
}

/// The camera on its way to a new place (`setCamera` and `resetCamera` with a duration).
#[derive(Debug)]
struct CameraTween {
    from: Camera,
    to: Camera,
    timing: Timing,
    /// The `now` of the first frame, so the tween begins at time 0 (see [`Anim::start_ms`]).
    start_ms: Option<f64>,
    elapsed: f32,
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
    /// The transition on screen, if any. `scene` is already its target.
    anim: Option<Anim>,
    /// The camera tween in progress, if any. `camera` is where it is now.
    camera_tween: Option<CameraTween>,
    /// The clock of the marks moving along animated diagram edges: seconds shown, the `now`
    /// it counts from (`None`: the next `tick` sets it so the marks go on from where they
    /// are), and whether it is held.
    flow_seconds: f64,
    flow_start_ms: Option<f64>,
    flow_paused: bool,
}

/// The laid-out chart, 2D or 3D.
// One is kept per chart, so the variants' sizes do not matter.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone)]
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
            anim: None,
            camera_tween: None,
            flow_seconds: 0.0,
            flow_start_ms: None,
            flow_paused: false,
        };
        chart.relayout();
        if bool_option(
            options.as_deref().unwrap_or(&JsValue::UNDEFINED),
            "animateIn",
        )? == Some(true)
        {
            chart.enter_from_blank();
        }
        Ok(chart)
    }

    /// Replaces the spec and re-lays out. On an invalid spec this throws `invalid-spec` and
    /// the previous spec stays in effect. Call `render` afterwards.
    ///
    /// The camera is kept (the user may have orbited) unless the new spec's `camera` differs
    /// from the old one's, in which case it restarts from the spec.
    ///
    /// Unless animation is off (the spec's `transition: false`, or `options.transition`), the
    /// chart moves to the new picture over time: call `tick` every animation frame while
    /// `animating`. See `Plan` in facetry-core for what enters, moves and leaves.
    pub fn update(
        &mut self,
        #[wasm_bindgen(unchecked_param_type = "ChartSpec")] spec: JsValue,
        #[wasm_bindgen(unchecked_param_type = "UpdateOptions | undefined")] options: JsValue,
    ) -> Result<(), JsValue> {
        let mut spec = read_spec(spec)?;
        // `options.transition` replaces the spec's for this call.
        let wanted = if options.is_undefined() || options.is_null() {
            None
        } else {
            transition_option(&js_sys::Reflect::get(&options, &"transition".into())?)?
        };
        let timing = wanted.unwrap_or_else(|| resolve_transition(spec.transition.as_ref()));
        // What the viewer hid stays hidden, for series that still exist, unless the new spec
        // says (`legend.hidden`).
        settle_hidden(&mut spec, self.spec.hidden_override.as_deref());
        // A new camera in the spec (or, for a diagram, a new starting view) moves the camera
        // there, over the same time as the data.
        let (size, dpr) = (self.renderer.size(), self.dpr);
        if spec.start_camera(size, dpr) != self.spec.start_camera(size, dpr) {
            self.move_camera(start_camera(&spec, size, dpr), timing);
        }
        let before = std::mem::replace(&mut self.spec, spec);
        self.relayout_animated(timing, Origin::Spec(Box::new(before)));
        Ok(())
    }

    /// Hides (`visible = false`) or shows one series: its marks leave every layer that
    /// colours by that category, and stacks, shares, pies and axes are computed again without
    /// it. The legend entry stays, dimmed. Returns whether anything changed: `false` for an
    /// unknown name, a series already in that state, and for hiding the last visible series
    /// (which is refused). The camera is kept. Call `render` afterwards.
    ///
    /// Animates like `update` (with the spec's `transition`, or `options.transition`): stacks
    /// restack, pies re-divide and axes refit as a movement.
    #[wasm_bindgen(js_name = setSeriesVisible)]
    pub fn set_series_visible(
        &mut self,
        name: &str,
        visible: bool,
        #[wasm_bindgen(unchecked_param_type = "UpdateOptions | undefined")] options: JsValue,
    ) -> Result<bool, JsValue> {
        let wanted = if options.is_undefined() || options.is_null() {
            None
        } else {
            transition_option(&js_sys::Reflect::get(&options, &"transition".into())?)?
        };
        let timing = wanted.unwrap_or_else(|| resolve_transition(self.spec.transition.as_ref()));
        let before = self.spec.hidden_override.clone();
        let changed = set_series_visible(&mut self.spec, name, visible);
        if changed {
            self.relayout_animated(timing, Origin::Hidden(before));
        }
        Ok(changed)
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
    ///
    /// A resize never animates: the layout jumps to the new size. A transition that is running
    /// carries on in the new layout: its starting picture (the spec before the update) and its
    /// end picture are both laid out at the new size and it continues from the same moment.
    /// If it began by interrupting another transition there is no such spec to lay out, so it
    /// ends at once instead.
    pub fn resize(&mut self, width: u32, height: u32, dpr: f64) {
        self.dpr = dpr as f32;
        self.renderer.resize(width, height);
        let Some(anim) = self.anim.take() else {
            self.relayout();
            return;
        };
        let timing = anim.timing;
        self.scene = self.layout(&self.spec);
        self.hover = None;
        let from = match anim.origin {
            Origin::Spec(ref spec) => Some(self.layout(spec)),
            Origin::Hidden(ref hidden) => {
                let now = std::mem::replace(&mut self.spec.hidden_override, hidden.clone());
                let from = self.layout(&self.spec);
                self.spec.hidden_override = now;
                Some(from)
            }
            Origin::Blank => Some(blank_scene(&self.scene)),
            Origin::Mid => None,
        };
        if let Some(plan) = from.and_then(|from| Plans::diff(&from, &self.scene, timing)) {
            self.anim = Some(Anim {
                plan,
                marks: Marks::None,
                ..anim
            });
        }
        self.reload_scene();
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
            // The viewer takes over: a tween in progress stops where it is.
            self.camera_tween = None;
            self.camera.orbit(dx_px as f32, dy_px as f32);
            self.renderer.set_camera(self.camera);
        }
    }

    /// Zooms by a wheel `delta` (positive: out, negative: in; a typical notch is about 100).
    pub fn zoom(&mut self, delta: f64) {
        if self.is_3d() {
            self.camera_tween = None;
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
        self.camera_tween = None;
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
            self.camera_tween = None;
            let s = f64::from(self.dpr);
            let height = self.renderer.size().1 as f32;
            self.camera
                .pan_by((dx * s) as f32, (dy * s) as f32, height, size);
            self.renderer.set_camera(self.camera);
        }
    }

    /// Back to the spec's camera (or the defaults), unpanned. With `options`
    /// (`{ duration, easing }`, `{}` for the defaults) the camera turns there over time instead
    /// of jumping, azimuth the short way round.
    #[wasm_bindgen(js_name = resetCamera)]
    pub fn reset_camera(
        &mut self,
        #[wasm_bindgen(unchecked_param_type = "CameraTweenOptions | undefined")] options: JsValue,
    ) -> Result<(), JsValue> {
        let timing = tween_timing(&options)?;
        if self.is_3d() {
            let start = start_camera(&self.spec, self.renderer.size(), self.dpr);
            self.move_camera(start, timing);
        }
        Ok(())
    }

    /// Sets any of `type`, `azimuth`, `elevation`, `distance` and `pan` (`[x, z]`); values are
    /// clamped (the pan to the plot box). Throws `invalid-options` if a field has the wrong
    /// type. With `options` (`{ duration, easing }`, `{}` for the defaults) the camera turns
    /// there over time instead of jumping, azimuth the short way round; the `type` (lens)
    /// changes at once.
    #[wasm_bindgen(js_name = setCamera)]
    pub fn set_camera(
        &mut self,
        #[wasm_bindgen(unchecked_param_type = "CameraUpdate")] update: JsValue,
        #[wasm_bindgen(unchecked_param_type = "CameraTweenOptions | undefined")] options: JsValue,
    ) -> Result<(), JsValue> {
        let timing = tween_timing(&options)?;
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
            // Where the camera is going: from where it is now, or from where it is headed if
            // a tween is already running (so two quick calls compose).
            let mut target = self.camera_tween.as_ref().map_or(self.camera, |t| t.to);
            if let Some(p) = projection {
                target.projection = p;
            }
            target.set(azimuth, elevation, distance);
            if let (Some([x, z]), Some(size)) = (pan, self.renderer.box_size()) {
                let current = target.pan;
                target.move_pan([x - current[0], z - current[1]], size);
            }
            self.move_camera(target, timing);
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

    /// Whether a transition is running: keep calling `tick` once per animation frame (and
    /// `render` after it) until this is false.
    #[wasm_bindgen(getter)]
    pub fn animating(&self) -> bool {
        self.anim.is_some() || self.camera_tween.is_some()
    }

    /// Whether marks move along animated diagram edges (and the flow is not held): keep calling
    /// `tick` and `render` once per animation frame while it is true.
    #[wasm_bindgen(getter)]
    pub fn flowing(&self) -> bool {
        self.renderer.has_flows() && !self.flow_paused
    }

    /// Holds the marks of animated diagram edges where they are (`true`), or lets them move
    /// on from there (`false`).
    #[wasm_bindgen(js_name = setFlowPaused)]
    pub fn set_flow_paused(&mut self, paused: bool) {
        self.flow_paused = paused;
        self.flow_start_ms = None;
    }

    /// Advances every animation (a transition, a camera tween) to the frame at `now_ms` (any
    /// clock that only goes forward, such as `requestAnimationFrame`'s timestamp). The first call
    /// after one began is its time 0. Returns whether anything is still running; when a
    /// transition ends the chart is the plain new scene, so `render` after the last `tick`
    /// draws the final picture.
    pub fn tick(&mut self, now_ms: f64) -> bool {
        if self.flowing() {
            let start = *self
                .flow_start_ms
                .get_or_insert(now_ms - self.flow_seconds * 1000.0);
            self.flow_seconds = ((now_ms - start) / 1000.0).max(0.0);
            self.renderer.set_flow_time(self.flow_seconds as f32);
        }
        if let Some(anim) = &mut self.anim {
            let start = *anim.start_ms.get_or_insert(now_ms);
            anim.elapsed = (now_ms - start).max(0.0) as f32;
        }
        if let Some(tween) = &mut self.camera_tween {
            let start = *tween.start_ms.get_or_insert(now_ms);
            tween.elapsed = (now_ms - start).max(0.0) as f32;
        }
        self.advance()
    }

    /// Jumps every running animation to `elapsed_ms` since it began (for tests and scrubbing).
    /// Returns whether anything is still running.
    pub fn seek(&mut self, elapsed_ms: f64) -> bool {
        let elapsed = elapsed_ms.max(0.0) as f32;
        // The flow's marks too, as if they had moved for `elapsed_ms`.
        self.flow_seconds = f64::from(elapsed) / 1000.0;
        self.flow_start_ms = None;
        self.renderer.set_flow_time(elapsed / 1000.0);
        if let Some(anim) = &mut self.anim {
            anim.elapsed = elapsed;
        }
        if let Some(tween) = &mut self.camera_tween {
            tween.elapsed = elapsed;
        }
        self.advance()
    }

    /// Makes the animations continue from their current time on the clock that reads `now_ms`
    /// (after `seek` was used to hold them still).
    pub fn rebase(&mut self, now_ms: f64) {
        self.flow_start_ms = Some(now_ms - self.flow_seconds * 1000.0);
        if let Some(anim) = &mut self.anim {
            anim.start_ms = Some(now_ms - f64::from(anim.elapsed));
        }
        if let Some(tween) = &mut self.camera_tween {
            tween.start_ms = Some(now_ms - f64::from(tween.elapsed));
        }
    }

    /// Ends every animation now: the chart shows its final picture and the camera is where it
    /// was headed (call `render` after).
    #[wasm_bindgen(js_name = finishTransition)]
    pub fn finish_transition(&mut self) {
        if let Some(tween) = self.camera_tween.take() {
            self.camera = tween.to;
            self.renderer.set_camera(self.camera);
        }
        if self.anim.take().is_some() {
            self.reload_scene();
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

    /// Lays `spec` out at the current size.
    fn layout(&self, spec: &ChartSpec) -> SceneKind {
        if spec.dimension == Dimension::ThreeD {
            SceneKind::ThreeD(build_scene_3d_with_theme(
                spec,
                self.renderer.size(),
                self.dpr,
                FontMetrics::inter(),
                &self.theme,
            ))
        } else {
            SceneKind::TwoD(build_scene_with_theme(
                spec,
                self.renderer.size(),
                self.dpr,
                FontMetrics::inter(),
                &self.theme,
            ))
        }
    }

    /// spec + size -> scene -> GPU buffers, at once (no animation, and any transition in
    /// progress is dropped). The hover is dropped: the bars may have moved or vanished (the
    /// TypeScript wrapper tells listeners and picks again).
    fn relayout(&mut self) {
        self.anim = None;
        self.scene = self.layout(&self.spec);
        self.hover = None;
        self.reload_scene();
    }

    /// Lays the (already replaced) spec out and moves to it over `timing`, from what is on
    /// screen now: the settled scene, or the interpolated one if a transition is running (so
    /// an update that arrives mid-way continues from where the marks are, without a jump).
    /// `origin` says what was on screen, for laying it out again if the canvas is resized
    /// before the transition ends.
    fn relayout_animated(&mut self, timing: Option<Timing>, origin: Origin) {
        let shown = match &self.anim {
            Some(a) => (a.plan.sample(a.elapsed), Origin::Mid),
            None => (self.scene.clone(), origin),
        };
        let target = self.layout(&self.spec);
        self.hover = None;
        let plan =
            timing.and_then(|timing| Some((Plans::diff(&shown.0, &target, timing)?, timing)));
        self.scene = target;
        self.anim = plan.map(|(plan, timing)| Anim {
            plan,
            timing,
            start_ms: None,
            elapsed: 0.0,
            origin: shown.1,
            marks: Marks::None,
        });
        self.reload_scene();
    }

    /// The first frame, animated: everything enters, as if the chart had been empty.
    fn enter_from_blank(&mut self) {
        let Some(timing) = resolve_transition(self.spec.transition.as_ref()) else {
            return;
        };
        if let Some(plan) = Plans::diff(&blank_scene(&self.scene), &self.scene, timing) {
            self.anim = Some(Anim {
                plan,
                timing,
                start_ms: None,
                elapsed: 0.0,
                origin: Origin::Blank,
                marks: Marks::None,
            });
            self.reload_scene();
        }
    }

    /// Moves every running animation to its `elapsed`; at the end of a transition the plain new
    /// scene replaces it, at the end of a tween the camera is where it was going. Whether
    /// anything is still running.
    fn advance(&mut self) -> bool {
        if let Some(anim) = &self.anim {
            if anim.elapsed >= anim.plan.total_ms() {
                self.anim = None;
                self.reload_scene();
            } else {
                self.upload_frame();
            }
        }
        if let Some(tween) = &self.camera_tween {
            let e = tween.timing.eased(tween.elapsed, 0.0);
            self.camera = if tween.elapsed >= tween.timing.duration_ms {
                tween.to
            } else {
                tween.from.tween(&tween.to, e)
            };
            if tween.elapsed >= tween.timing.duration_ms {
                self.camera_tween = None;
            }
            self.renderer.set_camera(self.camera);
        }
        self.animating()
    }

    /// Moves the camera to `target`: at once, or over `timing`. A camera that is already there
    /// stays put. The lens (perspective or orthographic) cannot be in between, so it changes at
    /// the start.
    fn move_camera(&mut self, target: Camera, timing: Option<Timing>) {
        self.camera_tween = None;
        let moving = Camera {
            projection: target.projection,
            ..self.camera
        };
        match timing.filter(|_| target != moving) {
            Some(timing) => {
                self.camera = moving;
                self.camera_tween = Some(CameraTween {
                    from: moving,
                    to: target,
                    timing,
                    start_ms: None,
                    elapsed: 0.0,
                });
            }
            None => self.camera = target,
        }
        self.renderer.set_camera(self.camera);
    }

    /// Hands the renderer what changes from frame to frame at the transition's current time:
    /// the clock the GPU mixes by and what the CPU tessellates (and in 3D the plot box's size).
    fn upload_frame(&mut self) {
        let Some(anim) = &mut self.anim else { return };
        let elapsed = anim.elapsed;
        self.renderer.set_time(elapsed);
        match &anim.plan {
            Plans::TwoD(plan) => {
                if plan.has_marks() {
                    let Marks::TwoD(marks) = &mut anim.marks else {
                        anim.marks = Marks::TwoD(Box::new(plan.marks_at(elapsed)));
                        return self.upload_frame();
                    };
                    plan.fill_marks(marks, elapsed);
                    self.renderer.set_marks(marks);
                }
            }
            Plans::Build(scene) => {
                // Small enough to upload whole each frame.
                self.renderer.set_scene_3d(&scene.built_at(elapsed));
                self.renderer.set_camera(self.camera);
            }
            Plans::ThreeD(plan) => {
                self.renderer
                    .set_view_3d(plan.size_at(elapsed), plan.plot_at(elapsed));
                if plan.has_marks() {
                    let Marks::ThreeD(marks) = &mut anim.marks else {
                        anim.marks = Marks::ThreeD(Box::new(plan.marks_at(elapsed)));
                        return self.upload_frame();
                    };
                    plan.fill_marks(marks, elapsed);
                    self.renderer.set_marks_3d(marks);
                }
            }
        }
    }

    /// Uploads the current scene (and camera) to the GPU again, keeping the hover. A transition
    /// in progress is uploaded as it is, at its current time.
    fn reload_scene(&mut self) {
        match (&self.scene, &self.anim) {
            (
                SceneKind::ThreeD(_),
                Some(Anim {
                    plan: Plans::ThreeD(plan),
                    ..
                }),
            ) => {
                self.renderer.begin_transition_3d(plan);
                self.renderer.set_camera(self.camera);
                self.upload_frame();
            }
            (
                SceneKind::TwoD(_),
                Some(Anim {
                    plan: Plans::TwoD(plan),
                    ..
                }),
            ) => {
                self.renderer.begin_transition(plan);
                self.upload_frame();
            }
            (
                SceneKind::ThreeD(_),
                Some(Anim {
                    plan: Plans::Build(_),
                    ..
                }),
            ) => self.upload_frame(),
            (SceneKind::ThreeD(scene), _) => {
                self.renderer.set_scene_3d(scene);
                self.renderer.set_camera(self.camera);
            }
            (SceneKind::TwoD(scene), _) => self.renderer.set_scene(scene),
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
