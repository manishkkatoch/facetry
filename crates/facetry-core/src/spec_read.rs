//! Turns a [`Value`] into a [`ChartSpec`] by hand, with the same strictness the serde
//! derives it replaced had: unknown fields are errors, so are wrong types, and an error
//! carries the JSON-pointer path of the value that caused it.
//!
//! Only the **first** problem is reported (a bad shape makes the rest meaningless; semantic
//! checks live in [`crate::validate`]). Objects are read in input order, so when a document
//! has several problems the first one in the text wins, as before.
//!
//! The messages follow serde's wording (`unknown field`, `missing field`, `invalid type: ...,
//! expected ...`) so they stay familiar and tests can compare them with the old parser.

use std::collections::{BTreeMap, HashMap};

use crate::spec::{
    AxisSpec, CameraKind, CameraSpec, ChartSpec, ColorEncoding, Column, DiagramEdge, DiagramNode,
    DiagramSpec, DiagramStyle, Dimension, EdgeEnd, EdgeStroke, Encoding, InteractionSpec,
    KeyEncoding, LabelFormat, LabelShow, LabelSpec, LayerSpec, LegendConfig, LegendOrient,
    LegendSpec, Mark, NodeShape, Orient, PointConfig, PointSpec, Position, PositionEncoding,
    ScaleSpec, StyleSpec, Table, ThetaEncoding,
};
use crate::validate::{Issue, pointer};
use crate::value::Value;

/// A shape error. `path` is built from the inside out as the error travels up, so it holds
/// the pointer segments in reverse.
struct Fail {
    path: Vec<String>,
    message: String,
}

type Read<T> = Result<T, Fail>;

impl Fail {
    fn new(message: impl Into<String>) -> Self {
        Self {
            path: Vec::new(),
            message: message.into(),
        }
    }

    /// The error happened inside the value at `key` (or index) of the current container.
    fn under(mut self, segment: String) -> Self {
        self.path.push(segment);
        self
    }

    fn at_key(self, key: &str) -> Self {
        self.under(pointer(&[key]))
    }
}

/// `invalid type: <what it was>, expected <what we wanted>`.
fn bad_type(got: &Value, expected: &str) -> Fail {
    Fail::new(format!(
        "invalid type: {}, expected {expected}",
        got.describe()
    ))
}

/// serde's list of valid names: `` `a` ``, `` `a` or `b` ``, `` one of `a`, `b`, `c` ``.
fn one_of(names: &[&str]) -> String {
    match names {
        [one] => format!("`{one}`"),
        [a, b] => format!("`{a}` or `{b}`"),
        _ => {
            let list: Vec<String> = names.iter().map(|n| format!("`{n}`")).collect();
            format!("one of {}", list.join(", "))
        }
    }
}

// --- scalars and containers -----------------------------------------------------------------

fn string(v: Value) -> Read<String> {
    match v {
        Value::Str(s) => Ok(s),
        other => Err(bad_type(&other, "a string")),
    }
}

fn boolean(v: Value) -> Read<bool> {
    match v {
        Value::Bool(b) => Ok(b),
        other => Err(bad_type(&other, "a boolean")),
    }
}

fn number(v: Value) -> Read<f64> {
    match v {
        Value::Int(i) => Ok(i as f64),
        Value::Uint(u) => Ok(u as f64),
        Value::Float(f) => Ok(f),
        other => Err(bad_type(&other, "f64")),
    }
}

/// A whole number from 0 to `u32::MAX`; `2.0` is not one.
fn count(v: Value) -> Read<u32> {
    let out_of_range = |n: &dyn std::fmt::Display| {
        Fail::new(format!("invalid value: integer `{n}`, expected u32"))
    };
    match v {
        Value::Int(i) => u32::try_from(i).map_err(|_| out_of_range(&i)),
        Value::Uint(u) => Err(out_of_range(&u)),
        other => Err(bad_type(&other, "u32")),
    }
}

/// `null` is "absent"; anything else goes to `read`.
fn opt<T>(read: impl FnOnce(Value) -> Read<T>) -> impl FnOnce(Value) -> Read<Option<T>> {
    move |v| match v {
        Value::Null => Ok(None),
        v => read(v).map(Some),
    }
}

/// An array, each element read by `read`; an error names the element's index.
fn list<T>(read: fn(Value) -> Read<T>) -> impl FnOnce(Value) -> Read<Vec<T>> {
    move |v| match v {
        Value::Array(items) => items
            .into_iter()
            .enumerate()
            .map(|(i, item)| read(item).map_err(|e| e.under(format!("/{i}"))))
            .collect(),
        other => Err(bad_type(&other, "a sequence")),
    }
}

/// An object as `(key, value)` pairs in input order.
fn entries(v: Value, expected: &str) -> Read<Vec<(String, Value)>> {
    match v {
        Value::Object(entries) => Ok(entries),
        other => Err(bad_type(&other, expected)),
    }
}

/// An object used as a map: every value read by `read`. A repeated key keeps the last value.
fn map<T>(read: fn(Value) -> Read<T>) -> impl FnOnce(Value) -> Read<BTreeMap<String, T>> {
    move |v| {
        let mut out = BTreeMap::new();
        for (key, value) in entries(v, "a map")? {
            let item = read(value).map_err(|e| e.at_key(&key))?;
            out.insert(key, item);
        }
        Ok(out)
    }
}

/// A fixed-size pair. Only the first two elements are looked at, so extra ones are an error
/// of the array itself, not of the element.
fn pair(v: Value) -> Read<[f64; 2]> {
    const EXPECTED: &str = "an array of length 2";
    let Value::Array(items) = v else {
        return Err(bad_type(&v, EXPECTED));
    };
    let len = items.len();
    let mut out = [0.0; 2];
    for (i, item) in items.into_iter().take(2).enumerate() {
        out[i] = number(item).map_err(|e| e.under(format!("/{i}")))?;
    }
    if len == 2 {
        Ok(out)
    } else {
        Err(Fail::new(format!(
            "invalid length {len}, expected {EXPECTED}"
        )))
    }
}

/// A unit enum written as its name, or (as serde_json also allowed) `{ "name": null }`.
fn variant<T: Copy>(v: Value, variants: &[(&str, T)]) -> Read<T> {
    let (name, payload, extra) = match v {
        Value::Str(s) => (s, Value::Null, 0),
        Value::Object(mut e) if !e.is_empty() => {
            let (name, payload) = e.remove(0);
            (name, payload, e.len())
        }
        other => return Err(bad_type(&other, "a string")),
    };
    let Some(&(_, found)) = variants.iter().find(|(n, _)| *n == name) else {
        let names: Vec<&str> = variants.iter().map(|(n, _)| *n).collect();
        let expected = one_of(&names);
        return Err(Fail::new(format!(
            "unknown variant `{name}`, expected {expected}"
        )));
    };
    match payload {
        Value::Null if extra == 0 => Ok(found),
        Value::Null => Err(Fail::new(format!(
            "invalid length {}, expected a map with one entry",
            extra + 1
        ))),
        other => Err(bad_type(&other, "unit")),
    }
}

/// Reads an object field by field, in input order, like a derived `Deserialize` with
/// `deny_unknown_fields`:
///
/// * `Name { field [req|def] = "key" => reader, ... }`: `req` fields must be present
///   (otherwise `missing field`), `def` fields default when absent;
/// * an unknown key, or a key given twice, is an error (the first at the key's path, the
///   second at the object's);
/// * trailing `; field = expr` entries fill fields that are not in the JSON.
macro_rules! object {
    ($v:expr, $name:literal, $T:ident {
        $($f:ident [$kind:ident] = $key:literal => $read:expr),+ $(,)?
        $(; $($extra:ident = $e:expr),+)?
    }) => {{
        $(let mut $f = None;)+
        for (key, value) in entries($v, concat!("struct ", $name))? {
            match key.as_str() {
                $($key => {
                    if $f.is_some() {
                        return Err(Fail::new(format!("duplicate field `{}`", $key)));
                    }
                    $f = Some(($read)(value).map_err(|e| e.at_key(&key))?);
                })+
                _ => {
                    let msg = format!(
                        "unknown field `{key}`, expected {}",
                        one_of(&[$($key),+])
                    );
                    return Err(Fail::new(msg).at_key(&key));
                }
            }
        }
        Ok($T {
            $($f: object!(@finish $kind, $f, $key),)+
            $($($extra: $e,)+)?
        })
    }};
    (@finish req, $f:ident, $key:literal) => {
        $f.ok_or_else(|| Fail::new(concat!("missing field `", $key, "`")))?
    };
    (@finish def, $f:ident, $key:literal) => {
        $f.unwrap_or_default()
    };
}

/// A `bool | { ... }` field (serde's untagged enums): anything else, or an object that does
/// not fit, is the same single error at the field's path.
fn untagged<T>(
    v: Value,
    name: &str,
    on_bool: fn(bool) -> T,
    on_object: fn(Value) -> Read<T>,
) -> Read<T> {
    let no_match = || {
        Fail::new(format!(
            "data did not match any variant of untagged enum {name}"
        ))
    };
    match v {
        Value::Bool(b) => Ok(on_bool(b)),
        Value::Object(_) => on_object(v).map_err(|_| no_match()),
        _ => Err(no_match()),
    }
}

// --- the spec -------------------------------------------------------------------------------

/// Reads a whole spec. The one place the shape errors turn into [`Issue`]s.
pub fn parse_spec_value(v: Value) -> Result<ChartSpec, Vec<Issue>> {
    read_chart_spec(v).map_err(|f| {
        let path: String = f.path.iter().rev().map(String::as_str).collect();
        vec![Issue::new(path, f.message)]
    })
}

fn read_chart_spec(v: Value) -> Read<ChartSpec> {
    object!(v, "ChartSpec", ChartSpec {
        dimension [def] = "dimension" => read_dimension,
        data [def] = "data" => map(read_table),
        axes [def] = "axes" => list(read_axis),
        layers [def] = "layers" => list(read_layer),
        legend [def] = "legend" => opt(read_legend),
        camera [def] = "camera" => opt(read_camera),
        interaction [def] = "interaction" => opt(read_interaction),
        diagram [def] = "diagram" => opt(read_diagram);
        hidden_override = None
    })
}

fn read_dimension(v: Value) -> Read<Dimension> {
    variant(v, &[("2d", Dimension::TwoD), ("3d", Dimension::ThreeD)])
}

fn read_table(v: Value) -> Read<Table> {
    map(read_column)(v)
}

/// A column is all numbers (`null` = NaN) or all strings, which are dictionary-encoded.
fn read_column(v: Value) -> Read<Column> {
    const EXPECTED: &str = "an array of numbers or an array of strings";
    let Value::Array(cells) = v else {
        return Err(bad_type(&v, EXPECTED));
    };
    let mut numbers = Vec::new();
    let mut texts: Vec<String> = Vec::new();
    let len = cells.len();
    for (i, cell) in cells.into_iter().enumerate() {
        match cell {
            Value::Null => numbers.push(f64::NAN),
            Value::Str(s) => texts.push(s),
            Value::Int(n) => numbers.push(n as f64),
            Value::Uint(n) => numbers.push(n as f64),
            Value::Float(n) => numbers.push(n),
            other => {
                return Err(bad_type(&other, "a number, a string or null").under(format!("/{i}")));
            }
        }
    }
    if texts.is_empty() {
        return Ok(Column::Numeric(numbers));
    }
    // Anything that is not a string (numbers, nulls) makes the column ambiguous.
    if texts.len() != len {
        return Err(Fail::new(
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

fn read_axis(v: Value) -> Read<AxisSpec> {
    object!(v, "AxisSpec", AxisSpec {
        id [req] = "id" => string,
        orient [req] = "orient" => read_orient,
        title [def] = "title" => opt(string),
        scale [def] = "scale" => opt(read_scale),
    })
}

fn read_orient(v: Value) -> Read<Orient> {
    variant(
        v,
        &[
            ("bottom", Orient::Bottom),
            ("left", Orient::Left),
            ("right", Orient::Right),
            ("depth", Orient::Depth),
        ],
    )
}

fn read_scale(v: Value) -> Read<ScaleSpec> {
    object!(v, "ScaleSpec", ScaleSpec {
        domain [def] = "domain" => opt(pair),
        nice [def] = "nice" => opt(boolean),
        zero [def] = "zero" => opt(boolean),
    })
}

fn read_legend(v: Value) -> Read<LegendSpec> {
    untagged(v, "LegendSpec", LegendSpec::Shown, |v| {
        read_legend_config(v).map(LegendSpec::Config)
    })
}

fn read_legend_config(v: Value) -> Read<LegendConfig> {
    object!(v, "LegendConfig", LegendConfig {
        orient [def] = "orient" => opt(read_legend_orient),
        title [def] = "title" => opt(string),
        interactive [def] = "interactive" => opt(boolean),
        hidden [def] = "hidden" => opt(list(string)),
    })
}

fn read_legend_orient(v: Value) -> Read<LegendOrient> {
    variant(
        v,
        &[
            ("right", LegendOrient::Right),
            ("bottom", LegendOrient::Bottom),
            ("top", LegendOrient::Top),
            ("left", LegendOrient::Left),
        ],
    )
}

fn read_camera(v: Value) -> Read<CameraSpec> {
    object!(v, "CameraSpec", CameraSpec {
        kind [def] = "type" => opt(read_camera_kind),
        azimuth [def] = "azimuth" => opt(number),
        elevation [def] = "elevation" => opt(number),
        distance [def] = "distance" => opt(number),
    })
}

fn read_camera_kind(v: Value) -> Read<CameraKind> {
    variant(
        v,
        &[
            ("perspective", CameraKind::Perspective),
            ("orthographic", CameraKind::Orthographic),
        ],
    )
}

fn read_interaction(v: Value) -> Read<InteractionSpec> {
    object!(v, "InteractionSpec", InteractionSpec {
        orbit [def] = "orbit" => opt(boolean),
    })
}

fn read_layer(v: Value) -> Read<LayerSpec> {
    object!(v, "LayerSpec", LayerSpec {
        mark [req] = "mark" => read_mark,
        data [req] = "data" => string,
        encoding [req] = "encoding" => read_encoding,
        position [def] = "position" => read_position,
        point [def] = "point" => opt(read_point),
        style [def] = "style" => opt(read_style),
        label [def] = "label" => opt(read_label),
    })
}

fn read_mark(v: Value) -> Read<Mark> {
    variant(
        v,
        &[("bar", Mark::Bar), ("line", Mark::Line), ("arc", Mark::Arc)],
    )
}

fn read_position(v: Value) -> Read<Position> {
    variant(
        v,
        &[
            ("overlay", Position::Overlay),
            ("stack", Position::Stack),
            ("normalize", Position::Normalize),
            ("group", Position::Group),
        ],
    )
}

fn read_point(v: Value) -> Read<PointSpec> {
    untagged(v, "PointSpec", PointSpec::On, |v| {
        object!(v, "PointConfig", PointConfig {
            size [def] = "size" => opt(number),
        })
        .map(PointSpec::Config)
    })
}

fn read_style(v: Value) -> Read<StyleSpec> {
    object!(v, "StyleSpec", StyleSpec {
        stroke_width [def] = "strokeWidth" => opt(number),
        area [def] = "area" => opt(boolean),
        inner_radius [def] = "innerRadius" => opt(number),
        pad_angle [def] = "padAngle" => opt(number),
        start_angle [def] = "startAngle" => opt(number),
        depth [def] = "depth" => opt(number),
    })
}

fn read_label(v: Value) -> Read<LabelSpec> {
    object!(v, "LabelSpec", LabelSpec {
        show [req] = "show" => read_label_show,
        format [def] = "format" => opt(read_label_format),
    })
}

fn read_label_show(v: Value) -> Read<LabelShow> {
    variant(
        v,
        &[("always", LabelShow::Always), ("hover", LabelShow::Hover)],
    )
}

fn read_label_format(v: Value) -> Read<LabelFormat> {
    object!(v, "LabelFormat", LabelFormat {
        percent [def] = "percent" => opt(boolean),
        decimals [def] = "decimals" => opt(count),
    })
}

fn read_encoding(v: Value) -> Read<Encoding> {
    object!(v, "Encoding", Encoding {
        x [def] = "x" => opt(read_position_encoding),
        y [def] = "y" => opt(read_position_encoding),
        theta [def] = "theta" => opt(read_theta),
        z [def] = "z" => opt(read_position_encoding),
        color [def] = "color" => opt(read_color),
        key [def] = "key" => opt(read_key),
    })
}

fn read_position_encoding(v: Value) -> Read<PositionEncoding> {
    object!(v, "PositionEncoding", PositionEncoding {
        field [req] = "field" => string,
        axis [def] = "axis" => opt(string),
    })
}

fn read_color(v: Value) -> Read<ColorEncoding> {
    object!(v, "ColorEncoding", ColorEncoding {
        value [def] = "value" => opt(string),
        field [def] = "field" => opt(string),
    })
}

fn read_theta(v: Value) -> Read<ThetaEncoding> {
    object!(v, "ThetaEncoding", ThetaEncoding {
        field [req] = "field" => string,
    })
}

fn read_key(v: Value) -> Read<KeyEncoding> {
    object!(v, "KeyEncoding", KeyEncoding {
        field [req] = "field" => string,
    })
}

// --- diagrams -------------------------------------------------------------------------------

fn read_diagram(v: Value) -> Read<DiagramSpec> {
    object!(v, "DiagramSpec", DiagramSpec {
        nodes [req] = "nodes" => list(read_diagram_node),
        edges [def] = "edges" => list(read_diagram_edge),
        font_size [def] = "fontSize" => opt(number),
        style [def] = "style" => read_diagram_style,
    })
}

fn read_diagram_style(v: Value) -> Read<DiagramStyle> {
    variant(
        v,
        &[
            ("flat", DiagramStyle::Flat),
            ("glossy", DiagramStyle::Glossy),
        ],
    )
}

fn read_diagram_node(v: Value) -> Read<DiagramNode> {
    object!(v, "DiagramNode", DiagramNode {
        id [req] = "id" => string,
        label [def] = "label" => opt(string),
        shape [def] = "shape" => read_node_shape,
        x [req] = "x" => number,
        y [req] = "y" => number,
        width [req] = "width" => number,
        height [req] = "height" => number,
        color [def] = "color" => opt(string),
        stroke [def] = "stroke" => opt(string),
        stroke_width [def] = "strokeWidth" => opt(number),
        text_color [def] = "textColor" => opt(string),
    })
}

fn read_node_shape(v: Value) -> Read<NodeShape> {
    variant(
        v,
        &[
            ("rect", NodeShape::Rect),
            ("rounded", NodeShape::Rounded),
            ("stadium", NodeShape::Stadium),
            ("cylinder", NodeShape::Cylinder),
            ("circle", NodeShape::Circle),
            ("diamond", NodeShape::Diamond),
            ("hexagon", NodeShape::Hexagon),
            ("parallelogram", NodeShape::Parallelogram),
            ("parallelogramAlt", NodeShape::ParallelogramAlt),
            ("trapezoid", NodeShape::Trapezoid),
            ("trapezoidAlt", NodeShape::TrapezoidAlt),
        ],
    )
}

fn read_diagram_edge(v: Value) -> Read<DiagramEdge> {
    object!(v, "DiagramEdge", DiagramEdge {
        from [req] = "from" => string,
        to [req] = "to" => string,
        points [req] = "points" => list(pair),
        label [def] = "label" => opt(string),
        label_position [def] = "labelPosition" => opt(pair),
        stroke [def] = "stroke" => read_edge_stroke,
        head [def] = "head" => opt(read_edge_end),
        tail [def] = "tail" => opt(read_edge_end),
    })
}

fn read_edge_stroke(v: Value) -> Read<EdgeStroke> {
    variant(
        v,
        &[
            ("solid", EdgeStroke::Solid),
            ("dotted", EdgeStroke::Dotted),
            ("thick", EdgeStroke::Thick),
        ],
    )
}

fn read_edge_end(v: Value) -> Read<EdgeEnd> {
    variant(
        v,
        &[
            ("none", EdgeEnd::None),
            ("arrow", EdgeEnd::Arrow),
            ("circle", EdgeEnd::Circle),
            ("cross", EdgeEnd::Cross),
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::value::parse_json;

    fn column(json: &str) -> Result<Column, String> {
        read_column(parse_json(json).unwrap()).map_err(|f| f.message)
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

    #[test]
    fn only_the_first_problem_is_reported_with_its_path() {
        let spec = parse_json(r#"{"axes": [{"id": 1}], "layers": 5}"#).unwrap();
        let issues = parse_spec_value(spec).unwrap_err();
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].path, "/axes/0/id");
        assert_eq!(
            issues[0].message,
            "invalid type: integer `1`, expected a string"
        );
    }

    #[test]
    fn host_values_that_are_not_data_are_rejected_where_they_sit() {
        let spec = Value::Object(vec![(
            "data".into(),
            Value::Object(vec![(
                "t".into(),
                Value::Object(vec![(
                    "c".into(),
                    Value::Array(vec![Value::Other("function")]),
                )]),
            )]),
        )]);
        let issues = parse_spec_value(spec).unwrap_err();
        assert_eq!(issues[0].path, "/data/t/c/0");
        assert!(issues[0].message.starts_with("invalid type: function"));
    }
}
