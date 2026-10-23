//! Validation reports every problem with a JSON-pointer path.

use facetry_core::{Issue, parse_spec_json, validate};

fn issues(json: &str) -> Vec<Issue> {
    match parse_spec_json(json) {
        Ok(spec) => validate(&spec),
        Err(issues) => issues,
    }
}

fn paths(json: &str) -> Vec<String> {
    issues(json).into_iter().map(|i| i.path).collect()
}

/// A valid spec; tests break one thing at a time with `replace`.
const GOOD: &str = r#"{
  "data": { "t": { "c": ["a", "b"], "v": [1, 2] } },
  "axes": [ { "id": "x", "orient": "bottom" }, { "id": "y", "orient": "left" } ],
  "layers": [ { "mark": "bar", "data": "t",
    "encoding": { "x": { "field": "c", "axis": "x" }, "y": { "field": "v", "axis": "y" } } } ]
}"#;

fn broken(from: &str, to: &str) -> String {
    assert!(GOOD.contains(from), "test bug: {from:?} not in GOOD");
    GOOD.replace(from, to)
}

#[test]
fn a_good_spec_has_no_issues() {
    assert_eq!(issues(GOOD), vec![]);
}

#[test]
fn missing_table() {
    let got = issues(&broken(r#""data": "t""#, r#""data": "nope""#));
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].path, "/layers/0/data");
    assert!(
        got[0].message.contains("\"nope\" not found"),
        "{}",
        got[0].message
    );
}

#[test]
fn missing_fields() {
    let got = paths(&broken(r#""field": "c""#, r#""field": "zzz""#));
    assert_eq!(got, ["/layers/0/encoding/x/field"]);
    let got = paths(&broken(r#""field": "v""#, r#""field": "zzz""#));
    assert_eq!(got, ["/layers/0/encoding/y/field"]);
}

#[test]
fn wrong_column_kinds() {
    // x numeric, y categorical.
    let got = issues(&broken(r#""field": "c""#, r#""field": "v""#));
    assert_eq!(got[0].path, "/layers/0/encoding/x/field");
    assert!(got[0].message.contains("needs a categorical"));
    let got = issues(&broken(r#""field": "v""#, r#""field": "c""#));
    assert_eq!(got[0].path, "/layers/0/encoding/y/field");
    assert!(got[0].message.contains("needs a numeric"));
}

#[test]
fn axis_not_found() {
    let got = issues(&broken(
        r#""field": "v", "axis": "y""#,
        r#""field": "v", "axis": "q""#,
    ));
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].path, "/layers/0/encoding/y/axis");
    assert!(
        got[0].message.contains("declared axes: x, y"),
        "{}",
        got[0].message
    );
}

#[test]
fn orient_unsuitable_for_the_channel() {
    // y bound to the bottom axis, x bound to the left axis.
    let got = issues(&broken(
        r#""field": "v", "axis": "y""#,
        r#""field": "v", "axis": "x""#,
    ));
    assert_eq!(got[0].path, "/layers/0/encoding/y/axis");
    assert!(got[0].message.contains("needs a left or right axis"));
    let got = issues(&broken(
        r#""field": "c", "axis": "x""#,
        r#""field": "c", "axis": "y""#,
    ));
    assert_eq!(got[0].path, "/layers/0/encoding/x/axis");
    assert!(got[0].message.contains("needs a bottom axis"));
}

#[test]
fn no_suitable_axis_declared_when_binding_is_implicit() {
    let json = r#"{
      "data": { "t": { "c": ["a"], "v": [1] } },
      "axes": [],
      "layers": [ { "mark": "bar", "data": "t",
        "encoding": { "x": { "field": "c" }, "y": { "field": "v" } } } ]
    }"#;
    assert_eq!(
        paths(json),
        ["/layers/0/encoding/x/axis", "/layers/0/encoding/y/axis"]
    );
}

#[test]
fn unequal_column_lengths() {
    let got = issues(&broken(r#""v": [1, 2]"#, r#""v": [1]"#));
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].path, "/data/t");
    assert!(got[0].message.contains("equal lengths"));
}

#[test]
fn depth_axis_and_3d_are_rejected_with_a_clear_message() {
    let got = issues(&broken(r#""orient": "left""#, r#""orient": "depth""#));
    assert!(
        got.iter()
            .any(|i| i.path == "/axes/1/orient" && i.message.contains("3D"))
    );
    let got = issues(&broken(r#""data": {"#, r#""dimension": "3d", "data": {"#));
    assert_eq!(got[0].path, "/dimension");
    assert!(got[0].message.contains("3D charts are not supported yet"));
}

#[test]
fn duplicate_axis_ids_and_sides() {
    let json = broken(
        r#"{ "id": "y", "orient": "left" }"#,
        r#"{ "id": "x", "orient": "left" }, { "id": "z", "orient": "left" }"#,
    );
    let got = issues(&json);
    assert!(
        got.iter()
            .any(|i| i.path == "/axes/1/id" && i.message.contains("duplicate")),
        "{got:?}"
    );
    assert!(
        got.iter()
            .any(|i| i.path == "/axes/2/orient" && i.message.contains("only one left")),
        "{got:?}"
    );
}

#[test]
fn colour_problems() {
    let enc = |color: &str| {
        broken(
            r#""y": { "field": "v", "axis": "y" }"#,
            &format!(r#""y": {{ "field": "v", "axis": "y" }}, "color": {color}"#),
        )
    };
    assert_eq!(
        paths(&enc(r##"{ "value": "#nothex" }"##)),
        ["/layers/0/encoding/color/value"]
    );
    assert_eq!(
        paths(&enc(r##"{ "value": "#fff000", "field": "c" }"##)),
        ["/layers/0/encoding/color"]
    );
    assert_eq!(paths(&enc("{}")), ["/layers/0/encoding/color"]);
    assert_eq!(
        paths(&enc(r#"{ "field": "zzz" }"#)),
        ["/layers/0/encoding/color/field"]
    );
    assert_eq!(
        paths(&enc(r#"{ "field": "v" }"#)),
        ["/layers/0/encoding/color/field"]
    );
    assert_eq!(paths(&enc(r#"{ "field": "c" }"#)), Vec::<String>::new());
}

#[test]
fn key_field_must_exist() {
    let json = broken(
        r#""y": { "field": "v", "axis": "y" }"#,
        r#""y": { "field": "v", "axis": "y" }, "key": { "field": "nope" }"#,
    );
    assert_eq!(paths(&json), ["/layers/0/encoding/key/field"]);
}

#[test]
fn scale_override_problems() {
    let json = broken(
        r#"{ "id": "y", "orient": "left" }"#,
        r#"{ "id": "y", "orient": "left", "scale": { "domain": [5, 1] } }"#,
    );
    assert_eq!(paths(&json), ["/axes/1/scale/domain"]);
    let json = broken(
        r#"{ "id": "x", "orient": "bottom" }"#,
        r#"{ "id": "x", "orient": "bottom", "scale": { "zero": false } }"#,
    );
    assert_eq!(paths(&json), ["/axes/0/scale"]);
}

#[test]
fn all_issues_are_reported_together() {
    let json = r#"{
      "data": { "t": { "c": ["a"], "v": [1, 2] } },
      "axes": [ { "id": "x", "orient": "depth" } ],
      "layers": [
        { "mark": "bar", "data": "missing", "encoding": { "x": { "field": "c" }, "y": { "field": "v" } } },
        { "mark": "bar", "data": "t", "encoding": { "x": { "field": "nope", "axis": "x" }, "y": { "field": "v", "axis": "x" } } }
      ]
    }"#;
    let got = paths(json);
    for want in [
        "/data/t",
        "/axes/0/orient",
        "/layers/0/data",
        "/layers/1/encoding/x/field",
        "/layers/1/encoding/y/axis",
    ] {
        assert!(got.iter().any(|p| p == want), "missing {want}: {got:?}");
    }
}

#[test]
fn empty_data_is_valid() {
    let json = broken(r#""c": ["a", "b"], "v": [1, 2]"#, r#""c": [], "v": []"#);
    assert_eq!(issues(&json), vec![]);
}

#[test]
fn unknown_fields_are_errors_with_a_path() {
    let got = issues(&broken(
        r#""mark": "bar""#,
        r#""mark": "bar", "colour": "red""#,
    ));
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].path, "/layers/0/colour");
    assert!(
        got[0].message.contains("unknown field `colour`"),
        "{}",
        got[0].message
    );

    let got = issues(&broken(r#""id": "x""#, r#""id": "x", "titel": "t""#));
    assert_eq!(got[0].path, "/axes/0/titel");
}

#[test]
fn shape_errors_carry_the_path_of_the_bad_value() {
    let got = issues(&broken(r#""v": [1, 2]"#, r#""v": [1, "two"]"#));
    assert_eq!(got[0].path, "/data/t/v");
    let got = issues(&broken(r#""orient": "bottom""#, r#""orient": "top""#));
    assert_eq!(got[0].path, "/axes/0/orient");
    let got = issues(&broken(r#""mark": "bar""#, r#""mark": "line""#));
    assert_eq!(got[0].path, "/layers/0/mark");
}

#[test]
fn pointer_paths_escape_slashes_in_names() {
    let json = r#"{ "data": { "a/b": { "c": [1], "d": [1, 2] } } }"#;
    assert_eq!(paths(json), ["/data/a~1b"]);
}
