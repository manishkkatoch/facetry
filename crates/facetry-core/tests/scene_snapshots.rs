//! Scene snapshots for every example spec, laid out with the real font metrics.
//!
//! The spec files in `examples/specs/` are shared with the playground and the browser golden
//! test. To accept intentional changes: `cargo insta review` (or `INSTA_UPDATE=always cargo test`).
#![cfg(not(target_arch = "wasm32"))]

use facetry_core::{build_scene, parse_spec_json, validate};
use facetry_font::FontMetrics;

fn scene_for(name: &str, viewport: (u32, u32), dpr: f32) -> facetry_core::Scene {
    let path = format!(
        "{}/../../examples/specs/{name}.json",
        env!("CARGO_MANIFEST_DIR")
    );
    let json = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
    let spec = parse_spec_json(&json).unwrap_or_else(|e| panic!("{name} does not parse: {e:?}"));
    assert_eq!(validate(&spec), vec![], "{name} must be valid");
    build_scene(&spec, viewport, dpr, FontMetrics::inter())
}

macro_rules! snapshot {
    ($test:ident, $spec:literal) => {
        #[test]
        fn $test() {
            insta::assert_json_snapshot!(scene_for($spec, (800, 500), 1.0));
        }
    };
}

snapshot!(basic_bars, "bar-basic");
snapshot!(negative_values, "bar-negative");
snapshot!(field_coloured_bars, "bar-colored");
snapshot!(right_axis_layer, "bar-right-axis");
snapshot!(empty_data, "bar-empty");
snapshot!(many_categories_thin_labels, "bar-many");

#[test]
fn basic_bars_at_dpr_2() {
    insta::assert_json_snapshot!(scene_for("bar-basic", (1600, 1000), 2.0));
}

/// The structural properties the snapshots are really protecting.
#[test]
fn basic_bars_sit_on_the_axis_and_inside_the_plot() {
    let scene = scene_for("bar-basic", (800, 500), 1.0);
    assert_eq!(scene.rects.len(), 4);
    let keys: Vec<_> = scene.rects.iter().map(|r| r.key.as_str()).collect();
    assert_eq!(keys, ["layer0/Q1", "layer0/Q2", "layer0/Q3", "layer0/Q4"]);
    let axis = scene.rules.iter().find(|r| r.key == "axis:x/line").unwrap();
    for r in &scene.rects {
        // Positive bars all end exactly where the x axis line starts.
        assert_eq!(r.bounds.y + r.bounds.h, axis.bounds.y, "{}", r.key);
        assert!(
            r.bounds.x >= axis.bounds.x && r.bounds.x + r.bounds.w <= axis.bounds.x + axis.bounds.w
        );
    }
}

#[test]
fn right_axis_layer_uses_its_own_domain() {
    let scene = scene_for("bar-right-axis", (800, 500), 1.0);
    // Revenue (up to 140) and margin (up to 0.21) differ by 3 orders of magnitude, yet the
    // tallest bar of each layer reaches a similar height because each axis has its own scale.
    let tallest = |layer: &str| {
        scene
            .rects
            .iter()
            .filter(|r| r.key.starts_with(layer))
            .map(|r| r.bounds.h)
            .fold(0.0, f32::max)
    };
    let (a, b) = (tallest("layer0/"), tallest("layer1/"));
    assert!((a - b).abs() / a < 0.3, "{a} vs {b}");
    assert!(scene.texts.iter().any(|t| t.key == "axis:margin/title"));
}

#[test]
fn building_twice_gives_identical_scenes() {
    assert_eq!(
        scene_for("bar-colored", (640, 400), 1.5),
        scene_for("bar-colored", (640, 400), 1.5)
    );
}

#[test]
fn tiny_viewports_do_not_panic() {
    for size in [(0, 0), (1, 1), (20, 20), (50, 400)] {
        let scene = scene_for("bar-right-axis", size, 1.0);
        for r in scene.rects.iter().map(|r| r.bounds) {
            assert!(r.w >= 0.0 && r.h >= 0.0);
        }
    }
}
