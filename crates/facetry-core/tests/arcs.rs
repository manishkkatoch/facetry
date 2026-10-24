//! Pie and donut layers: slices, angles, labels, picking and validation (2D and 3D).
#![cfg(not(target_arch = "wasm32"))]

use std::f32::consts::{PI, TAU};

use facetry_core::scene::Scene;
use facetry_core::scene3d::Scene3d;
use facetry_core::{build_scene, build_scene_3d, parse_spec_json, validate};
use facetry_font::FontMetrics;

fn pie_json(dimension: &str, table: &str, extra: &str, label: &str) -> String {
    format!(
        r#"{{
  "dimension": "{dimension}",
  "data": {{ "t": {table} }},
  "layers": [ {{ "mark": "arc", "data": "t" {extra} {label},
    "encoding": {{ "theta": {{ "field": "v" }}, "color": {{ "field": "c" }} }} }} ]
}}"#
    )
}

const TABLE: &str = r#"{ "c": ["a", "b", "c", "d"], "v": [1, 1, 1, 1] }"#;

fn scene_at(json: &str, size: (u32, u32)) -> Scene {
    let spec = parse_spec_json(json).unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(validate(&spec), vec![], "{json}");
    build_scene(&spec, size, 1.0, FontMetrics::inter())
}

fn pie(table: &str, extra: &str, label: &str) -> Scene {
    scene_at(&pie_json("2d", table, extra, label), (800, 500))
}

fn scene3d(table: &str, extra: &str, label: &str) -> Scene3d {
    let json = pie_json("3d", table, extra, label);
    let spec = parse_spec_json(&json).unwrap();
    assert_eq!(validate(&spec), vec![], "{json}");
    build_scene_3d(&spec, (800, 500), 1.0, FontMetrics::inter())
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

#[test]
fn slices_share_the_circle_in_order_clockwise_from_twelve_o_clock() {
    let sc = pie(r#"{ "c": ["a", "b", "c"], "v": [2, 1, 1] }"#, "", "");
    let keys: Vec<_> = sc.arcs.iter().map(|a| a.key.as_str()).collect();
    assert_eq!(keys, ["layer0/a", "layer0/b", "layer0/c"]);
    let angles: Vec<(f32, f32)> = sc.arcs.iter().map(|a| (a.start, a.end)).collect();
    assert_eq!(angles[0].0, 0.0);
    assert!((angles[0].1 - PI).abs() < 1e-5, "half the circle");
    assert!((angles[1].1 - 1.5 * PI).abs() < 1e-5);
    assert!((angles[2].1 - TAU).abs() < 1e-5);
    assert_eq!(angles[0].1, angles[1].0, "no gap, no overlap");
    // Centred in the viewport, with a radius that fits.
    let a = &sc.arcs[0];
    assert_eq!(a.center, [400.0, 250.0]);
    assert!(
        a.outer > 200.0 && a.outer <= 250.0 - 12.0 + 0.5,
        "{}",
        a.outer
    );
    assert_eq!(a.inner, 0.0);
    // Different slices have different colours, and the first slice is the first palette colour.
    assert_ne!(sc.arcs[0].color, sc.arcs[1].color);
}

#[test]
fn start_angle_pad_and_inner_radius_come_from_the_style() {
    let sc = pie(
        TABLE,
        r#", "style": { "startAngle": 90, "padAngle": 4, "innerRadius": 0.5 }"#,
        "",
    );
    let a = &sc.arcs[0];
    assert!((a.start - PI / 2.0).abs() < 1e-5, "3 o'clock");
    assert!((a.pad - 4.0_f32.to_radians()).abs() < 1e-6);
    assert!((a.inner - a.outer / 2.0).abs() < 1e-3);
    // Padding cuts half of it off each side.
    let (s0, s1) = a.span();
    assert!((s0 - a.start - 2.0_f32.to_radians()).abs() < 1e-5);
    assert!((a.end - s1 - 2.0_f32.to_radians()).abs() < 1e-5);
}

#[test]
fn rows_with_the_same_colour_merge_and_empty_slices_vanish() {
    let sc = pie(
        r#"{ "c": ["a", "b", "a", "c"], "v": [1, 2, 3, 0] }"#,
        "",
        "",
    );
    assert_eq!(sc.arcs.len(), 2, "a (summed) and b; c has no size");
    assert_eq!(sc.arcs[0].key, "layer0/a");
    assert_eq!(sc.arcs[0].sum, Some(4.0));
    assert_eq!(sc.arcs[0].row, 0);
    assert_eq!(sc.arcs[1].sum, None);
    assert!((sc.arcs[0].end - sc.arcs[0].start - 2.0 / 3.0 * TAU).abs() < 1e-4);
    // Missing numbers are no slice either.
    assert_eq!(
        pie(r#"{ "c": ["a", "b"], "v": [1, null] }"#, "", "")
            .arcs
            .len(),
        1
    );
}

#[test]
fn one_slice_is_a_full_circle_without_padding_and_no_data_is_an_empty_scene() {
    let sc = pie(
        r#"{ "c": ["a"], "v": [5] }"#,
        r#", "style": { "padAngle": 10 }"#,
        "",
    );
    let a = &sc.arcs[0];
    assert!((a.end - a.start - TAU).abs() < 1e-5);
    assert_eq!(a.span(), (a.start, a.end));
    assert!(a.contains(a.center[0] - 50.0, a.center[1] + 50.0));
    assert!(pie(r#"{ "c": [], "v": [] }"#, "", "").arcs.is_empty());
    assert!(pie(r#"{ "c": ["a"], "v": [0] }"#, "", "").arcs.is_empty());
}

#[test]
fn picking_tests_angle_and_radius() {
    let sc = pie(
        TABLE,
        r#", "style": { "innerRadius": 0.4, "padAngle": 6 }"#,
        "",
    );
    let a = &sc.arcs[0];
    let [cx, cy] = a.center;
    let r = (a.inner + a.outer) / 2.0;
    // Slice a is the first quarter: 12 to 3 o'clock, so up-right of the centre.
    let at = |deg: f32, r: f32| {
        (
            cx + r * deg.to_radians().sin(),
            cy - r * deg.to_radians().cos(),
        )
    };
    for (deg, want) in [(45.0, 0), (135.0, 1), (225.0, 2), (315.0, 3)] {
        let (x, y) = at(deg, r);
        assert_eq!(sc.pick(x, y), Some(want), "{deg} degrees");
    }
    // The hole, outside the pie, and the gap between slices (at 90 degrees, +-3 of padding).
    assert_eq!(sc.pick(cx, cy), None);
    let (x, y) = at(45.0, a.outer + 3.0);
    assert_eq!(sc.pick(x, y), None);
    let (x, y) = at(90.0, r);
    assert_eq!(sc.pick(x, y), None, "padding is a gap");
    // Marks are the slices; mark info says which row.
    assert_eq!(sc.mark_count(), 4);
    assert_eq!(sc.mark(1).unwrap().key, "layer0/b");
    let [mx, my] = sc.mark_center("layer0/c").unwrap();
    assert_eq!(sc.pick(mx, my), Some(2));
}

#[test]
fn a_slice_across_twelve_o_clock_is_picked_on_both_sides() {
    // Starting at -45 degrees: the first slice covers 315 to 45.
    let sc = pie(TABLE, r#", "style": { "startAngle": -45 }"#, "");
    let a = &sc.arcs[0];
    let [cx, cy] = a.center;
    let r = a.outer / 2.0;
    assert_eq!(sc.pick(cx - 10.0, cy - r), Some(0));
    assert_eq!(sc.pick(cx + 10.0, cy - r), Some(0));
    assert_eq!(sc.pick(cx + r, cy + 10.0), Some(1));
}

#[test]
fn labels_go_inside_when_they_fit_and_outside_on_a_leader_when_not() {
    let sc = pie(
        r#"{ "c": ["a", "b", "c"], "v": [90, 8, 2] }"#,
        "",
        r#", "label": { "show": "always", "format": { "percent": true } }"#,
    );
    let texts: Vec<_> = sc.labels.iter().map(|l| l.text.text.as_str()).collect();
    assert_eq!(texts, ["90%", "8%", "2%"]);
    let a = &sc.arcs[0];
    let dist = |t: &facetry_core::scene::Text| {
        ((t.x - a.center[0]).powi(2) + (t.y - a.center[1]).powi(2)).sqrt()
    };
    assert!(
        dist(&sc.labels[0].text) < a.outer,
        "the big slice's label is inside"
    );
    // The thin ones are beyond the rim, each with a leader from the rim outward.
    assert!(dist(&sc.labels[2].text) > a.outer);
    assert!(sc.polylines.iter().any(|l| l.key == "layer0/c/leader"));
    for line in &sc.polylines {
        let [p, q] = [line.runs[0][0], line.runs[0][1]];
        let (dp, dq) = (
            ((p[0] - a.center[0]).powi(2) + (p[1] - a.center[1]).powi(2)).sqrt(),
            ((q[0] - a.center[0]).powi(2) + (q[1] - a.center[1]).powi(2)).sqrt(),
        );
        assert!(
            (dp - a.outer).abs() < 0.1 && dq > dp,
            "leader runs outward from the rim"
        );
    }
    // Labels never leave the canvas, and the pie shrank to make room for them.
    for l in &sc.labels {
        assert!(l.text.x > 0.0 && l.text.x < 800.0 && l.text.y > 0.0 && l.text.y < 500.0);
    }
    let plain = pie(r#"{ "c": ["a", "b", "c"], "v": [90, 8, 2] }"#, "", "");
    assert!(sc.arcs[0].outer < plain.arcs[0].outer);
}

#[test]
fn hover_labels_have_no_leader_and_values_can_be_formatted() {
    let sc = pie(
        r#"{ "c": ["a", "b"], "v": [1200, 3] }"#,
        "",
        r#", "label": { "show": "hover", "format": { "decimals": 1 } }"#,
    );
    assert_eq!(sc.labels[0].text.text, "1,200.0");
    assert!(
        sc.labels
            .iter()
            .all(|l| l.show == facetry_core::spec::LabelShow::Hover)
    );
    assert!(
        sc.polylines.is_empty(),
        "a leader would be there even when the label is not"
    );
    assert_eq!(sc.labels[1].mark, 1);
}

#[test]
fn donut_labels_sit_in_the_ring() {
    let sc = pie(
        TABLE,
        r#", "style": { "innerRadius": 0.5 }"#,
        r#", "label": { "show": "always" }"#,
    );
    let a = &sc.arcs[0];
    for l in &sc.labels {
        let d = ((l.text.x - a.center[0]).powi(2) + (l.text.y - 4.0 - a.center[1]).powi(2)).sqrt();
        assert!(
            d > a.inner && d < a.outer,
            "{d} not in {}..{}",
            a.inner,
            a.outer
        );
    }
}

#[test]
fn labels_that_would_touch_are_dropped() {
    // Many tiny slices at the top: their outside labels would pile up.
    let sc = pie(
        r#"{ "c": ["a", "b", "c", "d", "e", "f", "g", "h"], "v": [1000, 5, 5, 5, 5, 5, 5, 5] }"#,
        "",
        r#", "label": { "show": "always" }"#,
    );
    let rect = |l: &facetry_core::scene::MarkLabel| {
        let t = &l.text;
        let w = FontMetrics::inter().text_width(&t.text, t.size_px);
        let x0 = match t.anchor {
            facetry_core::scene::Anchor::Start => t.x,
            facetry_core::scene::Anchor::End => t.x - w,
            facetry_core::scene::Anchor::Middle => t.x - w / 2.0,
        };
        [x0, t.y - 9.0, x0 + w, t.y + 3.0]
    };
    assert!(sc.labels.len() < 8, "some were dropped");
    for (i, a) in sc.labels.iter().enumerate() {
        for b in &sc.labels[i + 1..] {
            let (p, q) = (rect(a), rect(b));
            let overlap = p[0] < q[2] && q[0] < p[2] && p[1] < q[3] && q[1] < p[3];
            assert!(!overlap, "{} overlaps {}", a.text.text, b.text.text);
        }
    }
}

#[test]
fn tiny_viewports_do_not_panic() {
    let json = pie_json(
        "2d",
        r#"{ "c": ["a", "b", "c"], "v": [90, 8, 2] }"#,
        r#", "style": { "innerRadius": 0.6 }"#,
        r#", "label": { "show": "always" }"#,
    );
    for size in [(0, 0), (1, 1), (20, 20), (50, 400), (400, 50)] {
        let sc = scene_at(&json, size);
        assert!(
            sc.arcs
                .iter()
                .all(|a| a.outer >= 0.0 && a.outer.is_finite())
        );
    }
}

// --- 3D -----------------------------------------------------------------------------------

#[test]
fn a_3d_pie_is_extruded_slices_around_the_middle_of_its_box() {
    let sc = scene3d(TABLE, r#", "style": { "depth": 0.5 }"#, "");
    assert_eq!(sc.slices.len(), 4);
    assert!(
        sc.boxes.is_empty() && sc.lines.is_empty(),
        "no axes, no walls"
    );
    let s = &sc.slices[0];
    assert_eq!(s.center, [sc.size[0] / 2.0, sc.size[2] / 2.0]);
    assert!((s.height - 0.5 * s.outer).abs() < 1e-5);
    assert_eq!(sc.size[1], s.height);
    // Same angle convention as 2D; the far side (-z) is 12 o'clock.
    assert_eq!(
        (s.start, s.contains(s.center[0] + 0.2, s.center[1] - 0.2)),
        (0.0, true)
    );
    assert!(
        !s.contains(s.center[0] - 0.2, s.center[1] - 0.2),
        "the far left belongs to the last slice"
    );
    assert!(sc.slices[3].contains(s.center[0] - 0.2, s.center[1] - 0.2));
    assert_eq!(sc.slices[2].key, "layer0/c");
}

#[test]
fn a_3d_pie_labels_the_top_face() {
    let sc = scene3d(
        r#"{ "c": ["a", "b", "c"], "v": [90, 8, 2] }"#,
        "",
        r#", "label": { "show": "always", "format": { "percent": true } }"#,
    );
    let texts: Vec<_> = sc.labels.iter().map(|l| l.text.text.as_str()).collect();
    assert_eq!(texts, ["90%", "8%", "2%"]);
    let top = sc.size[1];
    assert!(
        sc.labels
            .iter()
            .all(|l| (l.text.anchor[1] - top).abs() < 1e-6)
    );
    // The big slice's label is within its radius; thin slices' labels are beyond the rim, with
    // a leader line each.
    let s = &sc.slices[0];
    let r = |l: &facetry_core::scene3d::MarkLabel3| {
        ((l.text.anchor[0] - s.center[0]).powi(2) + (l.text.anchor[2] - s.center[1]).powi(2)).sqrt()
    };
    assert!(r(&sc.labels[0]) < s.outer);
    assert!(r(&sc.labels[2]) > s.outer);
    assert!(sc.lines.iter().any(|l| l.key == "layer0/c/leader"));
}

// --- validation ----------------------------------------------------------------------------

#[test]
fn arcs_take_no_axes() {
    let json = r#"{
      "data": { "t": { "c": ["a"], "v": [1] } },
      "axes": [ { "id": "x", "orient": "bottom" }, { "id": "y", "orient": "left" } ],
      "layers": [ { "mark": "arc", "data": "t",
        "encoding": { "theta": { "field": "v" }, "color": { "field": "c" } } } ] }"#;
    let got = issues(json);
    let paths: Vec<_> = got.iter().map(|i| i.0.as_str()).collect();
    assert_eq!(paths, ["/axes/0", "/axes/1"]);
    assert!(got[0].1.contains("no axes"), "{}", got[0].1);
    // Without axes the same spec is fine.
    assert_eq!(issues(&pie_json("2d", TABLE, "", "")), vec![]);
}

#[test]
fn arcs_cannot_mix_with_other_marks_or_each_other() {
    let json = r#"{
      "data": { "t": { "c": ["a"], "v": [1] } },
      "layers": [
        { "mark": "arc", "data": "t", "encoding": { "theta": { "field": "v" }, "color": { "field": "c" } } },
        { "mark": "arc", "data": "t", "encoding": { "theta": { "field": "v" }, "color": { "field": "c" } } },
        { "mark": "bar", "data": "t", "encoding": { "x": { "field": "c" }, "y": { "field": "v" } } } ] }"#;
    let got: Vec<_> = issues(json).into_iter().map(|i| i.0).collect();
    assert!(got.contains(&"/layers/1/mark".to_owned()), "{got:?}");
    assert!(got.contains(&"/layers/2/mark".to_owned()), "{got:?}");
}

#[test]
fn theta_and_colour_are_checked() {
    let enc = |theta: &str, color: &str| {
        let parts: Vec<&str> = [theta, color]
            .into_iter()
            .filter(|p| !p.is_empty())
            .collect();
        format!(
            r#"{{ "data": {{ "t": {{ "c": ["a", "b"], "v": [1, -2], "n": [3, 4] }} }},
              "layers": [ {{ "mark": "arc", "data": "t", "encoding": {{ {} }} }} ] }}"#,
            parts.join(",")
        )
    };
    let ok_color = r#""color": { "field": "c" }"#;
    let got = issues(&enc(r#""theta": { "field": "v" }"#, ok_color));
    assert_eq!(got[0].0, "/layers/0/encoding/theta/field");
    assert!(
        got[0].1.contains("row 1") && got[0].1.contains("-2"),
        "{}",
        got[0].1
    );
    assert_eq!(issues(&enc("", ok_color))[0].0, "/layers/0/encoding/theta");
    assert_eq!(
        issues(&enc(r#""theta": { "field": "c" }"#, ok_color))[0].0,
        "/layers/0/encoding/theta/field"
    );
    assert_eq!(
        issues(&enc(r#""theta": { "field": "n" }"#, ""))[0].0,
        "/layers/0/encoding/color"
    );
    assert_eq!(
        issues(&enc(
            r##""theta": { "field": "n" }"##,
            r##""color": { "value": "#ff0000" }"##
        ))[0]
            .0,
        "/layers/0/encoding/color"
    );
    let got = issues(&enc(
        r#""theta": { "field": "n" }, "x": { "field": "c" }"#,
        ok_color,
    ));
    assert_eq!(got[0].0, "/layers/0/encoding/x");
    assert!(got[0].1.contains("no `x` channel"), "{}", got[0].1);
}

#[test]
fn bars_and_lines_without_x_or_y_say_so_with_a_path() {
    let json = r#"{
      "data": { "t": { "c": ["a"], "v": [1] } },
      "axes": [ { "id": "x", "orient": "bottom" }, { "id": "y", "orient": "left" } ],
      "layers": [ { "mark": "bar", "data": "t", "encoding": { "x": { "field": "c" } } },
                  { "mark": "line", "data": "t", "encoding": { "y": { "field": "v" } } } ] }"#;
    let got: Vec<_> = issues(json).into_iter().map(|i| i.0).collect();
    assert_eq!(got, ["/layers/0/encoding/y", "/layers/1/encoding/x"]);
}

#[test]
fn style_numbers_must_make_a_pie() {
    let bad = |style: &str| {
        issues(&pie_json(
            "2d",
            TABLE,
            &format!(r#", "style": {style}"#),
            "",
        ))
    };
    assert_eq!(
        bad(r#"{ "innerRadius": 1 }"#)[0].0,
        "/layers/0/style/innerRadius"
    );
    assert_eq!(
        bad(r#"{ "innerRadius": -0.1 }"#)[0].0,
        "/layers/0/style/innerRadius"
    );
    assert_eq!(
        bad(r#"{ "padAngle": -1 }"#)[0].0,
        "/layers/0/style/padAngle"
    );
    assert_eq!(bad(r#"{ "depth": 0.3 }"#)[0].0, "/layers/0/style/depth");
    assert!(bad(r#"{ "depth": 0.3 }"#)[0].1.contains("3d"));
    assert_eq!(
        bad(r#"{ "strokeWidth": 2 }"#)[0].0,
        "/layers/0/style/strokeWidth"
    );
    assert_eq!(
        bad(r#"{ "cornerRadius": 2 }"#)[0].0,
        "/layers/0/style/cornerRadius"
    );
    // And an arc's options on a bar layer.
    let json = r#"{ "data": { "t": { "c": ["a"], "v": [1] } },
      "axes": [ { "id": "x", "orient": "bottom" }, { "id": "y", "orient": "left" } ],
      "layers": [ { "mark": "bar", "data": "t", "style": { "innerRadius": 0.5 },
        "encoding": { "x": { "field": "c" }, "y": { "field": "v" } } } ] }"#;
    assert_eq!(issues(json)[0].0, "/layers/0/style/innerRadius");
}

#[test]
fn percent_labels_are_for_arcs_and_stacked_bars() {
    let json = r#"{ "data": { "t": { "c": ["a"], "v": [1] } },
      "axes": [ { "id": "x", "orient": "bottom" }, { "id": "y", "orient": "left" } ],
      "layers": [ { "mark": "bar", "data": "t", "label": { "show": "always", "format": { "percent": true } },
        "encoding": { "x": { "field": "c" }, "y": { "field": "v" } } } ] }"#;
    assert_eq!(issues(json)[0].0, "/layers/0/label/format/percent");
}

#[test]
fn a_3d_pie_with_axes_or_depth_axis_is_reported_too() {
    let json = r#"{ "dimension": "3d", "data": { "t": { "c": ["a"], "v": [1] } },
      "axes": [ { "id": "z", "orient": "depth" } ],
      "layers": [ { "mark": "arc", "data": "t",
        "encoding": { "theta": { "field": "v" }, "color": { "field": "c" } } } ] }"#;
    assert_eq!(issues(json)[0].0, "/axes/0");
}
