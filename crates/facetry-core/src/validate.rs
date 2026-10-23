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
    AxisSpec, ChartSpec, ColorEncoding, Column, Dimension, LayerSpec, Orient, PositionEncoding,
    Table,
};

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
    /// Indices into `spec.axes`.
    pub x_axis: usize,
    pub y_axis: usize,
    pub x_codes: &'a [u32],
    pub x_dict: &'a [String],
    pub y: &'a [f64],
    pub color: ColorSource<'a>,
    pub key: Option<&'a Column>,
}

impl ResolvedLayer<'_> {
    /// Rows to draw. Tables are validated to have equal column lengths; `min` keeps this
    /// total (no out-of-bounds) even for a spec that was never validated.
    pub fn rows(&self) -> usize {
        let mut n = self.x_codes.len().min(self.y.len());
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

    let x_axis = resolve_axis(spec, i, X, &enc.x, &mut issues);
    let y_axis = resolve_axis(spec, i, Y, &enc.y, &mut issues);

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
    if let Some(x) = x {
        match x {
            Column::Categorical { codes, dict } => (x_codes, x_dict) = (codes, dict),
            c if c.is_empty() => {}
            c => issues.push(Issue::new(
                at(&["encoding", "x", "field"]),
                format!(
                    "field {:?} is {}, but the x channel needs a categorical (string) field",
                    enc.x.field,
                    c.kind()
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

    match (x_axis, y_axis, color) {
        (Some(x_axis), Some(y_axis), Some(color)) if issues.is_empty() => Ok(ResolvedLayer {
            index: i,
            x_axis,
            y_axis,
            x_codes,
            x_dict,
            y: y_values,
            color,
            key,
        }),
        _ => Err(issues),
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

fn validate_axes(axes: &[AxisSpec], issues: &mut Vec<Issue>) {
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
            Orient::Depth => issues.push(Issue::new(
                at(&["orient"]),
                "depth axes belong to 3D charts, which are not supported yet",
            )),
            o if axes[..i].iter().any(|a| a.orient == o) => issues.push(Issue::new(
                at(&["orient"]),
                format!("only one {} axis is supported yet", orient_name(o)),
            )),
            _ => {}
        }
        if let Some(scale) = &axis.scale {
            if axis.orient == Orient::Bottom {
                issues.push(Issue::new(
                    at(&["scale"]),
                    "scale overrides apply to numeric axes (left/right); a bottom axis shows categories",
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

/// Checks `spec` and returns every problem found (empty means it will draw).
///
/// Empty data is **valid**: a table whose columns have no rows draws the axes (numeric
/// domain `[0, 1]`, no categories) and no bars, so a chart can be shown before its data arrives.
pub fn validate(spec: &ChartSpec) -> Vec<Issue> {
    let mut issues = Vec::new();

    if spec.dimension == Dimension::ThreeD {
        issues.push(Issue::new(
            pointer(&["dimension"]),
            "3D charts are not supported yet; use \"2d\"",
        ));
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

    validate_axes(&spec.axes, &mut issues);

    for i in 0..spec.layers.len() {
        if let Err(mut layer_issues) = resolve_layer(spec, i) {
            issues.append(&mut layer_issues);
        }
    }
    issues
}
