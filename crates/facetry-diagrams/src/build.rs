//! A parsed flowchart -> a laid-out [`DiagramSpec`]: node sizes from the label metrics,
//! layered layout in the flowchart's direction, and edges routed between the node outlines.

use facetry_core::diagram::shape_outline;
use facetry_core::spec::{
    DEFAULT_DIAGRAM_FONT_SIZE, DiagramEdge, DiagramNode, DiagramSpec, DiagramStyle, EdgeEnd,
    NodeShape,
};
use facetry_font::FontMetrics;

use crate::flowchart::{Direction, Flowchart};
use crate::layout::{self, Graph, Size, Spacing};
use crate::orthogonal;
use crate::route;

/// How to lay a diagram out and draw it.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Options {
    pub style: DiagramStyle,
}

/// Spacing for the glossy and isometric styles: their thick edges need longer runs between
/// ranks.
const GLOSSY_SPACING: Spacing = Spacing {
    node: 50.0,
    edge: 24.0,
    rank: 90.0,
};
/// Corner radius of a square-cornered edge.
const CORNER_RADIUS: f64 = 12.0;

/// Labels are measured at this size; the spec's `fontSize` says so to the renderer.
pub const FONT: f64 = DEFAULT_DIAGRAM_FONT_SIZE;
/// Line spacing of multi-line labels (the renderer uses the same).
const LINE_HEIGHT: f64 = 1.25;
/// Room round a label inside its node.
const PAD_X: f64 = 18.0;
const PAD_Y: f64 = 13.0;
/// Edge labels are drawn smaller than node labels (the renderer's `EDGE_LABEL_SCALE`).
const EDGE_LABEL_SCALE: f64 = 0.9;
/// Room round an edge label.
const EDGE_LABEL_PAD: f64 = 4.0;
/// Narrowest node.
const MIN_WIDTH: f64 = 40.0;

/// A laid-out flowchart: the spec, and the control polylines of the edges (through the
/// layout's dummy nodes, before smoothing) for the quality metrics.
#[derive(Debug, Clone)]
pub struct Built {
    pub spec: DiagramSpec,
    pub control: Vec<Vec<[f64; 2]>>,
}

/// Width and height of a label's text block, layout units.
fn text_box(label: &str, size: f64) -> (f64, f64) {
    let font = FontMetrics::inter();
    let lines: Vec<&str> = label.split('\n').collect();
    let width = lines
        .iter()
        .map(|l| f64::from(font.text_width(l, size as f32)))
        .fold(0.0, f64::max);
    let height = (lines.len() as f64 - 1.0) * LINE_HEIGHT * size + size;
    (width, height)
}

/// The footprint a node needs so its label fits inside its shape.
pub fn node_size(shape: NodeShape, label: &str) -> (f64, f64) {
    let (tw, th) = text_box(label, FONT);
    let (w, h) = match shape {
        NodeShape::Rect | NodeShape::Rounded => (tw + 2.0 * PAD_X, th + 2.0 * PAD_Y),
        NodeShape::Stadium => {
            let h = th + 2.0 * PAD_Y;
            (tw + PAD_X + h, h)
        }
        NodeShape::Cylinder => {
            // An ellipse holds a centred rectangle when both are scaled by sqrt(2).
            let k = std::f64::consts::SQRT_2;
            ((tw + 16.0) * k, (th + 10.0) * k)
        }
        NodeShape::Circle => {
            let d = (tw + 16.0).hypot(th + 12.0);
            (d, d)
        }
        NodeShape::Diamond => {
            // A rhombus W x H holds a centred w x h box when w / W + h / H <= 1.
            let (bw, bh) = (tw + 12.0, th + 10.0);
            let h = (2.0 * bh).max((bw + bh) * 0.5);
            (bw / (1.0 - bh / h), h)
        }
        NodeShape::Hexagon
        | NodeShape::Parallelogram
        | NodeShape::ParallelogramAlt
        | NodeShape::Trapezoid
        | NodeShape::TrapezoidAlt => {
            let h = th + 2.0 * PAD_Y;
            (tw + 16.0 + h, h)
        }
    };
    (
        w.max(MIN_WIDTH)
            .max(if shape == NodeShape::Circle { 0.0 } else { h }),
        h,
    )
}

/// Lays out `chart`.
pub fn build(chart: &Flowchart, options: &Options) -> Built {
    // The glossy and isometric styles draw solid edges on the floor, routed at right angles.
    let square_routes = options.style != DiagramStyle::Flat;
    let sizes: Vec<(f64, f64)> = chart
        .nodes
        .iter()
        .map(|n| node_size(n.shape, &n.label))
        .collect();
    let sideways = matches!(chart.direction, Direction::LeftRight | Direction::RightLeft);
    // The layout runs top to bottom: rotate sizes going in, points coming out.
    let rotate_size = |(w, h): (f64, f64)| {
        if sideways {
            Size {
                width: h,
                height: w,
            }
        } else {
            Size {
                width: w,
                height: h,
            }
        }
    };
    let graph = Graph {
        nodes: sizes.iter().copied().map(rotate_size).collect(),
        edges: chart
            .edges
            .iter()
            .map(|e| layout::Edge {
                from: e.from,
                to: e.to,
                minlen: e.length,
                weight: 1.0,
                label: e.label.as_deref().map(|l| {
                    let (w, h) = text_box(l, FONT * EDGE_LABEL_SCALE);
                    rotate_size((w + 2.0 * EDGE_LABEL_PAD, h + 2.0 * EDGE_LABEL_PAD))
                }),
            })
            .collect(),
    };
    let laid = layout::layout(
        &graph,
        if square_routes {
            GLOSSY_SPACING
        } else {
            Spacing::default()
        },
    );
    let pairs: Vec<(usize, usize)> = chart.edges.iter().map(|e| (e.from, e.to)).collect();
    let square = square_routes.then(|| orthogonal::route(&laid, &graph.nodes, &pairs));
    let out = |p: [f64; 2]| match chart.direction {
        Direction::TopBottom => p,
        Direction::BottomTop => [p[0], -p[1]],
        Direction::LeftRight => [p[1], p[0]],
        Direction::RightLeft => [-p[1], p[0]],
    };
    let centres: Vec<[f64; 2]> = laid.nodes.iter().map(|&p| out(p)).collect();
    let outlines: Vec<Vec<[f64; 2]>> = chart
        .nodes
        .iter()
        .zip(&sizes)
        .zip(&centres)
        .map(|((n, &(w, h)), c)| {
            shape_outline(n.shape, w, h)
                .0
                .into_iter()
                .map(|p| [c[0] + p[0], c[1] + p[1]])
                .collect()
        })
        .collect();

    let mut control = Vec::with_capacity(chart.edges.len());
    let mut edges = Vec::with_capacity(chart.edges.len());
    for (i, (e, r)) in chart.edges.iter().zip(&laid.edges).enumerate() {
        let (points, drawn) = if let Some(square) = &square {
            // Square corners: from port to port, the ends fitted to the outlines along the
            // first and last segments, then the corners rounded.
            let s = &square[i];
            let mut points: Vec<[f64; 2]> = s.points.iter().map(|&p| out(p)).collect();
            let n = points.len();
            points[0] = route::clip(&outlines[e.from], out(s.start_inside), points[1]);
            points[n - 1] = route::clip(&outlines[e.to], out(s.end_inside), points[n - 2]);
            let drawn = orthogonal::round_corners(&points, CORNER_RADIUS);
            (points, drawn)
        } else {
            let mut points: Vec<[f64; 2]> = r.points.iter().map(|&p| out(p)).collect();
            if e.from == e.to {
                let (w, h) = sizes[e.from];
                points = route::self_loop(centres[e.from], w, h, !sideways);
            }
            // Leave and enter through the outlines, toward the neighbouring control point.
            let n = points.len();
            let first = route::clip(&outlines[e.from], centres[e.from], points[1.min(n - 1)]);
            let last = route::clip(&outlines[e.to], centres[e.to], points[n.saturating_sub(2)]);
            points[0] = first;
            points[n - 1] = last;
            let smooth = route::basis(&points);
            (points, smooth)
        };
        control.push(points);
        let label_position = match (&e.label, r.label) {
            (Some(_), Some(p)) => Some(out(p)),
            (Some(_), None) if e.from == e.to => {
                // Beyond the loop.
                let (w, h) = sizes[e.from];
                let c = centres[e.from];
                let (tw, th) = text_box(e.label.as_deref().unwrap_or(""), FONT * EDGE_LABEL_SCALE);
                Some(if sideways {
                    [c[0], c[1] + h / 2.0 + 30.0 + th / 2.0]
                } else {
                    [c[0] + w / 2.0 + 30.0 + tw / 2.0, c[1]]
                })
            }
            _ => None,
        };
        edges.push(DiagramEdge {
            from: chart.nodes[e.from].id.clone(),
            to: chart.nodes[e.to].id.clone(),
            points: drawn.iter().map(|&p| round(p)).collect(),
            label: e.label.clone(),
            label_position: label_position.map(round),
            stroke: e.stroke,
            head: (e.head != EdgeEnd::Arrow).then_some(e.head),
            tail: (e.tail != EdgeEnd::None).then_some(e.tail),
        });
    }

    // Shift everything so the drawing starts at the origin.
    let mut lo = [f64::INFINITY; 2];
    for ((c, &(w, h)), _) in centres.iter().zip(&sizes).zip(&chart.nodes) {
        lo[0] = lo[0].min(c[0] - w / 2.0);
        lo[1] = lo[1].min(c[1] - h / 2.0);
    }
    for e in &edges {
        for p in e.points.iter().chain(&e.label_position) {
            lo[0] = lo[0].min(p[0]);
            lo[1] = lo[1].min(p[1]);
        }
    }
    if !lo[0].is_finite() {
        lo = [0.0; 2];
    }
    let shift = |p: [f64; 2]| round([p[0] - lo[0], p[1] - lo[1]]);
    for e in &mut edges {
        e.points.iter_mut().for_each(|p| *p = shift(*p));
        if let Some(p) = &mut e.label_position {
            *p = shift(*p);
        }
    }
    for c in &mut control {
        c.iter_mut().for_each(|p| *p = [p[0] - lo[0], p[1] - lo[1]]);
    }
    let nodes = chart
        .nodes
        .iter()
        .zip(&sizes)
        .zip(&centres)
        .map(|((n, &(w, h)), &c)| {
            let [x, y] = shift(c);
            DiagramNode {
                id: n.id.clone(),
                label: (n.label != n.id).then(|| n.label.clone()),
                shape: n.shape,
                x,
                y,
                width: round1(w),
                height: round1(h),
                color: n.style.fill.clone(),
                stroke: n.style.stroke.clone(),
                stroke_width: n.style.stroke_width,
                text_color: n.style.color.clone(),
            }
        })
        .collect();
    Built {
        spec: DiagramSpec {
            nodes,
            edges,
            font_size: Some(FONT),
            style: options.style,
        },
        control,
    }
}

/// To a tenth of a unit: far below a pixel, and short in JSON.
fn round1(v: f64) -> f64 {
    (v * 10.0).round() / 10.0
}

fn round(p: [f64; 2]) -> [f64; 2] {
    [round1(p[0]), round1(p[1])]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flowchart::parse;

    fn build_flat(chart: &Flowchart) -> Built {
        build(chart, &Options::default())
    }

    #[test]
    fn labels_fit_their_nodes() {
        for shape in [
            NodeShape::Rect,
            NodeShape::Rounded,
            NodeShape::Stadium,
            NodeShape::Cylinder,
            NodeShape::Circle,
            NodeShape::Diamond,
            NodeShape::Hexagon,
            NodeShape::Parallelogram,
            NodeShape::Trapezoid,
        ] {
            for label in [
                "A",
                "Is it valid?",
                "A much longer label here",
                "two\nlines",
            ] {
                let (w, h) = node_size(shape, label);
                let (tw, th) = text_box(label, FONT);
                // The text box's corners are inside the outline.
                let (outline, _) = shape_outline(shape, w, h);
                for corner in [[tw / 2.0, th / 2.0], [-tw / 2.0, -th / 2.0]] {
                    let n = outline.len();
                    let inside = (0..n).all(|i| {
                        let (a, b) = (outline[i], outline[(i + 1) % n]);
                        (b[0] - a[0]) * (corner[1] - a[1]) - (b[1] - a[1]) * (corner[0] - a[0])
                            >= -1e-6
                    });
                    assert!(inside, "{shape:?} {label:?} {w}x{h}");
                }
            }
        }
    }

    #[test]
    fn directions_rotate_the_drawing() {
        let at = |src: &str| {
            let b = build_flat(&parse(src).unwrap());
            let n = &b.spec.nodes;
            ([n[0].x, n[0].y], [n[1].x, n[1].y])
        };
        let (a, b) = at("flowchart TB\nA-->B");
        assert!(b[1] > a[1] && (a[0] - b[0]).abs() < 1e-6);
        let (a, b) = at("flowchart BT\nA-->B");
        assert!(b[1] < a[1]);
        let (a, b) = at("flowchart LR\nA-->B");
        assert!(b[0] > a[0] && (a[1] - b[1]).abs() < 1e-6);
        let (a, b) = at("flowchart RL\nA-->B");
        assert!(b[0] < a[0]);
    }

    #[test]
    fn edges_start_and_end_on_the_outlines() {
        let b =
            build_flat(&parse("flowchart TD\nA[Start] --> B{Choice}\nB -->|yes| C[(DB)]").unwrap());
        let n = |id: &str| b.spec.nodes.iter().find(|n| n.id == id).unwrap();
        for e in &b.spec.edges {
            let (from, to) = (n(&e.from), n(&e.to));
            let first = e.points[0];
            let last = *e.points.last().unwrap();
            // On the bottom of the source and the top of the target (to the rounding).
            assert!(
                (first[1] - (from.y + from.height / 2.0)).abs() < 0.2,
                "{e:?}"
            );
            assert!((last[1] - (to.y - to.height / 2.0)).abs() < 0.2, "{e:?}");
        }
        assert!(b.spec.edges[1].label_position.is_some());
    }
}
