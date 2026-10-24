//! Line layers: series, gaps, numeric x, composite charts, picking and validation.
#![cfg(not(target_arch = "wasm32"))]

use facetry_core::scene::Scene;
use facetry_core::{build_scene, parse_spec_json, validate};
use facetry_font::FontMetrics;

fn scene_at(json: &str, size: (u32, u32)) -> Scene {
    let spec = parse_spec_json(json).unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(validate(&spec), vec![], "{json}");
    build_scene(&spec, size, 1.0, FontMetrics::inter())
}

fn scene(json: &str) -> Scene {
    scene_at(json, (800, 500))
}

fn issues(json: &str) -> Vec<(String, String)> {
    match parse_spec_json(json) {
        Ok(spec) => validate(&spec)
            .into_iter()
            .map(|i| (i.path, i.message))
            .collect(),
        Err(e) => e.into_iter().map(|i| (i.path, i.message)).collect(),
    }
}

/// A categorical-x line chart; `extra` goes into the layer.
fn categorical(table: &str, extra: &str) -> String {
    format!(
        r#"{{
  "data": {{ "t": {table} }},
  "axes": [ {{ "id": "x", "orient": "bottom" }}, {{ "id": "y", "orient": "left" }} ],
  "layers": [ {{ "mark": "line", "data": "t" {extra},
    "encoding": {{ "x": {{ "field": "c" }}, "y": {{ "field": "v" }}, "color": {{ "field": "s" }} }} }} ]
}}"#
    )
}

fn numeric(table: &str, axis_extra: &str) -> String {
    format!(
        r#"{{
  "data": {{ "t": {table} }},
  "axes": [ {{ "id": "x", "orient": "bottom"{axis_extra} }}, {{ "id": "y", "orient": "left" }} ],
  "layers": [ {{ "mark": "line", "data": "t",
    "encoding": {{ "x": {{ "field": "c" }}, "y": {{ "field": "v" }} }} }} ]
}}"#
    )
}

#[test]
fn each_series_is_one_polyline_in_domain_order() {
    // Rows arrive shuffled; the line still runs a, b, c.
    let sc = scene(&categorical(
        r#"{ "c": ["c", "a", "b", "a", "c", "b"], "s": ["x", "x", "x", "y", "y", "y"], "v": [3, 1, 2, 9, 7, 8] }"#,
        "",
    ));
    assert_eq!(sc.polylines.len(), 2);
    assert_eq!(sc.polylines[0].key, "layer0/x");
    assert_eq!(sc.polylines[1].key, "layer0/y");
    for line in &sc.polylines {
        assert_eq!(line.runs.len(), 1);
        let xs: Vec<f32> = line.runs[0].iter().map(|p| p[0]).collect();
        assert!(xs.windows(2).all(|w| w[0] < w[1]), "{xs:?}");
    }
    // The first series is lower than the second everywhere, so its points sit lower on screen
    // (a larger y in pixels).
    for k in 0..3 {
        assert!(sc.polylines[0].runs[0][k][1] > sc.polylines[1].runs[0][k][1]);
    }
    assert_eq!(sc.points.len(), 6);
    assert!(
        sc.points.iter().all(|p| !p.visible),
        "no markers unless asked"
    );
}

#[test]
fn a_missing_value_breaks_the_line_into_runs() {
    let sc = scene(&categorical(
        r#"{ "c": ["a", "b", "c", "d", "e"], "s": ["x", "x", "x", "x", "x"], "v": [1, 2, null, 4, 5] }"#,
        "",
    ));
    assert_eq!(sc.polylines.len(), 1);
    let lens: Vec<_> = sc.polylines[0].runs.iter().map(Vec::len).collect();
    assert_eq!(lens, [2, 2]);
    assert_eq!(sc.points.len(), 4, "no point for the missing value");
    // A lone point between two gaps is a run of one (drawn as a dot).
    let sc = scene(&categorical(
        r#"{ "c": ["a", "b", "c"], "s": ["x", "x", "x"], "v": [null, 2, null] }"#,
        "",
    ));
    assert_eq!(sc.polylines[0].runs.len(), 1);
    assert_eq!(sc.polylines[0].runs[0].len(), 1);
}

#[test]
fn markers_and_width_come_from_the_spec() {
    let table = r#"{ "c": ["a", "b"], "s": ["x", "x"], "v": [1, 2] }"#;
    let sc = scene(&categorical(
        table,
        r#", "point": { "size": 12 }, "style": { "strokeWidth": 5 }"#,
    ));
    assert!(sc.points.iter().all(|p| p.visible && p.size == 12.0));
    assert_eq!(sc.polylines[0].width, 5.0);
    let sc = scene(&categorical(table, r#", "point": true"#));
    assert!(sc.points.iter().all(|p| p.visible && p.size == 8.0));
    assert_eq!(sc.polylines[0].width, 2.0, "default width");
    let sc = scene(&categorical(table, r#", "point": false"#));
    assert!(sc.points.iter().all(|p| !p.visible));
    // The width scales with the pixel ratio like everything else.
    let spec = parse_spec_json(&categorical(table, "")).unwrap();
    let sc2 = build_scene(&spec, (1600, 1000), 2.0, FontMetrics::inter());
    assert_eq!(sc2.polylines[0].width, 4.0);
    assert_eq!(sc2.pick_radius, 24.0);
}

#[test]
fn numeric_x_uses_a_linear_scale_with_nice_ticks_and_sorts_by_x() {
    let sc = scene(&numeric(
        r#"{ "c": [30, 10, 20, 0], "v": [3, 1, 2, 0.5] }"#,
        "",
    ));
    let xs: Vec<f32> = sc.polylines[0].runs[0].iter().map(|p| p[0]).collect();
    assert!(xs.windows(2).all(|w| w[0] < w[1]), "sorted by x: {xs:?}");
    // Positions are proportional to the values: 10 is a third of the way from 0 to 30.
    let step = xs[1] - xs[0];
    assert!(((xs[2] - xs[1]) - step).abs() < 0.01 && ((xs[3] - xs[2]) - step).abs() < 0.01);
    let ticks: Vec<_> = sc
        .texts
        .iter()
        .filter(|t| t.key.starts_with("axis:x/label:"))
        .map(|t| t.text.as_str())
        .collect();
    assert_eq!(ticks.first(), Some(&"0"));
    assert!(ticks.contains(&"30") || ticks.contains(&"35"), "{ticks:?}");
    // Tick labels sit under their tick marks.
    let tick = sc.rules.iter().find(|r| r.key == "axis:x/tick:0").unwrap();
    let label = sc.texts.iter().find(|t| t.key == "axis:x/label:0").unwrap();
    assert!((tick.bounds.x - label.x).abs() <= 1.0);
}

#[test]
fn years_have_no_thousands_separator_and_the_axis_does_not_include_zero() {
    let sc = scene(&numeric(
        r#"{ "c": [2015, 2016, 2017, 2018, 2019], "v": [1, 2, 3, 2, 1] }"#,
        "",
    ));
    let labels: Vec<_> = sc
        .texts
        .iter()
        .filter(|t| t.key.starts_with("axis:x/label:"))
        .map(|t| t.text.as_str())
        .collect();
    assert_eq!(labels.first(), Some(&"2015"), "{labels:?}");
    assert!(labels.iter().all(|l| !l.contains(',')));
    // The first tick label stays inside the canvas.
    let first = sc
        .texts
        .iter()
        .find(|t| t.key.starts_with("axis:x/label:"))
        .unwrap();
    assert!(first.x > 10.0);
}

#[test]
fn numeric_x_scale_overrides_apply() {
    let sc = scene(&numeric(
        r#"{ "c": [1, 2, 3], "v": [1, 2, 3] }"#,
        r#", "scale": { "domain": [0, 10] }"#,
    ));
    let xs: Vec<f32> = sc.polylines[0].runs[0].iter().map(|p| p[0]).collect();
    // The data covers 10% to 30% of the fixed 0..10 domain.
    let tick0 = sc
        .rules
        .iter()
        .find(|r| r.key == "axis:x/tick:0")
        .unwrap()
        .bounds
        .x;
    let tick10 = sc
        .rules
        .iter()
        .find(|r| r.key == "axis:x/tick:10")
        .unwrap()
        .bounds
        .x;
    let frac = (xs[0] - tick0) / (tick10 - tick0);
    assert!((frac - 0.1).abs() < 0.01, "{frac}");
}

#[test]
fn bars_and_a_line_share_the_band_centres() {
    let json = r#"{
      "data": { "k": { "q": ["Q1", "Q2", "Q3"], "rev": [10, 20, 15], "m": [1, 3, 2] } },
      "axes": [ { "id": "x", "orient": "bottom" }, { "id": "a", "orient": "left" }, { "id": "b", "orient": "right" } ],
      "layers": [
        { "mark": "bar", "data": "k", "encoding": { "x": { "field": "q" }, "y": { "field": "rev", "axis": "a" } } },
        { "mark": "line", "data": "k", "encoding": { "x": { "field": "q" }, "y": { "field": "m", "axis": "b" } } } ] }"#;
    let sc = scene(json);
    assert_eq!(sc.rects.len(), 3);
    for (bar, point) in sc.rects.iter().zip(&sc.points) {
        let centre = bar.bounds.x + bar.bounds.w / 2.0;
        assert!((centre - point.x).abs() <= 0.5, "{} vs {}", centre, point.x);
    }
    assert_eq!(sc.points[0].key, "layer1/Q1");
    // The line is on the right axis: its own domain, so the largest value is near the top
    // even though the bars' axis goes to 20.
    assert!(sc.points[1].y < sc.rects[0].bounds.y);
    assert_eq!(sc.mark_count(), 6);
}

#[test]
fn picking_prefers_a_nearby_point_over_the_bar_behind_it() {
    let json = r#"{
      "data": { "k": { "q": ["Q1", "Q2"], "rev": [10, 20], "m": [5, 6] } },
      "axes": [ { "id": "x", "orient": "bottom" }, { "id": "a", "orient": "left" } ],
      "layers": [
        { "mark": "bar", "data": "k", "encoding": { "x": { "field": "q" }, "y": { "field": "rev" } } },
        { "mark": "line", "data": "k", "encoding": { "x": { "field": "q" }, "y": { "field": "m" } } } ] }"#;
    let sc = scene(json);
    let p = &sc.points[0];
    // Right on the point: the point (mark index after the 2 bars), though it is inside a bar.
    assert_eq!(sc.pick(p.x, p.y), Some(2));
    assert_eq!(sc.mark(2).unwrap().key, "layer1/Q1");
    assert_eq!(sc.mark(2).unwrap().layer, 1);
    // Within the pick radius counts; beyond it falls through to the bar underneath.
    assert_eq!(sc.pick(p.x + 11.0, p.y), Some(2));
    assert_eq!(sc.pick(p.x + 14.0, p.y), Some(0));
    // The nearest of two candidates wins.
    let q = &sc.points[1];
    assert_eq!(sc.pick(q.x - 2.0, q.y + 1.0), Some(3));
    // Nothing under the pointer.
    assert_eq!(sc.pick(2.0, 2.0), None);
}

#[test]
fn hover_labels_and_always_labels_belong_to_the_points() {
    let sc = scene(&categorical(
        r#"{ "c": ["a", "b"], "s": ["x", "x"], "v": [1200, 2] }"#,
        r#", "label": { "show": "always" }"#,
    ));
    assert_eq!(sc.labels.len(), 2);
    assert_eq!(sc.labels[0].text.text, "1,200");
    // The first point is at mark index 0 (no bars), centred above the point.
    assert_eq!(sc.labels[0].mark, 0);
    assert_eq!(sc.labels[0].text.x, sc.points[0].x.round());
    // Above the point, except at the very top of the plot where there is no room: below it.
    assert!(
        sc.labels[0].text.y > sc.points[0].y,
        "1,200 is the top of the axis"
    );
    assert!(sc.labels[1].text.y < sc.points[1].y);
}

// --- validation ----------------------------------------------------------------------------

#[test]
fn lines_are_rejected_in_3d_with_a_clear_message() {
    let json = r#"{ "dimension": "3d",
      "data": { "t": { "c": ["a"], "v": [1] } },
      "axes": [ { "id": "x", "orient": "bottom" }, { "id": "y", "orient": "left" } ],
      "layers": [ { "mark": "line", "data": "t", "encoding": { "x": { "field": "c" }, "y": { "field": "v" } } } ] }"#;
    let got = issues(json);
    assert_eq!(got.len(), 1, "{got:?}");
    assert_eq!(got[0].0, "/layers/0/mark");
    assert!(
        got[0].1.contains("line charts are 2D only for now"),
        "{}",
        got[0].1
    );
}

#[test]
fn only_lines_take_style_and_point_and_only_bars_take_position() {
    let table = r#"{ "c": ["a"], "s": ["x"], "v": [1] }"#;
    let bad = |extra: &str| {
        let json = categorical(table, extra);
        issues(&json)
    };
    assert_eq!(bad(r#", "position": "stack""#)[0].0, "/layers/0/position");
    assert_eq!(
        bad(r#", "style": { "strokeWidth": 0 }"#)[0].0,
        "/layers/0/style/strokeWidth"
    );
    assert_eq!(
        bad(r#", "style": { "strokeWidth": 500 }"#)[0].0,
        "/layers/0/style/strokeWidth"
    );
    assert_eq!(
        bad(r#", "point": { "size": -1 }"#)[0].0,
        "/layers/0/point/size"
    );
    // The same options on a bar layer.
    let bar = categorical(table, r#", "point": true, "style": { "strokeWidth": 2 }"#)
        .replace("\"line\"", "\"bar\"");
    let got: Vec<_> = issues(&bar).into_iter().map(|i| i.0).collect();
    assert_eq!(got, ["/layers/0/style/strokeWidth", "/layers/0/point"]);
}

#[test]
fn a_bottom_axis_cannot_mix_categories_and_numbers() {
    let json = r#"{
      "data": { "t": { "c": ["a", "b"], "n": [1, 2], "v": [1, 2] } },
      "axes": [ { "id": "x", "orient": "bottom" }, { "id": "y", "orient": "left" } ],
      "layers": [
        { "mark": "line", "data": "t", "encoding": { "x": { "field": "n" }, "y": { "field": "v" } } },
        { "mark": "bar", "data": "t", "encoding": { "x": { "field": "c" }, "y": { "field": "v" } } } ] }"#;
    let got = issues(json);
    assert_eq!(got.len(), 1, "{got:?}");
    assert_eq!(got[0].0, "/layers/1/encoding/x/field");
}

#[test]
fn scale_on_a_categorical_bottom_axis_is_still_an_error() {
    let json = categorical(r#"{ "c": ["a"], "s": ["x"], "v": [1] }"#, "").replace(
        r#"{ "id": "x", "orient": "bottom" }"#,
        r#"{ "id": "x", "orient": "bottom", "scale": { "nice": false } }"#,
    );
    assert_eq!(issues(&json)[0].0, "/axes/0/scale");
}

#[test]
fn building_with_a_line_in_3d_skips_it_instead_of_panicking() {
    let json = r#"{ "dimension": "3d",
      "data": { "t": { "c": ["a"], "v": [1] } },
      "axes": [ { "id": "x", "orient": "bottom" }, { "id": "y", "orient": "left" } ],
      "layers": [ { "mark": "line", "data": "t", "encoding": { "x": { "field": "c" }, "y": { "field": "v" } } } ] }"#;
    let spec = parse_spec_json(json).unwrap();
    let sc = facetry_core::build_scene_3d(&spec, (800, 500), 1.0, FontMetrics::inter());
    assert!(sc.boxes.is_empty());
}

#[test]
fn tiny_viewports_do_not_panic_with_lines() {
    let json = std::fs::read_to_string(format!(
        "{}/../../examples/specs/combo-bar-line.json",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap();
    for size in [(0, 0), (1, 1), (20, 20), (50, 400), (400, 50)] {
        let sc = scene_at(&json, size);
        assert_eq!(sc.rects.len(), 4);
    }
    let json = std::fs::read_to_string(format!(
        "{}/../../examples/specs/line-numeric-x.json",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap();
    for size in [(0, 0), (1, 1), (20, 20), (50, 400), (400, 50)] {
        scene_at(&json, size);
    }
}
