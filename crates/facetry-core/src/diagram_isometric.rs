//! The isometric diagram style, after infographic timelines: every node is a layered
//! pedestal (a pale plate on the floor, a white body, a coloured cap, and a white inset on the
//! cap where the label is printed), nodes take the colours of a palette in turn, and edges are
//! light grey roads on the floor with chevrons that show which way they run. Long soft shadows
//! fall toward the viewer's right. Seen through an orthographic lens from 45 degrees round, the
//! classic isometric view; labels are printed on the pedestals and on cards on the roads, so
//! they read along the axes of the drawing like the text of a printed isometric chart.
//!
//! Roads lie flat on the floor and run under the pedestals they join, which hide their ends;
//! the chevrons along them, and one just before the node at a head, say where they go.

use crate::Color;
use crate::Theme;
use crate::diagram::{Plan, dashes, parse_color, path_length, push_rim, shape_outline};
use crate::diagram_glossy::{lighten, print, push_card, push_shadow_with, rgb};
use crate::scene3d::{PrismMark, RibbonMark, Scene3d};
use crate::spec::{DiagramEdge, DiagramNode, DiagramSpec, EdgeEnd, EdgeStroke, NodeShape};
use crate::text::TextMeasure;

/// The pedestal's layers, heights in font sizes: plate, body, cap and inset.
const PLATE_HEIGHT: f64 = 0.25;
const BODY_HEIGHT: f64 = 1.1;
const CAP_HEIGHT: f64 = 0.45;
const INSET_HEIGHT: f64 = 0.12;
/// The whole pedestal, font sizes (the plot box is this high).
pub(crate) const STACK_HEIGHT: f64 = PLATE_HEIGHT + BODY_HEIGHT + CAP_HEIGHT + INSET_HEIGHT;
/// The plate reaches this far beyond the node's outline, and the inset stops this far inside
/// it (layout units).
const PLATE_REACH: f64 = 9.0;
const INSET_MARGIN: f64 = 7.0;
/// Roads: width (and for `==>`), thickness, colour; chevrons: spacing, length, width.
const ROAD_WIDTH: f64 = 15.0;
const THICK_ROAD_WIDTH: f64 = 22.0;
const ROAD_HEIGHT: f64 = 0.8;
const CHEVRON_EVERY: f64 = 36.0;
const CHEVRON_LENGTH: f64 = 7.0;
const CHEVRON_WIDTH: f64 = 9.0;
const CHEVRON_HEIGHT: f64 = 0.6;
/// Dashes of a dotted road.
const DASH: f64 = 14.0;
const DASH_GAP: f64 = 8.0;
/// How far before its end a road's head marker sits (the pedestal hides the road's end).
const HEAD_SETBACK: f64 = 14.0;
/// Shadows: offset toward the viewer's right, blur, darkness. Layout units.
const SHADOW_OFFSET: [f64; 2] = [10.0, 8.0];
const SHADOW_BLUR: f64 = 24.0;
const SHADOW_ALPHA: f32 = 0.16;
/// Gloss of the cap, the inset and the body.
const CAP_GLOSS: f32 = 0.35;
const WHITE_GLOSS: f32 = 0.15;

/// Node colours, used in turn: the warm-to-cool sequence of infographic timelines.
const PALETTE: [u32; 8] = [
    0xf6b33f, 0xe8553e, 0x3cb371, 0x3d8fe0, 0x9b5cd6, 0x1fb5a8, 0xf0812f, 0x56677d,
];

fn plate_color() -> Color {
    rgb(0xe4e8ee)
}

fn body_color() -> Color {
    rgb(0xf3f5f8)
}

fn road_color() -> Color {
    rgb(0xdadfe6)
}

fn chevron_color() -> Color {
    rgb(0xa3adba)
}

/// `c` mixed toward black by `t`.
fn darken(c: Color, t: f32) -> Color {
    let [r, g, b, a] = c.to_array();
    let m = |v: f32| v * (1.0 - t);
    Color::linear(m(r), m(g), m(b), a)
}

pub(crate) fn build(
    diagram: &DiagramSpec,
    viewport_px: (u32, u32),
    dpr: f32,
    measure: &dyn TextMeasure,
    theme: &Theme,
) -> Scene3d {
    let s = f64::from(dpr).clamp(0.25, 16.0);
    let plan = Plan::new(diagram);
    let mut scene = Scene3d::empty(viewport_px, theme.background, plan.size());
    scene.halo_px = s.round().max(1.0) as f32;
    let shadow = Color::linear(0.0, 0.0, 0.0, SHADOW_ALPHA);
    // Roads first: they lie under everything.
    for (i, edge) in diagram.edges.iter().enumerate() {
        let known = |id: &str| diagram.nodes.iter().any(|n| n.id == id);
        if edge.points.len() < 2 || !known(&edge.from) || !known(&edge.to) {
            continue;
        }
        push_edge(&mut scene, &plan, i, edge, shadow, measure, theme);
    }
    for (i, node) in diagram.nodes.iter().enumerate() {
        push_node(&mut scene, &plan, i, node, s, shadow, theme.text);
    }
    scene
}

/// The outline of `node`'s shape grown (or shrunk, for a negative `by`) by `by` on every
/// side, absolute layout units.
fn outline(node: &DiagramNode, by: f64) -> (Vec<[f64; 2]>, bool) {
    let (w, h) = (
        (node.width + 2.0 * by).max(4.0),
        (node.height + 2.0 * by).max(4.0),
    );
    let (points, smooth) = shape_outline(node.shape, w, h);
    (
        points
            .iter()
            .map(|p| [node.x + p[0], node.y + p[1]])
            .collect(),
        smooth,
    )
}

fn push_node(
    scene: &mut Scene3d,
    plan: &Plan,
    index: usize,
    node: &DiagramNode,
    s: f64,
    shadow: Color,
    text: Color,
) {
    let fill = parse_color(&node.color).unwrap_or_else(|| rgb(PALETTE[index % PALETTE.len()]));
    let floor = |points: &[[f64; 2]]| points.iter().map(|&p| plan.floor_point(p)).collect();
    let layer =
        |key: &str, points: &[[f64; 2]], smooth, base: f64, height: f64, color, gloss| PrismMark {
            key: format!("{}/{key}", node.id),
            color,
            outline: floor(points),
            base: plan.len(base * plan.font) as f32,
            top: plan.len((base + height) * plan.font) as f32,
            smooth,
            gloss,
            node: index,
        };
    let (plate, smooth) = outline(node, PLATE_REACH);
    let (body, _) = outline(node, 0.0);
    let (inset, _) = outline(node, -INSET_MARGIN);
    push_shadow_with(scene, plan, &plate, SHADOW_OFFSET, SHADOW_BLUR, shadow);
    let body_base = PLATE_HEIGHT;
    let cap_base = body_base + BODY_HEIGHT;
    let inset_base = cap_base + CAP_HEIGHT;
    scene.decor.push(layer(
        "plate",
        &plate,
        smooth,
        0.0,
        PLATE_HEIGHT,
        plate_color(),
        0.0,
    ));
    scene.decor.push(layer(
        "body",
        &body,
        smooth,
        body_base,
        BODY_HEIGHT,
        body_color(),
        WHITE_GLOSS,
    ));
    // The cap is the node's mark: what hover lights and picking finds (on its top face).
    let mut cap = layer("cap", &body, smooth, cap_base, CAP_HEIGHT, fill, CAP_GLOSS);
    cap.key = node.id.clone();
    scene.prisms.push(cap);
    scene.decor.push(layer(
        "inset",
        &inset,
        smooth,
        inset_base,
        INSET_HEIGHT,
        Color::linear(1.0, 1.0, 1.0, 1.0),
        WHITE_GLOSS,
    ));
    // A light catch-light round the cap's top edge, or the node's own stroke.
    let (rim, width) = match parse_color(&node.stroke) {
        Some(c) => (c, node.stroke_width.unwrap_or(1.5)),
        None => (lighten(fill, 0.45), node.stroke_width.unwrap_or(1.0)),
    };
    let (relative, _) = shape_outline(node.shape, node.width, node.height);
    push_rim(
        scene,
        plan,
        node,
        &relative,
        plan.len(inset_base * plan.font),
        rim,
        width * s,
    );
    // The label goes on the white inset: in a deep shade of the node's colour, or the node's
    // own text colour if that reads on white (a `color:#fff` meant for a coloured fill would
    // vanish), or the theme's dark text if the fill is too pale to shade.
    let white = Color::linear(1.0, 1.0, 1.0, 1.0);
    let readable = |c: &Color| c.contrast(white) >= 3.0;
    let ink = parse_color(&node.text_color)
        .filter(readable)
        .or_else(|| Some(darken(fill, 0.35)).filter(readable))
        .unwrap_or(text);
    let label = node.label.as_deref().unwrap_or(&node.id);
    print(
        scene,
        plan,
        &node.id,
        label,
        [node.x, node.y],
        plan.len(STACK_HEIGHT * plan.font),
        plan.font,
        ink,
    );
}

fn road(plan: &Plan, key: String, points: &[[f64; 2]], width: f64) -> RibbonMark {
    RibbonMark {
        key,
        color: road_color(),
        path: points.iter().map(|&p| plan.floor_point(p)).collect(),
        width: plan.len(width) as f32,
        base: 0.0,
        top: plan.len(ROAD_HEIGHT) as f32,
        gloss: 0.0,
    }
}

/// The point at `distance` along a path and the unit direction there.
fn along(points: &[[f64; 2]], distance: f64) -> ([f64; 2], [f64; 2]) {
    let mut left = distance.max(0.0);
    for w in points.windows(2) {
        let (dx, dy) = (w[1][0] - w[0][0], w[1][1] - w[0][1]);
        let seg = dx.hypot(dy);
        if seg <= 1e-9 {
            continue;
        }
        if left <= seg {
            let t = left / seg;
            return ([w[0][0] + dx * t, w[0][1] + dy * t], [dx / seg, dy / seg]);
        }
        left -= seg;
    }
    let n = points.len();
    let (a, b) = (points[n.saturating_sub(2)], points[n - 1]);
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let l = dx.hypot(dy).max(1e-9);
    (b, [dx / l, dy / l])
}

/// A chevron painted on the road at `at`, pointing along `dir`, `scale` times the usual size.
fn chevron(
    plan: &Plan,
    key: String,
    at: [f64; 2],
    dir: [f64; 2],
    scale: f64,
    color: Color,
) -> PrismMark {
    let (l, w) = (CHEVRON_LENGTH * scale, CHEVRON_WIDTH * scale / 2.0);
    let n = [-dir[1], dir[0]];
    let tip = [at[0] + dir[0] * l / 2.0, at[1] + dir[1] * l / 2.0];
    let back = [at[0] - dir[0] * l / 2.0, at[1] - dir[1] * l / 2.0];
    let outline = [
        tip,
        [back[0] + n[0] * w, back[1] + n[1] * w],
        [back[0] - n[0] * w, back[1] - n[1] * w],
    ];
    PrismMark {
        key,
        color,
        outline: outline.iter().map(|&p| plan.floor_point(p)).collect(),
        base: plan.len(ROAD_HEIGHT) as f32,
        top: plan.len(ROAD_HEIGHT + CHEVRON_HEIGHT) as f32,
        smooth: false,
        gloss: 0.0,
        node: 0,
    }
}

#[allow(clippy::too_many_arguments)]
fn push_edge(
    scene: &mut Scene3d,
    plan: &Plan,
    index: usize,
    edge: &DiagramEdge,
    shadow: Color,
    measure: &dyn TextMeasure,
    theme: &Theme,
) {
    let key = format!("edge{index}");
    let (tail, head) = edge.ends();
    let width = match edge.stroke {
        EdgeStroke::Thick => THICK_ROAD_WIDTH,
        _ => ROAD_WIDTH,
    };
    let path = edge.points.clone();
    let runs = match edge.stroke {
        EdgeStroke::Dotted => dashes(&path, DASH, DASH_GAP),
        _ => vec![path.clone()],
    };
    for (k, run) in runs.iter().enumerate() {
        if run.len() >= 2 {
            scene
                .ribbons
                .push(road(plan, format!("{key}/{k}"), run, width));
        }
    }
    // Chevrons along the way, pointing toward the head (both ways on a two-headed edge, none
    // on an edge with no heads at all).
    let total = path_length(&path);
    let forward = head != EdgeEnd::None;
    let backward = tail != EdgeEnd::None;
    if forward || backward {
        let count = ((total - 2.0 * HEAD_SETBACK) / CHEVRON_EVERY)
            .floor()
            .max(0.0) as usize;
        let start = (total - count.saturating_sub(1) as f64 * CHEVRON_EVERY) / 2.0;
        for k in 0..count {
            let (at, dir) = along(&path, start + k as f64 * CHEVRON_EVERY);
            let dir = if forward { dir } else { [-dir[0], -dir[1]] };
            scene.decor.push(chevron(
                plan,
                format!("{key}/chevron/{k}"),
                at,
                dir,
                1.0,
                chevron_color(),
            ));
        }
    }
    // Each end's marker, just short of the pedestal that hides the road's end.
    for (end, at_head) in [(head, true), (tail, false)] {
        let distance = if at_head {
            total - HEAD_SETBACK
        } else {
            HEAD_SETBACK
        };
        if total < 2.0 * HEAD_SETBACK {
            continue;
        }
        let (at, dir) = along(&path, distance);
        let dir = if at_head { dir } else { [-dir[0], -dir[1]] };
        let dark = darken(chevron_color(), 0.35);
        let name = if at_head { "head" } else { "tail" };
        match end {
            EdgeEnd::None => {}
            EdgeEnd::Arrow => {
                scene
                    .decor
                    .push(chevron(plan, format!("{key}/{name}"), at, dir, 1.4, dark))
            }
            EdgeEnd::Circle | EdgeEnd::Cross => {
                // A disc; a cross is a disc with a gap, which reads the same at this size
                // and keeps the road clean.
                let (disc, _) = shape_outline(NodeShape::Circle, 8.0, 8.0);
                scene.decor.push(PrismMark {
                    key: format!("{key}/{name}"),
                    color: if end == EdgeEnd::Circle {
                        dark
                    } else {
                        rgb(0xe8553e)
                    },
                    outline: disc
                        .iter()
                        .map(|p| plan.floor_point([at[0] + p[0], at[1] + p[1]]))
                        .collect(),
                    base: plan.len(ROAD_HEIGHT) as f32,
                    top: plan.len(ROAD_HEIGHT + CHEVRON_HEIGHT) as f32,
                    smooth: true,
                    gloss: 0.0,
                    node: 0,
                });
            }
        }
    }
    if let Some(text) = edge.label.as_deref().filter(|t| !t.is_empty()) {
        let at = edge
            .label_position
            .unwrap_or_else(|| crate::diagram::midpoint(&edge.points));
        push_card(
            scene,
            plan,
            &key,
            text,
            at,
            plan.len(ROAD_HEIGHT),
            shadow,
            measure,
            theme.text,
        );
    }
}
