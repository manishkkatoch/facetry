//! 3D axis labels as the scene describes them: which edges carry the axes, which way labels
//! hang, and the roles the renderer uses to thin them per frame (the placement itself is
//! tested in `facetry-render`).
#![cfg(not(target_arch = "wasm32"))]

use facetry_core::scene::Anchor;
use facetry_core::scene3d::{LabelRole, Scene3d, VAlign};
use facetry_core::{build_scene_3d, parse_spec_json, validate};
use facetry_font::FontMetrics;

fn scene_3d(name: &str) -> Scene3d {
    let path = format!(
        "{}/../../examples/specs/{name}.json",
        env!("CARGO_MANIFEST_DIR")
    );
    let json = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
    let spec = parse_spec_json(&json).expect("parses");
    assert_eq!(validate(&spec), vec![]);
    build_scene_3d(&spec, (800, 500), 1.0, FontMetrics::inter())
}

#[test]
fn axis_labels_are_tagged_for_the_renderer_to_thin() {
    let scene = scene_3d("bar3d-grid");
    let count =
        |pred: &dyn Fn(&LabelRole) -> bool| scene.texts.iter().filter(|t| pred(&t.role)).count();
    // Per 45 degree sector: 4 quarters + 3 regions + 5 y ticks = 12 tick labels (the y
    // ticks are the nice 0..200 by 50), and one title per axis.
    assert_eq!(count(&|r| matches!(r, LabelRole::Tick { .. })), 8 * 12);
    assert_eq!(count(&|r| matches!(r, LabelRole::Title { .. })), 8 * 3);
    assert_eq!(count(&|r| matches!(r, LabelRole::Fixed)), 0);
    // Indices run 0..count within a group.
    for t in &scene.texts {
        if let LabelRole::Tick { index, count, .. } = t.role {
            assert!(index < count);
        }
    }
}

#[test]
fn many_categories_are_capped_to_a_bounded_number_of_label_candidates() {
    let cats: Vec<String> = (0..450).map(|i| format!("c{i}")).collect();
    let spec = serde_json::json!({
        "dimension": "3d",
        "data": { "t": { "c": cats, "v": vec![1; 450] } },
        "axes": [ { "id": "x", "orient": "bottom" }, { "id": "y", "orient": "left" } ],
        "layers": [ { "mark": "bar", "data": "t",
            "encoding": { "x": { "field": "c" }, "y": { "field": "v" } } } ]
    });
    let spec = parse_spec_json(&spec.to_string()).unwrap();
    let scene = build_scene_3d(&spec, (800, 500), 1.0, FontMetrics::inter());
    let x_labels_in_sector_0 = scene
        .texts
        .iter()
        .filter(|t| t.key.starts_with("axis:x/label:") && t.key.ends_with("@000"))
        .count();
    assert!(x_labels_in_sector_0 <= 100, "{x_labels_in_sector_0}");
    assert!(x_labels_in_sector_0 >= 50);
}

#[test]
fn both_floor_axes_sit_on_the_edges_at_the_nearest_corner_in_every_sector() {
    // The other two floor edges are hidden behind the bars, and labels there would be drawn
    // over them. Sector `s` covers azimuths `45 s .. 45 (s + 1)`; its middle decides.
    let scene = scene_3d("bar3d-grid");
    let [w, _, d] = scene.size;
    for sector in 0..8 {
        let mid = (sector as f32 * 45.0 + 22.5).to_radians();
        let (sin, cos) = mid.sin_cos();
        let near_x = if sin > 0.0 { w } else { 0.0 };
        let near_z = if cos > 0.0 { d } else { 0.0 };
        let line = |axis: &str| {
            scene
                .lines
                .iter()
                .find(|l| l.key == format!("axis:{axis}/line@{:03}", sector * 45))
                .unwrap_or_else(|| panic!("axis {axis} in sector {sector}"))
        };
        let x = line("x");
        assert_eq!(
            (x.a[2], x.b[2]),
            (near_z, near_z),
            "x axis, sector {sector}"
        );
        let z = line("z");
        assert_eq!(
            (z.a[0], z.b[0]),
            (near_x, near_x),
            "z axis, sector {sector}"
        );
    }
}

#[test]
fn a_foreshortened_edges_labels_hang_away_from_the_box() {
    // Sector 0 (az 0-45, camera front right): depth runs up the right side, labels to the
    // right. Sector 1 (45-90, camera right front): x runs up the left side of the nearest
    // corner, labels to the left.
    let scene = scene_3d("bar3d-grid");
    let label = |key: &str| scene.texts.iter().find(|t| t.key == key).expect(key);
    let right = label("axis:z/label:North@000");
    assert_eq!((right.align, right.valign), (Anchor::Start, VAlign::Middle));
    assert!(right.offset_px[0] > 0.0);
    let left = label("axis:x/label:Q1@045");
    assert_eq!((left.align, left.valign), (Anchor::End, VAlign::Middle));
    assert!(left.offset_px[0] < 0.0);
    // The long edge's labels sit below it.
    let below = label("axis:x/label:Q1@000");
    assert_eq!((below.align, below.valign), (Anchor::Middle, VAlign::Top));
}
