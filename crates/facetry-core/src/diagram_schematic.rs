//! The schematic diagram style, after isometric plant illustrations: every node is a piece of
//! equipment modelled after its shape (a building, a silo on its plinth, a tank, a valve, a
//! reactor, a drum), in pale greys with dark outlines; edges are pipes raised on supports, with
//! green flow marks along their tops, and thick edges conveyors; labels are signs standing on
//! posts above the equipment, printed upright and turned to the isometric view. A faint grid
//! on the floor and soft contact shadows finish the drawing.
//!
//! Every piece of equipment stands on a body at least as tall as the pipes, so a pipe always
//! ends in a wall.

use crate::Color;
use crate::Theme;
use crate::diagram::{
    ISOMETRIC_AZIMUTH, Plan, dashes, midpoint, parse_color, path_length, push_icon_shape,
};
use crate::diagram_glossy::{push_shadow_with, rgb};
use crate::diagram_isometric::{along, darken, outline};
use crate::scene::Anchor;
use crate::scene3d::{LabelRole, Polyline3, PrismMark, RibbonMark, Scene3d, Stand, Text3, VAlign};
use crate::spec::{DiagramEdge, DiagramNode, DiagramSpec, EdgeEnd, EdgeStroke, NodeShape};
use crate::text::TextMeasure;

/// Height of the plot box, in font sizes: the tallest equipment with its sign.
pub(crate) const SCENE_HEIGHT: f64 = 9.0;

/// Pipes: heights of their bottom and top (font sizes), width (layout units); conveyors the
/// same for thick edges.
const PIPE_BASE: f64 = 0.55;
const PIPE_TOP: f64 = 0.95;
const PIPE_WIDTH: f64 = 7.0;
const CONVEYOR_BASE: f64 = 0.8;
const CONVEYOR_TOP: f64 = 1.05;
const CONVEYOR_WIDTH: f64 = 16.0;
/// Dotted edges: dash and gap along the pipe.
const DASH: f64 = 12.0;
const DASH_GAP: f64 = 7.0;
/// Supports under a pipe: every so far along it, this wide, kept this far from its ends.
const SUPPORT_EVERY: f64 = 48.0;
const SUPPORT_WIDTH: f64 = 3.5;
const SUPPORT_CLEAR: f64 = 12.0;
/// Flow marks on a pipe's top: spacing, length and width, and how far before the end the
/// head's mark sits.
const FLOW_EVERY: f64 = 40.0;
const FLOW_LENGTH: f64 = 5.0;
const FLOW_WIDTH: f64 = 6.0;
const HEAD_SETBACK: f64 = 9.0;
/// How far below the pipe's top a flow mark starts and above it it ends (font sizes).
const FLOW_RISE: f64 = 0.12;

/// Signs: the post's height above the equipment and its width, the board's padding (layout
/// units), depth and the frame round it (font sizes), and line height (font sizes).
const SIGN_LIFT: f64 = 1.3;
const SIGN_POST: f64 = 2.2;
const SIGN_PAD: f64 = 6.0;
const SIGN_DEPTH: f64 = 0.22;
const SIGN_FRAME: f64 = 1.6;
const LINE_HEIGHT: f64 = 1.25;
/// Edge labels: a smaller sign on a shorter post.
const TAG_SCALE: f64 = 0.85;
const TAG_LIFT: f64 = 0.7;
/// An icon on a sign: its size and the gap after it (font sizes).
const SIGN_ICON: f64 = 1.3;
const SIGN_ICON_GAP: f64 = 0.4;

/// Width of the drawn edges of the equipment (pixels).
const EDGE_WIDTH: f64 = 1.4;
/// Floor grid spacing (layout units).
const GRID: f64 = 40.0;
/// Contact shadows: offset, blur (layout units) and opacity.
const SHADOW_OFFSET: [f64; 2] = [6.0, 5.0];
const SHADOW_BLUR: f64 = 12.0;
const SHADOW_ALPHA: f32 = 0.13;

fn body_color() -> Color {
    rgb(0xe9edf2)
}

fn steel_color() -> Color {
    rgb(0xf5f7fa)
}

fn dark_color() -> Color {
    rgb(0x5b6573)
}

fn outline_color() -> Color {
    rgb(0x46505d)
}

fn pipe_color() -> Color {
    rgb(0xb9c2cd)
}

fn conveyor_color() -> Color {
    rgb(0x737d8a)
}

fn support_color() -> Color {
    rgb(0x9aa4b1)
}

fn flow_color() -> Color {
    rgb(0x2fae5a)
}

fn grid_color() -> Color {
    rgb(0xe4e8ed)
}

fn ink_color() -> Color {
    rgb(0x1f2733)
}

/// One solid of a piece of equipment: a convex outline (absolute layout units) from `base` to
/// `top` (font sizes).
struct Part {
    name: &'static str,
    outline: Vec<[f64; 2]>,
    smooth: bool,
    base: f64,
    top: f64,
    color: Color,
}

/// A circle of `diameter` round `centre`, layout units.
fn circle(centre: [f64; 2], diameter: f64) -> Vec<[f64; 2]> {
    let (points, _) = crate::diagram::shape_outline(NodeShape::Circle, diameter, diameter);
    points
        .iter()
        .map(|p| [centre[0] + p[0], centre[1] + p[1]])
        .collect()
}

/// The equipment `node` is drawn as. The first part is its body: the node's mark, with the
/// node's footprint, at least as tall as a pipe.
fn equipment(node: &DiagramNode, fill: Option<Color>) -> Vec<Part> {
    let body = fill.unwrap_or_else(body_color);
    let cap = darken(body, 0.08);
    let (footprint, smooth) = outline(node, 0.0);
    let centre = [node.x, node.y];
    let small = node.width.min(node.height);
    let part = |name, outline, smooth, base, top, color| Part {
        name,
        outline,
        smooth,
        base,
        top,
        color,
    };
    let shrunk = |by: f64| outline(node, -by).0;
    match node.shape {
        NodeShape::Rect | NodeShape::Rounded => {
            // A building with a roof slab and a unit on the roof.
            let unit = [node.x - node.width * 0.22, node.y - node.height * 0.12];
            let (uw, uh) = ((node.width * 0.22).max(6.0), (node.height * 0.3).max(6.0));
            let box_at = |c: [f64; 2], w: f64, h: f64| {
                vec![
                    [c[0] - w / 2.0, c[1] - h / 2.0],
                    [c[0] + w / 2.0, c[1] - h / 2.0],
                    [c[0] + w / 2.0, c[1] + h / 2.0],
                    [c[0] - w / 2.0, c[1] + h / 2.0],
                ]
            };
            vec![
                part("body", footprint, smooth, 0.0, 2.4, body),
                part("roof", shrunk(3.0), smooth, 2.4, 2.7, cap),
                part("unit", box_at(unit, uw, uh), false, 2.7, 3.3, steel_color()),
            ]
        }
        NodeShape::Cylinder => {
            // A silo on a square plinth, its roof in steps.
            let d = small * 0.85;
            let (plinth, _) =
                crate::diagram::shape_outline(NodeShape::Rect, node.width, node.height);
            let plinth = plinth
                .iter()
                .map(|p| [node.x + p[0], node.y + p[1]])
                .collect();
            vec![
                part("body", plinth, false, 0.0, 1.1, body),
                part("silo", circle(centre, d), true, 1.1, 4.4, steel_color()),
                part("roof", circle(centre, d * 0.78), true, 4.4, 4.75, cap),
                part("vent", circle(centre, d * 0.32), true, 4.75, 5.1, cap),
            ]
        }
        NodeShape::Circle => {
            // A tank with a band and a dome.
            let d = small;
            vec![
                part("body", footprint, true, 0.0, 2.1, steel_color()),
                part(
                    "band",
                    circle(centre, d + 3.0),
                    true,
                    0.9,
                    1.15,
                    dark_color(),
                ),
                part("dome", circle(centre, d * 0.7), true, 2.1, 2.4, cap),
                part("hatch", circle(centre, d * 0.3), true, 2.4, 2.65, cap),
            ]
        }
        NodeShape::Diamond => {
            // A valve: a block with a stem and a hand wheel.
            vec![
                part("body", footprint, smooth, 0.0, 1.2, body),
                part(
                    "stem",
                    circle(centre, (small * 0.12).max(3.0)),
                    true,
                    1.2,
                    2.3,
                    dark_color(),
                ),
                part(
                    "wheel",
                    circle(centre, small * 0.45),
                    true,
                    2.3,
                    2.45,
                    dark_color(),
                ),
            ]
        }
        NodeShape::Hexagon => {
            // A reactor with a cap and a chimney.
            let chimney = [node.x + node.width * 0.22, node.y - node.height * 0.1];
            vec![
                part("body", footprint, smooth, 0.0, 2.6, body),
                part("cap", shrunk(5.0), smooth, 2.6, 2.95, cap),
                part(
                    "chimney",
                    circle(chimney, (small * 0.2).max(5.0)),
                    true,
                    2.95,
                    4.6,
                    steel_color(),
                ),
            ]
        }
        NodeShape::Stadium => {
            // A drum lying on its side, read from above: a long body with two bands.
            vec![
                part("body", footprint, smooth, 0.0, 1.7, steel_color()),
                part(
                    "band",
                    outline(node, 1.5).0,
                    smooth,
                    0.35,
                    0.5,
                    dark_color(),
                ),
                part(
                    "band2",
                    outline(node, 1.5).0,
                    smooth,
                    1.2,
                    1.35,
                    dark_color(),
                ),
            ]
        }
        _ => vec![
            part("body", footprint, smooth, 0.0, 2.0, body),
            part("cap", shrunk(4.0), smooth, 2.0, 2.25, cap),
        ],
    }
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
    push_grid(&mut scene, &plan, s);
    for (i, edge) in diagram.edges.iter().enumerate() {
        let known = |id: &str| diagram.nodes.iter().any(|n| n.id == id);
        if edge.points.len() < 2 || !known(&edge.from) || !known(&edge.to) {
            continue;
        }
        push_edge(&mut scene, &plan, diagram, i, edge, s, measure);
    }
    for (i, node) in diagram.nodes.iter().enumerate() {
        push_node(&mut scene, &plan, i, node, s, measure);
    }
    scene
}

/// A faint square grid over the floor, like drafting paper.
fn push_grid(scene: &mut Scene3d, plan: &Plan, s: f64) {
    let (lo, hi) = plan.bounds();
    let y = plan.len(0.05);
    let mut runs = Vec::new();
    let first = |v: f64| (v / GRID).ceil() * GRID;
    let mut x = first(lo[0]);
    while x <= hi[0] {
        runs.push(vec![plan.world([x, lo[1]], y), plan.world([x, hi[1]], y)]);
        x += GRID;
    }
    let mut z = first(lo[1]);
    while z <= hi[1] {
        runs.push(vec![plan.world([lo[0], z], y), plan.world([hi[0], z], y)]);
        z += GRID;
    }
    scene.polylines.push(Polyline3 {
        key: "grid".into(),
        color: grid_color(),
        width_px: s as f32,
        runs,
        area: None,
        layer: 0,
    });
}

/// The world height of `font_sizes`.
fn up(plan: &Plan, font_sizes: f64) -> f64 {
    plan.len(font_sizes * plan.font)
}

fn push_node(
    scene: &mut Scene3d,
    plan: &Plan,
    index: usize,
    node: &DiagramNode,
    s: f64,
    measure: &dyn TextMeasure,
) {
    let parts = equipment(node, parse_color(&node.color));
    let (shadow, _) = outline(node, 2.0);
    push_shadow_with(
        scene,
        plan,
        &shadow,
        SHADOW_OFFSET,
        SHADOW_BLUR,
        Color::linear(0.0, 0.0, 0.0, SHADOW_ALPHA),
    );
    let edges = parse_color(&node.stroke).unwrap_or_else(outline_color);
    let edge_width = node.stroke_width.unwrap_or(EDGE_WIDTH) * s;
    let mut top = 0.0f64;
    for (k, part) in parts.iter().enumerate() {
        let mark = PrismMark {
            key: if k == 0 {
                node.id.clone()
            } else {
                format!("{}/{}", node.id, part.name)
            },
            color: part.color,
            outline: part.outline.iter().map(|&p| plan.floor_point(p)).collect(),
            base: up(plan, part.base) as f32,
            top: up(plan, part.top) as f32,
            smooth: part.smooth,
            gloss: 0.12,
            node: index,
            stand: None,
        };
        if k == 0 {
            scene.prisms.push(mark);
        } else {
            scene.decor.push(mark);
        }
        push_edges_of(scene, plan, &node.id, k, part, edges, edge_width);
        top = top.max(part.top);
    }
    let label = node.label.as_deref().unwrap_or(&node.id);
    let ink = parse_color(&node.text_color)
        .filter(|c| c.contrast(Color::linear(1.0, 1.0, 1.0, 1.0)) >= 3.0)
        .unwrap_or_else(ink_color);
    push_sign(
        scene,
        plan,
        &node.id,
        label,
        node.icon.as_deref(),
        [node.x, node.y],
        up(plan, top),
        up(plan, SIGN_LIFT),
        1.0,
        ink,
        s,
        measure,
    );
}

/// The drawn edges of a part: its top outline, its foot, and the corners between them (only
/// for a part with corners); the depth test hides those behind it.
fn push_edges_of(
    scene: &mut Scene3d,
    plan: &Plan,
    id: &str,
    k: usize,
    part: &Part,
    color: Color,
    width_px: f64,
) {
    if width_px <= 0.0 || part.outline.is_empty() {
        return;
    }
    let lift = plan.len(0.15);
    let (bottom, top) = (
        plan.len(part.base * plan.font) + lift,
        plan.len(part.top * plan.font) + lift,
    );
    let ring = |y: f64| {
        let mut run: Vec<[f32; 3]> = part.outline.iter().map(|&p| plan.world(p, y)).collect();
        run.push(run[0]);
        run
    };
    let mut runs = vec![ring(top), ring(bottom)];
    if !part.smooth {
        runs.extend(
            part.outline
                .iter()
                .map(|&p| vec![plan.world(p, bottom), plan.world(p, top)]),
        );
    }
    scene.polylines.push(Polyline3 {
        key: format!("{id}/edge/{k}"),
        color,
        width_px: width_px as f32,
        runs,
        area: None,
        layer: 0,
    });
}

/// A sign standing on a post above `at` (layout units) whose foot is at world height `foot`:
/// a white board in a dark frame, `post` world units up, with `text` printed on it (and
/// `icon` before it), facing the isometric view. `scale` sizes it against a node's sign.
#[allow(clippy::too_many_arguments)]
fn push_sign(
    scene: &mut Scene3d,
    plan: &Plan,
    key: &str,
    text: &str,
    icon: Option<&str>,
    at: [f64; 2],
    foot: f64,
    post: f64,
    scale: f64,
    ink: Color,
    s: f64,
    measure: &dyn TextMeasure,
) {
    let size = plan.font * scale;
    let lines: Vec<&str> = text.split('\n').collect();
    let text_w = lines
        .iter()
        .map(|l| f64::from(measure.width(l, size as f32)))
        .fold(0.0, f64::max);
    let icon_w = if icon.is_some() {
        (SIGN_ICON + SIGN_ICON_GAP) * size
    } else {
        0.0
    };
    let text_h = (lines.len() as f64 * LINE_HEIGHT).max(SIGN_ICON) * size;
    let pad = SIGN_PAD * scale;
    let (w, h) = (text_w + icon_w + 2.0 * pad, text_h + 2.0 * pad);
    // The sign's plane: right along the view's horizontal, toward the viewer its normal.
    let az = ISOMETRIC_AZIMUTH.to_radians();
    let right = [az.cos(), -az.sin()];
    let toward = [-right[1], right[0]];
    let centre_y = foot + post + plan.len(h / 2.0);
    let [cx, _, cz] = plan.world(at, 0.0);
    let (cx, cz) = (f64::from(cx), f64::from(cz));
    let stand = Stand {
        origin: [cx as f32, centre_y as f32, cz as f32],
        right: [right[0] as f32, right[1] as f32],
    };
    let depth = plan.len(SIGN_DEPTH * plan.font * scale);
    let rect = |w: f64, h: f64| {
        let (w, h) = (plan.len(w) as f32 / 2.0, plan.len(h) as f32 / 2.0);
        vec![[-w, -h], [w, -h], [w, h], [-w, h]]
    };
    let frame = SIGN_FRAME * scale;
    let mut board = |name: &str, outline, base: f64, top: f64, color| {
        scene.decor.push(PrismMark {
            key: format!("{key}/sign/{name}"),
            color,
            outline,
            base: base as f32,
            top: top as f32,
            smooth: false,
            gloss: 0.1,
            node: 0,
            stand: Some(stand),
        });
    };
    board(
        "frame",
        rect(w + 2.0 * frame, h + 2.0 * frame),
        -depth / 2.0,
        depth * 0.2,
        dark_color(),
    );
    board(
        "board",
        rect(w, h),
        -depth / 2.0,
        depth / 2.0,
        rgb(0xffffff),
    );
    // The post, from the equipment up to the frame's foot.
    let post_top = centre_y - plan.len(h / 2.0 + frame);
    if post_top > foot {
        let half = SIGN_POST * scale / 2.0;
        let square = [[-half, -half], [half, -half], [half, half], [-half, half]];
        scene.decor.push(PrismMark {
            key: format!("{key}/sign/post"),
            color: dark_color(),
            outline: square
                .iter()
                .map(|p| plan.floor_point([at[0] + p[0], at[1] + p[1]]))
                .collect(),
            base: foot as f32,
            top: post_top as f32,
            smooth: false,
            gloss: 0.0,
            node: 0,
            stand: None,
        });
    }
    // Printed on the board's face, just in front of it.
    let face = depth / 2.0 + plan.len(0.1);
    let anchor = [
        (cx + toward[0] * face) as f32,
        centre_y as f32,
        (cz + toward[1] * face) as f32,
    ];
    let left = -(text_w + icon_w) / 2.0;
    if let Some(name) = icon {
        let icon_size = SIGN_ICON * size;
        let k = icon_size / 24.0;
        // In the board's plane: x along `right`, y (down in the icon) down.
        let place = |p: [f64; 2]| {
            let u = plan.len(left + p[0] * k);
            let v = plan.len(icon_size / 2.0 - p[1] * k);
            [
                anchor[0] + (u * right[0]) as f32,
                anchor[1] + v as f32,
                anchor[2] + (u * right[1]) as f32,
            ]
        };
        push_icon_shape(
            scene,
            &format!("{key}/sign/icon"),
            &plan.icon(name),
            place,
            flow_color(),
            1.6 * s,
        );
    }
    let step = size * LINE_HEIGHT;
    for (k, line) in lines.iter().enumerate() {
        if line.is_empty() {
            continue;
        }
        let dy = (k as f64 - (lines.len() as f64 - 1.0) / 2.0) * step;
        scene.texts.push(Text3 {
            key: format!("{key}/sign/{k}"),
            text: (*line).to_owned(),
            anchor,
            offset_px: [(left + icon_w) as f32, dy as f32],
            size_px: size as f32,
            color: ink,
            align: Anchor::Start,
            valign: VAlign::Middle,
            azimuths: None,
            role: LabelRole::Fixed,
            halo: false,
            flat: Some(plan.scale as f32),
            upright: Some([right[0] as f32, right[1] as f32]),
        });
    }
}

/// Whether `p` (layout units) is on or near a node, where pipes go into equipment.
fn in_equipment(diagram: &DiagramSpec, p: [f64; 2], margin: f64) -> bool {
    diagram.nodes.iter().any(|n| {
        (p[0] - n.x).abs() <= n.width / 2.0 + margin
            && (p[1] - n.y).abs() <= n.height / 2.0 + margin
    })
}

fn push_edge(
    scene: &mut Scene3d,
    plan: &Plan,
    diagram: &DiagramSpec,
    index: usize,
    edge: &DiagramEdge,
    s: f64,
    measure: &dyn TextMeasure,
) {
    let key = format!("edge{index}");
    let (tail, head) = edge.ends();
    let thick = edge.stroke == EdgeStroke::Thick;
    let (base, top, width, color) = if thick {
        (
            CONVEYOR_BASE,
            CONVEYOR_TOP,
            CONVEYOR_WIDTH,
            conveyor_color(),
        )
    } else {
        (PIPE_BASE, PIPE_TOP, PIPE_WIDTH, pipe_color())
    };
    let (base, top) = (up(plan, base), up(plan, top));
    let path = edge.points.clone();
    let runs = match edge.stroke {
        EdgeStroke::Dotted => dashes(&path, DASH, DASH_GAP),
        _ => vec![path.clone()],
    };
    for (k, run) in runs.iter().enumerate() {
        if run.len() >= 2 {
            scene.ribbons.push(RibbonMark {
                key: format!("{key}/{k}"),
                color,
                path: run.iter().map(|&p| plan.floor_point(p)).collect(),
                width: plan.len(width) as f32,
                base: base as f32,
                top: top as f32,
                gloss: 0.2,
            });
        }
    }
    let total = path_length(&path);
    // Supports along the way, where the pipe is in the open.
    let count = ((total - 2.0 * SUPPORT_CLEAR) / SUPPORT_EVERY)
        .floor()
        .max(0.0) as usize
        + 1;
    let start = (total - (count - 1) as f64 * SUPPORT_EVERY) / 2.0;
    for k in 0..count {
        let (at, _) = along(&path, start + k as f64 * SUPPORT_EVERY);
        if total < 2.0 * SUPPORT_CLEAR || in_equipment(diagram, at, 2.0) {
            continue;
        }
        let half = if thick {
            SUPPORT_WIDTH * 1.5
        } else {
            SUPPORT_WIDTH
        } / 2.0;
        let square = [[-half, -half], [half, -half], [half, half], [-half, half]];
        scene.decor.push(PrismMark {
            key: format!("{key}/support/{k}"),
            color: support_color(),
            outline: square
                .iter()
                .map(|p| plan.floor_point([at[0] + p[0], at[1] + p[1]]))
                .collect(),
            base: 0.0,
            top: base as f32,
            smooth: false,
            gloss: 0.0,
            node: 0,
            stand: None,
        });
    }
    // Flow marks on top, pointing the way the edge runs (both ways on a two-headed edge).
    let forward = head != EdgeEnd::None;
    let backward = tail != EdgeEnd::None;
    let rise = up(plan, FLOW_RISE);
    let mark = |name: String, at: [f64; 2], dir: [f64; 2], scale: f64, color: Color| {
        let (l, w) = (FLOW_LENGTH * scale, FLOW_WIDTH * scale / 2.0);
        let n = [-dir[1], dir[0]];
        let tip = [at[0] + dir[0] * l / 2.0, at[1] + dir[1] * l / 2.0];
        let back = [at[0] - dir[0] * l / 2.0, at[1] - dir[1] * l / 2.0];
        let outline = [
            tip,
            [back[0] + n[0] * w, back[1] + n[1] * w],
            [back[0] - n[0] * w, back[1] - n[1] * w],
        ];
        PrismMark {
            key: name,
            color,
            outline: outline.iter().map(|&p| plan.floor_point(p)).collect(),
            base: (top - rise) as f32,
            top: (top + rise) as f32,
            smooth: false,
            gloss: 0.0,
            node: 0,
            stand: None,
        }
    };
    if (forward || backward) && total > 2.0 * HEAD_SETBACK {
        let count = ((total - 2.0 * HEAD_SETBACK) / FLOW_EVERY).floor().max(0.0) as usize;
        let start = (total - count.saturating_sub(1) as f64 * FLOW_EVERY) / 2.0;
        for k in 0..count {
            let (at, dir) = along(&path, start + k as f64 * FLOW_EVERY);
            if in_equipment(diagram, at, 0.0) {
                continue;
            }
            let dir = if forward { dir } else { [-dir[0], -dir[1]] };
            scene
                .decor
                .push(mark(format!("{key}/flow/{k}"), at, dir, 1.0, flow_color()));
        }
    }
    // Each end's marker, just before the pipe goes into the equipment.
    for (end, at_head) in [(head, true), (tail, false)] {
        if total < 2.0 * HEAD_SETBACK {
            continue;
        }
        let distance = if at_head {
            total - HEAD_SETBACK
        } else {
            HEAD_SETBACK
        };
        let (at, dir) = along(&path, distance);
        let dir = if at_head { dir } else { [-dir[0], -dir[1]] };
        let name = if at_head { "head" } else { "tail" };
        match end {
            EdgeEnd::None => {}
            EdgeEnd::Arrow => scene.decor.push(mark(
                format!("{key}/{name}"),
                at,
                dir,
                1.4,
                darken(flow_color(), 0.3),
            )),
            EdgeEnd::Circle | EdgeEnd::Cross => scene.decor.push(PrismMark {
                key: format!("{key}/{name}"),
                color: if end == EdgeEnd::Circle {
                    dark_color()
                } else {
                    rgb(0xe8553e)
                },
                outline: circle(at, width + 3.0)
                    .iter()
                    .map(|&p| plan.floor_point(p))
                    .collect(),
                base: (base - rise) as f32,
                top: (top + rise) as f32,
                smooth: true,
                gloss: 0.1,
                node: 0,
                stand: None,
            }),
        }
    }
    if let Some(text) = edge.label.as_deref().filter(|t| !t.is_empty()) {
        let at = edge
            .label_position
            .unwrap_or_else(|| midpoint(&edge.points));
        push_sign(
            scene,
            plan,
            &key,
            text,
            None,
            at,
            top,
            up(plan, TAG_LIFT),
            TAG_SCALE,
            ink_color(),
            s,
            measure,
        );
    }
}
