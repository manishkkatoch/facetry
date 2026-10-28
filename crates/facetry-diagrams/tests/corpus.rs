//! The golden corpus (`examples/diagrams`): every diagram must lay out, with hard guarantees
//! checked here (no overlapping nodes, edges that start and end on their nodes, the same
//! output every time), and its quality metrics are recorded in one snapshot so that any change
//! to the layout shows, diagram by diagram, what got better or worse.
#![cfg(not(target_arch = "wasm32"))]

use std::fmt::Write;
use std::path::PathBuf;

use facetry_core::spec::{DiagramStyle, DiagramTheme};
use facetry_diagrams::{Options, build, flowchart, from_mermaid, from_mermaid_with};

const GLOSSY: Options = Options {
    style: DiagramStyle::Glossy,
    theme: DiagramTheme::Default,
    flow: None,
    cards: false,
    icons: Vec::new(),
};
const CARDS: Options = Options {
    style: DiagramStyle::Isometric,
    theme: DiagramTheme::Default,
    flow: None,
    cards: true,
    icons: Vec::new(),
};

fn corpus() -> Vec<(String, String)> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/diagrams/flowchart");
    let mut files: Vec<(String, String)> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "mmd"))
        .map(|p| {
            let name = p.file_stem().unwrap().to_string_lossy().into_owned();
            (name, std::fs::read_to_string(&p).unwrap())
        })
        .collect();
    files.sort();
    assert!(files.len() >= 20, "the corpus went missing");
    files
}

#[test]
fn every_diagram_lays_out_cleanly() {
    for (name, source) in corpus() {
        let d = from_mermaid(&source).unwrap_or_else(|e| panic!("{name}: {e:?}"));
        let m = d.metrics;
        assert_eq!(m.node_overlaps, 0, "{name}: {m}");
        assert_eq!(m.group_overlaps, 0, "{name}: {m}");
        assert_eq!(m.edges_through_groups, 0, "{name}: {m}");
        // Same input, same output.
        assert_eq!(
            d.to_json(),
            from_mermaid(&source).unwrap().to_json(),
            "{name}"
        );
        // The output is a valid spec for the core module.
        let spec = facetry_core::parse_spec_json(&d.to_json()).unwrap();
        assert!(facetry_core::validate(&spec).is_empty(), "{name}");
        for e in &d.spec.edges {
            assert!(e.points.len() >= 2, "{name}");
            assert!(e.points.iter().flatten().all(|v| v.is_finite()), "{name}");
        }
    }
}

#[test]
fn quality_metrics() {
    let mut report = String::new();
    let mut total = [0usize; 4];
    for (name, source) in corpus() {
        let m = from_mermaid(&source).unwrap().metrics;
        writeln!(report, "{name:<26} {m}").unwrap();
        total[0] += m.crossings;
        total[1] += m.edge_node_overlaps;
        total[2] += m.label_overlaps;
        total[3] += m.bends;
    }
    writeln!(
        report,
        "TOTAL crossings {} edge-node overlaps {} label overlaps {} bends {}",
        total[0], total[1], total[2], total[3]
    )
    .unwrap();
    insta::assert_snapshot!(report);
}

#[test]
fn every_diagram_routes_square_cornered_in_the_glossy_style() {
    for (name, source) in corpus() {
        let chart = flowchart::parse(&source).unwrap();
        let built = build::build(&chart, &GLOSSY);
        // The isometric and schematic styles route the same way.
        for style in [DiagramStyle::Isometric, DiagramStyle::Schematic] {
            let other = build::build(
                &chart,
                &Options {
                    style,
                    ..Options::default()
                },
            );
            assert_eq!(other.control, built.control, "{name}");
            assert_eq!(other.spec.style, style);
            let json = from_mermaid_with(
                &source,
                &Options {
                    style,
                    ..Options::default()
                },
            )
            .unwrap()
            .to_json();
            let spec = facetry_core::parse_spec_json(&json).unwrap();
            assert!(facetry_core::validate(&spec).is_empty(), "{name}");
        }
        for (i, c) in built.control.iter().enumerate() {
            for w in c.windows(2) {
                let (dx, dy) = ((w[0][0] - w[1][0]).abs(), (w[0][1] - w[1][1]).abs());
                assert!(
                    dx < 1e-6 || dy < 1e-6,
                    "{name}: edge {i} has a slanted segment {w:?}"
                );
            }
        }
        let d = from_mermaid_with(&source, &GLOSSY).unwrap();
        assert_eq!(d.metrics.node_overlaps, 0, "{name}");
        assert_eq!(d.metrics.group_overlaps, 0, "{name}");
        assert_eq!(d.metrics.edges_through_groups, 0, "{name}");
        assert_eq!(
            d.to_json(),
            from_mermaid_with(&source, &GLOSSY).unwrap().to_json()
        );
        let spec = facetry_core::parse_spec_json(&d.to_json()).unwrap();
        assert!(facetry_core::validate(&spec).is_empty(), "{name}");
        assert_eq!(spec.diagram.unwrap().style, DiagramStyle::Glossy);
    }
}

#[test]
fn glossy_quality_metrics() {
    let mut report = String::new();
    let mut total = [0usize; 3];
    for (name, source) in corpus() {
        let m = from_mermaid_with(&source, &GLOSSY).unwrap().metrics;
        writeln!(report, "{name:<26} {m}").unwrap();
        total[0] += m.crossings;
        total[1] += m.edge_node_overlaps;
        total[2] += m.label_overlaps;
    }
    writeln!(
        report,
        "TOTAL crossings {} edge-node overlaps {} label overlaps {}",
        total[0], total[1], total[2]
    )
    .unwrap();
    insta::assert_snapshot!(report);
}

#[test]
fn cards_take_room_of_their_own() {
    let mut report = String::new();
    for (name, source) in corpus() {
        let d = from_mermaid_with(&source, &CARDS).unwrap();
        let m = d.metrics;
        // Cards never overlap a node or another card, and no edge runs over one.
        assert_eq!(m.node_overlaps, 0, "{name}: {m}");
        assert_eq!(m.group_overlaps, 0, "{name}: {m}");
        assert_eq!(m.edges_through_groups, 0, "{name}: {m}");
        assert_eq!(m.edge_node_overlaps, 0, "{name}: {m}");
        let spec = facetry_core::parse_spec_json(&d.to_json()).unwrap();
        assert!(facetry_core::validate(&spec).is_empty(), "{name}");
        // Every node has its card and shows its number.
        for (i, n) in d.spec.nodes.iter().enumerate() {
            assert!(n.card.is_some(), "{name}");
            assert_eq!(n.label.as_deref(), Some((i + 1).to_string().as_str()));
        }
        writeln!(report, "{name:<26} {m}").unwrap();
    }
    insta::assert_snapshot!(report);
}

#[test]
fn the_users_icons_travel_in_the_spec_and_draw() {
    use facetry_core::spec::{CustomIcon, FillRule, IconFill};
    let icons = vec![
        (
            "a b/c".to_owned(),
            CustomIcon {
                stroke: Some("M0 0 L48 24".into()),
                fill: Vec::new(),
                view_box: Some([0.0, 0.0, 48.5, 24.25]),
            },
        ),
        (
            "pump".to_owned(),
            CustomIcon {
                stroke: None,
                fill: vec![
                    IconFill {
                        d: "M2 2h20v20H2z M8 8v8h8V8z".into(),
                        fill_rule: FillRule::EvenOdd,
                    },
                    IconFill {
                        d: "M10 10h4v4h-4z".into(),
                        fill_rule: FillRule::NonZero,
                    },
                ],
                view_box: None,
            },
        ),
    ];
    let options = Options {
        style: DiagramStyle::Schematic,
        icons,
        ..Options::default()
    };
    let d = from_mermaid_with("flowchart LR\n  a[A]@{ icon: pump } --> b", &options).unwrap();
    let spec = facetry_core::parse_spec_json(&d.to_json()).unwrap();
    assert!(facetry_core::validate(&spec).is_empty());
    let diagram = spec.diagram.unwrap();
    // Unchanged, the viewBox exactly (not rounded like layout numbers), awkward names too.
    assert_eq!(diagram.icons, options.icons);
    assert_eq!(diagram.nodes[0].icon.as_deref(), Some("pump"));
}

#[test]
fn a_theme_travels_in_the_spec_and_the_text_s_own_wins() {
    let theme = |source: &str, given: DiagramTheme| {
        let options = Options {
            theme: given,
            ..Options::default()
        };
        let d = from_mermaid_with(source, &options).unwrap();
        let spec = facetry_core::parse_spec_json(&d.to_json()).unwrap();
        assert!(facetry_core::validate(&spec).is_empty());
        spec.diagram.unwrap().theme
    };
    let plain = "flowchart LR\n  a --> b";
    assert_eq!(theme(plain, DiagramTheme::Default), DiagramTheme::Default);
    for given in [
        DiagramTheme::Neutral,
        DiagramTheme::Dark,
        DiagramTheme::Forest,
    ] {
        assert_eq!(theme(plain, given), given);
    }
    let own = "%%{init: {'theme': 'forest'}}%%\nflowchart LR\n  a --> b";
    assert_eq!(theme(own, DiagramTheme::Dark), DiagramTheme::Forest);
}

#[test]
fn flow_animates_every_edge_unless_the_text_says_otherwise() {
    use facetry_core::spec::EdgeAnimation;
    let animations = |source: &str, flow: Option<EdgeAnimation>| {
        let options = Options {
            flow,
            ..Options::default()
        };
        let d = from_mermaid_with(source, &options).unwrap();
        let spec = facetry_core::parse_spec_json(&d.to_json()).unwrap();
        assert!(facetry_core::validate(&spec).is_empty());
        let edges = spec.diagram.unwrap().edges;
        edges.iter().map(|e| e.animation).collect::<Vec<_>>()
    };
    let source = "flowchart LR\n  a e1@--> b e2@--> c --> d\n  e1@{ animation: slow }\n  e2@{ animate: false }";
    assert_eq!(
        animations(source, None),
        [Some(EdgeAnimation::Slow), None, None]
    );
    assert_eq!(
        animations(source, Some(EdgeAnimation::Fast)),
        [Some(EdgeAnimation::Slow), None, Some(EdgeAnimation::Fast)]
    );
}
