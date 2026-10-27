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
    /// A laid-out diagram (nodes and routed edges) instead of `layers`. Usually written by
    /// `facetry/diagrams` from Mermaid text rather than by hand. Needs `dimension: "3d"`; a
    /// diagram spec has no `data`, `axes`, `layers` or `legend`.
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub diagram: Option<DiagramSpec>,
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

/// A diagram whose layout is already done: every node has its place and size and every edge
/// its path, in **layout units** on a plan seen from above (x to the right, y down the page,
/// as in an SVG; 1 unit is about a CSS pixel at the label size). The renderer extrudes the
/// nodes into solids standing on that plan and draws the edges just above the floor.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "ts", derive(Tsify))]
#[cfg_attr(feature = "ts", serde(rename_all = "camelCase"))]
pub struct DiagramSpec {
    pub nodes: Vec<DiagramNode>,
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub edges: Vec<DiagramEdge>,
    /// The size the labels were measured at, in layout units (default 14). Labels are drawn at
    /// the matching screen size for the starting camera, so they fit their nodes.
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub font_size: Option<f64>,
    /// How the diagram looks: `"flat"` (the default), `"glossy"`, `"isometric"` or
    /// `"schematic"`.
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub style: DiagramStyle,
    /// Icons of the diagram's own, by name, for nodes' `icon` (a custom icon wins over a
    /// built-in one of the same name). A JSON object; kept as pairs in its order, a name
    /// given twice keeping its last icon. (A list rather than a map: there are a handful, and
    /// a map type of its own would add kilobytes to the wasm.)
    #[cfg_attr(feature = "ts", tsify(optional, type = "Record<string, CustomIcon>"))]
    pub icons: Vec<(String, CustomIcon)>,
    /// Boxes round groups of nodes (Mermaid's subgraphs), drawn under them with their titles.
    /// Edges may start or end at a group, by its id.
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub groups: Vec<DiagramGroup>,
}

/// A box round some of a diagram's nodes: a Mermaid subgraph, laid out.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "ts", derive(Tsify))]
#[cfg_attr(feature = "ts", serde(rename_all = "camelCase"))]
pub struct DiagramGroup {
    /// Unique among the nodes' and the groups' ids.
    pub id: String,
    /// The title, lines separated by `\n`; the id if absent.
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub label: Option<String>,
    /// Centre and size of the box, layout units.
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    /// The group it is nested in.
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub parent: Option<String>,
    /// Fill, border and title colour (`#rgb` or `#rrggbb`); the style's own when absent.
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub color: Option<String>,
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub stroke: Option<String>,
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub text_color: Option<String>,
}

impl DiagramSpec {
    /// How deep group `g` is nested: 0 for a group at the top level (a broken `parent` chain
    /// stops counting).
    pub fn group_depth(&self, g: usize) -> usize {
        let mut depth = 0;
        let mut at = self.groups[g].parent.as_deref();
        while let Some(id) = at {
            let Some(p) = self.groups.iter().find(|h| h.id == id) else {
                break;
            };
            depth += 1;
            if depth > self.groups.len() {
                break;
            }
            at = p.parent.as_deref();
        }
        depth
    }
}

/// An icon brought by the user, as SVG path data: lines drawn like the built-in icons'
/// strokes, filled shapes, or both. At least one of `stroke` and `fill`.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "ts", derive(Tsify))]
#[cfg_attr(feature = "ts", serde(rename_all = "camelCase"))]
pub struct CustomIcon {
    /// Path data drawn as lines (the `d` of a line icon's stroked paths, run together).
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub stroke: Option<String>,
    /// Filled shapes, each filled on its own (as separate SVG elements are), in the ink colour.
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub fill: Vec<IconFill>,
    /// The area the path is drawn in, `[minX, minY, width, height]` as in an SVG `viewBox`;
    /// default `[0, 0, 24, 24]`. The icon is fitted into its node's icon square, centred.
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub view_box: Option<[f64; 4]>,
}

/// One filled shape of a [`CustomIcon`]: path data and its fill rule.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "ts", derive(Tsify))]
#[cfg_attr(feature = "ts", serde(rename_all = "camelCase"))]
pub struct IconFill {
    pub d: String,
    /// `"nonzero"` (the default, as in SVG) or `"evenodd"`.
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub fill_rule: FillRule,
}

/// Which parts of overlapping contours are inside, as SVG's `fill-rule`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "ts", derive(Tsify))]
#[cfg_attr(feature = "ts", serde(rename_all = "lowercase"))]
pub enum FillRule {
    /// Inside where the contours wind round a point a non-zero number of times (SVG's default).
    #[default]
    NonZero,
    /// Inside where a ray from the point crosses the contours an odd number of times.
    EvenOdd,
}

/// A diagram's visual style.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "ts", derive(Tsify))]
#[cfg_attr(feature = "ts", serde(rename_all = "lowercase"))]
pub enum DiagramStyle {
    /// Light blocks with a crisp rim, thin edges over the node tops, labels facing the viewer;
    /// seen square on from an orthographic camera.
    #[default]
    Flat,
    /// Glossy coloured slabs (coloured by shape unless the node says otherwise) with soft
    /// shadows, thick arrows on the floor routed at right angles, and labels printed on the
    /// slabs; seen at an angle through a perspective camera.
    Glossy,
    /// An isometric infographic: each node a layered pedestal (a plate, a white body, a
    /// coloured cap with a white inset the label is printed on), coloured in turn from a
    /// palette, joined by grey roads on the floor with chevrons showing the way, under long
    /// soft shadows; seen through an orthographic lens from 45 degrees round.
    Isometric,
    /// A plant schematic in the same isometric view: each node a piece of equipment modelled
    /// after its shape (building, silo, tank, valve, reactor, drum) in pale greys with dark
    /// outlines, edges pipes on supports with flow marks (thick edges conveyors), and labels
    /// on signs standing above the equipment, over a faint floor grid.
    Schematic,
}

/// Default label size of a diagram, in layout units.
pub const DEFAULT_DIAGRAM_FONT_SIZE: f64 = 14.0;
/// Largest `diagram.fontSize` accepted.
pub const MAX_DIAGRAM_FONT_SIZE: f64 = 200.0;

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "ts", derive(Tsify))]
#[cfg_attr(feature = "ts", serde(rename_all = "camelCase"))]
pub struct DiagramNode {
    /// Unique among the nodes; what edges, hover and click refer to.
    pub id: String,
    /// The text on the node. Default: the id. `""` for none.
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub label: Option<String>,
    /// Default: `"rect"`.
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub shape: NodeShape,
    /// The centre of the node's footprint.
    pub x: f64,
    pub y: f64,
    /// The footprint's size (the shape is fitted into this rectangle).
    pub width: f64,
    pub height: f64,
    /// Fill colour, `#rrggbb`. Default: the style's colour for the shape.
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub color: Option<String>,
    /// Outline colour, `#rrggbb`: the rim round the top face. Default: the style's (the flat
    /// style draws a rim, the glossy one none).
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub stroke: Option<String>,
    /// Rim width in CSS px (0 to 20). Default: the style's.
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub stroke_width: Option<f64>,
    /// Label colour, `#rrggbb`. Default: dark or white, whichever reads better on the fill.
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub text_color: Option<String>,
    /// A built-in icon (`"bulb"`, `"gear"`, `"database"`, ...; `fa:`-style prefixes and common
    /// aliases are understood), drawn as a line drawing: on the node's card if it has one, else
    /// on its top above the label (the layout leaves room for it there).
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub icon: Option<String>,
    /// A card lying on the floor beside the node, with a title, a description and the icon
    /// (isometric style only).
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub card: Option<NodeCard>,
}

/// A node's card: a rectangle on the floor (layout units, like the node), its title printed in
/// the node's colour and its text, if any, below it in grey.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "ts", derive(Tsify))]
pub struct NodeCard {
    /// The card's centre and size.
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub title: String,
    /// Lines separated by `\n`, already wrapped to the card.
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub text: Option<String>,
}

/// Widest node rim accepted, CSS px.
pub const MAX_NODE_STROKE_WIDTH: f64 = 20.0;

/// The outline a node is extruded from. All are convex, fitted into the node's rectangle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "ts", derive(Tsify))]
#[cfg_attr(feature = "ts", serde(rename_all = "camelCase"))]
pub enum NodeShape {
    /// Square corners (Mermaid `a[text]`).
    #[default]
    Rect,
    /// Rounded corners (`a(text)`).
    Rounded,
    /// Fully rounded ends (`a([text])`).
    Stadium,
    /// A database drum (`a[(text)]`): an ellipse in plan, with a rim drawn on top.
    Cylinder,
    /// `a((text))`: an ellipse (a circle when the footprint is square).
    Circle,
    /// A decision (`a{text}`).
    Diamond,
    /// `a{{text}}`.
    Hexagon,
    /// Leaning right (`a[/text/]`).
    Parallelogram,
    /// Leaning left (`a[\text\]`).
    ParallelogramAlt,
    /// Wider at the bottom (`a[/text\]`).
    Trapezoid,
    /// Wider at the top (`a[\text/]`).
    TrapezoidAlt,
}

impl NodeShape {
    /// The name the spec uses.
    pub fn name(self) -> &'static str {
        match self {
            Self::Rect => "rect",
            Self::Rounded => "rounded",
            Self::Stadium => "stadium",
            Self::Cylinder => "cylinder",
            Self::Circle => "circle",
            Self::Diamond => "diamond",
            Self::Hexagon => "hexagon",
            Self::Parallelogram => "parallelogram",
            Self::ParallelogramAlt => "parallelogramAlt",
            Self::Trapezoid => "trapezoid",
            Self::TrapezoidAlt => "trapezoidAlt",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "ts", derive(Tsify))]
#[cfg_attr(feature = "ts", serde(rename_all = "camelCase"))]
pub struct DiagramEdge {
    /// Node ids.
    pub from: String,
    pub to: String,
    /// The routed path, at least two points: from the border of `from` to the border of `to`
    /// (where the head's tip is). Drawn as a polyline, so a curve comes as many points.
    #[cfg_attr(feature = "ts", tsify(type = "[number, number][]"))]
    pub points: Vec<[f64; 2]>,
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub label: Option<String>,
    /// Where the label's centre goes. Default: halfway along the path.
    #[cfg_attr(feature = "ts", tsify(optional, type = "[number, number]"))]
    pub label_position: Option<[f64; 2]>,
    /// Default: `"solid"`.
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub stroke: EdgeStroke,
    /// The end at `to`. Default: `"arrow"`.
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub head: Option<EdgeEnd>,
    /// The end at `from`. Default: `"none"`.
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub tail: Option<EdgeEnd>,
}

impl DiagramEdge {
    /// `(tail, head)` with the defaults filled in.
    pub fn ends(&self) -> (EdgeEnd, EdgeEnd) {
        (
            self.tail.unwrap_or(EdgeEnd::None),
            self.head.unwrap_or(EdgeEnd::Arrow),
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "ts", derive(Tsify))]
#[cfg_attr(feature = "ts", serde(rename_all = "lowercase"))]
pub enum EdgeStroke {
    /// `-->`
    #[default]
    Solid,
    /// `-.->`
    Dotted,
    /// `==>`
    Thick,
}

/// What an edge has at one of its ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "ts", derive(Tsify))]
#[cfg_attr(feature = "ts", serde(rename_all = "lowercase"))]
pub enum EdgeEnd {
    None,
    /// A filled triangle (`-->`).
    Arrow,
    /// A disc (`--o`).
    Circle,
    /// A cross (`--x`).
    Cross,
}
