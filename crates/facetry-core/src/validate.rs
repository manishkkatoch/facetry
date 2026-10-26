//! Parsing and validation. Both report **every** problem found, each with a JSON-pointer
//! `path` into the spec (`/layers/0/encoding/y/axis`), so an editor can underline them all
//! at once instead of making the user fix one error per run.
//!
//! The lookups a layer needs (its table, columns, axes, colour) are done once, in
//! [`resolve_layer`]. `validate` calls it and reports failures; `build_scene` calls it and
//! skips layers that fail. One code path means they can never disagree.

use std::borrow::Cow;

use serde::Serialize;
use serde::de::Deserializer;
#[cfg(feature = "ts")]
use tsify::Tsify;

use crate::Color;
use crate::spec::{
    AxisSpec, CameraSpec, ChartSpec, ColorEncoding, Column, Dimension, LayerSpec,
    MAX_LABEL_DECIMALS, MAX_STROKE_WIDTH, Mark, Orient, Position, PositionEncoding, Table,
};
use crate::transform::{self, Cell, Row};

/// One problem with a spec.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(Tsify))]
pub struct Issue {
    /// JSON pointer into the spec, e.g. `/layers/0/encoding/y/axis`; `""` is the whole spec.
    pub path: String,
    pub message: String,
}

impl Issue {
    fn new(path: String, message: impl Into<String>) -> Self {
        Self {
            path,
            message: message.into(),
        }
    }
}

/// Builds a JSON pointer, escaping `~` and `/` inside keys (RFC 6901).
fn pointer(parts: &[&str]) -> String {
    parts
        .iter()
        .map(|p| format!("/{}", p.replace('~', "~0").replace('/', "~1")))
        .collect()
}

/// Deserialises a spec from any serde source (JSON text, a JS object via
/// serde-wasm-bindgen, ...). Shape errors (unknown field, wrong type) come back as a single
/// [`Issue`] whose path says where in the input it went wrong.
pub fn parse_spec<'de, D>(deserializer: D) -> Result<ChartSpec, Vec<Issue>>
where
    D: Deserializer<'de>,
{
    serde_path_to_error::deserialize(deserializer).map_err(|e| {
        use serde_path_to_error::Segment;
        let path: String = e
            .path()
            .iter()
            .filter_map(|seg| match seg {
                Segment::Seq { index } => Some(format!("/{index}")),
                Segment::Map { key } => Some(pointer(&[key])),
                Segment::Enum { .. } | Segment::Unknown => None,
            })
            .collect();
        vec![Issue::new(path, e.inner().to_string())]
    })
}

/// [`parse_spec`] for JSON text.
pub fn parse_spec_json(json: &str) -> Result<ChartSpec, Vec<Issue>> {
    parse_spec(&mut serde_json::Deserializer::from_str(json))
}

/// Where a layer's colours come from.
#[derive(Debug, Clone, Copy)]
pub(crate) enum ColorSource<'a> {
    /// No `color` encoding: the first palette colour.
    Default,
    Fixed(Color),
    Field {
        codes: &'a [u32],
        dict: &'a [String],
    },
}

/// A layer with every reference looked up and type-checked.
#[derive(Debug)]
pub(crate) struct ResolvedLayer<'a> {
    pub index: usize,
    pub mark: Mark,
    /// Indices into `spec.axes`.
    pub x_axis: usize,
    pub y_axis: usize,
    /// 3D only; `None` when the layer has no `z` encoding (a single row of bars).
    pub z_axis: Option<usize>,
    pub x_codes: &'a [u32],
    pub x_dict: &'a [String],
    /// A line layer's numeric x values (`x_numeric`); empty otherwise.
    pub x_num: &'a [f64],
    /// The x field is numeric (a line layer on a linear x scale), not categorical.
    pub x_numeric: bool,
    /// The y values, with the rows of hidden series set to NaN (missing). Every transform
    /// already ignores a missing number, which is how a hidden series drops out of stacks,
    /// shares, extents and lines without any row being renumbered: row indices stay those
    /// of the user's table, so hover and click still find the right datum.
    pub y: Cow<'a, [f64]>,
    pub z_codes: &'a [u32],
    pub z_dict: &'a [String],
    pub color: ColorSource<'a>,
    pub key: Option<&'a Column>,
    pub position: Position,
    /// Per colour category (by code): hidden. Empty when nothing is hidden in this layer.
    pub hidden: Vec<bool>,
}

impl ResolvedLayer<'_> {
    /// Whether `row` belongs to a hidden series.
    pub fn row_hidden(&self, row: usize) -> bool {
        match self.color {
            ColorSource::Field { codes, .. } if !self.hidden.is_empty() => codes
                .get(row)
                .is_some_and(|&c| self.hidden.get(c as usize).copied().unwrap_or(false)),
            _ => false,
        }
    }

    /// The series (colour codes, ascending) that are shown. For `group` this decides how many
    /// slots a band is split into: a hidden series gives its slot up.
    pub fn visible_series(&self) -> Vec<usize> {
        let ColorSource::Field { dict, .. } = self.color else {
            return vec![0];
        };
        (0..dict.len())
            .filter(|&c| !self.hidden.get(c).copied().unwrap_or(false))
            .collect()
    }

    /// The categories of `dict` that have at least one shown row, in dictionary order.
    /// Without hidden series that is all of them. A category only a hidden series used drops
    /// off the axis with it, so axes are computed from the data that is drawn.
    fn shown_categories(&self, codes: &[u32], dict: &[String]) -> Vec<String> {
        if self.hidden.is_empty() {
            return dict.to_vec();
        }
        let mut seen = vec![false; dict.len()];
        for row in (0..self.rows()).filter(|&r| !self.row_hidden(r)) {
            if let Some(slot) = codes.get(row).and_then(|&c| seen.get_mut(c as usize)) {
                *slot = true;
            }
        }
        dict.iter()
            .zip(seen)
            .filter(|&(_, shown)| shown)
            .map(|(c, _)| c.clone())
            .collect()
    }

    /// The x categories that are drawn (see [`shown_categories`](Self::shown_categories)).
    pub fn x_categories(&self) -> Vec<String> {
        self.shown_categories(self.x_codes, self.x_dict)
    }

    /// The z (depth) categories that are drawn.
    pub fn z_categories(&self) -> Vec<String> {
        self.shown_categories(self.z_codes, self.z_dict)
    }

    /// The rows merged into one cell per (x, series, z): what `stack`, `normalize` and
    /// `group` lay out. The series is the colour field's code (always a field for those
    /// positions; `validate` insists).
    pub fn cells(&self) -> Vec<Cell> {
        let ColorSource::Field { codes, .. } = self.color else {
            return Vec::new();
        };
        transform::aggregate((0..self.rows()).map(|row| Row {
            x: self.x_codes[row] as usize,
            series: codes[row] as usize,
            z: if self.z_axis.is_some() {
                self.z_codes[row] as usize
            } else {
                0
            },
            value: self.y[row],
            row,
        }))
    }

    /// The smallest and largest y value the layer's marks reach (`None` without finite
    /// values), after its `position` transform: raw values for `overlay`, stack tops and
    /// bottoms for `stack`, `(0, 1)` for `normalize` and summed values for `group`.
    pub fn extent(&self) -> Option<(f64, f64)> {
        match self.position {
            Position::Overlay => self
                .y
                .iter()
                .filter(|v| v.is_finite())
                .map(|&v| (v, v))
                .reduce(|(lo, hi), (a, b)| (lo.min(a), hi.max(b))),
            Position::Stack => {
                let cells = self.cells();
                transform::extent(&transform::stack(&cells, false))
            }
            Position::Normalize => (!self.cells().is_empty()).then_some((0.0, 1.0)),
            Position::Group => self
                .cells()
                .iter()
                .map(|c| (c.value, c.value))
                .reduce(|(lo, hi), (a, b)| (lo.min(a), hi.max(b))),
        }
    }

    /// Rows to draw. Tables are validated to have equal column lengths; `min` keeps this
    /// total (no out-of-bounds) even for a spec that was never validated.
    pub fn rows(&self) -> usize {
        let x_len = if self.x_numeric {
            self.x_num.len()
        } else {
            self.x_codes.len()
        };
        let mut n = x_len.min(self.y.len());
        if self.z_axis.is_some() {
            n = n.min(self.z_codes.len());
        }
        if let ColorSource::Field { codes, .. } = self.color {
            n = n.min(codes.len());
        }
        if let Some(k) = self.key {
            n = n.min(k.len());
        }
        n
    }

    /// The text of the key column at `row`, if the layer has an explicit key.
    pub fn key_text(&self, row: usize) -> Option<String> {
        Some(match self.key? {
            Column::Categorical { codes, dict } => dict[codes[row] as usize].clone(),
            // Rust's `Display` for f64 is the shortest text that round-trips: `1`, `2.5`.
            Column::Numeric(v) => v[row].to_string(),
        })
    }
}

/// Which axis orientations suit a channel.
#[derive(Clone, Copy)]
struct Channel {
    name: &'static str,
    allowed: &'static [Orient],
    needs: &'static str,
}

const X: Channel = Channel {
    name: "x",
    allowed: &[Orient::Bottom],
    needs: "a bottom axis",
};
const Y: Channel = Channel {
    name: "y",
    allowed: &[Orient::Left, Orient::Right],
    needs: "a left or right axis",
};

/// In 3D the value axis is the left one only; right axes are not supported there (yet).
const Y_3D: Channel = Channel {
    name: "y",
    allowed: &[Orient::Left],
    needs: "a left axis",
};
const Z: Channel = Channel {
    name: "z",
    allowed: &[Orient::Depth],
    needs: "a depth axis",
};

fn mark_name(mark: Mark) -> &'static str {
    match mark {
        Mark::Bar => "bar",
        Mark::Line => "line",
        Mark::Arc => "arc",
    }
}

fn orient_name(o: Orient) -> &'static str {
    match o {
        Orient::Bottom => "bottom",
        Orient::Left => "left",
        Orient::Right => "right",
        Orient::Depth => "depth",
    }
}

fn resolve_axis(
    spec: &ChartSpec,
    layer: usize,
    channel: Channel,
    enc: &PositionEncoding,
    issues: &mut Vec<Issue>,
) -> Option<usize> {
    let li = layer.to_string();
    let path = pointer(&["layers", &li, "encoding", channel.name, "axis"]);
    match &enc.axis {
        Some(id) => {
            let Some(i) = spec.axes.iter().position(|a| &a.id == id) else {
                let known: Vec<_> = spec.axes.iter().map(|a| a.id.as_str()).collect();
                let known = if known.is_empty() {
                    "no axes are declared".to_owned()
                } else {
                    format!("declared axes: {}", known.join(", "))
                };
                issues.push(Issue::new(path, format!("axis {id:?} not found ({known})")));
                return None;
            };
            let orient = spec.axes[i].orient;
            if !channel.allowed.contains(&orient) {
                issues.push(Issue::new(
                    path,
                    format!(
                        "axis {id:?} is oriented {}, but the {} channel needs {}",
                        orient_name(orient),
                        channel.name,
                        channel.needs
                    ),
                ));
                return None;
            }
            Some(i)
        }
        None => {
            let found = spec
                .axes
                .iter()
                .position(|a| channel.allowed.contains(&a.orient));
            if found.is_none() {
                issues.push(Issue::new(
                    path,
                    format!(
                        "the {} channel needs {}, but none is declared in `axes`",
                        channel.name, channel.needs
                    ),
                ));
            }
            found
        }
    }
}

/// Looks up a field in the layer's table, reporting a missing one at `path`.
fn column<'a>(
    table: &'a Table,
    field: &str,
    path: String,
    issues: &mut Vec<Issue>,
) -> Option<&'a Column> {
    let found = table.get(field);
    if found.is_none() {
        let known: Vec<_> = table.keys().map(String::as_str).collect();
        issues.push(Issue::new(
            path,
            format!(
                "field {field:?} not found in this table (fields: {})",
                known.join(", ")
            ),
        ));
    }
    found
}

/// Resolves layer `i`, or returns every problem that stops it from drawing.
pub(crate) fn resolve_layer(spec: &ChartSpec, i: usize) -> Result<ResolvedLayer<'_>, Vec<Issue>> {
    let layer: &LayerSpec = &spec.layers[i];
    let li = i.to_string();
    let at = |rest: &[&str]| {
        let mut parts = vec!["layers", li.as_str()];
        parts.extend_from_slice(rest);
        pointer(&parts)
    };
    let mut issues = Vec::new();
    let enc = &layer.encoding;
    // Arc layers have their own resolver; anything else here needs x and y.
    if layer.mark == Mark::Arc {
        return Err(issues);
    }
    let (Some(x_enc), Some(y_enc)) = (enc.x.as_ref(), enc.y.as_ref()) else {
        for (name, present) in [("x", enc.x.is_some()), ("y", enc.y.is_some())] {
            if !present {
                issues.push(Issue::new(
                    at(&["encoding", name]),
                    format!("{} layers need an `{name}` encoding", mark_name(layer.mark)),
                ));
            }
        }
        return Err(issues);
    };

    let three_d = spec.dimension == Dimension::ThreeD;
    let x_axis = resolve_axis(spec, i, X, x_enc, &mut issues);
    let y_axis = resolve_axis(spec, i, if three_d { Y_3D } else { Y }, y_enc, &mut issues);
    let mut z_axis = None;
    if let Some(z) = &enc.z {
        if three_d {
            z_axis = resolve_axis(spec, i, Z, z, &mut issues);
        } else {
            issues.push(Issue::new(
                at(&["encoding", "z"]),
                "the z channel needs `dimension: \"3d\"`",
            ));
        }
    }

    let Some(table) = spec.data.get(&layer.data) else {
        let known: Vec<_> = spec.data.keys().map(String::as_str).collect();
        issues.push(Issue::new(
            at(&["data"]),
            format!(
                "table {:?} not found (tables: {})",
                layer.data,
                known.join(", ")
            ),
        ));
        return Err(issues);
    };

    let x = column(
        table,
        &x_enc.field,
        at(&["encoding", "x", "field"]),
        &mut issues,
    );
    let y = column(
        table,
        &y_enc.field,
        at(&["encoding", "y", "field"]),
        &mut issues,
    );

    // An empty column has no values to disagree with, so it fits either kind: `"x": []`
    // is how "no data yet" is written.
    let (mut x_codes, mut x_dict): (&[u32], &[String]) = (&[], &[]);
    let (mut x_num, mut x_numeric): (&[f64], bool) = (&[], false);
    let is_line = layer.mark == Mark::Line;
    if let Some(x) = x {
        match x {
            Column::Categorical { codes, dict } => (x_codes, x_dict) = (codes, dict),
            // A line may run along a numeric x (a linear scale); bars need categories.
            Column::Numeric(v) if is_line && !v.is_empty() => (x_num, x_numeric) = (v, true),
            c if c.is_empty() => {}
            c => issues.push(Issue::new(
                at(&["encoding", "x", "field"]),
                format!(
                    "field {:?} is {}, but the x channel needs {}",
                    x_enc.field,
                    c.kind(),
                    if is_line {
                        "a categorical (string) or numeric field"
                    } else {
                        "a categorical (string) field"
                    }
                ),
            )),
        }
    }
    let mut y_values: &[f64] = &[];
    if let Some(y) = y {
        match y {
            Column::Numeric(v) => y_values = v,
            c => issues.push(Issue::new(
                at(&["encoding", "y", "field"]),
                format!(
                    "field {:?} is {}, but the y channel needs a numeric field",
                    y_enc.field,
                    c.kind()
                ),
            )),
        }
    }

    // z is categorical like x, with the same leniency for an empty column.
    let (mut z_codes, mut z_dict): (&[u32], &[String]) = (&[], &[]);
    if let (true, Some(z)) = (three_d, &enc.z) {
        if let Some(col) = column(
            table,
            &z.field,
            at(&["encoding", "z", "field"]),
            &mut issues,
        ) {
            match col {
                Column::Categorical { codes, dict } => (z_codes, z_dict) = (codes, dict),
                c if c.is_empty() => {}
                c => issues.push(Issue::new(
                    at(&["encoding", "z", "field"]),
                    format!(
                        "field {:?} is {}, but the z channel needs a categorical (string) field",
                        z.field,
                        c.kind()
                    ),
                )),
            }
        }
    }

    let color = resolve_color(
        table,
        enc.color.as_ref(),
        &at(&["encoding", "color"]),
        &mut issues,
    );

    let mut key = None;
    if let Some(k) = &enc.key {
        key = column(
            table,
            &k.field,
            at(&["encoding", "key", "field"]),
            &mut issues,
        );
    }

    check_position(layer, color.as_ref(), y_values, &at(&[]), &mut issues);
    check_style(layer, three_d, &at(&[]), &mut issues);

    match (x_axis, y_axis, color) {
        (Some(x_axis), Some(y_axis), Some(color)) if issues.is_empty() => {
            let hidden = hidden_codes(spec, &color);
            let y = match color {
                ColorSource::Field { codes, .. } if !hidden.is_empty() => Cow::Owned(
                    y_values
                        .iter()
                        .enumerate()
                        .map(|(row, &v)| hide(&hidden, codes, row, v))
                        .collect(),
                ),
                _ => Cow::Borrowed(y_values),
            };
            Ok(ResolvedLayer {
                index: i,
                mark: layer.mark,
                x_axis,
                y_axis,
                z_axis,
                x_codes,
                x_dict,
                x_num,
                x_numeric,
                y,
                z_codes,
                z_dict,
                color,
                key,
                position: layer.position,
                hidden,
            })
        }
        _ => Err(issues),
    }
}

/// Which of a colour field's categories are hidden, by code. Empty when none is (the common
/// case), so the rest of the code can skip the work.
fn hidden_codes(spec: &ChartSpec, color: &ColorSource) -> Vec<bool> {
    let ColorSource::Field { dict, .. } = color else {
        return Vec::new();
    };
    let names = spec.hidden_series();
    let flags: Vec<bool> = dict.iter().map(|c| names.contains(c)).collect();
    if flags.contains(&true) {
        flags
    } else {
        Vec::new()
    }
}

/// `value`, or NaN (missing) when `row` belongs to a hidden category.
fn hide(hidden: &[bool], codes: &[u32], row: usize, value: f64) -> f64 {
    let is_hidden = codes
        .get(row)
        .is_some_and(|&c| hidden.get(c as usize).copied().unwrap_or(false));
    if is_hidden { f64::NAN } else { value }
}

/// `style` and `point` only make sense on some marks, and their numbers must be usable.
fn check_style(layer: &LayerSpec, three_d: bool, layer_path: &str, issues: &mut Vec<Issue>) {
    let line = layer.mark == Mark::Line;
    let width_ok = |w: f64| w.is_finite() && w > 0.0 && w <= MAX_STROKE_WIDTH;
    if let Some(style) = &layer.style {
        if let Some(w) = style.stroke_width {
            if !line {
                issues.push(Issue::new(
                    format!("{layer_path}/style/strokeWidth"),
                    "strokeWidth applies to line layers",
                ));
            } else if !width_ok(w) {
                issues.push(Issue::new(
                    format!("{layer_path}/style/strokeWidth"),
                    format!(
                        "strokeWidth must be greater than 0 and at most {MAX_STROKE_WIDTH}, got {w}"
                    ),
                ));
            }
        }
    }
    if let Some(style) = &layer.style {
        if style.area.is_some() {
            if !line {
                issues.push(Issue::new(
                    format!("{layer_path}/style/area"),
                    "area applies to line layers",
                ));
            } else if !three_d {
                issues.push(Issue::new(
                    format!("{layer_path}/style/area"),
                    "area is the curtain under a 3D line; it needs `dimension: \"3d\"`",
                ));
            }
        }
    }
    if let Some(style) = &layer.style {
        if layer.mark != Mark::Arc {
            for (name, set) in [
                ("innerRadius", style.inner_radius.is_some()),
                ("padAngle", style.pad_angle.is_some()),
                ("startAngle", style.start_angle.is_some()),
                ("depth", style.depth.is_some()),
            ] {
                if set {
                    issues.push(Issue::new(
                        format!("{layer_path}/style/{name}"),
                        format!("{name} applies to arc layers"),
                    ));
                }
            }
        }
    }
    if let Some(point) = &layer.point {
        if !line {
            issues.push(Issue::new(
                format!("{layer_path}/point"),
                "point markers apply to line layers",
            ));
        } else if let Some(size) = point.size().filter(|&s| !width_ok(s)) {
            issues.push(Issue::new(
                format!("{layer_path}/point/size"),
                format!("size must be greater than 0 and at most {MAX_STROKE_WIDTH}, got {size}"),
            ));
        }
    }
}

/// `stack`, `normalize` and `group` need series (a colour field); `normalize` also needs
/// non-negative values, because a share of a total that mixes signs means nothing.
fn check_position(
    layer: &LayerSpec,
    color: Option<&ColorSource>,
    y: &[f64],
    layer_path: &str,
    issues: &mut Vec<Issue>,
) {
    if layer.mark != Mark::Bar && layer.position != Position::Overlay {
        issues.push(Issue::new(
            format!("{layer_path}/position"),
            "position applies to bar layers only",
        ));
        return;
    }
    let name = match layer.position {
        Position::Overlay => return,
        Position::Stack => "stack",
        Position::Normalize => "normalize",
        Position::Group => "group",
    };
    if matches!(color, Some(ColorSource::Default | ColorSource::Fixed(_))) {
        issues.push(Issue::new(
            format!("{layer_path}/position"),
            format!(
                "position {name:?} needs `encoding.color.field`: the series to {} come from it",
                if layer.position == Position::Group {
                    "place side by side"
                } else {
                    "stack"
                }
            ),
        ));
    }
    if layer.position == Position::Normalize {
        let negatives = y.iter().filter(|v| **v < 0.0).count();
        if let Some(first) = y.iter().position(|v| *v < 0.0) {
            issues.push(Issue::new(
                format!("{layer_path}/encoding/y/field"),
                format!(
                    "position \"normalize\" needs non-negative values, but row {first} is {} ({negatives} negative in all); use \"stack\" to stack positive and negative values",
                    y[first]
                ),
            ));
        }
    }
}

/// An arc layer (pie or donut) with its references resolved.
#[derive(Debug)]
pub(crate) struct ResolvedArc<'a> {
    pub index: usize,
    /// The slice sizes, with the rows of hidden slices set to NaN (no slice).
    pub theta: Cow<'a, [f64]>,
    /// Each row's slice (colour category) and the category names.
    pub codes: &'a [u32],
    pub dict: &'a [String],
    pub key: Option<&'a Column>,
}

impl ResolvedArc<'_> {
    pub fn rows(&self) -> usize {
        let n = self.theta.len().min(self.codes.len());
        self.key.map_or(n, |k| n.min(k.len()))
    }

    /// The text of the key column at `row`, if the layer has an explicit key.
    pub fn key_text(&self, row: usize) -> Option<String> {
        Some(match self.key? {
            Column::Categorical { codes, dict } => dict[codes[row] as usize].clone(),
            Column::Numeric(v) => v[row].to_string(),
        })
    }

    /// One cell per slice: rows of the same colour category are summed. Slices come in
    /// dictionary order of the colour field (the legend's order). Rows that are missing or
    /// not positive are no slice.
    pub fn cells(&self) -> Vec<Cell> {
        let mut cells = transform::aggregate((0..self.rows()).map(|row| Row {
            x: 0,
            series: self.codes[row] as usize,
            z: 0,
            value: self.theta[row],
            row,
        }));
        cells.retain(|c| c.value > 0.0);
        cells.sort_by_key(|c| c.series);
        cells
    }
}

/// The numbers of an arc layer's `style`, in the units the layout uses, with silly values
/// clamped (`validate` rejects them first; building stays total).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ArcStyle {
    /// Hole radius as a fraction of the outer radius, `0..0.95`.
    pub inner: f64,
    /// Gap between slices, radians.
    pub pad: f64,
    /// Where the first slice starts, radians clockwise from 12 o'clock.
    pub start: f64,
    /// 3D thickness as a fraction of the radius.
    pub depth: f64,
}

pub(crate) fn arc_style(layer: &LayerSpec) -> ArcStyle {
    let style = layer.style.as_ref();
    let get =
        |f: fn(&crate::spec::StyleSpec) -> Option<f64>| style.and_then(f).filter(|v| v.is_finite());
    ArcStyle {
        inner: get(|s| s.inner_radius).unwrap_or(0.0).clamp(0.0, 0.95),
        pad: get(|s| s.pad_angle)
            .unwrap_or(0.0)
            .clamp(0.0, 90.0)
            .to_radians(),
        start: get(|s| s.start_angle).unwrap_or(0.0).to_radians(),
        depth: get(|s| s.depth)
            .unwrap_or(crate::spec::DEFAULT_PIE_DEPTH)
            .clamp(0.0, 2.0),
    }
}

/// Resolves arc layer `i`, or returns every problem that stops it from drawing.
pub(crate) fn resolve_arc(spec: &ChartSpec, i: usize) -> Result<ResolvedArc<'_>, Vec<Issue>> {
    let layer: &LayerSpec = &spec.layers[i];
    let li = i.to_string();
    let at = |rest: &[&str]| {
        let mut parts = vec!["layers", li.as_str()];
        parts.extend_from_slice(rest);
        pointer(&parts)
    };
    let mut issues = Vec::new();
    let enc = &layer.encoding;

    for (name, set) in [
        ("x", enc.x.is_some()),
        ("y", enc.y.is_some()),
        ("z", enc.z.is_some()),
    ] {
        if set {
            issues.push(Issue::new(
                at(&["encoding", name]),
                format!(
                    "arc layers have no `{name}` channel: slices are sized by `theta` and coloured by `color`"
                ),
            ));
        }
    }
    check_position(layer, None, &[], &at(&[]), &mut issues);
    check_style(
        layer,
        spec.dimension == Dimension::ThreeD,
        &at(&[]),
        &mut issues,
    );
    check_arc_style(layer, spec.dimension, &at(&[]), &mut issues);

    let Some(table) = spec.data.get(&layer.data) else {
        let known: Vec<_> = spec.data.keys().map(String::as_str).collect();
        issues.push(Issue::new(
            at(&["data"]),
            format!(
                "table {:?} not found (tables: {})",
                layer.data,
                known.join(", ")
            ),
        ));
        return Err(issues);
    };

    let mut theta: &[f64] = &[];
    match &enc.theta {
        None => issues.push(Issue::new(
            at(&["encoding", "theta"]),
            "arc layers need a `theta` encoding: the numeric field that sizes each slice",
        )),
        Some(t) => {
            if let Some(col) = column(
                table,
                &t.field,
                at(&["encoding", "theta", "field"]),
                &mut issues,
            ) {
                match col {
                    Column::Numeric(v) => {
                        theta = v;
                        if let Some(row) = v.iter().position(|x| *x < 0.0) {
                            issues.push(Issue::new(
                                at(&["encoding", "theta", "field"]),
                                format!(
                                    "theta values must not be negative, but row {row} is {}",
                                    v[row]
                                ),
                            ));
                        }
                    }
                    c => issues.push(Issue::new(
                        at(&["encoding", "theta", "field"]),
                        format!(
                            "field {:?} is {}, but the theta channel needs a numeric field",
                            t.field,
                            c.kind()
                        ),
                    )),
                }
            }
        }
    }

    let color = match &enc.color {
        Some(ColorEncoding {
            field: Some(_),
            value: None,
        }) => resolve_color(
            table,
            enc.color.as_ref(),
            &at(&["encoding", "color"]),
            &mut issues,
        ),
        _ => {
            issues.push(Issue::new(
                at(&["encoding", "color"]),
                "arc layers colour each slice by a categorical field: `color: { \"field\": ... }`",
            ));
            None
        }
    };
    let key = enc.key.as_ref().and_then(|k| {
        column(
            table,
            &k.field,
            at(&["encoding", "key", "field"]),
            &mut issues,
        )
    });

    match color {
        Some(ColorSource::Field { codes, dict }) if issues.is_empty() => {
            let hidden = hidden_codes(spec, &ColorSource::Field { codes, dict });
            let theta = if hidden.is_empty() {
                Cow::Borrowed(theta)
            } else {
                Cow::Owned(
                    theta
                        .iter()
                        .enumerate()
                        .map(|(row, &v)| hide(&hidden, codes, row, v))
                        .collect(),
                )
            };
            Ok(ResolvedArc {
                index: i,
                theta,
                codes,
                dict,
                key,
            })
        }
        _ => Err(issues),
    }
}

/// The numbers in an arc's `style` must make a drawable pie.
fn check_arc_style(
    layer: &LayerSpec,
    dimension: Dimension,
    layer_path: &str,
    issues: &mut Vec<Issue>,
) {
    let Some(style) = &layer.style else { return };
    let mut bad = |name: &str, message: String| {
        issues.push(Issue::new(format!("{layer_path}/style/{name}"), message));
    };
    if let Some(v) = style.inner_radius {
        if !(v.is_finite() && (0.0..1.0).contains(&v)) {
            bad(
                "innerRadius",
                format!("innerRadius must be from 0 up to (not including) 1, got {v}"),
            );
        }
    }
    if let Some(v) = style.pad_angle {
        if !(v.is_finite() && (0.0..=90.0).contains(&v)) {
            bad(
                "padAngle",
                format!("padAngle is in degrees, from 0 to 90, got {v}"),
            );
        }
    }
    if let Some(v) = style.start_angle {
        if !v.is_finite() {
            bad(
                "startAngle",
                "startAngle must be a finite number of degrees".to_owned(),
            );
        }
    }
    if let Some(v) = style.depth {
        if dimension != Dimension::ThreeD {
            bad(
                "depth",
                "depth is the thickness of a 3D pie; it needs `dimension: \"3d\"`".to_owned(),
            );
        } else if !(v.is_finite() && (0.0..=2.0).contains(&v)) {
            bad(
                "depth",
                format!("depth is a fraction of the radius, from 0 to 2, got {v}"),
            );
        }
    }
}

/// Rules that involve more than one layer or the axes: arcs are a chart of their own.
fn check_arcs(spec: &ChartSpec, issues: &mut Vec<Issue>) {
    let arcs: Vec<usize> = (0..spec.layers.len())
        .filter(|&i| spec.layers[i].mark == Mark::Arc)
        .collect();
    let Some(&first) = arcs.first() else { return };
    for (i, axis) in spec.axes.iter().enumerate() {
        issues.push(Issue::new(
            pointer(&["axes", &i.to_string()]),
            format!(
                "pie and donut charts have no axes (layer {first} is an arc layer); remove axis {:?}",
                axis.id
            ),
        ));
    }
    for (i, layer) in spec.layers.iter().enumerate() {
        let path = pointer(&["layers", &i.to_string(), "mark"]);
        if layer.mark != Mark::Arc {
            issues.push(Issue::new(
                path,
                format!(
                    "arc layers cannot be combined with other marks (layer {first} is an arc layer)"
                ),
            ));
        } else if i != first {
            issues.push(Issue::new(
                path,
                format!("only one arc layer per chart for now (layer {first} is the first)"),
            ));
        }
    }
}

fn resolve_color<'a>(
    table: &'a Table,
    enc: Option<&ColorEncoding>,
    path: &str,
    issues: &mut Vec<Issue>,
) -> Option<ColorSource<'a>> {
    let Some(enc) = enc else {
        return Some(ColorSource::Default);
    };
    match (&enc.value, &enc.field) {
        (Some(value), None) => match Color::from_hex(value) {
            Ok(c) => Some(ColorSource::Fixed(c)),
            Err(e) => {
                issues.push(Issue::new(format!("{path}/value"), e.to_string()));
                None
            }
        },
        (None, Some(field)) => {
            let col = column(table, field, format!("{path}/field"), issues)?;
            match col {
                Column::Categorical { codes, dict } => Some(ColorSource::Field { codes, dict }),
                c if c.is_empty() => Some(ColorSource::Field {
                    codes: &[],
                    dict: &[],
                }),
                c => {
                    issues.push(Issue::new(
                        format!("{path}/field"),
                        format!(
                            "field {field:?} is {}; colouring by a numeric field is not supported yet, use a categorical (string) field",
                            c.kind()
                        ),
                    ));
                    None
                }
            }
        }
        _ => {
            issues.push(Issue::new(
                path.to_owned(),
                "color needs exactly one of `value` (a fixed colour) or `field` (a categorical field)",
            ));
            None
        }
    }
}

fn validate_axes(axes: &[AxisSpec], dimension: Dimension, issues: &mut Vec<Issue>) {
    let three_d = dimension == Dimension::ThreeD;
    for (i, axis) in axes.iter().enumerate() {
        let n = i.to_string();
        let at = |rest: &[&str]| {
            let mut parts = vec!["axes", n.as_str()];
            parts.extend_from_slice(rest);
            pointer(&parts)
        };
        if axes[..i].iter().any(|a| a.id == axis.id) {
            issues.push(Issue::new(
                at(&["id"]),
                format!("duplicate axis id {:?}", axis.id),
            ));
        }
        match axis.orient {
            Orient::Depth if !three_d => issues.push(Issue::new(
                at(&["orient"]),
                "depth axes need `dimension: \"3d\"`",
            )),
            Orient::Right if three_d => issues.push(Issue::new(
                at(&["orient"]),
                "right axes are not supported in 3D charts yet; use `left`",
            )),
            o if axes[..i].iter().any(|a| a.orient == o) => issues.push(Issue::new(
                at(&["orient"]),
                format!("only one {} axis is supported yet", orient_name(o)),
            )),
            _ => {}
        }
        if let Some(scale) = &axis.scale {
            // A bottom axis is numeric only when a line layer's x is; `check_x_axes` knows.
            if axis.orient == Orient::Depth {
                issues.push(Issue::new(
                    at(&["scale"]),
                    "scale overrides apply to numeric axes (left/right/bottom); a depth axis shows categories",
                ));
            }
            if let Some([lo, hi]) = scale.domain {
                if !(lo.is_finite() && hi.is_finite() && lo < hi) {
                    issues.push(Issue::new(
                        at(&["scale", "domain"]),
                        "domain must be [min, max] with finite numbers and min < max",
                    ));
                }
            }
        }
    }
}

/// Numbers only need to be finite (the renderer clamps the angles) and the distance positive.
fn validate_camera(camera: &CameraSpec, issues: &mut Vec<Issue>) {
    let fields = [
        ("azimuth", camera.azimuth),
        ("elevation", camera.elevation),
        ("distance", camera.distance),
    ];
    for (name, value) in fields {
        match value {
            Some(v) if !v.is_finite() => issues.push(Issue::new(
                pointer(&["camera", name]),
                format!("{name} must be a finite number"),
            )),
            Some(v) if name == "distance" && v <= 0.0 => issues.push(Issue::new(
                pointer(&["camera", name]),
                "distance must be greater than 0",
            )),
            _ => {}
        }
    }
}

/// Checks `spec` and returns every problem found (empty means it will draw).
///
/// Empty data is **valid**: a table whose columns have no rows draws the axes (numeric
/// domain `[0, 1]`, no categories) and no bars, so a chart can be shown before its data arrives.
pub fn validate(spec: &ChartSpec) -> Vec<Issue> {
    let mut issues = Vec::new();

    match (spec.dimension, &spec.camera) {
        (Dimension::TwoD, Some(_)) => issues.push(Issue::new(
            pointer(&["camera"]),
            "a camera needs `dimension: \"3d\"`",
        )),
        (Dimension::ThreeD, Some(camera)) => validate_camera(camera, &mut issues),
        _ => {}
    }

    for (name, table) in &spec.data {
        let mut lens = table.iter().map(|(k, c)| (k, c.len()));
        if let Some((first_name, first_len)) = lens.next() {
            if let Some((bad_name, bad_len)) = lens.find(|&(_, n)| n != first_len) {
                issues.push(Issue::new(
                    pointer(&["data", name]),
                    format!(
                        "columns must have equal lengths: {first_name:?} has {first_len} but {bad_name:?} has {bad_len}"
                    ),
                ));
            }
        }
    }

    validate_axes(&spec.axes, spec.dimension, &mut issues);

    for (i, layer) in spec.layers.iter().enumerate() {
        let resolved = if layer.mark == Mark::Arc {
            resolve_arc(spec, i).map(drop)
        } else {
            resolve_layer(spec, i).map(drop)
        };
        if let Err(mut layer_issues) = resolved {
            issues.append(&mut layer_issues);
        }
        let percent = layer
            .label
            .as_ref()
            .and_then(|l| l.format.as_ref())
            .and_then(|f| f.percent)
            .unwrap_or(false);
        if percent
            && !matches!(layer.mark, Mark::Arc)
            && !matches!(layer.position, Position::Stack | Position::Normalize)
        {
            issues.push(Issue::new(
                pointer(&["layers", &i.to_string(), "label", "format", "percent"]),
                "percent labels need an arc layer or a `stack`/`normalize` bar layer (the share of what?)",
            ));
        }
        let decimals = layer
            .label
            .as_ref()
            .and_then(|l| l.format.as_ref())
            .and_then(|f| f.decimals);
        if let Some(d) = decimals.filter(|&d| d > MAX_LABEL_DECIMALS) {
            issues.push(Issue::new(
                pointer(&["layers", &i.to_string(), "label", "format", "decimals"]),
                format!("decimals must be between 0 and {MAX_LABEL_DECIMALS}, got {d}"),
            ));
        }
    }
    let resolved: Vec<ResolvedLayer> = (0..spec.layers.len())
        .filter_map(|i| resolve_layer(spec, i).ok())
        .collect();
    check_arcs(spec, &mut issues);
    check_hidden(spec, &mut issues);
    check_percent_axes(spec, &resolved, &mut issues);
    check_x_axes(spec, &resolved, &mut issues);
    issues
}

/// `legend.hidden` may name anything (a name that is not a series hides nothing, so a spec can
/// keep a hidden name while the data comes and goes), but it must leave a series to draw:
/// a chart with every series hidden is blank and the viewer could not tell why.
fn check_hidden(spec: &ChartSpec, issues: &mut Vec<Issue>) {
    let Some(crate::spec::LegendSpec::Config(config)) = &spec.legend else {
        return;
    };
    let Some(hidden) = config.hidden.as_deref().filter(|h| !h.is_empty()) else {
        return;
    };
    let series = crate::legend::series_names(spec);
    if !series.is_empty() && series.iter().all(|s| hidden.contains(s)) {
        issues.push(Issue::new(
            pointer(&["legend", "hidden"]),
            "legend.hidden hides every series; leave at least one visible",
        ));
    }
}

/// A bottom axis is a band scale (categories) or a linear one (numbers, from line layers). All
/// layers on it must agree, and `scale` overrides only make sense on the linear kind.
fn check_x_axes(spec: &ChartSpec, resolved: &[ResolvedLayer], issues: &mut Vec<Issue>) {
    for (i, axis) in spec.axes.iter().enumerate() {
        if axis.orient != Orient::Bottom {
            continue;
        }
        let bound: Vec<&ResolvedLayer> = resolved.iter().filter(|l| l.x_axis == i).collect();
        let numeric = bound.iter().any(|l| l.x_numeric);
        if axis.scale.is_some() && !numeric {
            issues.push(Issue::new(
                pointer(&["axes", &i.to_string(), "scale"]),
                "scale overrides apply to numeric axes; a bottom axis is numeric only when a line layer's x field is numeric",
            ));
        }
        if numeric {
            for l in bound
                .iter()
                .filter(|l| !l.x_numeric && !l.x_dict.is_empty())
            {
                issues.push(Issue::new(
                    pointer(&["layers", &l.index.to_string(), "encoding", "x", "field"]),
                    format!(
                        "axis {:?} is numeric (another layer's x is a number), but this x field is categorical; layers sharing a bottom axis need the same kind of x",
                        axis.id
                    ),
                ));
            }
        }
    }
}

/// A y axis that shows a normalised layer reads 0-100%, so a layer with raw values cannot
/// share it. Reported on every layer whose kind differs from the first one on that axis.
fn check_percent_axes(spec: &ChartSpec, resolved: &[ResolvedLayer], issues: &mut Vec<Issue>) {
    for (n, layer) in resolved.iter().enumerate() {
        let first = resolved[..n].iter().find(|l| l.y_axis == layer.y_axis);
        let Some(first) = first else { continue };
        if (first.position == Position::Normalize) != (layer.position == Position::Normalize) {
            let (normalised, raw) = if first.position == Position::Normalize {
                (first.index, layer.index)
            } else {
                (layer.index, first.index)
            };
            issues.push(Issue::new(
                pointer(&["layers", &layer.index.to_string(), "encoding", "y", "axis"]),
                format!(
                    "axis {:?} shows percentages for the \"normalize\" layer {normalised}, so it cannot also show the raw values of layer {raw}; give one of them its own axis",
                    spec.axes[layer.y_axis].id,
                ),
            ));
        }
    }
}
