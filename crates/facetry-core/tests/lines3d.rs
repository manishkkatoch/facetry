//! Lines in 3D: one polyline per series and depth row, in world space, with data points that
//! hover and click like bars; they share the x and z bands with bars, and take part in
//! hiding series, value labels and the y domain.
#![cfg(not(target_arch = "wasm32"))]

use facetry_core::scene3d::{LabelRole, Scene3d};
use facetry_core::{ChartSpec, build_scene_3d, parse_spec_json, validate};
use facetry_font::FontMetrics;

fn example(name: &str) -> String {
    std::fs::read_to_string(format!(
        "{}/../../examples/specs/{name}.json",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

fn spec(json: &str) -> ChartSpec {
    let spec = parse_spec_json(json).unwrap_or_else(|e| panic!("{e:?}\n{json}"));
    assert_eq!(validate(&spec), vec![], "{json}");
    spec
}

fn build(spec: &ChartSpec, dpr: f32) -> Scene3d {
    let size = (800.0 * dpr, 500.0 * dpr);
    build_scene_3d(
        spec,
        (size.0 as u32, size.1 as u32),
        dpr,
        FontMetrics::inter(),
    )
}

fn scene(name: &str) -> Scene3d {
    build(&spec(&example(name)), 1.0)
}

/// A one-table 3D line chart; `layer_extra` is spliced into the layer (style, point, label).
fn lines(table: &str, encoding: &str, layer_extra: &str, extra: &str) -> String {
    format!(
        r#"{{ "dimension": "3d", {extra}
      "data": {{ "t": {table} }},
      "axes": [ {{ "id": "x", "orient": "bottom" }}, {{ "id": "y", "orient": "left" }},
                {{ "id": "z", "orient": "depth" }} ],
      "layers": [ {{ "mark": "line", "data": "t" {layer_extra}, "encoding": {{ {encoding} }} }} ] }}"#
    )
}

#[test]
fn one_polyline_per_series_on_its_own_depth_row() {
    let sc = scene("line3d");
    assert_eq!(sc.polylines.len(), 3);
    assert_eq!(sc.points.len(), 18);
    let keys: Vec<_> = sc.polylines.iter().map(|p| p.key.as_str()).collect();
    assert_eq!(keys, ["layer0/North", "layer0/South", "layer0/West"]);
    // The first category is the front row (largest z); each line stays on its row.
    let zs: Vec<f32> = sc.polylines.iter().map(|p| p.runs[0][0][2]).collect();
    assert!(zs[0] > zs[1] && zs[1] > zs[2], "{zs:?}");
    for p in &sc.polylines {
        assert_eq!(p.runs.len(), 1);
        assert_eq!(p.runs[0].len(), 6);
        assert!(p.runs[0].iter().all(|v| v[2] == p.runs[0][0][2]));
        // x runs along the month band centres, in order.
        assert!(p.runs[0].windows(2).all(|w| w[0][0] < w[1][0]));
    }
    // Everything is inside the plot box.
    for v in sc.polylines.iter().flat_map(|p| p.runs.iter().flatten()) {
        for i in 0..3 {
            assert!(
                v[i] >= 0.0 && v[i] <= sc.size[i] + 1e-4,
                "{v:?} in {:?}",
                sc.size
            );
        }
    }
    // y is the value on the axis: the top of the axis (40) is height 1, 35 is 0.875.
    let top = sc.polylines[0].runs[0][5][1];
    assert!((top - 35.0 / 40.0).abs() < 1e-4, "{top}");
}

#[test]
fn points_carry_the_row_and_a_key_without_repeating_the_series() {
    let sc = scene("line3d");
    let p = sc
        .points
        .iter()
        .find(|p| p.key == "layer0/Mar/South")
        .unwrap();
    // Rows 6..12 are South: Mar is its third.
    assert_eq!((p.layer, p.row, p.visible), (0, 6 + 2, true));
    assert_eq!(p.size_px, 8.0);
    // A chart whose colour and z are different fields names both.
    let json = lines(
        r#"{ "x": ["a","b","a","b"], "s": ["u","u","v","v"], "r": ["p","p","q","q"], "y": [1,2,3,4] }"#,
        r#""x": {"field":"x"}, "y": {"field":"y"}, "z": {"field":"r"}, "color": {"field":"s"}"#,
        "",
        "",
    );
    let sc = build(&spec(&json), 1.0);
    assert!(
        sc.points.iter().any(|p| p.key == "layer0/a/u/p"),
        "{:?}",
        sc.points
    );
    assert_eq!(sc.polylines.len(), 2);
}

#[test]
fn stroke_width_and_marker_size_follow_the_pixel_ratio() {
    let json = example("line3d");
    let one = build(&spec(&json), 1.0);
    let two = build(&spec(&json), 2.0);
    assert_eq!(one.polylines[0].width_px, 3.0);
    assert_eq!(two.polylines[0].width_px, 6.0);
    assert_eq!((one.points[0].size_px, two.points[0].size_px), (8.0, 16.0));
    assert_eq!((one.pick_radius, two.pick_radius), (12.0, 24.0));
}

#[test]
fn no_markers_unless_asked_but_the_points_still_exist_to_hover() {
    let json = example("line3d").replace(r#""point": true,"#, "");
    let sc = build(&spec(&json), 1.0);
    assert_eq!(sc.points.len(), 18);
    assert!(sc.points.iter().all(|p| !p.visible));
}

#[test]
fn without_z_every_line_sits_on_the_front_row_and_they_overlap() {
    let json = lines(
        r#"{ "m": ["a","b","a","b"], "s": ["u","u","v","v"], "y": [1,2,3,4] }"#,
        r#""x": {"field":"m"}, "y": {"field":"y"}, "color": {"field":"s"}"#,
        "",
        "",
    );
    let sc = build(&spec(&json), 1.0);
    assert_eq!(sc.polylines.len(), 2);
    let z = sc.polylines[0].runs[0][0][2];
    assert!(
        sc.polylines
            .iter()
            .flat_map(|p| p.runs.iter().flatten())
            .all(|v| v[2] == z)
    );
    // One implicit row: its middle, a single cell deep.
    assert!((z - sc.size[2] / 2.0).abs() < 1e-4, "{z} in {:?}", sc.size);
    assert!(sc.lines.iter().all(|l| !l.key.starts_with("axis:z/")));
}

#[test]
fn a_missing_value_breaks_the_line_and_a_lone_point_is_a_run_of_one() {
    let json = lines(
        r#"{ "m": ["a","b","c","d","e"], "y": [1, 2, null, 4, null] }"#,
        r#""x": {"field":"m"}, "y": {"field":"y"}"#,
        "",
        "",
    );
    let sc = build(&spec(&json), 1.0);
    assert_eq!(sc.polylines.len(), 1);
    let lens: Vec<usize> = sc.polylines[0].runs.iter().map(Vec::len).collect();
    assert_eq!(lens, [2, 1]);
    // Only finite values are points (hover has nothing to land on at a gap).
    assert_eq!(sc.points.len(), 3);
}

#[test]
fn a_numeric_x_is_a_linear_axis_with_ticks_and_a_fixed_box() {
    let json = lines(
        r#"{ "year": [2000, 2010, 2020, 2005], "s": ["a","a","a","a"], "y": [1, 2, 3, 4] }"#,
        r#""x": {"field":"year"}, "y": {"field":"y"}, "color": {"field":"s"}"#,
        "",
        "",
    );
    let sc = build(&spec(&json), 1.0);
    // Ascending in x whatever the data order, spread over the whole box width.
    let xs: Vec<f32> = sc.polylines[0].runs[0].iter().map(|v| v[0]).collect();
    assert_eq!(xs.len(), 4);
    assert!(xs.windows(2).all(|w| w[0] < w[1]), "{xs:?}");
    assert!(
        xs[0].abs() < 1e-4 && (xs[3] - sc.size[0]).abs() < 1e-4,
        "{xs:?} {:?}",
        sc.size
    );
    // The 2005 point is a quarter of the way along 2000..2020.
    assert!((xs[1] - sc.size[0] * 0.25).abs() < 1e-4);
    // Tick labels are plain years, not "2,000".
    let labels: Vec<&str> = sc
        .texts
        .iter()
        .filter(|t| t.key.starts_with("axis:x/label:") && t.key.ends_with("@000"))
        .map(|t| t.text.as_str())
        .collect();
    assert!(
        labels.contains(&"2000") && labels.contains(&"2020"),
        "{labels:?}"
    );
    assert!(labels.iter().all(|l| !l.contains(',')));
    // And the floor has gridlines at the ticks (there are no cells to draw).
    assert!(sc.lines.iter().any(|l| l.key.starts_with("floor:x:")));
}

#[test]
fn bars_and_lines_share_the_x_and_z_bands() {
    let sc = scene("combo3d-bar-line");
    assert_eq!(sc.boxes.len(), 12);
    assert_eq!(
        sc.polylines.len(),
        3,
        "a line per row (one fixed colour, three depth rows)"
    );
    // Each line point is above the middle of the bar with the same quarter and region.
    for p in &sc.points {
        let parts: Vec<&str> = p.key.split('/').collect();
        let (q, region) = (parts[1], parts[2]);
        let bar = sc
            .boxes
            .iter()
            .find(|b| b.key == format!("layer0/{q}/{region}"))
            .unwrap_or_else(|| panic!("no bar for {}", p.key));
        let (cx, cz) = (
            (bar.min[0] + bar.max[0]) / 2.0,
            (bar.min[2] + bar.max[2]) / 2.0,
        );
        assert!(
            (p.pos[0] - cx).abs() < 1e-4 && (p.pos[2] - cz).abs() < 1e-4,
            "{}",
            p.key
        );
    }
    // One y axis for both: it reaches the larger of the bars and the line.
    let max_y = sc.points.iter().map(|p| p.pos[1]).fold(0.0, f32::max);
    assert!(max_y <= 1.0 && max_y > 0.8, "{max_y}");
    assert!(sc.boxes.iter().all(|b| b.max[1] < 0.8));
}

#[test]
fn the_y_domain_covers_lines_too_and_negative_values_go_below_zero() {
    let json = lines(
        r#"{ "m": ["a","b","c"], "y": [-10, 0, 30] }"#,
        r#""x": {"field":"m"}, "y": {"field":"y"}"#,
        "",
        "",
    );
    let sc = build(&spec(&json), 1.0);
    let ys: Vec<f32> = sc.polylines[0].runs[0].iter().map(|v| v[1]).collect();
    assert!(
        ys[0] >= 0.0 && ys[0] < ys[1] && ys[1] < ys[2] && ys[2] <= 1.0,
        "{ys:?}"
    );
}

#[test]
fn value_labels_hang_from_their_point_and_use_the_shared_mark_index_space() {
    let json = example("line3d").replace(
        r#""point": true,"#,
        r#""point": true, "label": { "show": "always", "format": { "decimals": 1 } },"#,
    );
    let sc = build(&spec(&json), 1.0);
    assert_eq!(sc.labels.len(), 18);
    for (i, l) in sc.labels.iter().enumerate() {
        assert_eq!(l.mark, sc.boxes.len() + sc.slices.len() + i);
        assert_eq!(l.text.role, LabelRole::OnPoint);
        assert_eq!(l.text.anchor, sc.points[i].pos);
        assert!(l.text.offset_px[1] < 0.0, "above the point");
    }
    assert_eq!(sc.labels[0].text.text, "12.0");
    // `hover` labels exist too (the renderer shows them when their point is hovered).
    let hover = example("line3d").replace(r#""point": true,"#, r#""label": { "show": "hover" },"#);
    let sc = build(&spec(&hover), 1.0);
    assert!(
        sc.labels
            .iter()
            .all(|l| l.show == facetry_core::spec::LabelShow::Hover)
    );
    // No label block, no labels.
    assert!(scene("line3d").labels.is_empty());
}

#[test]
fn area_adds_a_baseline_to_the_polyline() {
    assert!(scene("line3d").polylines.iter().all(|p| p.area.is_none()));
    let json =
        example("line3d").replace(r#""strokeWidth": 3"#, r#""strokeWidth": 3, "area": true"#);
    let sc = build(&spec(&json), 1.0);
    assert!(sc.polylines.iter().all(|p| p.area == Some(0.0)));
    // With negative values the baseline is the zero line, above the floor.
    let json = lines(
        r#"{ "m": ["a","b"], "y": [-10, 30] }"#,
        r#""x": {"field":"m"}, "y": {"field":"y"}"#,
        r#", "style": { "area": true }"#,
        "",
    );
    let sc = build(&spec(&json), 1.0);
    let base = sc.polylines[0].area.unwrap();
    assert!(base > 0.0 && base < sc.polylines[0].runs[0][1][1], "{base}");
}

#[test]
fn hiding_a_series_removes_its_line_points_and_depth_row() {
    let json = example("line3d").replace(
        r#""camera""#,
        r#""legend": { "hidden": ["South"] }, "camera""#,
    );
    let sc = build(&spec(&json), 1.0);
    assert_eq!(sc.polylines.len(), 2);
    assert!(sc.points.iter().all(|p| !p.key.contains("South")));
    assert_eq!(sc.points.len(), 12);
    // The depth axis fits the two rows that are left, and the legend still lists all three.
    let full = scene("line3d");
    assert!(
        (sc.size[2] - full.size[2] * 2.0 / 3.0).abs() < 1e-4,
        "{:?} {:?}",
        sc.size,
        full.size
    );
    assert_eq!(sc.overlay.rules.len(), 3);
    assert_eq!(sc.overlay.hits.len(), 3);
    // Colours do not move.
    assert_eq!(
        sc.polylines
            .iter()
            .find(|p| p.key == "layer0/West")
            .unwrap()
            .color,
        full.polylines
            .iter()
            .find(|p| p.key == "layer0/West")
            .unwrap()
            .color
    );
}

#[test]
fn duplicate_points_get_distinct_keys() {
    let json = lines(
        r#"{ "m": ["a","a","b"], "y": [1, 2, 3] }"#,
        r#""x": {"field":"m"}, "y": {"field":"y"}"#,
        "",
        "",
    );
    let sc = build(&spec(&json), 1.0);
    let keys: Vec<&str> = sc.points.iter().map(|p| p.key.as_str()).collect();
    assert_eq!(keys, ["layer0/a", "layer0/a#2", "layer0/b"]);
}

#[test]
fn building_is_deterministic_and_tiny_viewports_do_not_panic() {
    assert_eq!(scene("line3d"), scene("line3d"));
    assert_eq!(scene("combo3d-bar-line"), scene("combo3d-bar-line"));
    let s = spec(&example("combo3d-bar-line"));
    for size in [(0, 0), (1, 1), (20, 20), (50, 400)] {
        let sc = build_scene_3d(&s, size, 1.0, FontMetrics::inter());
        assert_eq!((sc.boxes.len(), sc.points.len()), (12, 12));
    }
}

#[test]
fn the_scene_has_no_line_marks_for_bar_charts() {
    let sc = scene("bar3d-grid");
    assert!(sc.polylines.is_empty() && sc.points.is_empty());
    assert_eq!(sc.mark_count(), 12);
}
