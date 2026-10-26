//! The declarative chart spec: what users write as JSON. Pure data, no behaviour beyond
//! parsing; [`crate::validate`] checks it against itself and [`crate::build_scene`] turns it
//! into geometry.
//!
//! Every struct rejects unknown fields so a typo (`"colour"`) is an error, not silence.

use std::collections::{BTreeMap, HashMap};
use std::fmt;

use serde::Deserialize;
use serde::de::{self, Deserializer, SeqAccess, Visitor};
#[cfg(feature = "ts")]
use tsify::Tsify;

/// A named table: field name to column. All columns must have the same length.
pub type Table = BTreeMap<String, Column>;

/// The whole chart.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[cfg_attr(feature = "ts", derive(Tsify))]
#[serde(deny_unknown_fields)]
pub struct ChartSpec {
    /// `"2d"` (the default) or `"3d"`. 3D enables `orient: "depth"` axes, the `z` encoding,
    /// `camera` and `interaction`.
    #[serde(default)]
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub dimension: Dimension,
    /// Columnar tables by name, e.g. `{ "sales": { "quarter": ["Q1"], "revenue": [120] } }`.
    /// A column is an array of numbers (`null` = missing) or an array of strings.
    #[serde(default)]
    #[cfg_attr(
        feature = "ts",
        tsify(
            optional,
            type = "Record<string, Record<string, (number | null)[] | string[]>>"
        )
    )]
    pub data: BTreeMap<String, Table>,
    #[serde(default)]
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub axes: Vec<AxisSpec>,
    #[serde(default)]
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub layers: Vec<LayerSpec>,
    /// The legend: `{ "orient": "right" | "bottom" | "top" | "left", "title": "...",
    /// "interactive": true, "hidden": ["series", ...] }`, `true` for the defaults, or `false`
    /// for none. Absent: a legend on the right when any layer colours by a field, none
    /// otherwise. Clicking an entry hides or shows its series (`interactive`, default on).
    #[serde(default)]
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub legend: Option<LegendSpec>,
    /// Where the viewer starts in a 3D chart. Rejected in 2D.
    #[serde(default)]
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub camera: Option<CameraSpec>,
    /// How the viewer may move the camera (3D only; ignored in 2D).
    #[serde(default)]
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub interaction: Option<InteractionSpec>,
    /// Series the host has hidden at run time (`Chart.setSeriesVisible`, a legend click).
    /// When `Some`, it **replaces** `legend.hidden`, so the host's state wins over the
    /// spec's starting state without editing the user's spec. Not part of the JSON.
    #[serde(skip)]
    #[cfg_attr(feature = "ts", tsify(skip))]
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

/// `legend`: `false`, `true` or a configuration.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[cfg_attr(feature = "ts", derive(Tsify))]
#[serde(untagged)]
pub enum LegendSpec {
    /// `false` hides the legend; `true` is the same as leaving the option out.
    Shown(bool),
    Config(LegendConfig),
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[cfg_attr(feature = "ts", derive(Tsify))]
#[serde(deny_unknown_fields)]
pub struct LegendConfig {
    /// Which side of the chart the legend is on. Default: `"right"`.
    #[serde(default)]
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub orient: Option<LegendOrient>,
    /// A heading above (right/left) or before (top/bottom) the entries.
    #[serde(default)]
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub title: Option<String>,
    /// Click an entry to hide or show its series (every layer that colours by that
    /// category). Default: `true`.
    #[serde(default)]
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub interactive: Option<bool>,
    /// Series that start hidden. A name that is not a series is ignored; hiding every
    /// series is an error. The viewer's later clicks replace this set (see
    /// `Chart.setSeriesVisible`); `Chart.update` keeps theirs unless this field is present.
    #[serde(default)]
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub hidden: Option<Vec<String>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[cfg_attr(feature = "ts", derive(Tsify))]
#[serde(rename_all = "lowercase")]
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
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[cfg_attr(feature = "ts", derive(Tsify))]
#[serde(deny_unknown_fields)]
pub struct CameraSpec {
    /// `"perspective"` (default) or `"orthographic"`.
    #[serde(default, rename = "type")]
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub kind: Option<CameraKind>,
    /// Degrees around the vertical axis. 0 looks at the front of the chart; positive moves
    /// the camera to the right.
    #[serde(default)]
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub azimuth: Option<f64>,
    /// Degrees above the floor (5 to 85).
    #[serde(default)]
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub elevation: Option<f64>,
    /// Distance from the plot box's centre, in units of the box's radius (half its diagonal,
    /// at least 1, so flat charts are framed like cubes).
    #[serde(default)]
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub distance: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[cfg_attr(feature = "ts", derive(Tsify))]
#[serde(rename_all = "lowercase")]
pub enum CameraKind {
    #[default]
    Perspective,
    Orthographic,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[cfg_attr(feature = "ts", derive(Tsify))]
#[serde(deny_unknown_fields)]
pub struct InteractionSpec {
    /// Drag to orbit, wheel to zoom, double-click to reset. Default: `true` in 3D.
    #[serde(default)]
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub orbit: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[cfg_attr(feature = "ts", derive(Tsify))]
pub enum Dimension {
    #[default]
    #[serde(rename = "2d")]
    TwoD,
    #[serde(rename = "3d")]
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

/// One JSON array element.
enum Cell {
    Number(f64),
    Text(String),
    Null,
}

impl<'de> Deserialize<'de> for Cell {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct CellVisitor;
        impl Visitor<'_> for CellVisitor {
            type Value = Cell;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a number, a string or null")
            }
            fn visit_f64<E>(self, v: f64) -> Result<Cell, E> {
                Ok(Cell::Number(v))
            }
            fn visit_i64<E>(self, v: i64) -> Result<Cell, E> {
                Ok(Cell::Number(v as f64))
            }
            fn visit_u64<E>(self, v: u64) -> Result<Cell, E> {
                Ok(Cell::Number(v as f64))
            }
            fn visit_str<E>(self, v: &str) -> Result<Cell, E> {
                Ok(Cell::Text(v.to_owned()))
            }
            fn visit_unit<E>(self) -> Result<Cell, E> {
                Ok(Cell::Null)
            }
            fn visit_none<E>(self) -> Result<Cell, E> {
                Ok(Cell::Null)
            }
        }
        d.deserialize_any(CellVisitor)
    }
}

impl<'de> Deserialize<'de> for Column {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct ColumnVisitor;
        impl<'de> Visitor<'de> for ColumnVisitor {
            type Value = Column;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("an array of numbers or an array of strings")
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Column, A::Error> {
                let mut numbers = Vec::new();
                let mut texts: Vec<String> = Vec::new();
                let mut len = 0usize;
                while let Some(cell) = seq.next_element::<Cell>()? {
                    len += 1;
                    match cell {
                        Cell::Number(v) => numbers.push(v),
                        Cell::Null => numbers.push(f64::NAN),
                        Cell::Text(s) => texts.push(s),
                    }
                }
                if texts.is_empty() {
                    return Ok(Column::Numeric(numbers));
                }
                // Anything that is not a string (numbers, nulls) makes the column ambiguous.
                if texts.len() != len {
                    return Err(de::Error::custom(
                        "a column is either all numbers (null allowed) or all strings, not a mix",
                    ));
                }
                let mut index: HashMap<&str, u32> = HashMap::new();
                let mut dict: Vec<&str> = Vec::new();
                let codes = texts
                    .iter()
                    .map(|s| {
                        *index.entry(s).or_insert_with(|| {
                            dict.push(s);
                            (dict.len() - 1) as u32
                        })
                    })
                    .collect();
                let dict = dict.into_iter().map(str::to_owned).collect();
                Ok(Column::Categorical { codes, dict })
            }
        }
        d.deserialize_seq(ColumnVisitor)
    }
}

/// Where an axis sits. `Depth` (the z axis) belongs to 3D charts; `Right` to 2D ones.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[cfg_attr(feature = "ts", derive(Tsify))]
#[serde(rename_all = "lowercase")]
pub enum Orient {
    Bottom,
    Left,
    Right,
    Depth,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[cfg_attr(feature = "ts", derive(Tsify))]
#[serde(deny_unknown_fields)]
pub struct AxisSpec {
    /// Referenced by `encoding.*.axis`.
    pub id: String,
    pub orient: Orient,
    #[serde(default)]
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub title: Option<String>,
    /// Overrides for a numeric (linear) axis.
    #[serde(default)]
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub scale: Option<ScaleSpec>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[cfg_attr(feature = "ts", derive(Tsify))]
#[serde(deny_unknown_fields)]
pub struct ScaleSpec {
    /// Fixed `[min, max]`; by default the domain is computed from the data.
    #[serde(default)]
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub domain: Option<[f64; 2]>,
    /// Round the domain out to tick values. Default: `true`, unless `domain` is given.
    #[serde(default)]
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub nice: Option<bool>,
    /// Make the computed domain include 0. Default: `true` (bars grow from zero).
    #[serde(default)]
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub zero: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[cfg_attr(feature = "ts", derive(Tsify))]
#[serde(rename_all = "lowercase")]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[cfg_attr(feature = "ts", derive(Tsify))]
#[serde(rename_all = "lowercase")]
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

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[cfg_attr(feature = "ts", derive(Tsify))]
#[serde(deny_unknown_fields)]
pub struct LayerSpec {
    pub mark: Mark,
    /// Name of a table in `data`.
    pub data: String,
    pub encoding: Encoding,
    /// How bars that share an x category are placed: `"overlay"` (default), `"stack"`,
    /// `"normalize"` or `"group"`. The last three need `encoding.color.field`.
    #[serde(default)]
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub position: Position,
    /// Line layers: markers on the data points, `true` or `{ "size": px }`. Default: none
    /// (the points still react to hover). In 3D the markers are discs that face the viewer.
    #[serde(default)]
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub point: Option<PointSpec>,
    /// Mark styling, e.g. `{ "strokeWidth": 3 }` for a line.
    #[serde(default)]
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub style: Option<StyleSpec>,
    /// Value labels on the bars. Default: none.
    #[serde(default)]
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub label: Option<LabelSpec>,
}

/// Markers on a line's data points: `true` for the default size, or `{ "size": px }`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[cfg_attr(feature = "ts", derive(Tsify))]
#[serde(untagged)]
pub enum PointSpec {
    On(bool),
    Config(PointConfig),
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[cfg_attr(feature = "ts", derive(Tsify))]
#[serde(deny_unknown_fields)]
pub struct PointConfig {
    /// Marker diameter in CSS px (default 8).
    #[serde(default)]
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
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[cfg_attr(feature = "ts", derive(Tsify))]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct StyleSpec {
    /// Line width in CSS px (default 2). Lines only.
    #[serde(default)]
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub stroke_width: Option<f64>,
    /// Lines in 3D: a translucent vertical "curtain" from the line down to the baseline, which
    /// helps read depth. 3D line layers only.
    #[serde(default)]
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub area: Option<bool>,
    /// Arcs: the hole's radius as a fraction of the outer radius, `0` (a pie, the default) up
    /// to but not including `1`. Above 0 it is a donut.
    #[serde(default)]
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub inner_radius: Option<f64>,
    /// Arcs: the angular gap between slices, in degrees (default 0).
    #[serde(default)]
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub pad_angle: Option<f64>,
    /// Arcs: where the first slice starts, in degrees clockwise from 12 o'clock (default 0).
    #[serde(default)]
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub start_angle: Option<f64>,
    /// Arcs in 3D: the thickness of the pie as a fraction of its radius (default 0.2).
    #[serde(default)]
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub depth: Option<f64>,
}

/// Default thickness of an extruded pie, as a fraction of its radius.
pub const DEFAULT_PIE_DEPTH: f64 = 0.2;

/// Value labels: the `y` value written above each bar (below a negative one).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[cfg_attr(feature = "ts", derive(Tsify))]
#[serde(deny_unknown_fields)]
pub struct LabelSpec {
    /// `"always"`: every bar has its label. `"hover"`: only the bar under the pointer.
    pub show: LabelShow,
    #[serde(default)]
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub format: Option<LabelFormat>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, serde::Serialize)]
#[cfg_attr(feature = "ts", derive(Tsify))]
#[serde(rename_all = "lowercase")]
pub enum LabelShow {
    Always,
    Hover,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[cfg_attr(feature = "ts", derive(Tsify))]
#[serde(deny_unknown_fields)]
pub struct LabelFormat {
    /// Print the share of the total as a percentage (`33.3%`) instead of the value: of the
    /// pie for arcs, of the stack for stacked bars. Arcs and `stack`/`normalize` layers only
    /// (a `normalize` layer always prints percentages).
    #[serde(default)]
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub percent: Option<bool>,
    /// Digits after the decimal point (0 to 10). Default: as many as the value needs, up to 4,
    /// so `1200` reads `1,200` and `0.25` reads `0.25`; percentages default to at most one.
    #[serde(default)]
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub decimals: Option<u32>,
}

/// Most decimals a label may ask for; beyond that `f64` digits are noise anyway.
pub const MAX_LABEL_DECIMALS: u32 = 10;

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[cfg_attr(feature = "ts", derive(Tsify))]
#[serde(deny_unknown_fields)]
pub struct Encoding {
    /// Bars: a categorical field (band scale). Lines: categorical or numeric. Arcs: none.
    #[serde(default)]
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub x: Option<PositionEncoding>,
    /// A numeric field (linear scale). Arcs: none.
    #[serde(default)]
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub y: Option<PositionEncoding>,
    /// Arcs: the numeric field (not negative) that sizes each slice: a slice's angle is its
    /// share of the total.
    #[serde(default)]
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub theta: Option<ThetaEncoding>,
    /// 3D only: a categorical field (band scale) that spreads bars along the depth axis.
    /// Optional: without it the bars form a single row at the front.
    #[serde(default)]
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub z: Option<PositionEncoding>,
    /// A fixed colour, or a categorical field mapped onto the theme palette.
    #[serde(default)]
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub color: Option<ColorEncoding>,
    /// Stable identity of each mark across updates. Default: the `x` value.
    #[serde(default)]
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub key: Option<KeyEncoding>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[cfg_attr(feature = "ts", derive(Tsify))]
#[serde(deny_unknown_fields)]
pub struct PositionEncoding {
    pub field: String,
    /// Id of the axis to bind to. Default: the first declared axis that fits the channel
    /// (`bottom` for x, `left` or `right` for y, `depth` for z).
    #[serde(default)]
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub axis: Option<String>,
}

/// Exactly one of `value` and `field`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[cfg_attr(feature = "ts", derive(Tsify))]
#[serde(deny_unknown_fields)]
pub struct ColorEncoding {
    /// A CSS hex colour, `#rrggbb` or `#rrggbbaa`.
    #[serde(default)]
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub value: Option<String>,
    /// A categorical field: each distinct value gets the next palette colour.
    #[serde(default)]
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub field: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[cfg_attr(feature = "ts", derive(Tsify))]
#[serde(deny_unknown_fields)]
pub struct ThetaEncoding {
    pub field: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[cfg_attr(feature = "ts", derive(Tsify))]
#[serde(deny_unknown_fields)]
pub struct KeyEncoding {
    pub field: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn column(json: &str) -> Result<Column, serde_json::Error> {
        serde_json::from_str(json)
    }

    #[test]
    fn numeric_columns_turn_null_into_nan() {
        let Column::Numeric(v) = column("[1, null, 2.5]").unwrap() else {
            panic!("expected numeric");
        };
        assert_eq!(v[0], 1.0);
        assert!(v[1].is_nan());
        assert_eq!(v[2], 2.5);
    }

    #[test]
    fn string_columns_are_dictionary_encoded_in_first_seen_order() {
        let c = column(r#"["b", "a", "b", "c", "a"]"#).unwrap();
        assert_eq!(
            c,
            Column::Categorical {
                codes: vec![0, 1, 0, 2, 1],
                dict: vec!["b".into(), "a".into(), "c".into()],
            }
        );
    }

    #[test]
    fn empty_column_is_numeric_and_empty() {
        let c = column("[]").unwrap();
        assert!(c.is_empty());
        assert_eq!(c.kind(), "numeric");
    }

    #[test]
    fn mixed_and_exotic_columns_are_rejected() {
        assert!(column(r#"["a", 1]"#).is_err());
        assert!(column(r#"["a", null]"#).is_err());
        assert!(column("[true]").is_err());
        assert!(column("5").is_err());
    }
}
