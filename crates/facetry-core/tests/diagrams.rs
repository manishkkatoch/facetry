//! Diagram specs: reading, validation, the scene, and the starting camera.
#![cfg(not(target_arch = "wasm32"))]

use facetry_core::spec::{CameraKind, ChartSpec, EdgeEnd, EdgeStroke, NodeShape};
use facetry_core::{Issue, Theme, build_diagram_3d, build_scene_3d, parse_spec_json, validate};
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

/// The label size of a diagram whose nodes are `n` in a row, in a canvas of `size`.
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
    let scene = build_diagram_3d(
        &spec(&json),
        size,
        1.0,
        FontMetrics::inter(),
        &Theme::default(),
    );
    scene.texts[0].size_px
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
