//! Validation reports every problem with a JSON-pointer path.
#![cfg(not(target_arch = "wasm32"))]

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
fn depth_axis_z_and_camera_need_3d() {
    let got = issues(&broken(r#""orient": "left""#, r#""orient": "depth""#));
    assert!(
        got.iter()
            .any(|i| i.path == "/axes/1/orient" && i.message.contains("`dimension: \"3d\"`")),
        "{got:?}"
    );
    let got = issues(&broken(
        r#""y": { "field": "v", "axis": "y" }"#,
        r#""y": { "field": "v", "axis": "y" }, "z": { "field": "c" }"#,
    ));
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].path, "/layers/0/encoding/z");
    let got = issues(&broken(
        r#""data": {"#,
        r#""camera": { "azimuth": 10 }, "data": {"#,
    ));
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].path, "/camera");
}

/// A valid 3D spec with a depth axis and a z channel.
const GOOD_3D: &str = r#"{
  "dimension": "3d",
  "data": { "t": { "c": ["a", "b"], "r": ["p", "q"], "v": [1, 2] } },
  "axes": [ { "id": "x", "orient": "bottom" }, { "id": "y", "orient": "left" },
            { "id": "z", "orient": "depth" } ],
  "layers": [ { "mark": "bar", "data": "t", "encoding": {
    "x": { "field": "c", "axis": "x" }, "y": { "field": "v", "axis": "y" },
    "z": { "field": "r", "axis": "z" } } } ],
  "camera": { "type": "orthographic", "azimuth": 10, "elevation": 20, "distance": 3 },
  "interaction": { "orbit": false }
}"#;

fn broken_3d(from: &str, to: &str) -> String {
    assert!(GOOD_3D.contains(from), "test bug: {from:?} not in GOOD_3D");
    GOOD_3D.replace(from, to)
}

#[test]
fn a_good_3d_spec_has_no_issues_and_z_is_optional() {
    assert_eq!(issues(GOOD_3D), vec![]);
    let no_z = broken_3d(",\n    \"z\": { \"field\": \"r\", \"axis\": \"z\" }", "");
    assert_eq!(issues(&no_z), vec![]);
    let spec = parse_spec_json(GOOD_3D).unwrap();
    assert!(!spec.orbit_enabled());
    let spec = parse_spec_json(&broken_3d(r#""orbit": false"#, r#""orbit": true"#)).unwrap();
    assert!(spec.orbit_enabled());
}

#[test]
fn right_axes_are_rejected_in_3d() {
    let got = issues(&broken_3d(r#""orient": "left""#, r#""orient": "right""#));
    assert!(
        got.iter()
            .any(|i| i.path == "/axes/1/orient" && i.message.contains("3D")),
        "{got:?}"
    );
}

#[test]
fn z_problems_in_3d() {
    // Numeric z field, unknown field, wrong axis orientation, unknown axis.
    let got = issues(&broken_3d(r#""field": "r""#, r#""field": "v""#));
    assert_eq!(got[0].path, "/layers/0/encoding/z/field");
    assert!(got[0].message.contains("needs a categorical"));
    let got = paths(&broken_3d(r#""field": "r""#, r#""field": "nope""#));
    assert_eq!(got, ["/layers/0/encoding/z/field"]);
    let got = issues(&broken_3d(
        r#""field": "r", "axis": "z""#,
        r#""field": "r", "axis": "x""#,
    ));
    assert_eq!(got[0].path, "/layers/0/encoding/z/axis");
    assert!(got[0].message.contains("needs a depth axis"));
    let got = paths(&broken_3d(
        r#""field": "r", "axis": "z""#,
        r#""field": "r", "axis": "q""#,
    ));
    assert_eq!(got, ["/layers/0/encoding/z/axis"]);
}

#[test]
fn depth_axis_rules_in_3d() {
    // Two depth axes; a numeric scale override on the categorical depth axis.
    let got = paths(&broken_3d(
        r#"{ "id": "z", "orient": "depth" }"#,
        r#"{ "id": "z", "orient": "depth" }, { "id": "z2", "orient": "depth" }"#,
    ));
    assert_eq!(got, ["/axes/3/orient"]);
    let got = paths(&broken_3d(
        r#"{ "id": "z", "orient": "depth" }"#,
        r#"{ "id": "z", "orient": "depth", "scale": { "nice": true } }"#,
    ));
    assert_eq!(got, ["/axes/2/scale"]);
}

#[test]
fn camera_problems_in_3d() {
    let got = paths(&broken_3d(r#""distance": 3"#, r#""distance": 0"#));
    assert_eq!(got, ["/camera/distance"]);
    let got = paths(&broken_3d(r#""distance": 3"#, r#""distance": -2"#));
    assert_eq!(got, ["/camera/distance"]);
    // Angles are clamped by the renderer, not rejected.
    assert_eq!(
        issues(&broken_3d(r#""elevation": 20"#, r#""elevation": 400"#)),
        vec![]
    );
    let got = paths(&broken_3d(
        r#""type": "orthographic""#,
        r#""type": "fisheye""#,
    ));
    assert_eq!(got, ["/camera/type"]);
    let got = paths(&broken_3d(
        r#""azimuth": 10"#,
        r#""azimuth": 10, "roll": 3"#,
    ));
    assert_eq!(got, ["/camera/roll"]);
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

// --- value labels --------------------------------------------------------------------------

fn with_label(label: &str) -> String {
    broken(
        r#""encoding": {"#,
        &format!(r#""label": {label}, "encoding": {{"#),
    )
}

#[test]
fn good_labels_validate() {
    for label in [
        r#"{ "show": "always" }"#,
        r#"{ "show": "hover" }"#,
        r#"{ "show": "always", "format": {} }"#,
        r#"{ "show": "hover", "format": { "decimals": 0 } }"#,
        r#"{ "show": "hover", "format": { "decimals": 10 } }"#,
    ] {
        assert_eq!(issues(&with_label(label)), vec![], "{label}");
    }
}

#[test]
fn label_show_must_be_always_or_hover() {
    let got = issues(&with_label(r#"{ "show": "sometimes" }"#));
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].path, "/layers/0/label/show");
    assert!(got[0].message.contains("always"), "{}", got[0].message);
    assert_eq!(
        paths(&with_label(r#"{ "show": true }"#)),
        ["/layers/0/label/show"]
    );
}

#[test]
fn label_needs_show_and_rejects_unknown_fields() {
    let got = issues(&with_label("{}"));
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].path, "/layers/0/label");
    assert!(got[0].message.contains("show"), "{}", got[0].message);
    assert_eq!(
        paths(&with_label(r#"{ "show": "always", "colour": "red" }"#)),
        ["/layers/0/label/colour"]
    );
    assert_eq!(
        paths(&with_label(
            r#"{ "show": "always", "format": { "places": 2 } }"#
        )),
        ["/layers/0/label/format/places"]
    );
    // A label must be an object.
    assert_eq!(paths(&with_label(r#""always""#)), ["/layers/0/label"]);
}

#[test]
fn label_decimals_must_be_a_whole_number_from_0_to_10() {
    for bad in ["-1", "1.5", r#""2""#] {
        let json = with_label(&format!(
            r#"{{ "show": "hover", "format": {{ "decimals": {bad} }} }}"#
        ));
        assert_eq!(paths(&json), ["/layers/0/label/format/decimals"], "{bad}");
    }
    let got = issues(&with_label(
        r#"{ "show": "hover", "format": { "decimals": 11 } }"#,
    ));
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].path, "/layers/0/label/format/decimals");
    assert!(
        got[0].message.contains("between 0 and 10"),
        "{}",
        got[0].message
    );
}

#[test]
fn label_problems_in_the_second_layer_point_at_it() {
    let json = r#"{
      "data": { "t": { "c": ["a"], "v": [1] } },
      "axes": [ { "id": "x", "orient": "bottom" }, { "id": "y", "orient": "left" } ],
      "layers": [
        { "mark": "bar", "data": "t", "encoding": { "x": { "field": "c" }, "y": { "field": "v" } } },
        { "mark": "bar", "data": "t", "encoding": { "x": { "field": "c" }, "y": { "field": "v" } },
          "label": { "show": "always", "format": { "decimals": 99 } } }
      ]
    }"#;
    assert_eq!(paths(json), ["/layers/1/label/format/decimals"]);
}
