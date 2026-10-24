//! Bar positions (`stack`, `normalize`, `group`) in the laid-out scenes, 2D and 3D.
#![cfg(not(target_arch = "wasm32"))]

use facetry_core::scene::{Rect, Scene};
use facetry_core::scene3d::Scene3d;
use facetry_core::{build_scene, build_scene_3d, parse_spec_json, validate};
use facetry_font::FontMetrics;

fn spec(position: &str, rows: &str) -> String {
    format!(
        r#"{{
  "legend": false,
  "data": {{ "t": {rows} }},
  "axes": [ {{ "id": "x", "orient": "bottom" }}, {{ "id": "y", "orient": "left" }} ],
  "layers": [ {{ "mark": "bar", "data": "t", "position": "{position}",
    "encoding": {{ "x": {{ "field": "c" }}, "y": {{ "field": "v" }}, "color": {{ "field": "s" }} }},
    "label": {{ "show": "always" }} }} ]
}}"#
    )
}

fn scene(json: &str) -> Scene {
    let spec = parse_spec_json(json).expect("parses");
    assert_eq!(validate(&spec), vec![], "{json}");
    build_scene(&spec, (800, 500), 1.0, FontMetrics::inter())
}

fn rect<'a>(scene: &'a Scene, key: &str) -> &'a Rect {
    scene
        .rects
        .iter()
        .find(|r| r.key == key)
        .unwrap_or_else(|| {
            panic!(
                "no {key}: {:?}",
                scene.rects.iter().map(|r| &r.key).collect::<Vec<_>>()
            )
        })
}

const TABLE: &str =
    r#"{ "c": ["a", "a", "b", "b"], "s": ["x", "y", "x", "y"], "v": [3, 1, 2, 2] }"#;

#[test]
fn stacked_segments_touch_and_follow_series_order() {
    let sc = scene(&spec("stack", TABLE));
    assert_eq!(sc.rects.len(), 4);
    for c in ["a", "b"] {
        let x = rect(&sc, &format!("layer0/{c}/x"));
        let y = rect(&sc, &format!("layer0/{c}/y"));
        // Series x is the bottom one: y sits exactly on top of it, in the same column.
        assert_eq!(y.bounds.y + y.bounds.h, x.bounds.y, "{c}");
        assert_eq!((x.bounds.x, x.bounds.w), (y.bounds.x, y.bounds.w));
    }
    // Heights are proportional to the values (the value 3 is 3x the value 1).
    let (a_x, a_y) = (rect(&sc, "layer0/a/x"), rect(&sc, "layer0/a/y"));
    assert!((a_x.bounds.h / a_y.bounds.h - 3.0).abs() < 0.05);
    // The y axis was sized for the stack total (4), not the largest value (3).
    assert_eq!(top_tick(&sc), 4.0);
}

/// The largest number on the y axis.
fn top_tick(scene: &Scene) -> f64 {
    scene
        .texts
        .iter()
        .filter(|t| t.key.starts_with("axis:y/label:"))
        .map(|t| t.text.replace(',', "").parse::<f64>().unwrap())
        .fold(f64::MIN, f64::max)
}

#[test]
fn negative_values_stack_down_from_zero() {
    let sc = scene(&spec(
        "stack",
        r#"{ "c": ["a", "a", "a"], "s": ["x", "y", "z"], "v": [4, -2, -1] }"#,
    ));
    let (x, y, z) = (
        rect(&sc, "layer0/a/x"),
        rect(&sc, "layer0/a/y"),
        rect(&sc, "layer0/a/z"),
    );
    let zero = x.bounds.y + x.bounds.h;
    assert_eq!(y.bounds.y, zero, "the first negative starts at zero");
    assert_eq!(
        z.bounds.y,
        y.bounds.y + y.bounds.h,
        "the next one hangs below it"
    );
    // The zero line is drawn as the baseline.
    assert!(
        sc.rules.iter().any(|r| r.key == "axis:y/zero:0"
            || r.key.ends_with("/zero:0")
            || r.key.contains("zero"))
    );
}

#[test]
fn normalized_stacks_all_reach_the_top_and_label_percentages() {
    let sc = scene(&spec(
        "normalize",
        r#"{ "c": ["a", "a", "b", "b"], "s": ["x", "y", "x", "y"], "v": [1, 3, 500, 1500] }"#,
    ));
    let top = |c: &str| {
        let (x, y) = (
            rect(&sc, &format!("layer0/{c}/x")),
            rect(&sc, &format!("layer0/{c}/y")),
        );
        x.bounds.y.min(y.bounds.y)
    };
    assert_eq!(
        top("a"),
        top("b"),
        "a stack of 4 and a stack of 2000 are both 100%"
    );
    let labels: Vec<_> = sc.labels.iter().map(|l| l.text.text.as_str()).collect();
    assert_eq!(labels, ["25%", "75%", "25%", "75%"]);
    // The axis reads percentages.
    let ticks: Vec<_> = sc
        .texts
        .iter()
        .filter(|t| t.key.starts_with("axis:y/label:"))
        .map(|t| t.text.as_str())
        .collect();
    assert_eq!(ticks.first(), Some(&"0%"));
    assert_eq!(ticks.last(), Some(&"100%"));
    assert!(ticks.iter().all(|t| t.ends_with('%')), "{ticks:?}");
}

#[test]
fn grouped_bars_share_the_band_side_by_side_from_the_baseline() {
    let sc = scene(&spec("group", TABLE));
    let (ax, ay) = (rect(&sc, "layer0/a/x"), rect(&sc, "layer0/a/y"));
    assert_eq!(ax.bounds.w, ay.bounds.w, "equal slots");
    assert!(
        ax.bounds.x + ax.bounds.w <= ay.bounds.x,
        "series x is left of series y"
    );
    assert_eq!(
        ax.bounds.y + ax.bounds.h,
        ay.bounds.y + ay.bounds.h,
        "both stand on the baseline"
    );
    // Both fit inside the band of `a`, which is as wide as a plain bar would be.
    let plain = scene(&spec(
        "overlay",
        r#"{ "c": ["a", "b"], "s": ["x", "x"], "v": [3, 2] }"#,
    ));
    let band = rect(&plain, "layer0/a");
    assert!(ax.bounds.x >= band.bounds.x - 1.0);
    assert!(ay.bounds.x + ay.bounds.w <= band.bounds.x + band.bounds.w + 1.0);
    // Heights are not stacked: the axis only needs to reach the largest value.
    assert_eq!(top_tick(&sc), 3.0);
}

#[test]
fn duplicate_rows_are_summed_into_one_mark() {
    let sc = scene(&spec(
        "stack",
        r#"{ "c": ["a", "a", "a"], "s": ["x", "x", "y"], "v": [1, 2, 4] }"#,
    ));
    assert_eq!(sc.rects.len(), 2);
    let x = rect(&sc, "layer0/a/x");
    assert_eq!(x.sum, Some(3.0));
    assert_eq!(x.row, 0, "the first row stands for the group");
    assert_eq!(rect(&sc, "layer0/a/y").sum, None);
    assert_eq!(sc.labels[0].text.text, "3");
}

#[test]
fn stacked_labels_sit_centred_inside_or_are_left_out() {
    let sc = scene(
        &std::fs::read_to_string(format!(
            "{}/../../examples/specs/bar-stacked.json",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap(),
    );
    // Twelve segments; Q4 Services (5 of a ~245 stack) is too thin for its label.
    assert_eq!(sc.rects.len(), 12);
    assert_eq!(sc.labels.len(), 11);
    assert!(
        !sc.labels
            .iter()
            .any(|l| l.text.key == "layer0/Q4/Services/label")
    );
    for l in &sc.labels {
        let r = &sc.rects[l.mark];
        assert_eq!(l.text.x, (r.bounds.x + r.bounds.w / 2.0).round());
        // Between the segment's top and bottom.
        assert!(
            l.text.y > r.bounds.y && l.text.y < r.bounds.y + r.bounds.h,
            "{}",
            r.key
        );
    }
}

fn scene3d(json: &str) -> Scene3d {
    let spec = parse_spec_json(json).expect("parses");
    assert_eq!(validate(&spec), vec![], "{json}");
    build_scene_3d(&spec, (800, 500), 1.0, FontMetrics::inter())
}

#[test]
fn stacked_3d_boxes_pile_up_in_each_cell_and_keep_stable_keys() {
    let sc = scene3d(
        &std::fs::read_to_string(format!(
            "{}/../../examples/specs/bar3d-stacked.json",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap(),
    );
    assert_eq!(sc.boxes.len(), 36);
    let b = |key: &str| {
        sc.boxes
            .iter()
            .find(|b| b.key == key)
            .unwrap_or_else(|| panic!("{key}"))
    };
    let (h, s, v) = (
        b("layer0/Q1/Hardware/North"),
        b("layer0/Q1/Software/North"),
        b("layer0/Q1/Services/North"),
    );
    assert_eq!(h.min[1], 0.0);
    assert_eq!(s.min[1], h.max[1]);
    assert_eq!(v.min[1], s.max[1]);
    assert!(
        v.max[1] <= 1.0 && v.max[1] > 0.7,
        "the tallest stack fills the box: {}",
        v.max[1]
    );
    // Same footprint, same cell.
    assert_eq!((h.min[0], h.max[2]), (v.min[0], v.max[2]));
    // Another region is another cell.
    assert_ne!(b("layer0/Q1/Hardware/South").max[2], h.max[2]);
}

#[test]
fn grouped_3d_boxes_split_the_x_cell() {
    let json = r#"{
      "dimension": "3d",
      "data": { "t": { "c": ["a", "a"], "s": ["x", "y"], "v": [1, 2] } },
      "axes": [ { "id": "x", "orient": "bottom" }, { "id": "y", "orient": "left" } ],
      "layers": [ { "mark": "bar", "data": "t", "position": "group",
        "encoding": { "x": { "field": "c" }, "y": { "field": "v" }, "color": { "field": "s" } } } ]
    }"#;
    let sc = scene3d(json);
    let (x, y) = (&sc.boxes[0], &sc.boxes[1]);
    assert_eq!(x.key, "layer0/a/x");
    assert!(x.max[0] <= y.min[0], "side by side");
    assert_eq!((x.min[2], x.max[2]), (y.min[2], y.max[2]), "same depth");
    assert_eq!((x.min[1], y.min[1]), (0.0, 0.0));
}

// --- validation ----------------------------------------------------------------------------

fn issues(json: &str) -> Vec<(String, String)> {
    match parse_spec_json(json) {
        Ok(spec) => validate(&spec)
            .into_iter()
            .map(|i| (i.path, i.message))
            .collect(),
        Err(e) => e.into_iter().map(|i| (i.path, i.message)).collect(),
    }
}

#[test]
fn positions_need_a_colour_field() {
    for position in ["stack", "normalize", "group"] {
        let json = format!(
            r##"{{ "data": {{ "t": {{ "c": ["a"], "v": [1] }} }},
              "axes": [ {{ "id": "x", "orient": "bottom" }}, {{ "id": "y", "orient": "left" }} ],
              "layers": [ {{ "mark": "bar", "data": "t", "position": "{position}",
                "encoding": {{ "x": {{ "field": "c" }}, "y": {{ "field": "v" }}, "color": {{ "value": "#ff0000" }} }} }} ] }}"##
        );
        let got = issues(&json);
        assert_eq!(got.len(), 1, "{got:?}");
        assert_eq!(got[0].0, "/layers/0/position");
        assert!(got[0].1.contains("encoding.color.field"), "{}", got[0].1);
    }
}

#[test]
fn normalize_rejects_negative_values_with_the_row() {
    let json = spec(
        "normalize",
        r#"{ "c": ["a", "a"], "s": ["x", "y"], "v": [3, -1] }"#,
    );
    let got = issues(&json);
    assert_eq!(got.len(), 1, "{got:?}");
    assert_eq!(got[0].0, "/layers/0/encoding/y/field");
    assert!(
        got[0].1.contains("row 1") && got[0].1.contains("-1"),
        "{}",
        got[0].1
    );
}

#[test]
fn a_percent_axis_cannot_be_shared_with_raw_values() {
    let json = r#"{
      "data": { "t": { "c": ["a", "a"], "s": ["x", "y"], "v": [3, 1] } },
      "axes": [ { "id": "x", "orient": "bottom" }, { "id": "y", "orient": "left" } ],
      "layers": [
        { "mark": "bar", "data": "t", "position": "normalize",
          "encoding": { "x": { "field": "c" }, "y": { "field": "v" }, "color": { "field": "s" } } },
        { "mark": "bar", "data": "t",
          "encoding": { "x": { "field": "c" }, "y": { "field": "v" } } } ] }"#;
    let got = issues(json);
    assert_eq!(got.len(), 1, "{got:?}");
    assert_eq!(got[0].0, "/layers/1/encoding/y/axis");
    assert!(got[0].1.contains("own axis"), "{}", got[0].1);
}

#[test]
fn unknown_positions_are_a_parse_error_with_a_path() {
    let json = spec("pile", TABLE);
    let got = issues(&json);
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].0, "/layers/0/position");
}
