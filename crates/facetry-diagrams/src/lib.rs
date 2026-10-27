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
pub mod orthogonal;
pub mod route;
pub mod style;

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

pub use build::Options;

/// Parses Mermaid text and lays it out in the flat style. On failure, every problem found,
/// each with its line and column; nothing is laid out.
pub fn from_mermaid(source: &str) -> Result<Diagram, Vec<DiagramIssue>> {
    from_mermaid_with(source, &Options::default())
}

/// [`from_mermaid`] with options (the style, which also decides how edges are routed).
pub fn from_mermaid_with(source: &str, options: &Options) -> Result<Diagram, Vec<DiagramIssue>> {
    let built = lay_out(source, options)?;
    let metrics = metrics::measure(&built.spec, &built.control, build::FONT);
    Ok(Diagram {
        spec: built.spec,
        metrics,
    })
}

/// The spec alone, as JSON for `Chart.create`: what the wasm module returns. The quality
/// metrics are not measured (they are for the tests), which keeps them out of the module.
pub fn chart_json(source: &str, options: &Options) -> Result<String, Vec<DiagramIssue>> {
    Ok(json::chart_spec_json(&lay_out(source, options)?.spec))
}

/// Parses and lays out.
fn lay_out(source: &str, options: &Options) -> Result<build::Built, Vec<DiagramIssue>> {
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
    let names: Vec<&str> = options.icons.iter().map(|(n, _)| n.as_str()).collect();
    let chart = flowchart::parse_with_icons(source, &names)?;
    Ok(build::build(&chart, options))
}
