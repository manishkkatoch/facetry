//! Value labels and picking: the scene-level behaviour that hover builds on.
#![cfg(not(target_arch = "wasm32"))]

use facetry_core::scene::{Anchor, Scene};
use facetry_core::scene3d::{LabelRole, Scene3d, VAlign};
use facetry_core::spec::LabelShow;
use facetry_core::{build_scene, build_scene_3d, parse_spec_json, validate};
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

fn scene_3d(name: &str) -> Scene3d {
    let spec = parse_spec_json(&load(name)).expect("parses");
    assert_eq!(validate(&spec), vec![]);
    build_scene_3d(&spec, (800, 500), 1.0, FontMetrics::inter())
}

// --- value labels, 2D ----------------------------------------------------------------------

#[test]
fn always_labels_sit_centred_above_bars_and_below_negative_ones() {
    let scene = scene_2d(&load("bar-labels"), (800, 500), 1.0);
    assert_eq!(scene.labels.len(), 5);
    assert!(scene.labels.iter().all(|l| l.show == LabelShow::Always));
    let by_key = |k: &str| scene.labels.iter().find(|l| l.text.key == k).unwrap();
    let rect_of = |l: &facetry_core::scene::MarkLabel| &scene.rects[l.mark];

    let q1 = by_key("layer0/Q1/label");
    assert_eq!(q1.text.text, "1,200");
    let r = rect_of(q1);
    assert_eq!(r.key, "layer0/Q1");
    assert_eq!(q1.text.anchor, Anchor::Middle);
    assert!((q1.text.x - (r.bounds.x + r.bounds.w / 2.0)).abs() <= 1.0);
    assert!(q1.text.y < r.bounds.y, "baseline above the bar's top");
    assert_eq!(q1.text.color, facetry_core::Theme::default().text);

    let q5 = by_key("layer0/Q5/label");
    assert_eq!(q5.text.text, "-300");
    let r = rect_of(q5);
    assert!(
        q5.text.y - FontMetrics::inter().cap_height * q5.text.size_px > r.bounds.y + r.bounds.h,
        "text top below the bar's bottom"
    );
}

#[test]
fn a_label_that_would_leave_the_plot_goes_inside_the_bar_in_a_contrasting_colour() {
    let scene = scene_2d(&load("bar-labels"), (800, 500), 1.0);
    // Q3 (1,400) reaches the top of the plot: no room above.
    let q3 = scene
        .labels
        .iter()
        .find(|l| l.text.key == "layer0/Q3/label")
        .unwrap();
    let bar = &scene.rects[q3.mark];
    assert!(q3.text.y > bar.bounds.y, "baseline inside the bar");
    assert!(q3.text.y < bar.bounds.y + bar.bounds.h);
    // Blue bar, so white (the theme background) reads better than the dark text colour.
    assert_eq!(q3.text.color, facetry_core::Theme::default().background);
}

#[test]
fn labels_use_decimals_from_the_spec_and_the_mode() {
    let json = load("bar-basic").replace(
        r#""key": { "field": "quarter" }
      }"#,
        r#""key": { "field": "quarter" }
      },
      "label": { "show": "hover", "format": { "decimals": 1 } }"#,
    );
    let scene = scene_2d(&json, (800, 500), 1.0);
    assert!(scene.labels.iter().all(|l| l.show == LabelShow::Hover));
    assert_eq!(scene.labels[0].text.text, "120.0");
}

#[test]
fn layers_without_a_label_block_get_no_labels() {
    assert!(
        scene_2d(&load("bar-basic"), (800, 500), 1.0)
            .labels
            .is_empty()
    );
}

#[test]
fn picking_finds_the_bar_under_a_pixel_and_nothing_between_bars() {
    let scene = scene_2d(&load("bar-basic"), (800, 500), 1.0);
    for (i, r) in scene.rects.iter().enumerate() {
        let b = r.bounds;
        assert_eq!(scene.pick(b.x + b.w / 2.0, b.y + b.h / 2.0), Some(i));
        // The very corners: top-left is in, just outside is out.
        assert_eq!(scene.pick(b.x, b.y), Some(i));
        assert_eq!(scene.pick(b.x - 0.5, b.y), None);
        assert_eq!(scene.pick(b.x + b.w / 2.0, b.y - 0.5), None);
    }
    // Above a short bar, inside its column: nothing.
    let q2 = scene.rects[1].bounds;
    assert_eq!(scene.pick(q2.x + 5.0, q2.y - 10.0), None);
}

#[test]
fn the_topmost_of_overlapping_bars_is_picked() {
    // Two layers on the same axes draw over each other; the later layer is on top.
    let json = r#"{
      "data": { "a": { "c": ["x"], "v": [5] }, "b": { "c": ["x"], "v": [3] } },
      "axes": [ { "id": "x", "orient": "bottom" }, { "id": "y", "orient": "left" } ],
      "layers": [
        { "mark": "bar", "data": "a", "encoding": { "x": { "field": "c" }, "y": { "field": "v" } } },
        { "mark": "bar", "data": "b", "encoding": { "x": { "field": "c" }, "y": { "field": "v" } } }
      ]
    }"#;
    let scene = scene_2d(json, (800, 500), 1.0);
    let low = scene.rects[1].bounds; // the shorter bar, drawn second
    assert_eq!(scene.pick(low.x + 2.0, low.y + low.h - 2.0), Some(1));
    // Above it only the taller bar of layer 0 is there.
    assert_eq!(scene.pick(low.x + 2.0, low.y - 5.0), Some(0));
    assert_eq!((scene.rects[0].layer, scene.rects[1].layer), (0, 1));
    assert_eq!((scene.rects[0].row, scene.rects[1].row), (0, 0));
}

// --- value labels, 3D ----------------------------------------------------------------------

#[test]
fn hover_labels_in_3d_hang_above_the_top_face_centre() {
    let scene = scene_3d("bar3d-labels");
    assert_eq!(scene.labels.len(), 12);
    for l in &scene.labels {
        assert_eq!(l.show, LabelShow::Hover);
        let b = &scene.boxes[l.mark];
        assert_eq!(l.text.key, format!("{}/label", b.key));
        assert_eq!(l.text.anchor[1], b.max[1]);
        assert!((l.text.anchor[0] - (b.min[0] + b.max[0]) / 2.0).abs() < 1e-6);
        assert!((l.text.anchor[2] - (b.min[2] + b.max[2]) / 2.0).abs() < 1e-6);
        assert_eq!(
            (l.text.align, l.text.valign),
            (Anchor::Middle, VAlign::Baseline)
        );
        assert!(l.text.offset_px[1] < 0.0, "nudged up");
        assert_eq!(l.text.role, LabelRole::Fixed);
        assert!(l.text.azimuths.is_none());
    }
    let q3 = scene
        .labels
        .iter()
        .find(|l| l.text.key == "layer0/Q3/North/label")
        .unwrap();
    assert_eq!(q3.text.text, "140");
}

#[test]
fn negative_bars_get_their_3d_label_below_the_bottom_face() {
    let mut spec: serde_json::Value = serde_json::from_str(&load("bar3d-negative")).unwrap();
    spec["layers"][0]["label"] = serde_json::json!({ "show": "always" });
    let spec = parse_spec_json(&spec.to_string()).unwrap();
    assert_eq!(validate(&spec), vec![]);
    let scene = build_scene_3d(&spec, (800, 500), 1.0, FontMetrics::inter());
    let feb = scene
        .labels
        .iter()
        .find(|l| l.text.key == "layer0/Feb/Product A/label")
        .unwrap();
    let b = &scene.boxes[feb.mark];
    assert_eq!(feb.text.anchor[1], b.min[1]);
    assert_eq!(feb.text.valign, VAlign::Top);
    assert!(feb.text.offset_px[1] > 0.0, "nudged down");
    assert!(feb.text.text.starts_with('-'));
}
