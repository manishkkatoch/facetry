//! The golden corpus (`examples/diagrams`): every diagram must lay out, with hard guarantees
//! checked here (no overlapping nodes, edges that start and end on their nodes, the same
//! output every time), and its quality metrics are recorded in one snapshot so that any change
//! to the layout shows, diagram by diagram, what got better or worse.
#![cfg(not(target_arch = "wasm32"))]

use std::fmt::Write;
use std::path::PathBuf;

use facetry_core::spec::DiagramStyle;
use facetry_diagrams::{Options, build, flowchart, from_mermaid, from_mermaid_with};

const GLOSSY: Options = Options {
    style: DiagramStyle::Glossy,
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
