//! Scene snapshots for every example spec, laid out with the real font metrics.
//!
//! The spec files in `examples/specs/` are shared with the playground and the browser golden
//! test. To accept intentional changes: `cargo insta review` (or `INSTA_UPDATE=always cargo test`).
#![cfg(not(target_arch = "wasm32"))]

use facetry_core::scene3d::Scene3d;
use facetry_core::{build_scene, build_scene_3d, parse_spec_json, validate};
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
snapshot!(value_labels_always, "bar-labels");
snapshot!(stacked_bars, "bar-stacked");
snapshot!(stacked_bars_with_a_series_hidden, "bar-stacked-hidden");
snapshot!(normalized_bars, "bar-normalized");
snapshot!(grouped_bars, "bar-grouped");
snapshot!(basic_lines, "line-basic");
snapshot!(numeric_x_lines, "line-numeric-x");
snapshot!(combo_bar_line, "combo-bar-line");
snapshot!(pie_with_percent_labels, "pie");
snapshot!(donut_with_padding, "donut");

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

// --- 3D ----------------------------------------------------------------------------------

fn scene3d_for(name: &str) -> Scene3d {
    let path = format!(
        "{}/../../examples/specs/{name}.json",
        env!("CARGO_MANIFEST_DIR")
    );
    let json = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
    let spec = parse_spec_json(&json).unwrap_or_else(|e| panic!("{name} does not parse: {e:?}"));
    assert_eq!(validate(&spec), vec![], "{name} must be valid");
    build_scene_3d(&spec, (800, 500), 1.0, FontMetrics::inter())
}

macro_rules! snapshot3d {
    ($test:ident, $spec:literal) => {
        #[test]
        fn $test() {
            insta::assert_json_snapshot!(scene3d_for($spec));
        }
    };
}

snapshot3d!(grid_3d, "bar3d-grid");
snapshot3d!(single_row_3d_without_z, "bar3d-row");
snapshot3d!(negative_values_3d, "bar3d-negative");
snapshot3d!(value_labels_hover_3d, "bar3d-labels");
snapshot3d!(stacked_3d, "bar3d-stacked");
snapshot3d!(pie_3d, "pie3d");
snapshot3d!(lines_3d, "line3d");
snapshot3d!(combo_bars_and_lines_3d, "combo3d-bar-line");

#[test]
fn grid_bars_are_square_boxes_in_the_right_cells() {
    let scene = scene3d_for("bar3d-grid");
    assert_eq!(scene.boxes.len(), 12);
    // 4 x 3 cells of equal size: the box is 1.2 wide and 0.9 deep, 1 high.
    assert_eq!(scene.size, [1.2, 1.0, 0.9]);
    for b in &scene.boxes {
        let (w, d) = (b.max[0] - b.min[0], b.max[2] - b.min[2]);
        assert!((w - d).abs() < 1e-5, "{}: footprint {w} x {d}", b.key);
        assert!(
            b.min[1] == 0.0 && b.max[1] > 0.0 && b.max[1] <= 1.0,
            "{}",
            b.key
        );
        assert!(b.min[0] > 0.0 && b.max[0] < scene.size[0], "{}", b.key);
        assert!(b.min[2] > 0.0 && b.max[2] < scene.size[2], "{}", b.key);
    }
    // The first region (North) is the front row, so it has the largest z.
    let z = |key: &str| scene.boxes.iter().find(|b| b.key == key).unwrap().min[2];
    assert!(z("layer0/Q1/North") > z("layer0/Q1/South"));
    assert!(z("layer0/Q1/South") > z("layer0/Q1/West"));
}

#[test]
fn without_z_the_bars_form_one_row_and_no_depth_axis_is_drawn() {
    let scene = scene3d_for("bar3d-row");
    assert_eq!(scene.boxes.len(), 4);
    let z0 = scene.boxes[0].min[2];
    assert!(
        scene
            .boxes
            .iter()
            .all(|b| b.min[2] == z0 && b.max[2] == scene.boxes[0].max[2])
    );
    assert!(scene.lines.iter().all(|l| !l.key.starts_with("axis:z/")));
}

#[test]
fn negative_bars_hang_below_the_zero_line() {
    let scene = scene3d_for("bar3d-negative");
    let zero_y = scene
        .boxes
        .iter()
        .find(|b| b.key == "layer0/Jan/Product A")
        .unwrap()
        .min[1];
    let feb = scene
        .boxes
        .iter()
        .find(|b| b.key == "layer0/Feb/Product A")
        .unwrap();
    assert_eq!(feb.max[1], zero_y);
    assert!(feb.min[1] < zero_y && feb.min[1] >= 0.0);
    // Zero is drawn in the axis colour on the back walls.
    assert!(scene.lines.iter().any(|l| l.key.contains("wall:")
        && l.key.ends_with("/y:0")
        && l.kind == facetry_core::scene::RuleKind::Axis));
}

#[test]
fn exactly_one_variant_of_each_axis_is_drawn_at_any_azimuth() {
    // Replays what the shaders do: keep the items whose azimuth range holds for the camera.
    let scene = scene3d_for("bar3d-grid");
    let visible =
        |l: &Option<facetry_core::scene3d::AzimuthRange>, az: f32| l.is_none_or(|r| r.contains(az));
    for tenth in 0..3600 {
        let az = tenth as f32 / 10.0;
        for axis in ["x", "y", "z"] {
            let prefix = format!("axis:{axis}/line@");
            let n = scene
                .lines
                .iter()
                .filter(|l| l.key.starts_with(&prefix) && visible(&l.azimuths, az))
                .count();
            assert_eq!(n, 1, "axis {axis} at azimuth {az}");
            let labels = |t: &str| {
                scene
                    .texts
                    .iter()
                    .filter(|x| {
                        x.key.starts_with(&format!("axis:{axis}/{t}@")) && visible(&x.azimuths, az)
                    })
                    .count()
            };
            // Every tick label and title exists in exactly one variant, too.
            assert_eq!(labels("title"), 1, "{axis} title at {az}");
        }
        // Two of the four back walls (one per direction) are drawn, and they are adjacent
        // ones, never opposite: the walls farthest from the camera.
        let wall = |p: &str| {
            scene
                .lines
                .iter()
                .any(|l| l.key.starts_with(p) && visible(&l.azimuths, az))
        };
        let (x0, x1, z0, z1) = (
            wall("wall:x0/"),
            wall("wall:x1/"),
            wall("wall:z0/"),
            wall("wall:z1/"),
        );
        assert!(x0 != x1 && z0 != z1, "walls at {az}: {x0} {x1} {z0} {z1}");
        // The camera at +x (sin > 0) sees the x = 0 wall from inside; at +z (cos > 0), z = 0.
        let (sin, cos) = az.to_radians().sin_cos();
        if sin.abs() > 1e-3 {
            assert_eq!(x0, sin > 0.0, "x0 wall at {az}");
        }
        if cos.abs() > 1e-3 {
            assert_eq!(z0, cos > 0.0, "z0 wall at {az}");
        }
        // Exactly one wall-corner line (the far corner).
        let corners = scene
            .lines
            .iter()
            .filter(|l| l.key.starts_with("wall:corner:") && visible(&l.azimuths, az))
            .count();
        assert_eq!(corners, 1, "corner at {az}");
    }
}

#[test]
fn building_3d_twice_gives_identical_scenes() {
    assert_eq!(scene3d_for("bar3d-grid"), scene3d_for("bar3d-grid"));
}

#[test]
fn tiny_viewports_do_not_panic_in_3d() {
    let json = std::fs::read_to_string(format!(
        "{}/../../examples/specs/bar3d-grid.json",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap();
    let spec = parse_spec_json(&json).unwrap();
    for size in [(0, 0), (1, 1), (20, 20), (50, 400)] {
        let scene = build_scene_3d(&spec, size, 1.0, FontMetrics::inter());
        assert_eq!(scene.boxes.len(), 12);
    }
}

// A hand-laid-out diagram: prisms of four shapes, rims, edges (one dotted, so in dashes) with
// arrowheads, node and edge labels.
snapshot3d!(diagram_3d, "diagram-basic");

// The same diagram in the glossy style: glossy prisms, ribbons for edges, solid arrowheads,
// label cards, soft shadows and printed labels.
snapshot3d!(diagram_glossy_3d, "diagram-glossy");
