//! A parsed flowchart -> a laid-out [`DiagramSpec`]: node sizes from the label metrics,
//! layered layout in the flowchart's direction, and edges routed between the node outlines.

use facetry_core::diagram::{
    CARD_BADGE, CARD_TEXT, CARD_TEXT_WIDTH, ICON_GAP, ICON_SIZE, card_layout, shape_outline,
};
use facetry_core::spec::{
    CustomIcon, DEFAULT_DIAGRAM_FONT_SIZE, DiagramEdge, DiagramGroup, DiagramNode, DiagramSpec,
    DiagramStyle, DiagramTheme, EdgeAnimation, EdgeEnd, NodeCard, NodeShape,
};
use facetry_font::FontMetrics;

use crate::flowchart::{Direction, End, Flowchart};
use crate::layout::{self, Graph, Size, Spacing};
use crate::orthogonal;
use crate::route;

/// How to lay a diagram out and draw it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Options {
    pub style: DiagramStyle,
    /// The colours, unless the text sets its own theme (a directive or front matter wins, as
    /// in Mermaid).
    pub theme: DiagramTheme,
    /// Every edge moves at this pace, unless the text animates (or stills) it itself.
    pub flow: Option<EdgeAnimation>,
    /// Names of the user's blocks, for `@{ shape: "<name>" }`; nodes name them in the spec,
    /// and the caller adds the blocks the spec's nodes use.
    pub blocks: Vec<String>,
    /// Isometric style only: a card on the floor beside each node, with the node's label (its
    /// first line as the title, the rest as text) and icon; the node then shows its number.
    pub cards: bool,
    /// Icons of the user's own, by name, for `@{ icon: "name" }` besides the built-in ones;
    /// those the diagram uses go into the spec with it.
    pub icons: Vec<(String, CustomIcon)>,
}

/// Cards: room between a pedestal and its card (past the pedestal's plate), and the smallest
/// pedestal (which only shows a number).
const CARD_GAP: f64 = 24.0;
const MIN_PEDESTAL: f64 = 84.0;

/// Spacing for the glossy and isometric styles: their thick edges need longer runs between
/// ranks.
const GLOSSY_SPACING: Spacing = Spacing {
    node: 50.0,
    edge: 24.0,
    rank: 90.0,
    group: 30.0,
};

/// Groups (subgraphs): room inside a group's box round what it holds, the title's size
/// against the labels', and the room under the title.
const GROUP_PAD: f64 = 16.0;
pub const GROUP_TITLE_SCALE: f64 = 0.9;
const GROUP_TITLE_GAP: f64 = 10.0;
/// The size of the invisible stand-in that gives an empty group its box.
const EMPTY_GROUP: (f64, f64) = (60.0, 24.0);
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
    node_size_at(shape, label, FONT)
}

/// [`node_size`] for a label printed at `font`.
fn node_size_at(shape: NodeShape, label: &str, font: f64) -> (f64, f64) {
    let (tw, th) = text_box(label, font);
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

/// Words wrapped into lines no wider than `width` at `size`.
fn wrap(text: &str, width: f64, size: f64) -> String {
    let font = FontMetrics::inter();
    let measure = |s: &str| f64::from(font.text_width(s, size as f32));
    let mut lines: Vec<String> = Vec::new();
    for word in text.split_whitespace() {
        match lines.last_mut() {
            Some(line) if measure(&format!("{line} {word}")) <= width => {
                line.push(' ');
                line.push_str(word);
            }
            _ => lines.push(word.to_owned()),
        }
    }
    lines.join("\n")
}

/// A node's card before it is placed: title, wrapped text, size (final frame).
struct CardContent {
    title: String,
    text: Option<String>,
    size: (f64, f64),
}

/// Lays out `chart`.
pub fn build(chart: &Flowchart, options: &Options) -> Built {
    // The glossy and isometric styles draw solid edges on the floor, routed at right angles.
    let square_routes = options.style != DiagramStyle::Flat;
    let with_cards = options.cards && options.style == DiagramStyle::Isometric;
    // What each node shows on itself: its label, or with cards its number.
    let shown: Vec<String> = chart
        .nodes
        .iter()
        .enumerate()
        .map(|(i, n)| {
            if with_cards {
                (i + 1).to_string()
            } else {
                n.label.clone()
            }
        })
        .collect();
    let sizes: Vec<(f64, f64)> = chart
        .nodes
        .iter()
        .zip(&shown)
        .map(|(n, label)| {
            if with_cards {
                let (w, h) = node_size_at(n.shape, label, FONT * CARD_BADGE);
                return (w.max(MIN_PEDESTAL), h.max(MIN_PEDESTAL));
            }
            let (w, h) = node_size(n.shape, label);
            if n.icon.is_none() {
                return (w, h);
            }
            // Room for the icon above the label.
            let extra = (ICON_SIZE + ICON_GAP) * FONT;
            let w = w.max(ICON_SIZE * FONT + 2.0 * PAD_X);
            if n.shape == NodeShape::Circle {
                let d = w.max(h + extra);
                (d, d)
            } else {
                (w, h + extra)
            }
        })
        .collect();
    let cards: Vec<Option<CardContent>> = chart
        .nodes
        .iter()
        .map(|n| {
            with_cards.then(|| {
                let mut lines = n.label.split('\n');
                let title = lines.next().unwrap_or("").to_owned();
                let rest: Vec<&str> = lines.collect();
                let text = (!rest.is_empty())
                    .then(|| wrap(&rest.join(" "), CARD_TEXT_WIDTH, FONT * CARD_TEXT));
                let layout = card_layout(
                    &title,
                    text.as_deref(),
                    n.icon.is_some(),
                    false,
                    FONT,
                    FontMetrics::inter(),
                );
                CardContent {
                    title,
                    text,
                    size: (layout.size[0], layout.size[1]),
                }
            })
        })
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
    // In the layout's frame a card sits beside its node, along the rank: the two take one
    // footprint.
    let footprint = |i: usize| {
        let node = rotate_size(sizes[i]);
        match &cards[i] {
            Some(card) => {
                let card = rotate_size(card.size);
                Size {
                    width: node.width + CARD_GAP + card.width,
                    height: node.height.max(card.height),
                }
            }
            None => node,
        }
    };
    // Groups: an invisible stand-in node gives an empty group its box, and an edge to a group
    // is laid out to one of its nodes (the first the text names) and cut at the box later.
    let mut layout_nodes: Vec<Size> = (0..chart.nodes.len()).map(footprint).collect();
    let mut node_group: Vec<Option<usize>> = chart.nodes.iter().map(|n| n.group).collect();
    let mut stand_in: Vec<Option<usize>> = vec![None; chart.subgraphs.len()];
    for (g, slot) in stand_in.iter_mut().enumerate() {
        let holds = |h: Option<usize>| h.is_some_and(|h| chart.within(h, g));
        if !node_group.iter().any(|&h| holds(h)) {
            *slot = Some(layout_nodes.len());
            layout_nodes.push(rotate_size(EMPTY_GROUP));
            node_group.push(Some(g));
        }
    }
    let representative = |end: End| match end {
        End::Node(i) => i,
        End::Group(g) => chart
            .nodes
            .iter()
            .position(|n| n.group.is_some_and(|h| chart.within(h, g)))
            .or(stand_in[g])
            .unwrap_or(0),
    };
    // A group's title is at the top of the drawing: in the layout's frame, before its first
    // rank (TB), after its last (BT), or along its left side (LR, RL).
    let title_room = |g: usize| {
        let (_, h) = text_box(&chart.subgraphs[g].title, FONT * GROUP_TITLE_SCALE);
        GROUP_PAD + h + GROUP_TITLE_GAP
    };
    let groups: Vec<layout::Group> = chart
        .subgraphs
        .iter()
        .enumerate()
        .map(|(g, sub)| {
            let t = title_room(g);
            let p = GROUP_PAD;
            let (before, after, left) = match chart.direction {
                Direction::TopBottom => (t, p, p),
                Direction::BottomTop => (p, t, p),
                Direction::LeftRight | Direction::RightLeft => (p, p, t),
            };
            layout::Group {
                parent: sub.parent,
                before,
                after,
                left,
                right: p,
            }
        })
        .collect();
    let graph = Graph {
        nodes: layout_nodes,
        groups,
        node_group,
        edges: chart
            .edges
            .iter()
            .map(|e| layout::Edge {
                from: representative(e.from),
                to: representative(e.to),
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
    // Where each node and its card are in the layout's frame: cards alternate sides rank by
    // rank (the zigzag of an infographic timeline).
    let mut node_at: Vec<[f64; 2]> = laid.nodes.clone();
    let mut card_at: Vec<Option<[f64; 2]>> = vec![None; chart.nodes.len()];
    for (i, card) in cards.iter().enumerate() {
        let Some(card) = card else { continue };
        let foot = graph.nodes[i];
        let (node, card) = (rotate_size(sizes[i]), rotate_size(card.size));
        let side = if (laid.ranks[i] / 2) % 2 == 0 {
            1.0
        } else {
            -1.0
        };
        let c = laid.nodes[i];
        node_at[i] = [c[0] - side * (foot.width - node.width) / 2.0, c[1]];
        card_at[i] = Some([c[0] + side * (foot.width - card.width) / 2.0, c[1]]);
    }
    let node_sizes: Vec<Size> = sizes.iter().copied().map(rotate_size).collect();
    // The routers see every layout node (stand-ins included) at its place and size.
    let mut route_at = node_at.clone();
    let mut route_sizes = node_sizes.clone();
    for (i, size) in graph.nodes.iter().enumerate().skip(chart.nodes.len()) {
        route_at.push(laid.nodes[i]);
        route_sizes.push(*size);
    }
    let ends: Vec<(usize, usize)> = chart
        .edges
        .iter()
        .map(|e| (representative(e.from), representative(e.to)))
        .collect();
    let square = square_routes.then(|| orthogonal::route(&laid, &route_at, &route_sizes, &ends));
    let out = |p: [f64; 2]| match chart.direction {
        Direction::TopBottom => p,
        Direction::BottomTop => [p[0], -p[1]],
        Direction::LeftRight => [p[1], p[0]],
        Direction::RightLeft => [-p[1], p[0]],
    };
    let centres: Vec<[f64; 2]> = route_at.iter().map(|&p| out(p)).collect();
    // Group boxes in the drawing's frame, `[left, top, right, bottom]`.
    let boxes: Vec<[f64; 4]> = laid
        .groups
        .iter()
        .map(|b| {
            let (a, c) = (out([b[0], b[1]]), out([b[2], b[3]]));
            [
                a[0].min(c[0]),
                a[1].min(c[1]),
                a[0].max(c[0]),
                a[1].max(c[1]),
            ]
        })
        .collect();
    let card_centres: Vec<Option<[f64; 2]>> = card_at.iter().map(|c| c.map(out)).collect();
    let mut route_sizes_out: Vec<(f64, f64)> = sizes.clone();
    route_sizes_out.extend(std::iter::repeat_n(
        EMPTY_GROUP,
        graph.nodes.len() - chart.nodes.len(),
    ));
    let shapes = chart
        .nodes
        .iter()
        .map(|n| n.shape)
        .chain(std::iter::repeat(NodeShape::Rect))
        .take(graph.nodes.len());
    let outlines: Vec<Vec<[f64; 2]>> = shapes
        .zip(&route_sizes_out)
        .zip(&centres)
        .map(|((shape, &(w, h)), c)| {
            shape_outline(shape, w, h)
                .0
                .into_iter()
                .map(|p| [c[0] + p[0], c[1] + p[1]])
                .collect()
        })
        .collect();

    let mut control = Vec::with_capacity(chart.edges.len());
    let mut edges = Vec::with_capacity(chart.edges.len());
    for (i, (e, r)) in chart.edges.iter().zip(&laid.edges).enumerate() {
        let (from, to) = ends[i];
        let (points, drawn) = if let Some(square) = &square {
            // Square corners: from port to port, the ends fitted to the outlines along the
            // first and last segments, then the corners rounded.
            let s = &square[i];
            let mut points: Vec<[f64; 2]> = s.points.iter().map(|&p| out(p)).collect();
            let n = points.len();
            points[0] = route::clip(&outlines[from], out(s.start_inside), points[1]);
            points[n - 1] = route::clip(&outlines[to], out(s.end_inside), points[n - 2]);
            let points = cut_at_groups(points, e.from, e.to, &boxes);
            let drawn = orthogonal::round_corners(&points, CORNER_RADIUS);
            (points, drawn)
        } else {
            let mut points: Vec<[f64; 2]> = r.points.iter().map(|&p| out(p)).collect();
            if from == to {
                let (w, h) = route_sizes_out[from];
                points = route::self_loop(centres[from], w, h, !sideways);
            }
            // Leave and enter through the outlines, toward the neighbouring control point.
            let n = points.len();
            let first = route::clip(&outlines[from], centres[from], points[1.min(n - 1)]);
            let last = route::clip(&outlines[to], centres[to], points[n.saturating_sub(2)]);
            points[0] = first;
            points[n - 1] = last;
            let points = cut_at_groups(points, e.from, e.to, &boxes);
            let smooth = route::basis(&points);
            (points, smooth)
        };
        control.push(points);
        let label_position = match (&e.label, r.label) {
            (Some(_), Some(p)) => Some(out(p)),
            (Some(_), None) if from == to => {
                // Beyond the loop.
                let (w, h) = route_sizes_out[from];
                let c = centres[from];
                let (tw, th) = text_box(e.label.as_deref().unwrap_or(""), FONT * EDGE_LABEL_SCALE);
                Some(if sideways {
                    [c[0], c[1] + h / 2.0 + 30.0 + th / 2.0]
                } else {
                    [c[0] + w / 2.0 + 30.0 + tw / 2.0, c[1]]
                })
            }
            _ => None,
        };
        let id = |end: End| match end {
            End::Node(n) => chart.nodes[n].id.clone(),
            End::Group(g) => chart.subgraphs[g].id.clone(),
        };
        edges.push(DiagramEdge {
            from: id(e.from),
            to: id(e.to),
            points: drawn.iter().map(|&p| round(p)).collect(),
            label: e.label.clone(),
            label_position: label_position.map(round),
            stroke: e.stroke,
            head: (e.head != EdgeEnd::Arrow).then_some(e.head),
            tail: (e.tail != EdgeEnd::None).then_some(e.tail),
            animation: e.animation.unwrap_or(options.flow),
            color: e.style.stroke.clone(),
            width: e.style.stroke_width,
            label_color: e.style.color.clone(),
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
    for (c, card) in card_centres.iter().zip(&cards) {
        if let (Some(c), Some(card)) = (c, card) {
            lo[0] = lo[0].min(c[0] - card.size.0 / 2.0);
            lo[1] = lo[1].min(c[1] - card.size.1 / 2.0);
        }
    }
    for b in &boxes {
        lo[0] = lo[0].min(b[0]);
        lo[1] = lo[1].min(b[1]);
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
        .enumerate()
        .map(|(i, ((n, &(w, h)), &c))| {
            let [x, y] = shift(c);
            let card = match (&cards[i], card_centres[i]) {
                (Some(card), Some(at)) => {
                    let [cx, cy] = shift(at);
                    Some(NodeCard {
                        x: cx,
                        y: cy,
                        width: round1(card.size.0),
                        height: round1(card.size.1),
                        title: card.title.clone(),
                        text: card.text.clone(),
                    })
                }
                _ => None,
            };
            DiagramNode {
                id: n.id.clone(),
                label: (shown[i] != n.id).then(|| shown[i].clone()),
                icon: n.icon.clone(),
                block: n.block.clone(),
                card,
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
    let groups = chart
        .subgraphs
        .iter()
        .zip(&boxes)
        .map(|(sub, b)| {
            let [x, y] = shift([(b[0] + b[2]) / 2.0, (b[1] + b[3]) / 2.0]);
            DiagramGroup {
                id: sub.id.clone(),
                label: (sub.title != sub.id).then(|| sub.title.clone()),
                x,
                y,
                width: round1(b[2] - b[0]),
                height: round1(b[3] - b[1]),
                parent: sub.parent.map(|p| chart.subgraphs[p].id.clone()),
                color: sub.style.fill.clone(),
                stroke: sub.style.stroke.clone(),
                text_color: sub.style.color.clone(),
            }
        })
        .collect();
    Built {
        spec: DiagramSpec {
            nodes,
            edges,
            font_size: Some(FONT),
            style: options.style,
            theme: chart.theme.unwrap_or(options.theme),
            // The text's colours; the caller lays its own palette under them.
            palette: chart.palette.clone(),
            // Only the icons the nodes use: a whole icon set may have been given.
            icons: options
                .icons
                .iter()
                .filter(|(name, _)| chart.nodes.iter().any(|n| n.icon.as_ref() == Some(name)))
                .cloned()
                .collect(),
            groups,
            // Blocks are named here; the caller brings their parts (they draw in the core).
            blocks: Vec::new(),
        },
        control,
    }
}

/// An edge to or from a group ends at the group's box: the part inside it (laid out to one of
/// the group's nodes) is cut off where the path first meets the box.
fn cut_at_groups(
    mut points: Vec<[f64; 2]>,
    from: End,
    to: End,
    boxes: &[[f64; 4]],
) -> Vec<[f64; 2]> {
    if let End::Group(g) = to {
        points = cut_into(&points, boxes[g]);
    }
    if let End::Group(g) = from {
        points.reverse();
        points = cut_into(&points, boxes[g]);
        points.reverse();
    }
    points
}

/// `points` up to where they first enter box `b`.
fn cut_into(points: &[[f64; 2]], b: [f64; 4]) -> Vec<[f64; 2]> {
    let inside = |p: [f64; 2]| p[0] > b[0] && p[0] < b[2] && p[1] > b[1] && p[1] < b[3];
    let outline = [[b[0], b[1]], [b[2], b[1]], [b[2], b[3]], [b[0], b[3]]];
    let mut out = vec![points[0]];
    for w in points.windows(2) {
        if inside(w[1]) {
            // From inside the box toward where we came from: where the path leaves the box.
            out.push(route::clip(&outline, w[1], w[0]));
            return out;
        }
        out.push(w[1]);
    }
    out
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
