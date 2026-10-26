//! The declarative chart spec: what users write as JSON. Pure data, no behaviour beyond
//! parsing; [`crate::validate`] checks it against itself and [`crate::build_scene`] turns it
//! into geometry.
//!
//! Every struct rejects unknown fields so a typo (`"colour"`) is an error, not silence.
//! That, the defaults, and every other part of reading JSON live in [`crate::spec_read`]: these
//! types derive nothing serde-related (that code was a large part of the wasm). The
//! `#[serde(...)]` attributes below are read only by `Tsify`, to name things in the generated
//! TypeScript.

use std::collections::BTreeMap;

#[cfg(feature = "ts")]
use tsify::Tsify;

/// A named table: field name to column. All columns must have the same length.
pub type Table = BTreeMap<String, Column>;

/// The whole chart.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "ts", derive(Tsify))]
pub struct ChartSpec {
    /// `"2d"` (the default) or `"3d"`. 3D enables `orient: "depth"` axes, the `z` encoding,
    /// `camera` and `interaction`.
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub dimension: Dimension,
    /// Columnar tables by name, e.g. `{ "sales": { "quarter": ["Q1"], "revenue": [120] } }`.
    /// A column is an array of numbers (`null` = missing) or an array of strings.
    #[cfg_attr(
        feature = "ts",
        tsify(
            optional,
            type = "Record<string, Record<string, (number | null)[] | string[]>>"
        )
    )]
    pub data: BTreeMap<String, Table>,
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub axes: Vec<AxisSpec>,
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub layers: Vec<LayerSpec>,
    /// The legend: `{ "orient": "right" | "bottom" | "top" | "left", "title": "...",
    /// "interactive": true, "hidden": ["series", ...] }`, `true` for the defaults, or `false`
    /// for none. Absent: a legend on the right when any layer colours by a field, none
    /// otherwise. Clicking an entry hides or shows its series (`interactive`, default on).
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub legend: Option<LegendSpec>,
    /// Where the viewer starts in a 3D chart. Rejected in 2D.
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub camera: Option<CameraSpec>,
    /// How the viewer may move the camera (3D only; ignored in 2D).
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub interaction: Option<InteractionSpec>,
    /// How `Chart.update` (and hiding a series) moves from the old picture to the new one:
    /// `false` for no animation, or `{ "duration": ms, "easing": "...", "stagger": ms }`.
    /// Absent: 400 ms, `cubicInOut`, no stagger. `Chart.update(spec, { transition })` overrides
    /// it for one call.
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub transition: Option<TransitionSpec>,
    /// Series the host has hidden at run time (`Chart.setSeriesVisible`, a legend click).
    /// When `Some`, it **replaces** `legend.hidden`, so the host's state wins over the
    /// spec's starting state without editing the user's spec. Not part of the JSON.
    #[cfg_attr(feature = "ts", serde(skip))]
    pub hidden_override: Option<Vec<String>>,
}

impl ChartSpec {
    /// The names of the series that are hidden: the run-time set if the host set one, else
    /// `legend.hidden`. Names that are not a series of any layer are harmless: they hide
    /// nothing.
    pub fn hidden_series(&self) -> &[String] {
        if let Some(h) = &self.hidden_override {
            return h;
        }
        match &self.legend {
            Some(LegendSpec::Config(c)) => c.hidden.as_deref().unwrap_or(&[]),
            _ => &[],
        }
    }

    /// Whether clicking a legend entry toggles its series: there is a legend and
    /// `legend.interactive` is not `false`.
    pub fn legend_interactive(&self) -> bool {
        match &self.legend {
            Some(LegendSpec::Shown(false)) => false,
            Some(LegendSpec::Config(c)) => c.interactive.unwrap_or(true),
            _ => true,
        }
    }

    /// Whether dragging/wheeling should move the camera: only in 3D, on unless
    /// `interaction.orbit` is `false`.
    pub fn orbit_enabled(&self) -> bool {
        self.dimension == Dimension::ThreeD
            && self
                .interaction
                .as_ref()
                .and_then(|i| i.orbit)
                .unwrap_or(true)
    }
}

/// `transition`: `false` (no animation), `true` (the defaults) or a configuration.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "ts", derive(Tsify))]
#[cfg_attr(feature = "ts", serde(untagged))]
pub enum TransitionSpec {
    /// `false` turns animation off; `true` is the same as leaving the option out.
    Enabled(bool),
    Config(TransitionConfig),
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "ts", derive(Tsify))]
pub struct TransitionConfig {
    /// How long each mark takes to move, in milliseconds (default 400). `0` is no animation.
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub duration: Option<f64>,
    /// The speed curve (default `"cubicInOut"`).
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub easing: Option<Easing>,
    /// Spreads the start of the marks over this many milliseconds, in drawing order (the
    /// first mark starts at once, the last one `stagger` ms later). The whole transition then
    /// lasts `duration + stagger`. Default 0: every mark moves together.
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub stagger: Option<f64>,
}

/// How progress through a transition maps to progress of the movement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "ts", derive(Tsify))]
#[cfg_attr(feature = "ts", serde(rename_all = "camelCase"))]
pub enum Easing {
    /// Constant speed.
    Linear,
    /// Slow start, fast middle, slow end.
    #[default]
    CubicInOut,
    /// Fast start, slow end.
    CubicOut,
    /// Like `cubicOut`, but overshoots the target a little and settles back.
    BackOut,
}

/// A transition with its defaults filled in. See [`TransitionSpec::resolve`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Timing {
    pub duration_ms: f32,
    pub easing: Easing,
    pub stagger_ms: f32,
}

/// Default duration of a transition, in milliseconds.
pub const DEFAULT_DURATION_MS: f64 = 400.0;

impl Default for Timing {
    fn default() -> Self {
        Self {
            duration_ms: DEFAULT_DURATION_MS as f32,
            easing: Easing::default(),
            stagger_ms: 0.0,
        }
    }
}

impl TransitionSpec {
    /// The timing to animate with, or `None` when animation is off (`false`, or a duration of
    /// 0 without a stagger). `None` as a whole spec (no `transition` field) is the default
    /// timing, see [`resolve_transition`].
    pub fn resolve(&self) -> Option<Timing> {
        match self {
            Self::Enabled(false) => None,
            Self::Enabled(true) => Some(Timing::default()),
            Self::Config(c) => {
                let timing = Timing {
                    duration_ms: c.duration.unwrap_or(DEFAULT_DURATION_MS) as f32,
                    easing: c.easing.unwrap_or_default(),
                    stagger_ms: c.stagger.unwrap_or(0.0) as f32,
                };
                (timing.duration_ms > 0.0).then_some(timing)
            }
        }
    }
}

/// The timing for an optional `transition` field: absent means animate with the defaults.
pub fn resolve_transition(spec: Option<&TransitionSpec>) -> Option<Timing> {
    spec.map_or_else(|| Some(Timing::default()), TransitionSpec::resolve)
}

/// `legend`: `false`, `true` or a configuration.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "ts", derive(Tsify))]
#[cfg_attr(feature = "ts", serde(untagged))]
pub enum LegendSpec {
    /// `false` hides the legend; `true` is the same as leaving the option out.
    Shown(bool),
    Config(LegendConfig),
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "ts", derive(Tsify))]
pub struct LegendConfig {
    /// Which side of the chart the legend is on. Default: `"right"`.
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub orient: Option<LegendOrient>,
    /// A heading above (right/left) or before (top/bottom) the entries.
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub title: Option<String>,
    /// Click an entry to hide or show its series (every layer that colours by that
    /// category). Default: `true`.
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub interactive: Option<bool>,
    /// Series that start hidden. A name that is not a series is ignored; hiding every
    /// series is an error. The viewer's later clicks replace this set (see
    /// `Chart.setSeriesVisible`); `Chart.update` keeps theirs unless this field is present.
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub hidden: Option<Vec<String>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "ts", derive(Tsify))]
#[cfg_attr(feature = "ts", serde(rename_all = "lowercase"))]
pub enum LegendOrient {
    #[default]
    Right,
    Bottom,
    Top,
    Left,
}

/// The starting camera of a 3D chart. Every field is optional; the renderer fills in
/// defaults (perspective, azimuth 35, elevation 25, distance 2.7) and clamps wild values
/// (elevation to 5-85 degrees, distance to a sensible range), so any numbers are safe.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "ts", derive(Tsify))]
pub struct CameraSpec {
    /// `"perspective"` (default) or `"orthographic"`.
    #[cfg_attr(feature = "ts", serde(rename = "type"))]
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub kind: Option<CameraKind>,
    /// Degrees around the vertical axis. 0 looks at the front of the chart; positive moves
    /// the camera to the right.
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub azimuth: Option<f64>,
    /// Degrees above the floor (5 to 85).
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub elevation: Option<f64>,
    /// Distance from the plot box's centre, in units of the box's radius (half its diagonal,
    /// at least 1, so flat charts are framed like cubes).
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub distance: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "ts", derive(Tsify))]
#[cfg_attr(feature = "ts", serde(rename_all = "lowercase"))]
pub enum CameraKind {
    #[default]
    Perspective,
    Orthographic,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "ts", derive(Tsify))]
pub struct InteractionSpec {
    /// Drag to orbit, wheel to zoom, double-click to reset. Default: `true` in 3D.
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub orbit: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "ts", derive(Tsify))]
pub enum Dimension {
    #[default]
    #[cfg_attr(feature = "ts", serde(rename = "2d"))]
    TwoD,
    #[cfg_attr(feature = "ts", serde(rename = "3d"))]
    ThreeD,
}

/// A data column. Strings are dictionary-encoded on the way in: each distinct string is
/// stored once, in order of first appearance, and rows hold a small index into that list.
#[derive(Debug, Clone, PartialEq)]
pub enum Column {
    /// `null` becomes NaN.
    Numeric(Vec<f64>),
    Categorical {
        codes: Vec<u32>,
        dict: Vec<String>,
    },
}

impl Column {
    pub fn len(&self) -> usize {
        match self {
            Self::Numeric(v) => v.len(),
            Self::Categorical { codes, .. } => codes.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// `"numeric"` or `"categorical"`, for error messages.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Numeric(_) => "numeric",
            Self::Categorical { .. } => "categorical",
        }
    }
}

/// Where an axis sits. `Depth` (the z axis) belongs to 3D charts; `Right` to 2D ones.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "ts", derive(Tsify))]
#[cfg_attr(feature = "ts", serde(rename_all = "lowercase"))]
pub enum Orient {
    Bottom,
    Left,
    Right,
    Depth,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "ts", derive(Tsify))]
pub struct AxisSpec {
    /// Referenced by `encoding.*.axis`.
    pub id: String,
    pub orient: Orient,
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub title: Option<String>,
    /// Overrides for a numeric (linear) axis.
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub scale: Option<ScaleSpec>,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "ts", derive(Tsify))]
pub struct ScaleSpec {
    /// Fixed `[min, max]`; by default the domain is computed from the data.
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub domain: Option<[f64; 2]>,
    /// Round the domain out to tick values. Default: `true`, unless `domain` is given.
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub nice: Option<bool>,
    /// Make the computed domain include 0. Default: `true` (bars grow from zero).
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub zero: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "ts", derive(Tsify))]
#[cfg_attr(feature = "ts", serde(rename_all = "lowercase"))]
pub enum Mark {
    /// Rectangles from a baseline (see [`Position`] for how they share an x category).
    Bar,
    /// One polyline per series, flat in 2D and in world space in 3D (where `encoding.z` puts a
    /// series on a depth row).
    Line,
    /// Pie and donut slices (`encoding.theta`), flat in 2D and extruded in 3D. No axes.
    Arc,
}

/// How the bars of one layer share an `x` category when they have a `color.field` (the
/// series). Rows with the same x and series are summed first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "ts", derive(Tsify))]
#[cfg_attr(feature = "ts", serde(rename_all = "lowercase"))]
pub enum Position {
    /// Every row is its own bar and bars of the same x overlap (the default).
    #[default]
    Overlay,
    /// Series stack on top of each other in dictionary order of the colour field; negative
    /// values stack downward from zero.
    Stack,
    /// Stacked, with each x scaled to 100%. Values must not be negative.
    Normalize,
    /// Series sit side by side inside the x band.
    Group,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "ts", derive(Tsify))]
pub struct LayerSpec {
    pub mark: Mark,
    /// Name of a table in `data`.
    pub data: String,
    pub encoding: Encoding,
    /// How bars that share an x category are placed: `"overlay"` (default), `"stack"`,
    /// `"normalize"` or `"group"`. The last three need `encoding.color.field`.
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub position: Position,
    /// Line layers: markers on the data points, `true` or `{ "size": px }`. Default: none
    /// (the points still react to hover). In 3D the markers are discs that face the viewer.
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub point: Option<PointSpec>,
    /// Mark styling, e.g. `{ "strokeWidth": 3 }` for a line.
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub style: Option<StyleSpec>,
    /// Value labels on the bars. Default: none.
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub label: Option<LabelSpec>,
}

/// Markers on a line's data points: `true` for the default size, or `{ "size": px }`.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "ts", derive(Tsify))]
#[cfg_attr(feature = "ts", serde(untagged))]
pub enum PointSpec {
    On(bool),
    Config(PointConfig),
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "ts", derive(Tsify))]
pub struct PointConfig {
    /// Marker diameter in CSS px (default 8).
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub size: Option<f64>,
}

/// Default marker diameter in CSS px.
pub const DEFAULT_POINT_SIZE: f64 = 8.0;
/// Default line width in CSS px.
pub const DEFAULT_STROKE_WIDTH: f64 = 2.0;
/// Largest line width and marker size accepted, CSS px.
pub const MAX_STROKE_WIDTH: f64 = 50.0;

impl PointSpec {
    /// The marker diameter in CSS px, or `None` when markers are off.
    pub fn size(&self) -> Option<f64> {
        match self {
            Self::On(false) => None,
            Self::On(true) => Some(DEFAULT_POINT_SIZE),
            Self::Config(c) => Some(c.size.unwrap_or(DEFAULT_POINT_SIZE)),
        }
    }
}

/// Per-layer styling. Every field is optional; fields that do not apply to the layer's mark
/// are validation errors.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "ts", derive(Tsify))]
#[cfg_attr(feature = "ts", serde(rename_all = "camelCase"))]
pub struct StyleSpec {
    /// Line width in CSS px (default 2). Lines only.
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub stroke_width: Option<f64>,
    /// Lines in 3D: a translucent vertical "curtain" from the line down to the baseline, which
    /// helps read depth. 3D line layers only.
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub area: Option<bool>,
    /// Arcs: the hole's radius as a fraction of the outer radius, `0` (a pie, the default) up
    /// to but not including `1`. Above 0 it is a donut.
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub inner_radius: Option<f64>,
    /// Arcs: the angular gap between slices, in degrees (default 0).
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub pad_angle: Option<f64>,
    /// Arcs: where the first slice starts, in degrees clockwise from 12 o'clock (default 0).
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub start_angle: Option<f64>,
    /// Arcs in 3D: the thickness of the pie as a fraction of its radius (default 0.2).
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub depth: Option<f64>,
}

/// Default thickness of an extruded pie, as a fraction of its radius.
pub const DEFAULT_PIE_DEPTH: f64 = 0.2;

/// Value labels: the `y` value written above each bar (below a negative one).
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "ts", derive(Tsify))]
pub struct LabelSpec {
    /// `"always"`: every bar has its label. `"hover"`: only the bar under the pointer.
    pub show: LabelShow,
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub format: Option<LabelFormat>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[cfg_attr(feature = "ts", derive(Tsify))]
#[cfg_attr(feature = "ts", serde(rename_all = "lowercase"))]
pub enum LabelShow {
    Always,
    Hover,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "ts", derive(Tsify))]
pub struct LabelFormat {
    /// Print the share of the total as a percentage (`33.3%`) instead of the value: of the
    /// pie for arcs, of the stack for stacked bars. Arcs and `stack`/`normalize` layers only
    /// (a `normalize` layer always prints percentages).
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub percent: Option<bool>,
    /// Digits after the decimal point (0 to 10). Default: as many as the value needs, up to 4,
    /// so `1200` reads `1,200` and `0.25` reads `0.25`; percentages default to at most one.
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub decimals: Option<u32>,
}

/// Most decimals a label may ask for; beyond that `f64` digits are noise anyway.
pub const MAX_LABEL_DECIMALS: u32 = 10;

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "ts", derive(Tsify))]
pub struct Encoding {
    /// Bars: a categorical field (band scale). Lines: categorical or numeric. Arcs: none.
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub x: Option<PositionEncoding>,
    /// A numeric field (linear scale). Arcs: none.
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub y: Option<PositionEncoding>,
    /// Arcs: the numeric field (not negative) that sizes each slice: a slice's angle is its
    /// share of the total.
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub theta: Option<ThetaEncoding>,
    /// 3D only: a categorical field (band scale) that spreads bars along the depth axis.
    /// Optional: without it the bars form a single row at the front.
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub z: Option<PositionEncoding>,
    /// A fixed colour, or a categorical field mapped onto the theme palette.
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub color: Option<ColorEncoding>,
    /// Stable identity of each mark across updates. Default: the `x` value.
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub key: Option<KeyEncoding>,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "ts", derive(Tsify))]
pub struct PositionEncoding {
    pub field: String,
    /// Id of the axis to bind to. Default: the first declared axis that fits the channel
    /// (`bottom` for x, `left` or `right` for y, `depth` for z).
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub axis: Option<String>,
}

/// Exactly one of `value` and `field`.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "ts", derive(Tsify))]
pub struct ColorEncoding {
    /// A CSS hex colour, `#rrggbb` or `#rrggbbaa`.
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub value: Option<String>,
    /// A categorical field: each distinct value gets the next palette colour.
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub field: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "ts", derive(Tsify))]
pub struct ThetaEncoding {
    pub field: String,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "ts", derive(Tsify))]
pub struct KeyEncoding {
    pub field: String,
}
