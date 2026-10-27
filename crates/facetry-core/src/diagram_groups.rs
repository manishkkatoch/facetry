//! Groups (Mermaid's subgraphs): a base under the nodes they hold, its title printed on it at
//! the top left, in each style's own look. Bases are thin and step up a little with every
//! level of nesting, so an inner group shows on the outer one; they stay below everything
//! that lies on the floor (edges, roads, cards), and the soft shadows of what stands on a base
//! are lifted onto it.
//!
//! * flat: a pale slab with a thin outline;
//! * glossy: a rounded tray with a light sheen;
//! * isometric: a plate tinted with a colour of the palette, outlined in it;
//! * schematic: a concrete pad inside a fence of posts and two rails.

use crate::Color;
use crate::diagram::{Plan, parse_color};
use crate::diagram_glossy::{lighten, print_aligned};
use crate::diagram_isometric::darken;
use crate::scene::Anchor;
use crate::scene3d::{Polyline3, PrismMark, RibbonMark, Scene3d};
use crate::spec::{DiagramGroup, DiagramSpec, DiagramStyle};
use crate::text::TextMeasure;

/// A base's thickness per level of nesting, layout units (under the isometric roads' 0.8).
const STEP: f64 = 0.22;
/// The title: size against the labels', inset from the box's corner, and how small it may
/// shrink to fit a narrow box.
const TITLE_SCALE: f64 = 0.9;
const TITLE_INSET: f64 = 12.0;
const TITLE_MIN: f64 = 0.6;
/// Line spacing of the title.
const LINE_HEIGHT: f64 = 1.25;
/// Corner radius of the glossy and isometric bases.
const RADIUS: f64 = 14.0;
/// Schematic fences: posts' spacing, size and height, the rails' heights (font sizes), and
/// how far inside the box the fence runs.
const POST_EVERY: f64 = 64.0;
const POST_SIZE: f64 = 2.6;
const FENCE_HEIGHT: f64 = 1.5;
const RAILS: [f64; 2] = [0.75, 1.4];
const RAIL_WIDTH: f64 = 1.2;
const FENCE_INSET: f64 = 4.0;

/// Group `g`'s box, `[left, top, right, bottom]` (layout units).
fn rect(g: &DiagramGroup) -> [f64; 4] {
    [
        g.x - g.width / 2.0,
        g.y - g.height / 2.0,
        g.x + g.width / 2.0,
        g.y + g.height / 2.0,
    ]
}

/// Box `r` with its corners rounded to radius `radius`, clockwise on screen.
fn rounded_rect(r: [f64; 4], radius: f64) -> Vec<[f64; 2]> {
    const STEPS: usize = 6;
    let corners = [
        (r[2] - radius, r[1] + radius, -90.0f64),
        (r[2] - radius, r[3] - radius, 0.0),
        (r[0] + radius, r[3] - radius, 90.0),
        (r[0] + radius, r[1] + radius, 180.0),
    ];
    let mut out = Vec::with_capacity(4 * (STEPS + 1));
    for (cx, cy, start) in corners {
        for k in 0..=STEPS {
            let a = (start + 90.0 * k as f64 / STEPS as f64).to_radians();
            out.push([cx + radius * a.cos(), cy + radius * a.sin()]);
        }
    }
    out
}

/// The world height of the top of a base at nesting `depth` (0 at the top level).
fn top(plan: &Plan, depth: usize) -> f64 {
    plan.len(STEP * (depth + 1) as f64)
}

/// Draws every group of `diagram`, outer ones first, in `diagram.style`.
pub(crate) fn push_groups(
    scene: &mut Scene3d,
    plan: &Plan,
    diagram: &DiagramSpec,
    measure: &dyn TextMeasure,
    ink: Color,
    s: f64,
) {
    let mut order: Vec<usize> = (0..diagram.groups.len()).collect();
    order.sort_by_key(|&g| diagram.group_depth(g));
    for g in order {
        push_group(scene, plan, diagram, g, measure, ink, s);
    }
}

fn push_group(
    scene: &mut Scene3d,
    plan: &Plan,
    diagram: &DiagramSpec,
    index: usize,
    measure: &dyn TextMeasure,
    ink: Color,
    s: f64,
) {
    let group = &diagram.groups[index];
    let depth = diagram.group_depth(index);
    let r = rect(group);
    let y = top(plan, depth);
    let style = diagram.style;
    let look = &plan.look;
    let tint = look.caps[index % look.caps.len()];
    let fill = parse_color(&group.color);
    let step = |c: Color, t: f64| look.deepen(c, (t * depth as f64) as f32);
    let (base, edge, title_ink, gloss, rounded) = match style {
        DiagramStyle::Flat => {
            let base = fill.unwrap_or_else(|| step(look.group[0], 0.035));
            (base, look.group_edge[0], ink, 0.0, false)
        }
        DiagramStyle::Glossy => {
            let base = fill.unwrap_or_else(|| step(look.group[1], 0.04));
            let edge = if look.dark {
                lighten(base, 0.15)
            } else {
                lighten(base, 0.6)
            };
            (base, edge, ink, 0.25, true)
        }
        DiagramStyle::Isometric => {
            let colour = fill.unwrap_or(tint);
            if look.dark {
                (
                    darken(colour, 0.84),
                    darken(colour, 0.4),
                    lighten(colour, 0.45),
                    0.1,
                    true,
                )
            } else {
                (
                    lighten(colour, 0.85),
                    lighten(colour, 0.25),
                    darken(colour, 0.35),
                    0.1,
                    true,
                )
            }
        }
        DiagramStyle::Schematic => {
            let base = fill.unwrap_or_else(|| step(look.group[2], 0.04));
            (base, look.group_edge[1], look.ink, 0.0, false)
        }
    };
    let edge = parse_color(&group.stroke).unwrap_or(edge);
    let title_ink = parse_color(&group.text_color)
        .filter(|c| c.contrast(base) >= 3.0)
        .unwrap_or(title_ink);
    // The base.
    let (w, h) = (r[2] - r[0], r[3] - r[1]);
    let outline: Vec<[f64; 2]> = if rounded {
        rounded_rect(r, RADIUS.min(w / 2.0).min(h / 2.0))
    } else {
        vec![[r[0], r[1]], [r[2], r[1]], [r[2], r[3]], [r[0], r[3]]]
    };
    scene.decor.push(PrismMark {
        key: format!("{}/base", group.id),
        color: base,
        outline: outline.iter().map(|&p| plan.floor_point(p)).collect(),
        base: 0.0,
        top: y as f32,
        smooth: rounded,
        gloss,
        node: 0,
        stand: None,
    });
    // Its outline, a hair above it.
    let lift = y + plan.len(0.1);
    let mut ring: Vec<[f32; 3]> = outline.iter().map(|&p| plan.world(p, lift)).collect();
    ring.push(ring[0]);
    scene.polylines.push(Polyline3 {
        reveal: 1.0,
        keys: Vec::new(),
        key: format!("{}/edge", group.id),
        color: edge,
        width_px: (1.2 * s) as f32,
        runs: vec![ring],
        area: None,
        layer: 0,
    });
    if style == DiagramStyle::Schematic {
        push_fence(scene, plan, group, r, y, s);
    }
    // The title, at the top left, shrunk to fit the box if it must.
    let title = group.label.as_deref().unwrap_or(&group.id);
    let lines: Vec<&str> = title.split('\n').collect();
    let mut size = plan.font * TITLE_SCALE;
    let widest = lines
        .iter()
        .map(|l| f64::from(measure.width(l, size as f32)))
        .fold(0.0, f64::max);
    let room = w - 2.0 * TITLE_INSET;
    if widest > room && widest > 0.0 {
        size = (size * room / widest).max(plan.font * TITLE_MIN);
    }
    let text_h = (lines.len() as f64 - 1.0) * LINE_HEIGHT * size + size;
    let at = [r[0] + TITLE_INSET, r[1] + TITLE_INSET + text_h / 2.0];
    print_aligned(
        scene,
        plan,
        &format!("{}/title", group.id),
        title,
        at,
        y,
        size,
        title_ink,
        Anchor::Start,
    );
}

/// A fence round a schematic group: posts along the box, a little inside it, and two rails.
fn push_fence(scene: &mut Scene3d, plan: &Plan, group: &DiagramGroup, r: [f64; 4], y: f64, s: f64) {
    let _ = s;
    let i = FENCE_INSET;
    let corners = [
        [r[0] + i, r[1] + i],
        [r[2] - i, r[1] + i],
        [r[2] - i, r[3] - i],
        [r[0] + i, r[3] - i],
    ];
    let height = plan.len(FENCE_HEIGHT * plan.font);
    let post = |at: [f64; 2], k: usize, scene: &mut Scene3d| {
        let h = POST_SIZE / 2.0;
        let square = [[-h, -h], [h, -h], [h, h], [-h, h]];
        scene.decor.push(PrismMark {
            key: format!("{}/post/{k}", group.id),
            color: plan.look.fence[0],
            outline: square
                .iter()
                .map(|q| plan.floor_point([at[0] + q[0], at[1] + q[1]]))
                .collect(),
            base: y as f32,
            top: (y + height) as f32,
            smooth: false,
            gloss: 0.0,
            node: 0,
            stand: None,
        });
    };
    let mut k = 0;
    for side in 0..4 {
        let (a, b) = (corners[side], corners[(side + 1) % 4]);
        let len = (b[0] - a[0]).hypot(b[1] - a[1]);
        let count = (len / POST_EVERY).ceil().max(1.0) as usize;
        for j in 0..count {
            let t = j as f64 / count as f64;
            post(
                [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t],
                k,
                scene,
            );
            k += 1;
        }
    }
    let mut path: Vec<[f32; 2]> = corners.iter().map(|&p| plan.floor_point(p)).collect();
    path.push(path[0]);
    for (n, rail) in RAILS.iter().enumerate() {
        let at = y + plan.len(rail * plan.font);
        scene.ribbons.push(RibbonMark {
            key: format!("{}/rail/{n}", group.id),
            color: plan.look.fence[1],
            path: path.clone(),
            width: plan.len(RAIL_WIDTH) as f32,
            base: (at - plan.len(RAIL_WIDTH / 2.0)) as f32,
            top: (at + plan.len(RAIL_WIDTH / 2.0)) as f32,
            gloss: 0.0,
            lift: Vec::new(),
        });
    }
}

/// The height of the base under layout point `p`: the innermost group's holding it, or 0.
pub(crate) fn floor_at(plan: &Plan, diagram: &DiagramSpec, p: [f64; 2]) -> f64 {
    let mut best: Option<usize> = None;
    for (g, group) in diagram.groups.iter().enumerate() {
        let r = rect(group);
        if p[0] > r[0] && p[0] < r[2] && p[1] > r[1] && p[1] < r[3] {
            let d = diagram.group_depth(g);
            if best.is_none_or(|b| d > b) {
                best = Some(d);
            }
        }
    }
    best.map_or(0.0, |d| top(plan, d))
}

/// Lifts every soft shadow onto the base it falls on, so a group's base does not hide it.
pub(crate) fn lift_shadows(scene: &mut Scene3d, plan: &Plan, diagram: &DiagramSpec) {
    if diagram.groups.is_empty() {
        return;
    }
    for shadow in &mut scene.shadows {
        let n = shadow.outline.len().max(1) as f32;
        let (x, z) = shadow
            .outline
            .iter()
            .fold((0.0, 0.0), |(x, z), p| (x + p[0], z + p[1]));
        let p = plan.layout_point(x / n, z / n);
        shadow.lift = floor_at(plan, diagram, p) as f32;
    }
}
