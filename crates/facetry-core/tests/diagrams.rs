//! Diagram specs: reading, validation, the scene, and the starting camera.
#![cfg(not(target_arch = "wasm32"))]

use facetry_core::scene3d::{FOV_Y_DEGREES, Scene3d};
use facetry_core::spec::{CameraKind, ChartSpec, EdgeEnd, EdgeStroke, NodeShape};
use facetry_core::{
    Issue, Theme, build_diagram_3d, build_scene_3d, framing_radius, parse_spec_json, validate,
};
use facetry_font::FontMetrics;

fn issues(json: &str) -> Vec<Issue> {
    match parse_spec_json(json) {
        Ok(spec) => validate(&spec),
        Err(issues) => issues,
    }
}

/// Two nodes and an edge; tests break one thing at a time with `replace`.
const GOOD: &str = r##"{
  "dimension": "3d",
  "diagram": {
    "nodes": [
      { "id": "a", "label": "Alpha", "x": 50, "y": 20, "width": 80, "height": 40 },
      { "id": "b", "shape": "cylinder", "x": 50, "y": 120, "width": 80, "height": 50, "color": "#ffeeaa" }
    ],
    "edges": [ { "from": "a", "to": "b", "points": [[50, 40], [50, 95]], "label": "go" } ]
  }
}"##;

fn broken(from: &str, to: &str) -> String {
    assert!(GOOD.contains(from), "test bug: {from:?} not in GOOD");
    GOOD.replace(from, to)
}

fn spec(json: &str) -> ChartSpec {
    parse_spec_json(json).unwrap()
}

#[test]
fn a_good_diagram_reads_with_its_defaults() {
    assert_eq!(issues(GOOD), vec![]);
    let s = spec(GOOD);
    let d = s.diagram.as_ref().unwrap();
    assert_eq!(d.nodes[0].shape, NodeShape::Rect);
    assert_eq!(d.nodes[1].shape, NodeShape::Cylinder);
    assert_eq!(d.nodes[1].label, None);
    let e = &d.edges[0];
    assert_eq!(e.stroke, EdgeStroke::Solid);
    assert_eq!(e.ends(), (EdgeEnd::None, EdgeEnd::Arrow));
}

#[test]
fn shape_errors_are_reported_where_they_are() {
    let got = issues(&broken(r#""shape": "cylinder""#, r#""shape": "blob""#));
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].path, "/diagram/nodes/1/shape");
    assert!(
        got[0].message.starts_with("unknown variant `blob`"),
        "{got:?}"
    );
    let got = issues(&broken(r#""label": "go""#, r#""label": "go", "colour": 1"#));
    assert_eq!(got[0].path, "/diagram/edges/0/colour");
}

#[test]
fn every_semantic_problem_is_reported() {
    let cases: &[(&str, &str, &str)] = &[
        (r#""dimension": "3d""#, r#""dimension": "2d""#, "/diagram"),
        (r#""id": "b""#, r#""id": "a""#, "/diagram/nodes/1/id"),
        (r#""id": "a""#, r#""id": """#, "/diagram/nodes/0/id"),
        (
            r#""width": 80, "height": 40"#,
            r#""width": 0, "height": 40"#,
            "/diagram/nodes/0/width",
        ),
        (
            r##""color": "#ffeeaa""##,
            r#""color": "yellow""#,
            "/diagram/nodes/1/color",
        ),
        (r#""to": "b""#, r#""to": "c""#, "/diagram/edges/0/to"),
        (
            r#""points": [[50, 40], [50, 95]]"#,
            r#""points": [[50, 40]]"#,
            "/diagram/edges/0/points",
        ),
        (
            r#""diagram": {"#,
            r#""diagram": { "fontSize": 0,"#,
            "/diagram/fontSize",
        ),
        (
            r#""diagram": {"#,
            r#""axes": [{ "id": "x", "orient": "bottom" }], "diagram": {"#,
            "/axes",
        ),
        (
            r#""diagram": {"#,
            r#""legend": true, "diagram": {"#,
            "/legend",
        ),
    ];
    for (from, to, path) in cases {
        let got = issues(&broken(from, to));
        assert!(
            got.iter().any(|i| i.path == *path),
            "{to}: expected an issue at {path}, got {got:?}"
        );
    }
}

#[test]
fn the_scene_has_a_prism_per_node_and_an_arrow_per_head() {
    let s = spec(GOOD);
    let scene = build_scene_3d(&s, (800, 500), 1.0, FontMetrics::inter());
    assert_eq!(scene.prisms.len(), 2);
    assert_eq!(scene.prisms[0].key, "a");
    assert!(!scene.prisms[0].smooth && scene.prisms[1].smooth);
    assert_eq!(scene.decor.len(), 1);
    assert_eq!(scene.polylines.len(), 1);
    assert_eq!(scene.mark_count(), 2);
    // Labels: the two node labels (b shows its id) and the edge label.
    let texts: Vec<&str> = scene.texts.iter().map(|t| t.text.as_str()).collect();
    assert_eq!(texts, ["Alpha", "b", "go"]);
    // The custom colour is used, the default elsewhere.
    assert_ne!(scene.prisms[0].color, scene.prisms[1].color);
    // Everything lies inside the plot box (edges and arrowheads a little above the nodes).
    let [w, h, d] = scene.size;
    for p in scene.prisms.iter().chain(&scene.decor) {
        assert!(p.top <= h * 1.5);
        for [x, z] in &p.outline {
            assert!((0.0..=w).contains(x) && (0.0..=d).contains(z));
        }
    }
    // The edge runs over the node tops, ending short of the arrowhead's tip.
    let run = &scene.polylines[0].runs[0];
    assert!(run.iter().all(|p| p[1] > scene.prisms[0].top));
}

#[test]
fn dotted_edges_are_cut_into_dashes() {
    let s = spec(&broken(r#""label": "go""#, r#""stroke": "dotted""#));
    let scene = build_scene_3d(&s, (800, 500), 1.0, FontMetrics::inter());
    assert!(scene.polylines[0].runs.len() > 3);
}

#[test]
fn multi_line_labels_are_stacked() {
    let s = spec(&broken(r#""label": "Alpha""#, r#""label": "one\ntwo""#));
    let scene = build_scene_3d(&s, (800, 500), 1.0, FontMetrics::inter());
    let lines: Vec<_> = scene
        .texts
        .iter()
        .filter(|t| t.key.starts_with("a/"))
        .collect();
    assert_eq!(lines.len(), 2);
    assert!(lines[0].offset_px[1] < 0.0 && lines[1].offset_px[1] > 0.0);
}

#[test]
fn diagrams_start_orthographic_and_square_on_unless_the_spec_says_otherwise() {
    let s = spec(GOOD);
    let c = s.start_camera((800, 500), 1.0).unwrap();
    assert_eq!(c.kind, Some(CameraKind::Orthographic));
    assert_eq!(c.azimuth, Some(0.0));
    assert!(c.distance.unwrap() > 0.0);
    let s = spec(&broken(
        r#""dimension": "3d","#,
        r#""dimension": "3d", "camera": { "type": "perspective", "azimuth": 30 },"#,
    ));
    let c = s.start_camera((800, 500), 1.0).unwrap();
    assert_eq!(c.kind, Some(CameraKind::Perspective));
    assert_eq!(c.azimuth, Some(30.0));
    // Charts keep their own camera, defaults and all.
    let chart = parse_spec_json(r#"{"dimension": "3d"}"#).unwrap();
    assert_eq!(chart.start_camera((800, 500), 1.0), None);
}

/// The label size (CSS px, at the starting camera) of a diagram whose nodes are `n` in a row,
/// in a canvas of `size`.
fn label_px(n: usize, size: (u32, u32)) -> f32 {
    let nodes: Vec<String> = (0..n)
        .map(|i| {
            format!(
                r#"{{ "id": "n{i}", "x": {}, "y": 20, "width": 80, "height": 40 }}"#,
                i * 130
            )
        })
        .collect();
    let json = format!(
        r#"{{ "dimension": "3d", "diagram": {{ "nodes": [{}] }} }}"#,
        nodes.join(",")
    );
    let s = spec(&json);
    let scene = build_diagram_3d(&s, size, 1.0, FontMetrics::inter(), &Theme::default());
    // Printed: `size_px` layout units of `flat` world units each, seen from the start.
    let label = &scene.texts[0];
    let distance = s.start_camera(size, 1.0).unwrap().distance.unwrap();
    let half_fov = (FOV_Y_DEGREES / 2.0).to_radians().tan();
    let px_per_world =
        f64::from(size.1) / (2.0 * distance * f64::from(framing_radius(scene.size)) * half_fov);
    (f64::from(label.size_px) * f64::from(label.flat.unwrap()) * px_per_world) as f32
}

#[test]
fn small_diagrams_are_not_blown_up_and_big_ones_shrink_to_fit() {
    // Two nodes: drawn at about their natural size (14 px labels), not zoomed to fill.
    let small = label_px(2, (800, 500));
    assert!((14.0..=17.0).contains(&small), "{small}");
    // Eight nodes in a row do not fit at that size: smaller labels.
    let big = label_px(8, (800, 500));
    assert!(big < small * 0.75, "{big}");
    // A wider canvas fits them bigger.
    let wide = label_px(8, (1600, 500));
    assert!(wide > big * 1.5, "{wide} {big}");
}

fn glossy() -> ChartSpec {
    spec(&broken(
        r#""diagram": {"#,
        r#""diagram": { "style": "glossy","#,
    ))
}

#[test]
fn the_glossy_style_draws_slabs_ribbons_cards_shadows_and_printed_labels() {
    let s = glossy();
    let scene = build_scene_3d(&s, (800, 500), 1.0, FontMetrics::inter());
    assert_eq!(scene.prisms.len(), 2);
    assert!(scene.prisms.iter().all(|p| p.gloss > 0.0));
    // Thinner than the flat style's blocks.
    let flat = build_scene_3d(&spec(GOOD), (800, 500), 1.0, FontMetrics::inter());
    assert!(scene.prisms[0].top < flat.prisms[0].top);
    // The edge is a ribbon on the floor, not a screen-space line.
    assert!(scene.polylines.is_empty());
    assert_eq!(scene.ribbons.len(), 1);
    assert_eq!(scene.ribbons[0].base, 0.0);
    // An arrowhead and the label's card.
    assert_eq!(scene.decor.len(), 2);
    // A shadow under each node and the card.
    assert_eq!(scene.shadows.len(), 3);
    // Every label is printed on its surface.
    assert_eq!(scene.texts.len(), 3);
    assert!(scene.texts.iter().all(|t| t.flat.is_some() && !t.halo));
    // The node without a colour is coloured by its shape; the one with keeps it.
    assert_ne!(scene.prisms[0].color, flat.prisms[0].color);
    assert_eq!(scene.prisms[1].color, flat.prisms[1].color);
}

#[test]
fn glossy_diagrams_start_in_perspective_at_an_angle() {
    let c = glossy().start_camera((800, 500), 1.0).unwrap();
    assert_eq!(c.kind, Some(CameraKind::Perspective));
    assert!(c.azimuth.unwrap() > 0.0);
    assert!(c.elevation.unwrap() < 50.0);
}

#[test]
fn node_colours_strokes_and_text_colours_are_used() {
    let s = spec(&broken(
        r#""label": "Alpha","#,
        r##""label": "Alpha", "stroke": "#ff0000", "strokeWidth": 3, "textColor": "#00ff00","##,
    ));
    let scene = build_scene_3d(&s, (800, 500), 1.0, FontMetrics::inter());
    let rim: Vec<_> = scene
        .lines
        .iter()
        .filter(|l| l.key.starts_with("a/rim"))
        .collect();
    assert!(!rim.is_empty());
    assert!(
        rim.iter()
            .all(|l| l.width_px == 3.0
                && l.color == facetry_core::Color::from_hex("#ff0000").unwrap())
    );
    let label = scene
        .texts
        .iter()
        .find(|t| t.key.starts_with("a/"))
        .unwrap();
    assert_eq!(
        label.color,
        facetry_core::Color::from_hex("#00ff00").unwrap()
    );
}

fn isometric() -> ChartSpec {
    spec(&broken(
        r#""diagram": {"#,
        r#""diagram": { "style": "isometric","#,
    ))
}

#[test]
fn the_isometric_style_stacks_pedestals_on_roads_with_chevrons() {
    let s = isometric();
    let scene = build_scene_3d(&s, (800, 500), 1.0, FontMetrics::inter());
    // Two marks (the caps), each with a plate, a body and an inset as decoration.
    assert_eq!(scene.prisms.len(), 2);
    assert_eq!(scene.prisms[0].key, "a");
    for id in ["a", "b"] {
        for part in ["plate", "body", "inset"] {
            assert!(
                scene.decor.iter().any(|p| p.key == format!("{id}/{part}")),
                "{id}/{part}"
            );
        }
    }
    // The layers stack: plate < body < cap < inset.
    let part = |key: &str| scene.decor.iter().find(|p| p.key == key).unwrap();
    assert!(part("a/plate").top <= part("a/body").base + 1e-6);
    assert!(part("a/body").top <= scene.prisms[0].base + 1e-6);
    assert!(scene.prisms[0].top <= part("a/inset").base + 1e-6);
    // The edge is a road with a head marker (a long enough road also gets chevrons).
    assert_eq!(scene.ribbons.len(), 1);
    assert!(scene.decor.iter().any(|p| p.key == "edge0/head"));
    // Nodes without a colour take the palette in turn; the one with a colour keeps it.
    let glossy = build_scene_3d(&glossy(), (800, 500), 1.0, FontMetrics::inter());
    assert_ne!(scene.prisms[0].color, glossy.prisms[0].color);
    assert_eq!(scene.prisms[1].color, glossy.prisms[1].color);
    // Labels are printed.
    assert!(scene.texts.iter().all(|t| t.flat.is_some()));
    // A shadow per pedestal and for the card.
    assert_eq!(scene.shadows.len(), 3);
}

#[test]
fn isometric_diagrams_start_in_the_isometric_view() {
    let c = isometric().start_camera((800, 500), 1.0).unwrap();
    assert_eq!(c.kind, Some(CameraKind::Orthographic));
    assert_eq!(c.azimuth, Some(45.0));
    assert!((c.elevation.unwrap() - 35.26).abs() < 0.01);
}

#[test]
fn isometric_labels_stay_readable_on_the_white_inset() {
    // White text meant for a coloured fill would vanish on the inset: a dark shade is used.
    let s = spec(
        &broken(r#""diagram": {"#, r#""diagram": { "style": "isometric","#).replace(
            r#""label": "Alpha","#,
            r##""label": "Alpha", "textColor": "#ffffff","##,
        ),
    );
    let scene = build_scene_3d(&s, (800, 500), 1.0, FontMetrics::inter());
    let white = facetry_core::Color::from_hex("#ffffff").unwrap();
    let label = scene
        .texts
        .iter()
        .find(|t| t.key.starts_with("a/"))
        .unwrap();
    assert!(label.color.contrast(white) >= 3.0);
}

#[test]
fn icons_are_drawn_above_the_label_in_every_style() {
    for style in ["flat", "glossy", "isometric"] {
        let json = broken(
            r#""diagram": {"#,
            &format!(r#""diagram": {{ "style": "{style}","#),
        )
        .replace(
            r#""label": "Alpha","#,
            r#""label": "Alpha", "icon": "fa:lightbulb","#,
        );
        let scene = build_scene_3d(&spec(&json), (800, 500), 1.0, FontMetrics::inter());
        let icon = scene
            .polylines
            .iter()
            .find(|p| p.key == "a/icon")
            .unwrap_or_else(|| panic!("{style}: no icon"));
        assert!(icon.runs.len() >= 2, "{style}");
        // The icon is above (farther up the page than) the label.
        let label = scene
            .texts
            .iter()
            .find(|t| t.key.starts_with("a/label"))
            .unwrap();
        let icon_z = icon
            .runs
            .iter()
            .flatten()
            .map(|p| p[2])
            .fold(f32::INFINITY, f32::min);
        assert!(icon_z < label.anchor[2], "{style}");
    }
}

#[test]
fn an_isometric_card_has_its_title_text_and_icon_and_the_node_its_number() {
    let json = broken(r#""diagram": {"#, r#""diagram": { "style": "isometric","#).replace(
        r#""label": "Alpha","#,
        r#""label": "1", "icon": "gear", "card": { "x": 160, "y": 20, "width": 140, "height": 50, "title": "Alpha", "text": "first line\nsecond line" },"#,
    );
    let s = spec(&json);
    assert_eq!(validate(&s), vec![]);
    let scene = build_scene_3d(&s, (800, 500), 1.0, FontMetrics::inter());
    let texts: Vec<&str> = scene.texts.iter().map(|t| t.text.as_str()).collect();
    for want in ["Alpha", "first line", "second line"] {
        assert!(texts.contains(&want), "{want} in {texts:?}");
    }
    assert!(scene.decor.iter().any(|p| p.key == "a/card"));
    assert!(scene.polylines.iter().any(|p| p.key == "a/card/icon"));
    // The card's text is printed left-aligned, on the card.
    let title = scene.texts.iter().find(|t| t.text == "Alpha").unwrap();
    assert_eq!(title.align, facetry_core::scene::Anchor::Start);
    // The number stands on the pedestal as solid digits (not printed), in the cap's colour,
    // above the pedestal's top and turned to face the isometric view.
    assert!(!texts.contains(&"1"));
    let cap = scene.prisms.iter().find(|p| p.key == "a").unwrap();
    let numeral: Vec<_> = scene
        .decor
        .iter()
        .filter(|p| p.key == "a/numeral")
        .collect();
    assert!(numeral.len() > 2);
    let inset = scene.decor.iter().find(|p| p.key == "a/inset").unwrap();
    for p in &numeral {
        let stand = p.stand.expect("standing");
        assert!((stand.origin[1] - inset.top).abs() < 1e-3);
        assert!(
            (stand.right[0] - 0.5f32.sqrt()).abs() < 1e-4
                && (stand.right[1] + 0.5f32.sqrt()).abs() < 1e-4
        );
        assert_eq!(p.color, cap.color);
        assert!(p.outline.iter().all(|q| q[1] >= -1e-3), "on its foot");
    }
}

#[test]
fn a_label_that_is_not_a_short_number_is_printed_on_the_pedestal() {
    let json = broken(r#""diagram": {"#, r#""diagram": { "style": "isometric","#).replace(
        r#""label": "Alpha","#,
        r#""label": "A1", "card": { "x": 160, "y": 20, "width": 140, "height": 50, "title": "Alpha" },"#,
    );
    let scene = build_scene_3d(&spec(&json), (800, 500), 1.0, FontMetrics::inter());
    assert!(scene.texts.iter().any(|t| t.text == "A1"));
    assert!(!scene.decor.iter().any(|p| p.key == "a/numeral"));
}

fn schematic() -> ChartSpec {
    spec(&broken(
        r#""diagram": {"#,
        r#""diagram": { "style": "schematic","#,
    ))
}

#[test]
fn the_schematic_style_draws_equipment_pipes_on_supports_and_upright_signs() {
    let s = schematic();
    assert_eq!(validate(&s), vec![]);
    let c = s.start_camera((800, 500), 1.0).unwrap();
    assert_eq!(c.kind, Some(CameraKind::Orthographic));
    assert_eq!(c.azimuth, Some(45.0));
    let scene = build_scene_3d(&s, (800, 500), 1.0, FontMetrics::inter());
    // One mark per node (its body); a cylinder is a silo standing on its plinth.
    assert_eq!(scene.prisms.len(), 2);
    assert_eq!(scene.prisms[1].key, "b");
    let silo = scene.decor.iter().find(|p| p.key == "b/silo").unwrap();
    assert!((silo.base - scene.prisms[1].top).abs() < 1e-6);
    // Every part has its drawn edges, which stand a hair above its top.
    for id in ["a", "b"] {
        assert!(
            scene
                .polylines
                .iter()
                .any(|l| l.key == format!("{id}/edge/0")),
            "{id}"
        );
    }
    let top = scene.prisms[0].top;
    let body = scene
        .polylines
        .iter()
        .find(|l| l.key == "a/edge/0")
        .unwrap();
    // A closed ring round the top and one round the foot, and the box's four corners.
    assert_eq!(body.runs.len(), 2 + 4);
    assert!(body.runs[0].iter().all(|p| p[1] > top));
    assert_eq!(body.runs[0].first(), body.runs[0].last());
    // The edge is a pipe off the floor, as high as a body is tall at the least.
    assert_eq!(scene.ribbons.len(), 1);
    let pipe = &scene.ribbons[0];
    assert!(pipe.base > 0.0 && pipe.top <= scene.prisms[0].top);
    assert!(scene.decor.iter().any(|p| p.key == "edge0/head"));
    // Labels are printed upright on signs that face the isometric view, above the equipment.
    let signs = scene.signs_at(45.0);
    let label = signs.texts.iter().find(|t| t.text == "Alpha").unwrap();
    let r = 0.5f32.sqrt();
    let upright = label.upright.expect("upright");
    assert!((upright[0] - r).abs() < 1e-4 && (upright[1] + r).abs() < 1e-4);
    assert!(label.flat.is_some());
    assert!(label.anchor[1] > top);
    let board = signs
        .decor
        .iter()
        .find(|p| p.key == "a/sign/board")
        .unwrap();
    assert!(board.stand.is_some());
    // Turned to face the camera wherever it orbits to: from behind, the face points back.
    let sign = scene.signs.iter().find(|s| s.key == "a/sign").unwrap();
    let behind = scene.signs_at(225.0);
    let turned = behind.texts.iter().find(|t| t.text == "Alpha").unwrap();
    let up = turned.upright.unwrap();
    assert!((up[0] + r).abs() < 1e-4 && (up[1] - r).abs() < 1e-4);
    // The anchor, in front of the board, moved to the pivot's other side, at the same height.
    let (a, b) = (label.anchor, turned.anchor);
    let (da, db) = (
        [a[0] - sign.pivot[0], a[2] - sign.pivot[1]],
        [b[0] - sign.pivot[0], b[2] - sign.pivot[1]],
    );
    assert!((da[0] + db[0]).abs() < 1e-4 && (da[1] + db[1]).abs() < 1e-4);
    assert_eq!(a[1], b[1]);
    assert!(scene.decor.iter().any(|p| p.key == "a/sign/post"));
    // A faint grid lies on the floor.
    assert!(
        scene
            .polylines
            .iter()
            .any(|p| p.key == "grid" && !p.runs.is_empty())
    );
}

#[test]
fn schematic_pipes_stand_on_supports_and_carry_flow_marks() {
    // A long edge: room for supports and flow marks along it.
    let json = broken(r#""diagram": {"#, r#""diagram": { "style": "schematic","#).replace(
        r#""points": [[50, 40], [50, 95]]"#,
        r#""points": [[50, 40], [50, 260], [300, 260]]"#,
    );
    let scene = build_scene_3d(&spec(&json), (800, 500), 1.0, FontMetrics::inter());
    let supports: Vec<_> = scene
        .decor
        .iter()
        .filter(|p| p.key.starts_with("edge0/support/"))
        .collect();
    assert!(supports.len() >= 3, "{}", supports.len());
    let pipe = &scene.ribbons[0];
    for p in &supports {
        assert_eq!(p.base, 0.0);
        assert!((p.top - pipe.base).abs() < 1e-6);
    }
    assert!(scene.decor.iter().any(|p| p.key.starts_with("edge0/flow/")));
}

/// `scene` with its signs (a schematic's) facing the isometric view, among the rest.
fn with_signs(mut scene: Scene3d) -> Scene3d {
    let signs = scene.signs_at(45.0);
    scene.decor.extend(signs.decor);
    scene.texts.extend(signs.texts);
    scene.polylines.extend(signs.polylines);
    scene.fills.extend(signs.fills);
    scene
}

/// GOOD with node `a` showing icon `name` and the diagram's own `icons` (a JSON object body).
fn with_icons(name: &str, icons: &str) -> String {
    broken(
        r#""diagram": {"#,
        &format!(r#""diagram": {{ "icons": {{ {icons} }},"#),
    )
    .replace(
        r#""label": "Alpha","#,
        &format!(r#""label": "Alpha", "icon": "{name}","#),
    )
}

#[test]
fn a_diagram_draws_its_own_icons_and_they_win_over_built_in_ones() {
    // A diagonal across a 48-wide viewBox: fitted into the 24 box, centred.
    let json = with_icons(
        "slash",
        r#""slash": { "stroke": "M0 0 L48 24", "viewBox": [0, 0, 48, 24] }"#,
    );
    let s = spec(&json);
    assert_eq!(validate(&s), vec![]);
    let scene = build_scene_3d(&s, (800, 500), 1.0, FontMetrics::inter());
    let icon = scene.polylines.iter().find(|p| p.key == "a/icon").unwrap();
    assert_eq!(icon.runs.len(), 1);
    let run = &icon.runs[0];
    // Wider than tall: it spans the icon's width at half its height.
    let (dx, dz) = (run[1][0] - run[0][0], run[1][2] - run[0][2]);
    assert!((dx - 2.0 * dz).abs() < 1e-4, "{dx} {dz}");
    // A custom icon named like a built-in one is drawn instead of it.
    let star = |json: &str| {
        let scene = build_scene_3d(&spec(json), (800, 500), 1.0, FontMetrics::inter());
        scene
            .polylines
            .into_iter()
            .find(|p| p.key == "a/icon")
            .unwrap()
    };
    let own = star(&with_icons("star", r#""star": { "stroke": "M2 2 H22" }"#));
    assert_eq!(own.runs.len(), 1);
    assert_eq!(own.runs[0].len(), 2);
    assert!(star(&with_icons("star", "")).runs[0].len() > 2);
    // And every style draws them.
    for style in ["glossy", "isometric", "schematic"] {
        let json = json.replace(r#""icons""#, &format!(r#""style": "{style}", "icons""#));
        let scene = with_signs(build_scene_3d(
            &spec(&json),
            (800, 500),
            1.0,
            FontMetrics::inter(),
        ));
        assert!(
            scene
                .polylines
                .iter()
                .any(|p| p.key.ends_with("/icon") && p.runs.len() == 1),
            "{style}"
        );
    }
}

#[test]
fn broken_custom_icons_are_reported_where_they_are() {
    let got = issues(&with_icons("pump", r#""pump": { "stroke": "M0 0 L 5" }"#));
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].path, "/diagram/icons/pump/stroke");
    assert_eq!(got[0].message, "expected a number at the end of the path");
    // A filled shape's path is reported at its place in the list.
    let got = issues(&with_icons(
        "pump",
        r#""pump": { "fill": [{ "d": "M0 0h4v4z" }, { "d": "M0 0 K" }] }"#,
    ));
    assert_eq!(got[0].path, "/diagram/icons/pump/fill/1/d");
    let got = issues(&with_icons(
        "pump",
        r#""pump": { "fill": [{ "d": "M0 0h4v4z", "fillRule": "odd" }] }"#,
    ));
    assert_eq!(got[0].path, "/diagram/icons/pump/fill/0/fillRule");
    // An icon with neither lines nor fills.
    let got = issues(&with_icons(
        "pump",
        r#""pump": { "viewBox": [0, 0, 24, 24] }"#,
    ));
    assert_eq!(got[0].path, "/diagram/icons/pump");
    assert_eq!(got[0].message, "an icon needs `stroke` or `fill` path data");
    let got = issues(&with_icons(
        "pump",
        r#""pump": { "stroke": "M0 0 L5 5", "viewBox": [0, 0, 0, 24] }"#,
    ));
    assert_eq!(got[0].path, "/diagram/icons/pump/viewBox");
    let got = issues(&with_icons(
        "pump",
        r#""pump": { "stroke": "M0 0 L5 5", "viewBox": [0, 0, 24] }"#,
    ));
    assert_eq!(got[0].path, "/diagram/icons/pump/viewBox");
    assert!(
        got[0].message.contains("expected an array of length 4"),
        "{got:?}"
    );
    // A name with a slash is escaped in the pointer.
    let got = issues(&with_icons("a/b", r#""a/b": { "stroke": "Q" }"#));
    assert_eq!(got[0].path, "/diagram/icons/a~1b/stroke");
    // A node may only name an icon that exists.
    let got = issues(&with_icons("pumpp", r#""pump": { "stroke": "M0 0 L5 5" }"#));
    assert_eq!(got[0].path, "/diagram/nodes/0/icon");
    assert!(
        got[0]
            .message
            .starts_with("no icon \"pumpp\": it is not in the diagram's `icons`")
    );
}

#[test]
fn filled_icons_are_printed_as_flat_triangles_with_their_holes() {
    // A 20 by 20 square with an 8 by 8 hole (even-odd), and a dot in the hole (its own shape).
    let json = with_icons(
        "frame",
        r#""frame": { "fill": [
            { "d": "M2 2h20v20H2z M8 8v8h8V8z", "fillRule": "evenodd" },
            { "d": "M11 11h2v2h-2z" }
        ], "stroke": "M2 23 H22" }"#,
    );
    let s = spec(&json);
    assert_eq!(validate(&s), vec![]);
    for style in ["flat", "glossy", "isometric", "schematic"] {
        let json = json.replace(r#""icons""#, &format!(r#""style": "{style}", "icons""#));
        let scene = with_signs(build_scene_3d(
            &spec(&json),
            (800, 500),
            1.0,
            FontMetrics::inter(),
        ));
        let fill = scene
            .fills
            .iter()
            .find(|f| f.key.ends_with("icon"))
            .expect(style);
        assert_eq!(fill.triangles.len() % 3, 0);
        // Its lines are drawn too, under the same key.
        assert!(scene.polylines.iter().any(|p| p.key == fill.key), "{style}");
        if style == "schematic" {
            continue;
        }
        // Lying on the node's top: the area is (400 - 64 + 4) / 576 of the icon's square.
        let area: f32 = fill
            .triangles
            .chunks_exact(3)
            .map(|t| {
                let (a, b, c) = (t[0], t[1], t[2]);
                ((b[0] - a[0]) * (c[2] - a[2]) - (c[0] - a[0]) * (b[2] - a[2])).abs() / 2.0
            })
            .sum();
        let side = scene
            .polylines
            .iter()
            .find(|p| p.key == fill.key)
            .unwrap()
            .runs[0]
            .windows(2)
            .map(|w| (w[1][0] - w[0][0]).abs())
            .sum::<f32>()
            / 20.0
            * 24.0;
        let want = side * side * 340.0 / 576.0;
        assert!(
            (area - want).abs() < want * 1e-3,
            "{style}: {area} vs {want}"
        );
        assert!(
            fill.triangles
                .iter()
                .all(|p| (p[1] - fill.triangles[0][1]).abs() < 1e-6)
        );
    }
}

/// GOOD with its two nodes in group `team` (titled), which sits in group `org`.
fn grouped() -> String {
    broken(
        r#""diagram": {"#,
        r#""diagram": { "groups": [
            { "id": "org", "label": "Organisation", "x": 50, "y": 70, "width": 160, "height": 200 },
            { "id": "team", "label": "Team", "x": 50, "y": 75, "width": 120, "height": 170, "parent": "org" }
        ],"#,
    )
}

#[test]
fn groups_validate_and_edges_may_end_at_them() {
    assert_eq!(issues(&grouped()), vec![]);
    // An edge to a group by its id.
    let to_group = grouped().replace(r#""to": "b""#, r#""to": "team""#);
    assert_eq!(issues(&to_group), vec![]);
    let bad = |from: &str, to: &str| issues(&grouped().replace(from, to));
    let got = bad(r#""to": "b""#, r#""to": "nobody""#);
    assert_eq!(got[0].path, "/diagram/edges/0/to");
    assert_eq!(got[0].message, "no node or group has the id \"nobody\"");
    let got = bad(r#""id": "team""#, r#""id": "org""#);
    assert!(
        got.iter()
            .any(|i| i.path == "/diagram/groups/1/id" && i.message.contains("duplicate group id"))
    );
    let got = bad(r#""id": "team""#, r#""id": "a""#);
    assert!(
        got.iter()
            .any(|i| i.path == "/diagram/nodes/0/id" && i.message.contains("id of a group too"))
    );
    let got = bad(r#""parent": "org""#, r#""parent": "nowhere""#);
    assert_eq!(got[0].path, "/diagram/groups/1/parent");
    let got = bad(
        r#""label": "Organisation","#,
        r#""label": "Organisation", "parent": "team","#,
    );
    assert!(
        got.iter().any(|i| i.message.contains("nested in itself")),
        "{got:?}"
    );
    let got = bad(r#""width": 120"#, r#""width": 0"#);
    assert_eq!(got[0].path, "/diagram/groups/1/width");
    let got = bad(
        r#""label": "Team","#,
        r#""label": "Team", "color": "blue","#,
    );
    assert_eq!(got[0].path, "/diagram/groups/1/color");
}

#[test]
fn every_style_draws_groups_as_bases_with_their_titles() {
    for style in ["flat", "glossy", "isometric", "schematic"] {
        let json = grouped().replace(r#""groups""#, &format!(r#""style": "{style}", "groups""#));
        let scene = build_scene_3d(&spec(&json), (800, 500), 1.0, FontMetrics::inter());
        let base = |id: &str| {
            scene
                .decor
                .iter()
                .find(|p| p.key == format!("{id}/base"))
                .unwrap_or_else(|| panic!("{style}: no base for {id}"))
        };
        // The inner group's base stands on the outer's.
        assert!(base("team").top > base("org").top, "{style}");
        assert!(base("org").top > 0.0);
        for title in ["Organisation", "Team"] {
            let t = scene.texts.iter().find(|t| t.text == title).expect(title);
            assert!(t.flat.is_some(), "{style}: {title} is printed");
        }
        // Outlined.
        assert!(
            scene.polylines.iter().any(|p| p.key == "team/edge"),
            "{style}"
        );
        // The nodes are still the marks.
        assert_eq!(scene.prisms.len(), 2, "{style}");
        if style == "schematic" {
            assert!(scene.decor.iter().any(|p| p.key.starts_with("org/post/")));
            assert!(scene.ribbons.iter().any(|r| r.key == "org/rail/0"));
        }
        if style == "glossy" || style == "isometric" {
            // Shadows of what stands on a base are lifted onto it.
            let team_top = base("team").top;
            assert!(
                scene
                    .shadows
                    .iter()
                    .any(|s| (s.lift - team_top).abs() < 1e-6),
                "{style}"
            );
        }
    }
}

#[test]
fn a_long_group_title_shrinks_to_fit_its_box() {
    let json = grouped().replace(
        r#""label": "Team""#,
        r#""label": "A very long title that will not fit""#,
    );
    let scene = build_scene_3d(&spec(&json), (800, 500), 1.0, FontMetrics::inter());
    let long = scene
        .texts
        .iter()
        .find(|t| t.text.starts_with("A very"))
        .unwrap();
    let short = scene
        .texts
        .iter()
        .find(|t| t.text == "Organisation")
        .unwrap();
    assert!(long.size_px < short.size_px);
}

/// Two edges crossing at (150, 100): `a`-`b` across, then `c`-`d` down.
fn crossing(style: &str) -> String {
    format!(
        r#"{{
  "dimension": "3d",
  "diagram": {{
    "style": "{style}",
    "nodes": [
      {{ "id": "a", "x": 20, "y": 100, "width": 30, "height": 30 }},
      {{ "id": "b", "x": 280, "y": 100, "width": 30, "height": 30 }},
      {{ "id": "c", "x": 150, "y": -30, "width": 30, "height": 30 }},
      {{ "id": "d", "x": 150, "y": 230, "width": 30, "height": 30 }}
    ],
    "edges": [
      {{ "from": "a", "to": "b", "points": [[35, 100], [265, 100]] }},
      {{ "from": "c", "to": "d", "points": [[150, -15], [150, 215]] }}
    ]
  }}
}}"#
    )
}

#[test]
fn where_edges_cross_the_later_arcs_over_the_earlier_in_every_style() {
    for style in ["flat", "glossy", "isometric", "schematic"] {
        let json = crossing(style);
        assert_eq!(validate(&spec(&json)), vec![], "{style}");
        let scene = build_scene_3d(&spec(&json), (800, 500), 1.0, FontMetrics::inter());
        // The highest point of an edge and how high it is at its ends.
        let (first, second) = if style == "flat" {
            let run = |key: &str| {
                let line = scene.polylines.iter().find(|p| p.key == key).unwrap();
                let ys: Vec<f32> = line.runs[0].iter().map(|p| p[1]).collect();
                (ys.iter().copied().fold(f32::MIN, f32::max), ys[0])
            };
            (run("edge0"), run("edge1"))
        } else {
            let band = |key: &str| {
                let r = scene
                    .ribbons
                    .iter()
                    .find(|r| r.key == format!("{key}/0"))
                    .unwrap();
                let top = |i: usize| r.top + r.lift.get(i).copied().unwrap_or(0.0);
                let max = (0..r.path.len()).map(top).fold(f32::MIN, f32::max);
                (max, top(0))
            };
            (band("edge0"), band("edge1"))
        };
        // The first stays level; the second rises over it, clear of its top.
        assert_eq!(first.0, first.1, "{style}");
        assert!(second.0 > second.1, "{style}");
        let r = scene.ribbons.iter().find(|r| r.key == "edge1/0");
        if let Some(r) = r {
            let crest = r.lift.iter().copied().fold(f32::MIN, f32::max);
            let under = scene.ribbons.iter().find(|r| r.key == "edge0/0").unwrap();
            assert!(r.base + crest > under.top, "{style}");
        }
    }
}

#[test]
fn a_dark_theme_darkens_the_page_and_lightens_the_text_in_every_style() {
    let light = facetry_core::Color::linear(1.0, 1.0, 1.0, 1.0);
    for style in ["flat", "glossy", "isometric", "schematic"] {
        let json = broken(
            r#""diagram": {"#,
            &format!(r#""diagram": {{ "style": "{style}", "theme": "dark","#),
        );
        let s = spec(&json);
        assert_eq!(validate(&s), vec![], "{style}");
        let theme = Theme::default();
        let scene = with_signs(build_diagram_3d(
            &s,
            (800, 500),
            1.0,
            FontMetrics::inter(),
            &theme,
        ));
        assert!(scene.background.contrast(light) > 10.0, "{style}");
        // The edge label is light on the dark page (or on its dark card or sign).
        let label = scene.texts.iter().find(|t| t.text == "go").expect(style);
        assert!(label.color.contrast(scene.background) >= 4.5, "{style}");
        // A page colour the chart was given (`clearColor`) is kept.
        let red = facetry_core::Color::linear(1.0, 0.0, 0.0, 1.0);
        let scene = build_diagram_3d(
            &s,
            (800, 500),
            1.0,
            FontMetrics::inter(),
            &Theme::with_background(red),
        );
        assert_eq!(scene.background, red, "{style}");
    }
    // Themes are named.
    let got = issues(&broken(
        r#""diagram": {"#,
        r#""diagram": { "theme": "pink","#,
    ));
    assert_eq!(got[0].path, "/diagram/theme");
}

#[test]
fn flow_marks_move_along_their_path_and_shrink_at_its_ends() {
    use facetry_core::scene3d::FlowMark;
    let flow = FlowMark {
        key: "f".into(),
        color: facetry_core::Color::linear(0.0, 0.0, 0.0, 1.0),
        path: vec![[0.0, 1.0, 0.0], [10.0, 1.0, 0.0]],
        every: 2.0,
        speed: 1.0,
        phase: 0.0,
        outline: vec![[0.5, 0.0], [-0.5, 0.25], [-0.5, -0.25]],
        base: 0.0,
        top: 0.1,
        smooth: false,
        gloss: 0.0,
    };
    // Marks centred at 1, 3, 5, 7, 9 a second in, pointing along +x; a second later each has
    // moved a unit on.
    let at = |t: f32| -> Vec<f32> {
        flow.marks_at(t)
            .iter()
            .map(|m| {
                (m.outline[1][0] + m.outline[2][0]) / 2.0
                    + (m.outline[0][0] - m.outline[1][0]) / 2.0
            })
            .collect()
    };
    let a = at(1.0);
    assert_eq!(a.len(), 5);
    for (k, x) in a.iter().enumerate() {
        assert!((x - (1.0 + 2.0 * k as f32)).abs() < 1e-4, "{a:?}");
    }
    let b = at(2.0);
    assert!(
        (b[0] - 2.0).abs() < 1e-4 && (b[1] - 4.0).abs() < 1e-4,
        "{b:?}"
    );
    // At its full size in the middle; smaller near the ends.
    let size = |m: &facetry_core::scene3d::PrismMark| m.outline[0][0] - m.outline[1][0];
    let marks = flow.marks_at(1.0);
    assert!((size(&marks[2]) - 1.0).abs() < 1e-4);
    assert!(size(&marks[0]) < 1.0 && size(&marks[4]) < 1.0);
    // On the path's height.
    assert!(marks.iter().all(|m| m.base >= 1.0 && m.top > m.base));
    // A mark at either end has no size and is left out.
    assert_eq!(flow.marks_at(0.0).len(), 4);
}

#[test]
fn animated_edges_flow_the_way_they_run_in_every_style() {
    for style in ["flat", "glossy", "isometric", "schematic"] {
        let json = |extra: &str| {
            broken(
                r#""label": "go" }"#,
                &format!(r#""label": "go", "points": [[50, 40], [50, 300]], "animation": "slow"{extra} }}"#),
            )
            .replace(r#""points": [[50, 40], [50, 95]], "#, "")
            .replace(r#""diagram": {"#, &format!(r#""diagram": {{ "style": "{style}","#))
        };
        let s = spec(&json(""));
        assert_eq!(validate(&s), vec![], "{style}");
        let scene = build_scene_3d(&s, (800, 500), 1.0, FontMetrics::inter());
        assert_eq!(scene.flows.len(), 1, "{style}");
        let flow = &scene.flows[0];
        // From `a` (y 40) down to `b`: along +z.
        assert!(flow.path.last().unwrap()[2] > flow.path[0][2], "{style}");
        assert!(flow.speed > 0.0 && flow.every > 0.0);
        // Moving marks replace the still ones of the styles that have them.
        assert!(
            !scene
                .decor
                .iter()
                .any(|p| p.key.contains("/chevron/") || p.key.contains("/flow/")),
            "{style}"
        );
        // A two-headed edge flows both ways.
        let both = spec(&json(r#", "tail": "arrow""#));
        let scene = build_scene_3d(&both, (800, 500), 1.0, FontMetrics::inter());
        assert_eq!(scene.flows.len(), 2, "{style}");
        // An edge that is not animated does not flow.
        let still = spec(&json("").replace(r#", "animation": "slow""#, ""));
        assert!(
            build_scene_3d(&still, (800, 500), 1.0, FontMetrics::inter())
                .flows
                .is_empty()
        );
    }
    let got = issues(&broken(
        r#""label": "go""#,
        r#""label": "go", "animation": "warp""#,
    ));
    assert_eq!(got[0].path, "/diagram/edges/0/animation");
}

#[test]
fn a_diagram_builds_in_rank_by_rank_and_ends_as_itself() {
    for style in ["flat", "glossy", "isometric", "schematic"] {
        let json = grouped().replace(r#""groups""#, &format!(r#""style": "{style}", "groups""#));
        let scene = build_scene_3d(&spec(&json), (800, 500), 1.0, FontMetrics::inter());
        let step = |owner: &str| {
            scene
                .build
                .iter()
                .find(|s| s.owner == owner)
                .unwrap_or_else(|| panic!("{style}: no step for {owner}"))
        };
        // `a` leads to `b`: `a` rises first, the edge leaves it, `b` rises as it arrives.
        let (a, b, e) = (step("a"), step("b"), step("edge0"));
        assert!(
            a.start_ms < e.start_ms && e.start_ms < b.start_ms,
            "{style}"
        );
        assert!(e.draw && !a.draw);
        // A group's base comes up before what stands on it.
        assert!(step("org").start_ms <= a.start_ms, "{style}");
        let total = scene.build_ms();
        assert!(total > 0.0 && total <= 3200.0);
        // At the start only what rises first; at the end everything, as it is.
        let first = scene.built_at(0.0);
        assert!(first.prisms.is_empty(), "{style}");
        let last = scene.built_at(total);
        assert_eq!(last.prisms, scene.prisms, "{style}");
        assert_eq!(last.decor, scene.decor, "{style}");
        assert_eq!(last.ribbons, scene.ribbons, "{style}");
        assert_eq!(last.polylines, scene.polylines, "{style}");
        assert_eq!(last.texts, scene.texts, "{style}");
        assert!(last.build.is_empty());
        // Half way through its edge's drawing, `b` is not up yet and the edge is partly drawn.
        let mid = scene.built_at(e.start_ms + e.duration_ms * 0.5);
        assert!(!mid.prisms.iter().any(|p| p.key == "b"), "{style}");
        let drawn = |s: &Scene3d| -> f32 {
            let bands: f32 = s
                .ribbons
                .iter()
                .filter(|r| r.key.starts_with("edge0"))
                .flat_map(|r| {
                    r.path
                        .windows(2)
                        .map(|w| (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1]))
                })
                .sum();
            let lines: f32 = s
                .polylines
                .iter()
                .filter(|l| l.key == "edge0")
                .flat_map(|l| {
                    l.runs.iter().flat_map(|r| {
                        r.windows(2)
                            .map(|w| (w[1][0] - w[0][0]).hypot(w[1][2] - w[0][2]))
                    })
                })
                .sum();
            bands + lines
        };
        let (part, whole) = (drawn(&mid), drawn(&scene));
        assert!(part > 0.0 && part < whole, "{style}: {part} of {whole}");
    }
}

/// GOOD with node `b` drawn as block `pump` (two parts) in `style`.
fn with_block(style: &str) -> String {
    broken(r#""diagram": {"#, &format!(r##""diagram": {{ "style": "{style}", "blocks": {{ "pump": {{ "parts": [
        {{ "name": "skid", "from": 0, "to": 0.5, "color": "frame" }},
        {{ "name": "casing", "shape": "circle", "at": [-0.2, 0], "size": [0.5, 0.7], "from": 0.5, "to": 2, "color": "#336699" }}
    ] }} }},"##))
    .replace(r#""shape": "cylinder","#, r#""shape": "cylinder", "block": "pump","#)
}

#[test]
fn blocks_replace_equipment_and_pedestals_and_other_styles_draw_the_shape() {
    for style in ["schematic", "isometric"] {
        let s = spec(&with_block(style));
        assert_eq!(validate(&s), vec![], "{style}");
        let scene = with_signs(build_scene_3d(&s, (800, 500), 1.0, FontMetrics::inter()));
        // The first part is the node's mark, the others are named after their parts.
        let mark = scene.prisms.iter().find(|p| p.key == "b").expect(style);
        let casing = scene
            .decor
            .iter()
            .find(|p| p.key == "b/casing")
            .expect(style);
        assert!(
            mark.top < casing.top && (casing.base - mark.top).abs() < 1e-6,
            "{style}"
        );
        assert_eq!(
            casing.color,
            facetry_core::Color::from_hex("#336699").unwrap()
        );
        // Fitted to the box: the casing is half the node's 80 units across, left of centre.
        let xs: Vec<f32> = casing.outline.iter().map(|p| p[0]).collect();
        let (lo, hi) = xs
            .iter()
            .fold((f32::MAX, f32::MIN), |(a, b), &x| (a.min(x), b.max(x)));
        let node = scene.prisms.iter().find(|p| p.key == "a").unwrap();
        let unit = {
            let xs: Vec<f32> = node.outline.iter().map(|p| p[0]).collect();
            (xs.iter().cloned().fold(f32::MIN, f32::max)
                - xs.iter().cloned().fold(f32::MAX, f32::min))
                / 80.0
        };
        assert!(
            ((hi - lo) / unit - 40.0).abs() < 0.5,
            "{style}: {}",
            (hi - lo) / unit
        );
        // No pedestal or equipment of the shape's: a silo would have a `b/silo`.
        assert!(
            !scene
                .decor
                .iter()
                .any(|p| p.key == "b/silo" || p.key == "b/cap")
        );
        // The label stands on a sign over the block.
        let label = scene
            .texts
            .iter()
            .find(|t| t.key.starts_with("b/sign/"))
            .expect(style);
        assert!(label.anchor[1] > casing.top, "{style}");
    }
    // The flat style draws the node's shape.
    let s = spec(&with_block("flat"));
    assert_eq!(validate(&s), vec![]);
    let scene = build_scene_3d(&s, (800, 500), 1.0, FontMetrics::inter());
    assert!(scene.decor.iter().all(|p| !p.key.starts_with("b/casing")));
}

#[test]
fn block_mistakes_are_reported_where_they_are() {
    let json = with_block("schematic");
    let got = issues(&json.replace(r#""block": "pump""#, r#""block": "pumpp""#));
    assert_eq!(got[0].path, "/diagram/nodes/1/block");
    assert_eq!(
        got[0].message,
        r#"no block "pumpp"; the diagram's blocks are pump"#
    );
    let got = issues(&json.replace(r#""from": 0.5, "to": 2"#, r#""from": 2, "to": 1"#));
    assert_eq!(got[0].path, "/diagram/blocks/pump/parts/1/to");
    let got = issues(&json.replace(r#""color": "frame""#, r#""color": "chrome""#));
    assert_eq!(got[0].path, "/diagram/blocks/pump/parts/0/color");
    assert!(
        got[0].message.contains("body, steel, frame, accent"),
        "{got:?}"
    );
    let got = issues(&json.replace(r#""size": [0.5, 0.7]"#, r#""size": [0, 0.7]"#));
    assert_eq!(got[0].path, "/diagram/blocks/pump/parts/1/size");
    let got = issues(&json.replace(r#""from": 0, "to": 0.5"#, r#""from": 0, "to": 50"#));
    assert_eq!(got[0].path, "/diagram/blocks/pump/parts/0/to");
}
