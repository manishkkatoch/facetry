//! The hand-written spec reader against the serde-derived parser it replaced.
//!
//! `serde_reference` is a frozen copy of the old types and parser. For every input here both
//! are run and must agree:
//!
//! * accepted: the same `ChartSpec` (compared through `Debug`, which prints every field);
//! * rejected: the same JSON-pointer path and an equivalent message (see [`equivalent`]);
//! * malformed JSON: the new parser's message and position are `serde_json`'s own, checked
//!   against `serde_json::Value`, because the old parser sometimes reported a shape error that
//!   came earlier in the text instead.
//!
//! Inputs: every file in `examples/specs` (but the diagram ones: the frozen parser predates
//! diagrams), hand-written edge cases, thousands of mutations of
//! the examples (unknown fields, wrong types, nulls, numbers as strings, empty arrays,
//! duplicate keys, ...), text-level damage (deleted, replaced and truncated characters), and
//! proptest-composed mutations.
//!
//! The deliberate differences from the old parser are listed near the bottom, each pinned by
//! its own test.
#![cfg(not(target_arch = "wasm32"))]

mod serde_reference;

use facetry_core::parse_spec_json;
use proptest::prelude::*;
use serde_json::{Value, json};

type Outcome = Result<String, (String, String)>;

fn old(text: &str) -> Outcome {
    serde_reference::parse_json(text).map(|s| format!("{s:#?}"))
}

fn new(text: &str) -> Outcome {
    parse_spec_json(text)
        .map(|s| format!("{s:#?}"))
        .map_err(|issues| {
            assert_eq!(issues.len(), 1, "{text}");
            (issues[0].path.clone(), issues[0].message.clone())
        })
}

/// The old message without serde_json's trailing position, and with a float's text
/// normalised (serde_json prints `1e+100`, the new reader `1e100`).
fn normalise_old(message: &str) -> String {
    let mut m = match message.rsplit_once(" at line ") {
        Some((head, _)) => head.to_owned(),
        None => message.to_owned(),
    };
    if let Some(start) = m.find("floating point `") {
        let from = start + "floating point `".len();
        let len = m[from..].find('`').unwrap();
        let f: f64 = m[from..from + len].parse().unwrap();
        m.replace_range(from..from + len, &format!("{f:?}"));
    }
    m
}

/// Whether `old` and `new` are the same complaint. Everything must match word for word
/// except where serde_json's wording depends on its position in the text, which the value
/// tree no longer has.
fn equivalent(old: &str, new: &str) -> bool {
    let old = normalise_old(old);
    // The top-level `version` and `diagram` fields came after the old parser was frozen, so the
    // reader's list of the spec's fields names two more.
    let new = &new.replace(", `diagram`", "").replace("`version`, ", "");
    old == *new
        // A unit enum given a number, bool, null, array or `{}`: serde_json says "expected
        // value" (a syntax-style error); the reader names the type.
        || (old == "expected value"
            && (new.starts_with("invalid type: ") || new.starts_with("invalid length ")))
        // `[1, 2, 3]` for a pair: serde_json says "trailing characters".
        || (old == "trailing characters" && new.starts_with("invalid length "))
        // `{"a": null, "b": null}` for a unit enum.
        || (matches!(old.as_str(), "trailing comma" | "trailing characters")
            && new.starts_with("invalid length "))
}

/// Where the old parser accepted something the reader rejects on purpose: a JSON array in
/// place of an object, which serde derives read positionally (`["x", "left"]` was an
/// `AxisSpec`). Nobody writes that, and the browser path never allowed it.
fn is_array_for_object(text: &str, path: &str, message: &str) -> bool {
    if message.starts_with("invalid type: sequence, expected struct ") {
        return true;
    }
    message.starts_with("data did not match any variant of untagged enum ")
        && serde_json::Deserializer::from_str(text)
            .into_iter::<Value>()
            .next()
            .and_then(Result::ok)
            .is_some_and(|v| v.pointer(path).is_some_and(Value::is_array))
}

/// Counts of what the comparisons saw, to make sure the test is not vacuous.
#[derive(Default)]
struct Tally {
    accepted: usize,
    rejected: usize,
    malformed: usize,
    array_for_object: usize,
}

impl Tally {
    fn compare(&mut self, text: &str) {
        let (o, n) = (old(text), new(text));
        let first_value = serde_json::Deserializer::from_str(text)
            .into_iter::<Value>()
            .next();
        if !matches!(first_value, Some(Ok(_))) {
            // Malformed JSON. The old parser may have stopped at a shape error before the
            // syntax error, so only the new parser is held to serde_json's exact message.
            self.malformed += 1;
            let e = serde_json::from_str::<Value>(text).unwrap_err();
            let (path, message) = n.as_ref().expect_err(text);
            assert_eq!(message, &e.to_string(), "message for {text:?}");
            assert!(o.is_err(), "old accepted malformed {text:?}");
            if let Err((old_path, old_message)) = &o {
                if old_message == message {
                    assert_eq!(old_path, path, "path of the syntax error in {text:?}");
                }
            }
            return;
        }
        match (&o, &n) {
            (Ok(a), Ok(b)) => {
                assert_eq!(a, b, "accepted differently: {text}");
                self.accepted += 1;
            }
            (Err((op, om)), Err((np, nm))) => {
                if is_array_for_object(text, np, nm) {
                    self.array_for_object += 1;
                    return;
                }
                assert_eq!(op, np, "path differs for {text}\n old: {om}\n new: {nm}");
                assert!(
                    equivalent(om, nm),
                    "message differs for {text}\n old: {om}\n new: {nm}"
                );
                self.rejected += 1;
            }
            (Ok(_), Err((np, nm))) if is_array_for_object(text, np, nm) => {
                self.array_for_object += 1;
            }
            (Ok(_), Err((p, m))) => panic!("new rejects what old accepted: {text}\n [{p}] {m}"),
            (Err((p, m)), Ok(_)) => panic!("new accepts what old rejected: {text}\n [{p}] {m}"),
        }
    }
}

fn example_names() -> Vec<String> {
    let dir = format!("{}/../../examples/specs", env!("CARGO_MANIFEST_DIR"));
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .map(|p| p.file_stem().unwrap().to_string_lossy().into_owned())
        .filter(|n| !n.starts_with("diagram"))
        .collect();
    names.sort();
    names
}

fn examples() -> Vec<(String, String)> {
    example_names()
        .into_iter()
        .map(|n| {
            let text = std::fs::read_to_string(format!(
                "{}/../../examples/specs/{n}.json",
                env!("CARGO_MANIFEST_DIR")
            ))
            .unwrap();
            (n, text)
        })
        .collect()
}

#[test]
fn every_example_spec_parses_to_the_same_chart_spec() {
    let all = examples();
    assert!(all.len() >= 20);
    for (name, text) in &all {
        let (o, n) = (old(text), new(text));
        assert!(
            o.is_ok(),
            "{name} no longer parses with the old parser: {o:?}"
        );
        assert_eq!(o, n, "{name}");
    }
}

// --- hand-written cases ---------------------------------------------------------------------

/// A line per interesting input: wrong types for every kind of field, missing fields, nulls,
/// duplicates, numbers that are not the right kind of number, enum spellings.
const CASES: &[&str] = &[
    "",
    "null",
    "5",
    "\"x\"",
    "true",
    "1.5",
    "-3",
    "{}",
    "{ } trailing text is ignored",
    "{",
    r#"{"a":}"#,
    r#"{"dimension":"2d",}"#,
    r#"{"x": 1e999}"#,
    r#"{"data": 01}"#,
    r#"{"axes": [1 2]}"#,
    r#"{"colour": 1}"#,
    r#"{"dimension": 3}"#,
    r#"{"dimension": null}"#,
    r#"{"dimension": "4d"}"#,
    r#"{"dimension": ["2d"]}"#,
    r#"{"dimension": {"2d": null}}"#,
    r#"{"dimension": {"3d": null}}"#,
    r#"{"dimension": {"2d": 1}}"#,
    r#"{"dimension": {"4d": null}}"#,
    r#"{"dimension": {}}"#,
    r#"{"dimension": {"2d": null, "3d": null}}"#,
    r#"{"dimension": true}"#,
    r#"{"dimension": "2d", "dimension": "3d"}"#,
    r#"{"data": null}"#,
    r#"{"data": []}"#,
    r#"{"data": {"t": null}}"#,
    r#"{"data": {"t": {"c": [true]}}}"#,
    r#"{"data": {"t": {"c": {}}}}"#,
    r#"{"data": {"t": {"c": "s"}}}"#,
    r#"{"data": {"t": {"c": [[1]]}}}"#,
    r#"{"data": {"t": {"c": ["a", 1]}}}"#,
    r#"{"data": {"t": {"c": ["a", null]}}}"#,
    r#"{"data": {"t": {"c": ["a", true]}}}"#,
    r#"{"data": {"t": {"c": [1, {}]}}}"#,
    r#"{"data": {"t": {"c": [1, "a"]}}}"#,
    r#"{"data": {"t": {"c": [1, 2], "c": [3]}}}"#,
    r#"{"data": {"t": {"c": [1, null, 2.5, -0.0, 1e3]}}}"#,
    r#"{"data": {"t": {"c": ["b", "a", "b", "c", "a"]}}}"#,
    r#"{"data": {"t": {"c": []}, "u": {}}, "data": {}}"#,
    r#"{"data": {"a/b": {"c": [1]}, "a~b": {"d": ["x"]}}}"#,
    r#"{"data": {"t": {"c": [18446744073709551615, -9223372036854775808, 1e-7]}}}"#,
    r#"{"axes": null}"#,
    r#"{"axes": {}}"#,
    r#"{"axes": [null]}"#,
    r#"{"axes": [{}]}"#,
    r#"{"axes": [{"id": 1}]}"#,
    r#"{"axes": [{"id": "x"}]}"#,
    r#"{"axes": [{"id": "x", "orient": "top"}]}"#,
    r#"{"axes": [{"orient": "left"}]}"#,
    r#"{"axes": [{"id": "x", "orient": "left", "id": "y"}]}"#,
    r#"{"axes": [{"id": "x", "orient": "left", "bogus": 1, "id": "y"}]}"#,
    r#"{"axes": [{"id": "x", "orient": "left", "title": null}]}"#,
    r#"{"axes": [{"id": "x", "orient": "left", "title": 4}]}"#,
    r#"{"axes": [{"id": "x", "orient": "left", "scale": {"domain": [1]}}]}"#,
    r#"{"axes": [{"id": "x", "orient": "left", "scale": {"domain": []}}]}"#,
    r#"{"axes": [{"id": "x", "orient": "left", "scale": {"domain": [1, 2, 3]}}]}"#,
    r#"{"axes": [{"id": "x", "orient": "left", "scale": {"domain": [1, 2, "a"]}}]}"#,
    r#"{"axes": [{"id": "x", "orient": "left", "scale": {"domain": [1, "a", 3]}}]}"#,
    r#"{"axes": [{"id": "x", "orient": "left", "scale": {"domain": [1, 2]}}]}"#,
    r#"{"axes": [{"id": "x", "orient": "left", "scale": {"domain": [1, null]}}]}"#,
    r#"{"axes": [{"id": "x", "orient": "left", "scale": {"domain": null}}]}"#,
    r#"{"axes": [{"id": "x", "orient": "left", "scale": {"domain": {}}}]}"#,
    r#"{"axes": [{"id": "x", "orient": "left", "scale": {"nice": 1}}]}"#,
    r#"{"axes": [{"id": "x", "orient": "left", "scale": 5}]}"#,
    r#"{"axes": [{"id": "x", "orient": "left", "scale": {"zero": "yes"}}]}"#,
    r#"{"axes": [{"id": "x", "orient": "left", "scale": {"zero": false, "nice": null}}]}"#,
    r#"{"legend": 5}"#,
    r#"{"legend": null}"#,
    r#"{"legend": true}"#,
    r#"{"legend": false}"#,
    r#"{"legend": "right"}"#,
    r#"{"legend": {}}"#,
    r#"{"legend": {"orient": "top", "bogus": 1}}"#,
    r#"{"legend": {"orient": "up"}}"#,
    r#"{"legend": {"orient": "left", "title": "T", "interactive": false, "hidden": ["a", "b"]}}"#,
    r#"{"legend": {"hidden": [1]}}"#,
    r#"{"legend": {"hidden": "a"}}"#,
    r#"{"legend": {"hidden": null}}"#,
    r#"{"legend": {"title": 4}}"#,
    r#"{"legend": {"interactive": null}}"#,
    r#"{"camera": 3}"#,
    r#"{"camera": {}}"#,
    r#"{"camera": {"type": "fisheye"}}"#,
    r#"{"camera": {"type": "orthographic", "azimuth": 10, "elevation": -2.5, "distance": 1e2}}"#,
    r#"{"camera": {"azimuth": "x"}}"#,
    r#"{"camera": {"azimuth": null, "distance": null}}"#,
    r#"{"camera": {"type": 5}}"#,
    r#"{"camera": {"foo": 5}}"#,
    r#"{"transition": false}"#,
    r#"{"transition": true}"#,
    r#"{"transition": null}"#,
    r#"{"transition": {}}"#,
    r#"{"transition": 400}"#,
    r#"{"transition": "fast"}"#,
    r#"{"transition": []}"#,
    r#"{"transition": [false]}"#,
    r#"{"transition": {"duration": 250, "easing": "backOut", "stagger": 120}}"#,
    r#"{"transition": {"duration": -5}}"#,
    r#"{"transition": {"duration": "slow"}}"#,
    r#"{"transition": {"duration": null, "easing": null, "stagger": null}}"#,
    r#"{"transition": {"easing": "bounce"}}"#,
    r#"{"transition": {"easing": "cubicinout"}}"#,
    r#"{"transition": {"easing": ["linear"]}}"#,
    r#"{"transition": {"easing": {"linear": null}}}"#,
    r#"{"transition": {"stagger": 1e3, "bogus": 1}}"#,
    r#"{"transition": {"duration": 1, "duration": 2}}"#,
    r#"{"transition": {"easing": "linear", "easing": "bounce"}}"#,
    r#"{"transition": false, "transition": true}"#,
    r#"{"interaction": {"orbit": 1}}"#,
    r#"{"interaction": {"orbit": false}}"#,
    r#"{"interaction": {"orbit": null}}"#,
    r#"{"interaction": {"x": null}}"#,
    r#"{"layers": [{}]}"#,
    r#"{"layers": [{"mark": "bar"}]}"#,
    r#"{"layers": [{"mark": "bar", "data": "t"}]}"#,
    r#"{"layers": [{"mark": "bar", "data": "t", "encoding": {}}]}"#,
    r#"{"layers": [{"mark": "pie", "data": "t", "encoding": {}}]}"#,
    r#"{"layers": [{"mark": "arc", "data": "t", "encoding": {"theta": {}}}]}"#,
    r##"{"layers": [{"mark": "arc", "data": "t", "encoding": {"theta": {"field": "v"}, "color": {"field": "c", "value": "#fff"}}}]}"##,
    r#"{"layers": [{"mark": "bar", "data": "t", "encoding": {"x": {}}}]}"#,
    r#"{"layers": [{"mark": "bar", "data": "t", "encoding": {"x": {"field": "a", "axis": 1}}}]}"#,
    r#"{"layers": [{"mark": "bar", "data": "t", "encoding": {"x": {"field": "a", "bogus": 1}}}]}"#,
    r#"{"layers": [{"mark": "bar", "data": "t", "encoding": {"key": {"field": 3}}}]}"#,
    r#"{"layers": [{"mark": "bar", "data": "t", "encoding": {"color": {"value": 3}}}]}"#,
    r#"{"layers": [{"mark": "bar", "data": "t", "encoding": {"z": null, "y": null}}]}"#,
    r#"{"layers": [{"mark": "bar", "data": "t", "encoding": {}, "position": "stacked"}]}"#,
    r#"{"layers": [{"mark": "bar", "data": "t", "encoding": {}, "position": "group"}]}"#,
    r#"{"layers": [{"mark": "bar", "data": "t", "encoding": {}, "position": null}]}"#,
    r#"{"layers": [{"mark": "bar", "data": "t", "encoding": {}, "point": null}]}"#,
    r#"{"layers": [{"mark": "bar", "data": "t", "encoding": {}, "point": true}]}"#,
    r#"{"layers": [{"mark": "bar", "data": "t", "encoding": {}, "point": 4}]}"#,
    r#"{"layers": [{"mark": "bar", "data": "t", "encoding": {}, "point": {}}]}"#,
    r#"{"layers": [{"mark": "bar", "data": "t", "encoding": {}, "point": {"size": 6}}]}"#,
    r#"{"layers": [{"mark": "bar", "data": "t", "encoding": {}, "point": {"size": "a"}}]}"#,
    r#"{"layers": [{"mark": "bar", "data": "t", "encoding": {}, "point": {"size": 5, "x": 1}}]}"#,
    r#"{"layers": [{"mark": "bar", "data": "t", "encoding": {}, "point": "on"}]}"#,
    r#"{"layers": [{"mark": "bar", "data": "t", "encoding": {}, "style": {"strokeWidth": "a"}}]}"#,
    r#"{"layers": [{"mark": "bar", "data": "t", "encoding": {}, "style": {"stroke_width": 1}}]}"#,
    r#"{"layers": [{"mark": "bar", "data": "t", "encoding": {}, "style": {"innerRadius": 1, "padAngle": 2, "startAngle": 3, "depth": 4, "area": true}}]}"#,
    r#"{"layers": [{"mark": "bar", "data": "t", "encoding": {}, "label": {}}]}"#,
    r#"{"layers": [{"mark": "bar", "data": "t", "encoding": {}, "label": {"show": "never"}}]}"#,
    r#"{"layers": [{"mark": "bar", "data": "t", "encoding": {}, "label": {"show": "hover", "format": {}}}]}"#,
    r#"{"layers": [{"mark": "bar", "data": "t", "encoding": {}, "label": {"show": "always", "format": {"percent": true, "decimals": 3}}}]}"#,
    r#"{"layers": [{"mark": "bar", "data": "t", "encoding": {}, "label": {"show": "always", "format": {"decimals": 2.5}}}]}"#,
    r#"{"layers": [{"mark": "bar", "data": "t", "encoding": {}, "label": {"show": "always", "format": {"decimals": -1}}}]}"#,
    r#"{"layers": [{"mark": "bar", "data": "t", "encoding": {}, "label": {"show": "always", "format": {"decimals": 4294967295}}}]}"#,
    r#"{"layers": [{"mark": "bar", "data": "t", "encoding": {}, "label": {"show": "always", "format": {"decimals": 4294967296}}}]}"#,
    r#"{"layers": [{"mark": "bar", "data": "t", "encoding": {}, "label": {"show": "always", "format": {"decimals": 2.0}}}]}"#,
    r#"{"layers": [{"mark": "bar", "data": "t", "encoding": {}, "label": {"show": "always", "format": {"decimals": 1e1}}}]}"#,
    r#"{"layers": [{"mark": "bar", "data": "t", "encoding": {}, "label": {"show": "always", "format": {"decimals": -0}}}]}"#,
    r#"{"layers": [{"mark": "bar", "data": "t", "encoding": {}, "label": {"show": "always", "format": {"decimals": 18446744073709551616}}}]}"#,
    r#"{"layers": [{"mark": "bar", "data": "t", "encoding": {}, "label": {"show": "always", "format": {"decimals": 1e100}}}]}"#,
    r#"{"layers": [{"mark": "bar", "data": "t", "encoding": {}, "label": {"show": "always", "format": {"percent": 1}}}]}"#,
    r#"{"layers": [{"mark": "bar", "data": "t", "encoding": {}, "label": {"show": "always", "format": {"percent": "a\nb"}}}]}"#,
    r#"{"layers": [{"mark": "bar", "data": "t", "encoding": {}, "label": {"show": "always", "format": {"percent": [1]}}}]}"#,
    r#"{"layers": [{"mark": "bar", "data": "t", "encoding": {}, "label": {"show": "always", "format": {"percent": {}}}}]}"#,
    r#"{"layers": [{"mark": "bar", "data": "t", "encoding": {}, "label": {"show": "always", "format": {"percent": 1e400}}}]}"#,
    r#"{"layers": [{"mark": "bar", "data": "t", "encoding": {}, "style": {"strokeWidth": 1e400}}]}"#,
    r#"{"layers": [{"mark": "bar", "data": "t", "encoding": {}, "style": {"strokeWidth": -0.0}}]}"#,
    r#"{"layers": [{"mark": "bar", "data": "t", "encoding": {}, "style": {"strokeWidth": 18446744073709551615}}]}"#,
    r#"{"layers": [{"mark": "bar", "data": "a\u00e9\ud83d\ude00\n\"\\/", "encoding": {}}]}"#,
    r#"{"layers": [{"mark": "bar", "data": "t", "encoding": {}}, {"mark": "line", "data": 5, "encoding": {}}, {}]}"#,
    // Two problems: the first in the text is the one reported.
    r#"{"layers": [{"mark": "nope", "data": 5}], "axes": [{"id": 1}]}"#,
    r#"{"axes": [{"id": 1}], "layers": [{"mark": "nope", "data": 5}]}"#,
    r#"{"zzz": 1, "dimension": 5}"#,
    r#"{"dimension": 5, "zzz": 1}"#,
    r#"{"axes": [{"orient": "left", "id": 5}]}"#,
    r#"{"axes": [{"id": "x"}], "bogus": null}"#,
];

#[test]
fn hand_written_cases_agree() {
    let mut tally = Tally::default();
    for case in CASES {
        tally.compare(case);
    }
    assert!(
        tally.accepted > 15 && tally.rejected > 100,
        "{}",
        tally.rejected
    );
}

// --- mutations of the examples ------------------------------------------------------------

/// Stands in front of a key that the writer emits without the prefix, giving a duplicate key
/// (a `serde_json::Value` cannot hold one).
const DUP: char = '\u{1}';

/// JSON text for `v`; `reverse` writes object entries last to first, so that the order in
/// which problems are met changes too.
fn emit(v: &Value, reverse: bool, out: &mut String) {
    match v {
        Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                emit(item, reverse, out);
            }
            out.push(']');
        }
        Value::Object(map) => {
            out.push('{');
            let mut entries: Vec<_> = map.iter().collect();
            if reverse {
                entries.reverse();
            }
            for (i, (k, item)) in entries.into_iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&Value::String(k.trim_start_matches(DUP).to_owned()).to_string());
                out.push(':');
                emit(item, reverse, out);
            }
            out.push('}');
        }
        other => out.push_str(&other.to_string()),
    }
}

fn text(v: &Value, reverse: bool) -> String {
    let mut s = String::new();
    emit(v, reverse, &mut s);
    s
}

/// JSON pointers of the nodes worth mutating: everything, but only the first, second and
/// last element of an array so that long columns do not dominate.
fn pointers(v: &Value, at: &str, out: &mut Vec<String>) {
    out.push(at.to_owned());
    match v {
        Value::Array(items) => {
            let n = items.len();
            let mut picks = vec![0, 1, n.saturating_sub(1)];
            picks.dedup();
            for i in picks.into_iter().filter(|&i| i < n) {
                pointers(&items[i], &format!("{at}/{i}"), out);
            }
        }
        Value::Object(map) => {
            for (k, item) in map {
                let k = k.replace('~', "~0").replace('/', "~1");
                pointers(item, &format!("{at}/{k}"), out);
            }
        }
        _ => {}
    }
}

fn replacements(original: &Value) -> Vec<Value> {
    let mut r = vec![
        json!(null),
        json!(true),
        json!(false),
        json!(0),
        json!(-1),
        json!(5),
        json!(2.5),
        json!(2.0),
        json!(4294967296u64),
        json!(1e300),
        json!(""),
        json!("x"),
        json!("right"),
        json!([]),
        json!({}),
        json!([1]),
        json!([1, 2]),
        json!(["a"]),
        json!([null]),
        json!([[]]),
        json!({"a": 1}),
        json!({"a": null}),
    ];
    // "numbers as strings" and the reverse.
    match original {
        Value::Number(n) => r.push(json!(n.to_string())),
        Value::String(s) => {
            if let Ok(n) = s.parse::<f64>() {
                r.push(json!(n));
            }
            r.push(json!([s]));
            r.push(json!({ s.clone(): null }));
        }
        Value::Array(items) => {
            r.push(json!(items.len()));
            r.push(Value::Array(items.iter().take(1).cloned().collect()));
            r.push(Value::Array(vec![]));
            let mut more = items.clone();
            more.push(Value::Null);
            r.push(Value::Array(more));
            let mut more = items.clone();
            more.push(json!("s"));
            r.push(Value::Array(more));
            let mut more = items.clone();
            more.push(json!(1));
            r.push(Value::Array(more));
        }
        Value::Object(map) => {
            let mut extra = map.clone();
            extra.insert("bogus".into(), json!(1));
            r.push(Value::Object(extra));
            if let Some((k, v)) = map.iter().next() {
                let mut dup = map.clone();
                dup.insert(format!("{DUP}{k}"), v.clone());
                r.push(Value::Object(dup));
                let mut dup = map.clone();
                dup.insert(format!("{DUP}{k}"), json!(null));
                r.push(Value::Object(dup));
                let mut typo = map.clone();
                let v = typo.remove(k).unwrap();
                typo.insert(format!("{k}x"), v);
                r.push(Value::Object(typo));
                let mut gone = map.clone();
                gone.remove(k);
                r.push(Value::Object(gone));
            }
            // Case changes are typos too.
            let mut upper = map.clone();
            if let Some(k) = map.keys().next().cloned() {
                let v = upper.remove(&k).unwrap();
                upper.insert(k.to_uppercase(), v);
                r.push(Value::Object(upper));
            }
        }
        _ => {}
    }
    r
}

/// `v` with the node at `pointer` replaced (or removed, with `None`).
fn with(v: &Value, pointer: &str, new: Option<Value>) -> Value {
    let mut out = v.clone();
    if pointer.is_empty() {
        return new.unwrap_or(Value::Null);
    }
    let (parent, last) = pointer.rsplit_once('/').unwrap();
    let parent = out.pointer_mut(parent).unwrap();
    match (parent, new) {
        (Value::Array(items), Some(n)) => items[last.parse::<usize>().unwrap()] = n,
        (Value::Array(items), None) => {
            items.remove(last.parse::<usize>().unwrap());
        }
        (Value::Object(map), Some(n)) => {
            map.insert(last.replace("~1", "/").replace("~0", "~"), n);
        }
        (Value::Object(map), None) => {
            map.remove(&last.replace("~1", "/").replace("~0", "~"));
        }
        _ => unreachable!(),
    }
    out
}

fn mutants_of(base: &Value) -> Vec<Value> {
    let mut ptrs = Vec::new();
    pointers(base, "", &mut ptrs);
    let mut out = Vec::new();
    for p in &ptrs {
        let original = base.pointer(p).unwrap();
        for r in replacements(original) {
            out.push(with(base, p, Some(r)));
        }
        if !p.is_empty() {
            out.push(with(base, p, None));
        }
    }
    out
}

#[test]
fn mutated_examples_agree() {
    let mut tally = Tally::default();
    for (name, original) in examples() {
        let base: Value = serde_json::from_str(&original).unwrap();
        for (i, m) in mutants_of(&base).iter().enumerate() {
            tally.compare(&text(m, false));
            // The same mutant with its keys in the opposite order, for a sample.
            if i % 3 == 0 {
                tally.compare(&text(m, true));
            }
        }
        eprintln!("{name}: ok");
    }
    eprintln!(
        "accepted {}, rejected {}, array-for-object {}",
        tally.accepted, tally.rejected, tally.array_for_object
    );
    assert!(tally.accepted > 1_000 && tally.rejected > 10_000);
}

// --- damaged text ---------------------------------------------------------------------------

#[test]
fn damaged_text_agrees() {
    let mut tally = Tally::default();
    let damage = [
        '{', '}', '[', ']', ',', ':', '"', '\\', '0', '-', 'e', '.', 'x', ' ', '\n',
    ];
    for (name, original) in examples() {
        // The compact form is shorter, so every position can be tried.
        let compact = text(&serde_json::from_str::<Value>(&original).unwrap(), false);
        for text in [compact.as_str(), original.as_str()] {
            let chars: Vec<char> = text.chars().collect();
            let step = if chars.len() > 600 { 7 } else { 2 };
            for i in (0..chars.len()).step_by(step) {
                let head: String = chars[..i].iter().collect();
                let tail: String = chars[i + 1..].iter().collect();
                tally.compare(&head); // truncated
                tally.compare(&format!("{head}{tail}")); // deleted
                for d in damage {
                    tally.compare(&format!("{head}{d}{tail}")); // replaced
                }
                for d in ['"', ',', '{', '['] {
                    tally.compare(&format!("{head}{d}{}{tail}", chars[i])); // inserted
                }
            }
        }
        eprintln!("{name}: ok");
    }
    eprintln!(
        "malformed {}, accepted {}, rejected {}, array-for-object {}",
        tally.malformed, tally.accepted, tally.rejected, tally.array_for_object
    );
    assert!(tally.malformed > 5_000, "{}", tally.malformed);
}

#[test]
fn json_syntax_errors_match_serde_json() {
    // Syntax only: the new parser's text must be serde_json's, including line and column.
    for bad in [
        "",
        " ",
        "\n\n",
        "nul",
        "tru",
        "fals",
        "nulx",
        "[",
        "[1",
        "[1,",
        "[1 2]",
        "[,1]",
        "[1,]",
        "{",
        "{\"a\"",
        "{\"a\":",
        "{\"a\":1",
        "{\"a\":1,",
        "{\"a\":1,}",
        "{,}",
        "{1:2}",
        "{\"a\" 1}",
        "\"abc",
        "\"a\\",
        "\"a\\q\"",
        "\"\\u12\"",
        "\"\\u12",
        "\"\\uzzzz\"",
        "\"\\ud800\"",
        "\"\\ud800\\u0041\"",
        "\"\\udc00\"",
        "\"a\tb\"",
        "01",
        "-",
        "-x",
        "1.",
        "1.x",
        "1e",
        "1e+",
        "1ex",
        "--1",
        "+1",
        ".5",
        "1e999",
        "-1e999",
        "[1e999]",
        "{\"a\": [\n1,\n2,\n}]",
        "{\"a\":\n\n  x}",
        "\u{feff}{}",
        "{\"é\": ñ}",
    ] {
        let expected = serde_json::from_str::<Value>(bad);
        let got = facetry_core::value::parse_json(bad);
        match (&expected, &got) {
            (Err(e), Err(g)) => assert_eq!(g.message, e.to_string(), "{bad:?}"),
            other => panic!("{bad:?}: {other:?}"),
        }
    }
    // Nesting.
    let deep = "[".repeat(200);
    assert_eq!(
        facetry_core::value::parse_json(&deep).unwrap_err().message,
        serde_json::from_str::<Value>(&deep)
            .unwrap_err()
            .to_string()
    );
    let ok = format!("{}{}", "[".repeat(127), "]".repeat(127));
    assert!(
        facetry_core::value::parse_json(&ok).is_ok() && serde_json::from_str::<Value>(&ok).is_ok()
    );
    let too_deep = format!("{}{}", "[".repeat(128), "]".repeat(128));
    assert!(facetry_core::value::parse_json(&too_deep).is_err());
    assert!(serde_json::from_str::<Value>(&too_deep).is_err());
}

#[test]
fn json_values_match_serde_json() {
    // Valid JSON: the same tree, so the same numbers (integer or float) and strings.
    fn same(a: &Value, b: &facetry_core::Value) -> bool {
        use facetry_core::Value as V;
        match (a, b) {
            (Value::Null, V::Null) => true,
            (Value::Bool(x), V::Bool(y)) => x == y,
            (Value::Number(n), V::Int(i)) => n.as_i64() == Some(*i),
            (Value::Number(n), V::Uint(u)) => n.as_u64() == Some(*u) && *u > i64::MAX as u64,
            (Value::Number(n), V::Float(f)) => n.is_f64() && n.as_f64() == Some(*f),
            (Value::String(x), V::Str(y)) => x == y,
            (Value::Array(x), V::Array(y)) => {
                x.len() == y.len() && x.iter().zip(y).all(|(a, b)| same(a, b))
            }
            (Value::Object(x), V::Object(y)) => {
                x.len() == y.len()
                    && x.iter()
                        .all(|(k, a)| y.iter().any(|(k2, b)| k == k2 && same(a, b)))
            }
            _ => false,
        }
    }
    for good in [
        "0",
        "-0",
        "7",
        "-7",
        "1.5",
        "-1.5e3",
        "1E2",
        "1e-2",
        "0.1",
        "123456789012345678901234567890",
        "9223372036854775807",
        "9223372036854775808",
        "-9223372036854775808",
        "-9223372036854775809",
        "18446744073709551615",
        "18446744073709551616",
        "0.30000000000000004",
        "[]",
        "{}",
        r#"{"a": [1, {"b": null}], "c": "\u00e9\n\ud83d\ude00"}"#,
        r#""""#,
        "  \t\r\n true  ",
    ] {
        let a: Value = serde_json::from_str(good).unwrap();
        let b = facetry_core::value::parse_json(good).unwrap();
        assert!(same(&a, &b), "{good}: {a:?} vs {b:?}");
    }
}

// --- composed mutations (proptest) -----------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig { failure_persistence: None, ..ProptestConfig::with_cases(3000) })]

    /// One to three random mutations on top of each other, on a random example.
    #[test]
    fn composed_mutations_agree(
        example in 0usize..24,
        picks in prop::collection::vec((any::<usize>(), any::<usize>()), 1..4),
        reverse in any::<bool>(),
    ) {
        let all = examples();
        let mut v: Value = serde_json::from_str(&all[example % all.len()].1).unwrap();
        for (node, how) in picks {
            let mut ptrs = Vec::new();
            pointers(&v, "", &mut ptrs);
            let p = &ptrs[node % ptrs.len()];
            let options = replacements(v.pointer(p).unwrap());
            // One more choice than there are replacements: removal.
            let pick = how % (options.len() + 1);
            v = match options.get(pick) {
                Some(r) => with(&v, p, Some(r.clone())),
                None if p.is_empty() => v,
                None => with(&v, p, None),
            };
        }
        Tally::default().compare(&text(&v, reverse));
    }
}

// --- the deliberate differences ---------------------------------------------------------------
//
// What the new reader does differently on purpose:
//
// * A JSON array where an object is expected. serde derives read it field by field
//   (`["x", "left"]` was an `AxisSpec`, `[]` a whole default `ChartSpec`); the browser path
//   never did. It is now `invalid type: sequence, expected struct X`.
// * Unknown fields in a JS object are now rejected, as they always were for JSON text. (The
//   old browser path, serde-wasm-bindgen, only looked at known field names.) Not testable
//   here; covered by the Playwright suite.
// * Float text in messages is Rust's (`1e100`), not serde_json's (`1e+100`).
// * Messages lose serde_json's ` at line L column C` suffix except for syntax errors.
// * Floats parse correctly rounded (serde_json's default can be one ULP off for inputs with
//   more than 15 significant digits, which the old browser path never had).

#[test]
fn arrays_are_not_objects() {
    for (text, path) in [
        ("[]", ""),
        (r#"{"axes": [["x", "left"]]}"#, "/axes/0"),
        (r#"{"camera": []}"#, "/camera"),
        (
            r#"{"layers": [{"mark": "bar", "data": "t", "encoding": []}]}"#,
            "/layers/0/encoding",
        ),
    ] {
        assert!(old(text).is_ok(), "the old parser took {text}");
        let (p, m) = new(text).unwrap_err();
        assert_eq!(p, path, "{text}");
        assert!(
            m.starts_with("invalid type: sequence, expected struct "),
            "{m}"
        );
    }
    // `legend` and `point` are untagged: the message is the generic one, at the field.
    let (p, m) = new(r#"{"legend": []}"#).unwrap_err();
    assert_eq!(p, "/legend");
    assert!(
        m.starts_with("data did not match any variant of untagged enum LegendSpec"),
        "{m}"
    );
    assert!(old(r#"{"legend": []}"#).is_ok(), "the old parser took it");
}

#[test]
fn error_order_follows_the_text_like_before() {
    for text in [
        r#"{"layers": [{"mark": "nope", "data": 5}], "axes": [{"id": 1}]}"#,
        r#"{"axes": [{"id": 1}], "layers": [{"mark": "nope", "data": 5}]}"#,
        r#"{"axes": [{"orient": "left", "id": 5, "orient": "right"}]}"#,
    ] {
        let (o, n) = (old(text).unwrap_err(), new(text).unwrap_err());
        assert_eq!(o.0, n.0, "{text}");
    }
    assert_eq!(
        new(r#"{"layers": [{"mark": "nope"}], "axes": [{"id": 1}]}"#)
            .unwrap_err()
            .0,
        "/layers/0/mark"
    );
    assert_eq!(
        new(r#"{"axes": [{"id": 1}], "layers": [{"mark": "nope"}]}"#)
            .unwrap_err()
            .0,
        "/axes/0/id"
    );
}

#[test]
fn syntax_errors_still_say_where() {
    let (path, message) = new("{\"axes\": [\n  {\"id\": \"x\",}\n]}").unwrap_err();
    assert_eq!(path, "/axes/0");
    assert_eq!(message, "trailing comma at line 2 column 14");
    let (path, message) = new(r#"{"dimension": tru}"#).unwrap_err();
    assert_eq!(
        (path.as_str(), message.as_str()),
        ("/dimension", "expected ident at line 1 column 18")
    );
}

#[test]
fn hidden_override_is_not_read_from_json() {
    let spec = parse_spec_json(r#"{"hidden_override": []}"#);
    assert_eq!(spec.unwrap_err()[0].path, "/hidden_override");
    let ok = parse_spec_json("{}").unwrap();
    assert_eq!(ok.hidden_override, None);
}
