//! The legend: which entries, where, wrapping, truncation, and the room it takes (2D and 3D).
#![cfg(not(target_arch = "wasm32"))]

use facetry_core::scene::{RuleKind, Scene};
use facetry_core::scene3d::Scene3d;
use facetry_core::{build_scene, build_scene_3d, parse_spec_json, validate};
use facetry_font::FontMetrics;

fn spec_json(legend: &str, categories: &[&str], mark: &str) -> String {
    let c: Vec<String> = categories.iter().map(|c| format!("\"{c}\"")).collect();
    let v: Vec<String> = (1..=categories.len())
        .map(|i| (i * 10).to_string())
        .collect();
    format!(
        r#"{{
  {legend}
  "data": {{ "t": {{ "c": [{}], "v": [{}] }} }},
  "axes": [ {{ "id": "x", "orient": "bottom", "title": "X axis" }}, {{ "id": "y", "orient": "left", "title": "Y axis" }} ],
  "layers": [ {{ "mark": "{mark}", "data": "t",
    "encoding": {{ "x": {{ "field": "c" }}, "y": {{ "field": "v" }}, "color": {{ "field": "c" }} }} }} ] }}"#,
        c.join(","),
        v.join(",")
    )
}

fn build(json: &str, size: (u32, u32)) -> Scene {
    let spec = parse_spec_json(json).unwrap_or_else(|e| panic!("{e:?}\n{json}"));
    assert_eq!(validate(&spec), vec![], "{json}");
    build_scene(&spec, size, 1.0, FontMetrics::inter())
}

fn swatches(sc: &Scene) -> Vec<&facetry_core::scene::Rule> {
    sc.rules
        .iter()
        .filter(|r| r.kind == RuleKind::Swatch)
        .collect()
}

fn legend_texts(sc: &Scene) -> Vec<&facetry_core::scene::Text> {
    sc.texts
        .iter()
        .filter(|t| t.key.starts_with("legend/"))
        .collect()
}

/// `[x0, y0, x1, y1]` of everything the legend draws.
fn legend_box(sc: &Scene) -> [f32; 4] {
    let font = FontMetrics::inter();
    let mut b = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
    for r in swatches(sc) {
        b = [
            b[0].min(r.bounds.x),
            b[1].min(r.bounds.y),
            b[2].max(r.bounds.x + r.bounds.w),
            b[3].max(r.bounds.y + r.bounds.h),
        ];
    }
    for t in legend_texts(sc) {
        let w = font.text_width(&t.text, t.size_px);
        b = [
            b[0].min(t.x),
            b[1].min(t.y - 9.0),
            b[2].max(t.x + w),
            b[3].max(t.y + 3.0),
        ];
    }
    b
}

#[test]
fn a_colour_field_gets_a_legend_on_the_right_by_default() {
    let sc = build(
        &spec_json("", &["Apples", "Pears", "Plums"], "bar"),
        (800, 500),
    );
    let sw = swatches(&sc);
    assert_eq!(sw.len(), 3);
    assert_eq!(
        sw.iter().map(|r| r.key.as_str()).collect::<Vec<_>>(),
        [
            "legend/swatch:Apples",
            "legend/swatch:Pears",
            "legend/swatch:Plums"
        ]
    );
    // Right of the plot, in one column, evenly spaced, each swatch next to its label.
    let axis = sc.rules.iter().find(|r| r.key == "axis:x/line").unwrap();
    assert!(
        sw[0].bounds.x > axis.bounds.x + axis.bounds.w,
        "the plot ends before the legend"
    );
    assert!(sw.iter().all(|r| r.bounds.x == sw[0].bounds.x));
    let pitch = sw[1].bounds.y - sw[0].bounds.y;
    assert!(pitch > 10.0 && (sw[2].bounds.y - sw[1].bounds.y - pitch).abs() < 1.0);
    let labels: Vec<_> = legend_texts(&sc);
    assert_eq!(labels.len(), 3);
    for (r, t) in sw.iter().zip(&labels) {
        assert!(t.x > r.bounds.x + r.bounds.w);
        assert!(
            (t.y - (r.bounds.y + r.bounds.h / 2.0)).abs() <= 6.0,
            "{} vs {}",
            t.y,
            r.bounds.y
        );
    }
    // Colours are the marks' colours.
    assert_eq!(sw[0].color, sc.rects[0].color);
    assert_eq!(sw[2].color, sc.rects[2].color);
    // Everything inside the canvas, with padding.
    let b = legend_box(&sc);
    assert!(
        b[0] > 0.0 && b[2] < 800.0 && b[1] > 0.0 && b[3] < 500.0,
        "{b:?}"
    );
}

#[test]
fn no_colour_field_no_legend_and_false_turns_it_off() {
    let json = r##"{ "data": { "t": { "c": ["a"], "v": [1] } },
      "axes": [ { "id": "x", "orient": "bottom" }, { "id": "y", "orient": "left" } ],
      "layers": [ { "mark": "bar", "data": "t", "encoding": { "x": { "field": "c" }, "y": { "field": "v" },
        "color": { "value": "#336699" } } } ] }"##;
    assert!(swatches(&build(json, (800, 500))).is_empty());
    let sc = build(
        &spec_json(r#""legend": false,"#, &["a", "b"], "bar"),
        (800, 500),
    );
    assert!(swatches(&sc).is_empty() && legend_texts(&sc).is_empty());
    // `true` is the default.
    assert_eq!(
        swatches(&build(
            &spec_json(r#""legend": true,"#, &["a", "b"], "bar"),
            (800, 500)
        ))
        .len(),
        2
    );
    // The plot gets the room back without a legend.
    let with = build(&spec_json("", &["a", "b"], "bar"), (800, 500));
    let rule = |sc: &Scene| {
        sc.rules
            .iter()
            .find(|r| r.key == "axis:x/line")
            .unwrap()
            .bounds
            .w
    };
    assert!(rule(&sc) > rule(&with));
}

#[test]
fn every_orientation_leaves_the_plot_clear_of_the_legend() {
    for (orient, size) in [
        ("right", (800, 500)),
        ("left", (800, 500)),
        ("top", (800, 500)),
        ("bottom", (800, 500)),
    ] {
        let json = spec_json(
            &format!(r#""legend": {{ "orient": "{orient}", "title": "Fruit" }},"#),
            &["Apples", "Pears", "Plums", "Quinces"],
            "bar",
        );
        let sc = build(&json, size);
        let b = legend_box(&sc);
        assert!(
            b[0] >= 0.0 && b[1] >= 0.0 && b[2] <= 800.0 && b[3] <= 500.0,
            "{orient}: {b:?}"
        );
        // Everything of the chart that is not the legend: bars, axis lines, ticks, text.
        let rects = sc.rects.iter().map(|r| r.bounds);
        let rules = sc
            .rules
            .iter()
            .filter(|r| r.kind != RuleKind::Swatch)
            .map(|r| r.bounds);
        for bounds in rects.chain(rules) {
            let (x1, y1) = (bounds.x + bounds.w, bounds.y + bounds.h);
            let overlap = bounds.x < b[2] && b[0] < x1 && bounds.y < b[3] && b[1] < y1;
            assert!(
                !overlap,
                "{orient}: plot item {bounds:?} under the legend {b:?}"
            );
        }
        for t in sc
            .texts
            .iter()
            .filter(|t| !t.key.starts_with("legend/") && !t.key.contains("/title"))
        {
            let w = FontMetrics::inter().text_width(&t.text, t.size_px);
            let (x0, y0, x1, y1) = (
                t.x - w / 2.0 - 1.0,
                t.y - 9.0,
                t.x + w / 2.0 + 1.0,
                t.y + 3.0,
            );
            let hit = x0 < b[2] && b[0] < x1 && y0 < b[3] && b[1] < y1;
            assert!(!hit, "{orient}: {} under the legend", t.key);
        }
        match orient {
            "right" => assert!(b[0] > 600.0),
            "left" => assert!(b[2] < 200.0),
            "top" => assert!(b[3] < 80.0),
            _ => assert!(b[1] > 440.0),
        }
        // The title is there.
        assert!(
            sc.texts
                .iter()
                .any(|t| t.key == "legend/title" && t.text == "Fruit"),
            "{orient}"
        );
    }
}

#[test]
fn top_and_bottom_legends_are_rows_that_wrap_and_side_legends_wrap_into_columns() {
    let many: Vec<String> = (0..24).map(|i| format!("Category {i}")).collect();
    let names: Vec<&str> = many.iter().map(String::as_str).collect();
    // A short chart: the right legend needs more than one column.
    let sc = build(&spec_json("", &names, "bar"), (800, 300));
    let xs: std::collections::BTreeSet<i32> =
        swatches(&sc).iter().map(|r| r.bounds.x as i32).collect();
    assert!(xs.len() > 1, "columns: {xs:?}");
    let b = legend_box(&sc);
    assert!(b[3] <= 300.0 && b[2] <= 800.0, "{b:?}");
    // A narrow chart: the bottom legend wraps into rows.
    let json = spec_json(r#""legend": { "orient": "bottom" },"#, &names, "bar");
    let sc = build(&json, (500, 500));
    let ys: std::collections::BTreeSet<i32> =
        swatches(&sc).iter().map(|r| r.bounds.y as i32).collect();
    assert!(ys.len() > 1, "rows: {ys:?}");
    let b = legend_box(&sc);
    assert!(b[0] >= 0.0 && b[2] <= 500.0 && b[3] <= 500.0, "{b:?}");
    // And all 24 entries are there.
    assert_eq!(swatches(&sc).len(), 24);
}

#[test]
fn long_labels_are_cut_with_an_ellipsis() {
    let long = "A very long category name that will never fit in a legend column at all";
    let sc = build(&spec_json("", &[long, "Short"], "bar"), (800, 500));
    let texts = legend_texts(&sc);
    let cut = texts
        .iter()
        .find(|t| t.key.contains("A very long"))
        .unwrap();
    assert!(
        cut.text.ends_with("...") && cut.text.len() < long.len(),
        "{}",
        cut.text
    );
    let w = FontMetrics::inter().text_width(&cut.text, cut.size_px);
    assert!(w <= 150.0 + 0.5, "{w}");
    assert_eq!(
        texts.iter().find(|t| t.key.contains("Short")).unwrap().text,
        "Short"
    );
}

#[test]
fn categories_shared_by_layers_appear_once_and_line_layers_get_line_swatches() {
    let json = r#"{
      "data": { "t": { "c": ["a", "b"], "s": ["x", "y"], "v": [1, 2] } },
      "axes": [ { "id": "x", "orient": "bottom" }, { "id": "y", "orient": "left" } ],
      "layers": [
        { "mark": "bar", "data": "t", "encoding": { "x": { "field": "c" }, "y": { "field": "v" }, "color": { "field": "s" } } },
        { "mark": "line", "data": "t", "style": { "strokeWidth": 3 },
          "encoding": { "x": { "field": "c" }, "y": { "field": "v" }, "color": { "field": "s" } } } ] }"#;
    let sc = build(json, (800, 500));
    let sw = swatches(&sc);
    assert_eq!(sw.len(), 2, "x and y once each");
    // The bar layer comes first, so its square wins.
    assert!(sw.iter().all(|r| r.bounds.w == r.bounds.h));
    // A line-only chart: line swatches, as wide as they are thin.
    let sc = build(&spec_json("", &["a", "b"], "line"), (800, 500));
    let sw = swatches(&sc);
    assert!(
        sw.iter().all(|r| r.bounds.w > 12.0 && r.bounds.h <= 4.0),
        "{:?}",
        sw[0].bounds
    );
    assert_eq!(sw[0].color, sc.polylines[0].color);
}

#[test]
fn a_pie_is_centred_in_what_the_legend_leaves() {
    let json = std::fs::read_to_string(format!(
        "{}/../../examples/specs/pie.json",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap();
    let sc = build(&json, (800, 500));
    assert_eq!(swatches(&sc).len(), 6);
    let (a, b) = (&sc.arcs[0], legend_box(&sc));
    assert!(
        a.center[0] + a.outer <= b[0],
        "the pie ends before the legend"
    );
    let hidden = build(
        &json.replace(
            r#""dimension": "2d","#,
            r#""dimension": "2d", "legend": false,"#,
        ),
        (800, 500),
    );
    assert_eq!(hidden.arcs[0].center, [400.0, 250.0]);
    assert!(a.center[0] < 400.0 && a.outer <= hidden.arcs[0].outer);
}

#[test]
fn the_legend_is_in_device_pixels_at_any_ratio() {
    let spec = parse_spec_json(&spec_json("", &["a", "b"], "bar")).unwrap();
    let one = build_scene(&spec, (800, 500), 1.0, FontMetrics::inter());
    let two = build_scene(&spec, (1600, 1000), 2.0, FontMetrics::inter());
    let (s1, s2) = (&swatches(&one)[0].bounds, &swatches(&two)[0].bounds);
    assert_eq!((s2.w, s2.h), (2.0 * s1.w, 2.0 * s1.h));
    assert!((s2.x - 2.0 * s1.x).abs() <= 2.0);
}

fn build3d(json: &str, size: (u32, u32)) -> Scene3d {
    let spec = parse_spec_json(json).unwrap();
    assert_eq!(validate(&spec), vec![], "{json}");
    build_scene_3d(&spec, size, 1.0, FontMetrics::inter())
}

#[test]
fn in_3d_the_legend_is_an_overlay_and_the_plot_is_framed_beside_it() {
    let base = std::fs::read_to_string(format!(
        "{}/../../examples/specs/bar3d-grid.json",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap();
    let plain = build3d(
        &base.replace(
            r#""dimension": "3d","#,
            r#""dimension": "3d", "legend": false,"#,
        ),
        (800, 500),
    );
    assert!(plain.plot.is_none() && plain.overlay.is_empty());
    assert_eq!(plain.plot_rect(), [0.0, 0.0, 800.0, 500.0]);
    for (orient, check) in [("right", 0), ("left", 1), ("top", 2), ("bottom", 3)] {
        let json = base.replace(
            r#""dimension": "3d","#,
            &format!(r#""dimension": "3d", "legend": {{ "orient": "{orient}" }},"#),
        );
        let sc = build3d(&json, (800, 500));
        let [x, y, w, h] = sc.plot_rect();
        assert!(w < 800.0 || h < 500.0, "{orient}");
        assert!(
            x >= 0.0 && y >= 0.0 && x + w <= 800.5 && y + h <= 500.5,
            "{orient}: {:?}",
            sc.plot_rect()
        );
        assert_eq!(sc.overlay.rules.len(), 3, "North, South, West");
        // No swatch or label inside the plot's rectangle.
        for r in &sc.overlay.rules {
            let b = r.bounds;
            assert!(
                b.x + b.w <= x + 0.5
                    || b.x >= x + w - 0.5
                    || b.y + b.h <= y + 0.5
                    || b.y >= y + h - 0.5,
                "{orient}: swatch {b:?} inside the plot {:?}",
                sc.plot_rect()
            );
        }
        match check {
            0 => assert!(x == 0.0 && w < 800.0),
            1 => assert!(x > 0.0 && x + w >= 799.5),
            2 => assert!(y > 0.0 && y + h >= 499.5),
            _ => assert!(y == 0.0 && h < 500.0),
        }
    }
}

#[test]
fn a_3d_pie_gets_a_legend_too() {
    let json = std::fs::read_to_string(format!(
        "{}/../../examples/specs/pie3d.json",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap();
    let sc = build3d(&json, (800, 500));
    assert_eq!(sc.overlay.rules.len(), 6);
    assert!(sc.plot.is_some());
    assert!(sc.overlay.texts.iter().any(|t| t.text == "Search"));
}

#[test]
fn tiny_canvases_do_not_panic_with_a_legend() {
    for orient in ["right", "left", "top", "bottom"] {
        let json = spec_json(
            &format!(r#""legend": {{ "orient": "{orient}", "title": "T" }},"#),
            &["Apples", "Pears", "Plums"],
            "bar",
        );
        for size in [(0, 0), (1, 1), (20, 20), (50, 400), (400, 50)] {
            let _ = build(&json, size);
            let spec = parse_spec_json(&json).unwrap();
            let sc = build_scene_3d(&spec, size, 1.0, FontMetrics::inter());
            assert!(sc.plot_rect()[2] >= 1.0 && sc.plot_rect()[3] >= 1.0);
        }
    }
}

#[test]
fn bad_legend_options_are_reported_with_a_path() {
    let bad = |legend: &str| match parse_spec_json(&spec_json(legend, &["a"], "bar")) {
        Ok(s) => validate(&s).into_iter().map(|i| i.path).collect::<Vec<_>>(),
        Err(e) => e.into_iter().map(|i| i.path).collect(),
    };
    assert_eq!(bad(r#""legend": { "orient": "middle" },"#).len(), 1);
    assert_eq!(bad(r#""legend": { "colour": 1 },"#).len(), 1);
    assert_eq!(bad(r#""legend": "yes","#).len(), 1);
}
