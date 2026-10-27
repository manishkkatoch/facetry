//! The glossy diagram style: coloured slabs with a specular sheen and soft shadows, thick
//! arrows lying on the floor, and labels printed on the slabs (and on small cards on the
//! arrows), seen at an angle through a perspective camera.
//!
//! Everything is placed in world units, text included, so the picture is the same at any
//! canvas size and zooms as one object. Edges stop short of the nodes they join, leaving a gap
//! like a drawn infographic: on the floor, the part of an edge right behind a slab is hidden
//! by it, and the gap keeps the arrowhead in view from the elevations the camera uses.

use crate::Color;
use crate::Theme;
use crate::diagram::{
    ICON_SIZE, Plan, dashes, icon_and_label, ink_on, parse_color, previous_distinct, push_icon,
    push_rim, shape_outline, trim_end,
};
use crate::scene::Anchor;
use crate::scene3d::{LabelRole, PrismMark, RibbonMark, Scene3d, ShadowMark, Text3, VAlign};
use crate::spec::{
    ChartSpec, DiagramEdge, DiagramNode, DiagramSpec, EdgeEnd, EdgeStroke, NodeShape,
};
use crate::text::TextMeasure;

/// How shiny the slabs, arrows and label cards are (0 matte to 1).
const NODE_GLOSS: f32 = 0.55;
const EDGE_GLOSS: f32 = 0.45;
const CARD_GLOSS: f32 = 0.25;
/// Arrows: band width (and for `==>`), thickness, arrowhead length and width. Layout units.
const EDGE_WIDTH: f64 = 6.0;
const THICK_EDGE_WIDTH: f64 = 10.0;
const EDGE_HEIGHT: f64 = 2.4;
const ARROW_LENGTH: f64 = 14.0;
const ARROW_WIDTH: f64 = 17.0;
/// Gaps between an edge and the nodes it joins, layout units: before the head (which must
/// clear the slab's own height from the camera) and after the tail.
const HEAD_GAP: f64 = 9.0;
const TAIL_GAP: f64 = 5.0;
/// Dotted edges: dash and gap lengths.
const DASH: f64 = 9.0;
const DASH_GAP: f64 = 7.0;
/// `circle` and `cross` ends: radius.
const END_RADIUS: f64 = 6.0;
/// Rim width when a node asks for a stroke colour without a width, CSS px.
const RIM_WIDTH: f64 = 1.5;
/// The catch-light along a slab's top edge: how much lighter than the fill, and its width.
const CATCH_LIGHT: f32 = 0.5;
const CATCH_LIGHT_WIDTH: f64 = 1.25;
/// Edge labels: text size against the node labels', and the card round the text.
const EDGE_LABEL_SCALE: f64 = 0.85;
const CARD_PAD: [f64; 2] = [7.0, 4.0];
const CARD_HEIGHT: f64 = 3.6;
/// Line spacing of multi-line labels, in font sizes.
const LINE_HEIGHT: f64 = 1.25;
/// Shadows: a soft dark halo on the floor round each slab and card, nudged a little toward
/// the viewer so it shows below the slab rather than only behind it; blur and darkness.
/// Layout units.
const SHADOW_OFFSET: [f64; 2] = [1.0, 2.0];
const SHADOW_BLUR: f64 = 14.0;
const SHADOW_ALPHA: f32 = 0.32;
/// Printed text floats this far above its surface (layout units), so the depth test never
/// lets the surface show through the letters.
const PRINT_LIFT: f64 = 0.3;

pub(crate) fn rgb(hex: u32) -> Color {
    Color::from_srgb8((hex >> 16) as u8, (hex >> 8) as u8, hex as u8, 0xff)
}

/// The fill of a node that does not choose one: by what the shape usually means in a
/// flowchart, so start and end, steps, decisions and stores read apart at a glance.
pub(crate) fn shape_fill(shape: NodeShape) -> Color {
    match shape {
        // Start and end.
        NodeShape::Stadium | NodeShape::Circle => rgb(0xe5484d),
        // Steps.
        NodeShape::Rect | NodeShape::Rounded => rgb(0x3e8ef7),
        // Decisions.
        NodeShape::Diamond => rgb(0xd6409f),
        // Data stores.
        NodeShape::Cylinder => rgb(0xf5a524),
        // Preparation.
        NodeShape::Hexagon => rgb(0x30a46c),
        // Input and output, manual steps.
        NodeShape::Parallelogram
        | NodeShape::ParallelogramAlt
        | NodeShape::Trapezoid
        | NodeShape::TrapezoidAlt => rgb(0x8e4ec6),
    }
}

/// `c` mixed toward white by `t` (0 to 1).
pub(crate) fn lighten(c: Color, t: f32) -> Color {
    let [r, g, b, a] = c.to_array();
    let m = |v: f32| v + (1.0 - v) * t;
    Color::linear(m(r), m(g), m(b), a)
}

/// The colour of the arrows and of dark text.
fn edge_color() -> Color {
    rgb(0x30323a)
}

fn card_color() -> Color {
    rgb(0xf8f8fb)
}

pub(crate) fn build(
    _spec: &ChartSpec,
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
    let top = plan.node_top();
    for (i, node) in diagram.nodes.iter().enumerate() {
        push_node(&mut scene, &plan, i, node, top, s, shadow, theme);
    }
    for (i, edge) in diagram.edges.iter().enumerate() {
        let known = |id: &str| diagram.nodes.iter().any(|n| n.id == id);
        if edge.points.len() < 2 || !known(&edge.from) || !known(&edge.to) {
            continue;
        }
        push_edge(&mut scene, &plan, i, edge, shadow, measure, theme);
    }
    scene
}

/// Text printed flat at `at` (layout units) on a surface at world height `y`, one line per
/// `\n`, centred: in layout units throughout, so `size` is the font size on the page.
#[allow(clippy::too_many_arguments)]
pub(crate) fn print(
    scene: &mut Scene3d,
    plan: &Plan,
    key: &str,
    text: &str,
    at: [f64; 2],
    y: f64,
    size: f64,
    color: Color,
) {
    print_aligned(scene, plan, key, text, at, y, size, color, Anchor::Middle);
}

/// [`print`] with the lines aligned on `at` by their start, middle or end.
#[allow(clippy::too_many_arguments)]
pub(crate) fn print_aligned(
    scene: &mut Scene3d,
    plan: &Plan,
    key: &str,
    text: &str,
    at: [f64; 2],
    y: f64,
    size: f64,
    color: Color,
    align: Anchor,
) {
    let lines: Vec<&str> = text.split('\n').collect();
    let step = size * LINE_HEIGHT;
    for (k, line) in lines.iter().enumerate() {
        if line.is_empty() {
            continue;
        }
        let dy = (k as f64 - (lines.len() as f64 - 1.0) / 2.0) * step;
        scene.texts.push(Text3 {
            tick: None,
            key: format!("{key}/label/{k}"),
            text: (*line).to_owned(),
            anchor: plan.world(at, y + plan.len(PRINT_LIFT)),
            offset_px: [0.0, dy as f32],
            size_px: size as f32,
            color,
            align,
            valign: VAlign::Middle,
            azimuths: None,
            role: LabelRole::Fixed,
            halo: false,
            flat: Some(plan.scale as f32),
            upright: None,
        });
    }
}

/// A soft shadow of the convex `outline` (layout units, absolute) cast onto the floor.
fn push_shadow(scene: &mut Scene3d, plan: &Plan, outline: &[[f64; 2]], color: Color) {
    push_shadow_with(scene, plan, outline, SHADOW_OFFSET, SHADOW_BLUR, color);
}

/// A soft shadow of the convex `outline` (layout units, absolute), moved by `offset` and
/// blurred over `blur` layout units.
pub(crate) fn push_shadow_with(
    scene: &mut Scene3d,
    plan: &Plan,
    outline: &[[f64; 2]],
    offset: [f64; 2],
    blur: f64,
    color: Color,
) {
    scene.shadows.push(ShadowMark {
        outline: outline
            .iter()
            .map(|p| plan.floor_point([p[0] + offset[0], p[1] + offset[1]]))
            .collect(),
        blur: plan.len(blur) as f32,
        color,
    });
}

#[allow(clippy::too_many_arguments)]
fn push_node(
    scene: &mut Scene3d,
    plan: &Plan,
    index: usize,
    node: &DiagramNode,
    top: f64,
    s: f64,
    shadow: Color,
    theme: &Theme,
) {
    let (outline, smooth) = shape_outline(node.shape, node.width, node.height);
    let absolute: Vec<[f64; 2]> = outline
        .iter()
        .map(|p| [node.x + p[0], node.y + p[1]])
        .collect();
    let fill = parse_color(&node.color).unwrap_or_else(|| shape_fill(node.shape));
    push_shadow(scene, plan, &absolute, shadow);
    scene.prisms.push(PrismMark {
        key: node.id.clone(),
        color: fill,
        outline: absolute.iter().map(|&p| plan.floor_point(p)).collect(),
        base: 0.0,
        top: top as f32,
        smooth,
        gloss: NODE_GLOSS,
        node: index,
        stand: None,
    });
    // The rim: the node's `stroke` if it has one, else a catch-light, a lighter line where the
    // top face meets the walls, as on a polished slab.
    let (rim, width) = match parse_color(&node.stroke) {
        Some(c) => (c, node.stroke_width.unwrap_or(RIM_WIDTH)),
        None => (
            lighten(fill, CATCH_LIGHT),
            node.stroke_width.unwrap_or(CATCH_LIGHT_WIDTH),
        ),
    };
    push_rim(scene, plan, node, &outline, top, rim, width * s);
    // White on the saturated fills, as printed infographics do; dark on light ones.
    let white = Color::linear(1.0, 1.0, 1.0, 1.0);
    let ink = parse_color(&node.text_color).unwrap_or_else(|| {
        if white.contrast(fill) >= 2.0 {
            white
        } else {
            ink_on(fill, theme.text)
        }
    });
    let label = node.label.as_deref().unwrap_or(&node.id);
    let (icon_at, label_at) = icon_and_label(node, plan.font);
    if let (Some(at), Some(icon)) = (icon_at, node.icon.as_deref()) {
        push_icon(
            scene,
            plan,
            &node.id,
            icon,
            at,
            ICON_SIZE * plan.font,
            top,
            ink,
            s,
        );
    }
    print(scene, plan, &node.id, label, label_at, top, plan.font, ink);
}

/// The band of an edge from `from` to `to` along `points`, `width` wide.
fn ribbon(plan: &Plan, key: String, points: &[[f64; 2]], width: f64) -> RibbonMark {
    RibbonMark {
        key,
        color: edge_color(),
        path: points.iter().map(|&p| plan.floor_point(p)).collect(),
        width: plan.len(width) as f32,
        base: 0.0,
        top: plan.len(EDGE_HEIGHT) as f32,
        gloss: EDGE_GLOSS,
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
        EdgeStroke::Thick => THICK_EDGE_WIDTH,
        _ => EDGE_WIDTH,
    };
    // Leave the gaps at both ends; the ends' markers sit at the new ends.
    let mut path = edge.points.clone();
    trim_end(&mut path, HEAD_GAP, true);
    trim_end(&mut path, TAIL_GAP, false);
    let ends = [(head, true), (tail, false)];
    let mut band = path.clone();
    for (end, at_head) in ends {
        let by = match end {
            EdgeEnd::Arrow => ARROW_LENGTH * 0.7,
            EdgeEnd::Circle => END_RADIUS,
            EdgeEnd::None | EdgeEnd::Cross => 0.0,
        };
        if by > 0.0 {
            trim_end(&mut band, by, at_head);
        }
    }
    let runs = match edge.stroke {
        EdgeStroke::Dotted => dashes(&band, DASH, DASH_GAP),
        _ => vec![band],
    };
    for (k, run) in runs.iter().enumerate() {
        if run.len() >= 2 {
            scene
                .ribbons
                .push(ribbon(plan, format!("{key}/{k}"), run, width));
        }
    }
    for (end, at_head) in ends {
        let n = path.len();
        let (tip, from) = if at_head {
            (path[n - 1], previous_distinct(&path, n - 1, -1))
        } else {
            (path[0], previous_distinct(&path, 0, 1))
        };
        push_end(scene, plan, &key, end, tip, from, at_head);
    }

    if let Some(text) = edge.label.as_deref().filter(|t| !t.is_empty()) {
        let at = edge
            .label_position
            .unwrap_or_else(|| crate::diagram::midpoint(&edge.points));
        push_card(
            scene, plan, &key, text, at, 0.0, shadow, measure, theme.text,
        );
    }
}

/// An edge's label: printed on a small light card lying at `at` (layout units) on a surface at
/// world height `base`, with a soft shadow.
#[allow(clippy::too_many_arguments)]
pub(crate) fn push_card(
    scene: &mut Scene3d,
    plan: &Plan,
    key: &str,
    text: &str,
    at: [f64; 2],
    base: f64,
    shadow: Color,
    measure: &dyn TextMeasure,
    ink: Color,
) {
    {
        let size = plan.font * EDGE_LABEL_SCALE;
        let lines: Vec<&str> = text.split('\n').collect();
        let w = lines
            .iter()
            .map(|l| f64::from(measure.width(l, size as f32)))
            .fold(0.0, f64::max);
        let h = (lines.len() as f64 - 1.0) * size * LINE_HEIGHT + size;
        let (card, _) = shape_outline(
            NodeShape::Rounded,
            w + 2.0 * CARD_PAD[0],
            h + 2.0 * CARD_PAD[1],
        );
        let card: Vec<[f64; 2]> = card.iter().map(|p| [at[0] + p[0], at[1] + p[1]]).collect();
        push_shadow(scene, plan, &card, shadow);
        let card_top = base + plan.len(CARD_HEIGHT);
        scene.decor.push(PrismMark {
            key: format!("{key}/card"),
            color: card_color(),
            outline: card.iter().map(|&p| plan.floor_point(p)).collect(),
            base: base as f32,
            top: card_top as f32,
            smooth: true,
            gloss: CARD_GLOSS,
            node: 0,
            stand: None,
        });
        print(scene, plan, key, text, at, card_top, size, ink);
    }
}

/// The marker at one end of an edge: its tip at `tip`, arriving from `from`.
fn push_end(
    scene: &mut Scene3d,
    plan: &Plan,
    key: &str,
    end: EdgeEnd,
    tip: [f64; 2],
    from: [f64; 2],
    at_head: bool,
) {
    let (dx, dy) = (tip[0] - from[0], tip[1] - from[1]);
    let len = dx.hypot(dy).max(1e-9);
    let (ux, uy) = (dx / len, dy / len);
    let (nx, ny) = (-uy, ux);
    let name = if at_head { "head" } else { "tail" };
    let solid = |outline: Vec<[f64; 2]>, smooth: bool| PrismMark {
        key: format!("{key}/{name}"),
        color: edge_color(),
        outline: outline.iter().map(|&p| plan.floor_point(p)).collect(),
        base: 0.0,
        top: plan.len(EDGE_HEIGHT) as f32,
        smooth,
        gloss: EDGE_GLOSS,
        node: 0,
        stand: None,
    };
    match end {
        EdgeEnd::None => {}
        EdgeEnd::Arrow => {
            let base = [tip[0] - ux * ARROW_LENGTH, tip[1] - uy * ARROW_LENGTH];
            let w = ARROW_WIDTH / 2.0;
            scene.decor.push(solid(
                vec![
                    tip,
                    [base[0] + nx * w, base[1] + ny * w],
                    [base[0] - nx * w, base[1] - ny * w],
                ],
                false,
            ));
        }
        EdgeEnd::Circle => {
            let c = [tip[0] - ux * END_RADIUS, tip[1] - uy * END_RADIUS];
            let (disc, _) = shape_outline(NodeShape::Circle, 2.0 * END_RADIUS, 2.0 * END_RADIUS);
            scene.decor.push(solid(
                disc.iter().map(|p| [c[0] + p[0], c[1] + p[1]]).collect(),
                true,
            ));
        }
        EdgeEnd::Cross => {
            let c = [tip[0] - ux * END_RADIUS, tip[1] - uy * END_RADIUS];
            let r = END_RADIUS;
            let arm = |a: f64, b: f64| [c[0] + (ux * a + nx * b) * r, c[1] + (uy * a + ny * b) * r];
            for (k, (p, q)) in [
                (arm(-1.0, -1.0), arm(1.0, 1.0)),
                (arm(-1.0, 1.0), arm(1.0, -1.0)),
            ]
            .into_iter()
            .enumerate()
            {
                scene.ribbons.push(ribbon(
                    plan,
                    format!("{key}/{name}/{k}"),
                    &[p, q],
                    EDGE_WIDTH * 0.6,
                ));
            }
        }
    }
}
