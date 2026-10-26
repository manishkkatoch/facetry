//! Mermaid diagrams for Facetry. Parses a subset of Mermaid and lays it out into a diagram
//! spec ([`facetry_core::spec::DiagramSpec`]) that the core module draws in 3D. Pure CPU
//! work: no GPU, no browser, so it is unit-tested natively and ships as its own small wasm
//! module that only pages with diagrams download.
//!
//! Supported so far: `flowchart` (see [`flowchart`] for the exact subset). Sequence and
//! architecture diagrams come next; anything else is an error that says so.

pub mod build;
pub mod flowchart;
pub mod issue;
pub mod json;
pub mod layout;
pub mod metrics;
pub mod route;

use facetry_core::spec::DiagramSpec;

pub use issue::DiagramIssue;
pub use metrics::Metrics;

/// A laid-out diagram and how good its layout is.
#[derive(Debug, Clone)]
pub struct Diagram {
    pub spec: DiagramSpec,
    pub metrics: Metrics,
}

impl Diagram {
    /// The whole chart spec, as JSON, for `Chart.create`.
    pub fn to_json(&self) -> String {
        json::chart_spec_json(&self.spec)
    }
}

/// Parses Mermaid text and lays it out. On failure, every problem found, each with its line
/// and column; nothing is laid out.
pub fn from_mermaid(source: &str) -> Result<Diagram, Vec<DiagramIssue>> {
    match flowchart::kind(source).map_err(|i| vec![i])? {
        flowchart::Kind::Flowchart => {}
        kind => {
            let name = match kind {
                flowchart::Kind::Sequence => "sequenceDiagram",
                _ => "architecture-beta",
            };
            let text: Vec<char> = source.chars().collect();
            let at = source.find(name).map_or(0, |b| source[..b].chars().count());
            return Err(vec![issue::Lines::new(&text).issue(
                at,
                format!("`{name}` diagrams are not supported yet; only `flowchart` for now"),
            )]);
        }
    }
    let chart = flowchart::parse(source)?;
    let built = build::build(&chart);
    let metrics = metrics::measure(&built.spec, &built.control, build::FONT);
    Ok(Diagram {
        spec: built.spec,
        metrics,
    })
}
