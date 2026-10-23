//! The room the 2D layout leaves around the plot: above the top tick label, and right of the
//! last x label.
#![cfg(not(target_arch = "wasm32"))]

use facetry_core::scene::Scene;
use facetry_core::{build_scene, parse_spec_json, validate};
use facetry_font::FontMetrics;

fn load(name: &str) -> String {
    let path = format!(
        "{}/../../examples/specs/{name}.json",
        env!("CARGO_MANIFEST_DIR")
    );
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"))
}

fn scene_2d(json: &str, viewport: (u32, u32), dpr: f32) -> Scene {
    let spec = parse_spec_json(json).expect("parses");
    assert_eq!(validate(&spec), vec![]);
    build_scene(&spec, viewport, dpr, FontMetrics::inter())
}

#[test]
fn the_top_tick_label_and_tallest_bar_keep_clear_of_the_canvas_top() {
    let scene = scene_2d(&load("bar-basic"), (800, 500), 1.0);
    let m = FontMetrics::inter();
    let top_label = scene
        .texts
        .iter()
        .filter(|t| t.key.starts_with("axis:y/label:"))
        .min_by(|a, b| a.y.total_cmp(&b.y))
        .unwrap();
    assert_eq!(top_label.text, "140");
    // The label's glyph tops: at least the padding (12px) away from the edge, with the old
    // layout they were 8px away.
    let glyph_top = top_label.y - m.cap_height * top_label.size_px;
    assert!(glyph_top >= 12.0, "glyph top at {glyph_top}");
    let tallest = scene
        .rects
        .iter()
        .map(|r| r.bounds.y)
        .fold(f32::MAX, f32::min);
    assert!(tallest >= 12.0 + 7.0, "tallest bar starts at {tallest}");
}

#[test]
fn a_long_last_x_label_narrows_the_plot_instead_of_leaving_the_canvas() {
    let json = r#"{
      "data": { "t": { "c": ["a", "b", "A very long final category name"], "v": [1, 2, 3] } },
      "axes": [ { "id": "x", "orient": "bottom" }, { "id": "y", "orient": "left" } ],
      "layers": [ { "mark": "bar", "data": "t",
        "encoding": { "x": { "field": "c", "axis": "x" }, "y": { "field": "v", "axis": "y" } } } ]
    }"#;
    let scene = scene_2d(json, (300, 200), 1.0);
    let m = FontMetrics::inter();
    for t in scene
        .texts
        .iter()
        .filter(|t| t.key.starts_with("axis:x/label:"))
    {
        let half = m.text_width(&t.text, t.size_px) / 2.0;
        assert!(
            t.x + half <= 300.0,
            "{} sticks out to {}",
            t.text,
            t.x + half
        );
    }
}
