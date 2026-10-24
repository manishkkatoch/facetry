//! Parsing and validation. Both report **every** problem found, each with a JSON-pointer
//! `path` into the spec (`/layers/0/encoding/y/axis`), so an editor can underline them all
//! at once instead of making the user fix one error per run.
//!
//! The lookups a layer needs (its table, columns, axes, colour) are done once, in
//! [`resolve_layer`]. `validate` calls it and reports failures; `build_scene` calls it and
//! skips layers that fail. One code path means they can never disagree.

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
    pub y: &'a [f64],
    pub z_codes: &'a [u32],
    pub z_dict: &'a [String],
    pub color: ColorSource<'a>,
    pub key: Option<&'a Column>,
    pub position: Position,
}

impl ResolvedLayer<'_> {
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

    /// Number of series (distinct colour categories) of a layer coloured by field.
    pub fn series_count(&self) -> usize {
        match self.color {
            ColorSource::Field { dict, .. } => dict.len(),
            _ => 1,
        }
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

    let three_d = spec.dimension == Dimension::ThreeD;
    let x_axis = resolve_axis(spec, i, X, &enc.x, &mut issues);
    let y_axis = resolve_axis(spec, i, if three_d { Y_3D } else { Y }, &enc.y, &mut issues);
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
        &enc.x.field,
        at(&["encoding", "x", "field"]),
        &mut issues,
    );
    let y = column(
        table,
        &enc.y.field,
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
                    enc.x.field,
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
                    enc.y.field,
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
    check_style(layer, &at(&[]), &mut issues);
    if three_d && is_line {
        issues.push(Issue::new(
            at(&["mark"]),
            "line charts are 2D only for now; use `dimension: \"2d\"`",
        ));
    }

    match (x_axis, y_axis, color) {
        (Some(x_axis), Some(y_axis), Some(color)) if issues.is_empty() => Ok(ResolvedLayer {
            index: i,
            mark: layer.mark,
            x_axis,
            y_axis,
            z_axis,
            x_codes,
            x_dict,
            x_num,
            x_numeric,
            y: y_values,
            z_codes,
            z_dict,
            color,
            key,
            position: layer.position,
        }),
        _ => Err(issues),
    }
}

/// `style` and `point` only make sense on some marks, and their numbers must be usable.
fn check_style(layer: &LayerSpec, layer_path: &str, issues: &mut Vec<Issue>) {
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
            "position applies to bar layers; a line is drawn as it is",
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
        if let Err(mut layer_issues) = resolve_layer(spec, i) {
            issues.append(&mut layer_issues);
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
    check_percent_axes(spec, &resolved, &mut issues);
    check_x_axes(spec, &resolved, &mut issues);
    issues
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
