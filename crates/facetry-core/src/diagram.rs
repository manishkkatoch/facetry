//! Diagrams: a laid-out [`DiagramSpec`] -> [`Scene3d`]. No layout happens here (that is the
//! `facetry-diagrams` add-on's job); this only turns placed nodes and routed edges into solids,
//! lines and labels in world units.
//!
//! **The plan.** Layout units (x right, y down the page) map onto the floor: x to world x and
//! y to world z, so the top of the page is the far side of the floor and a top-to-bottom
//! flowchart reads top to bottom on screen from the default camera. The plan is scaled so its
//! longer side is [`PLAN_LONG`] world units (a tiny diagram is scaled as if its longer side
//! were [`MIN_PLAN`] layout units, so two nodes do not fill the screen).
//!
//! **Nodes** are prisms: the shape's outline (fitted into the node's rectangle) extruded
//! [`NODE_HEIGHT`] font sizes up, with a crisp rim drawn round the top face. **Edges** are
//! screen-space polylines at the height of the node tops, ending at a flat arrowhead (or a
//! disc, or a cross) on the rim of their node. On the floor, an edge arriving from the far
//! side of a node would end hidden behind it (in a top-to-bottom flowchart, nearly every
//! arrowhead); at the top they are seen from every camera position. **Labels** face the viewer like every 3D label and keep their pixel size, so they
//! are sized for the *starting* camera: the size at which they fit the nodes the layout made.

use crate::Color;
use crate::Theme;
use crate::framing_radius;
use crate::scene::{Anchor, RuleKind};
use crate::scene3d::{
    FOV_Y_DEGREES, FRAMING_LIFT, LabelRole, Line3, Polyline3, PrismMark, Scene3d, Text3, VAlign,
};
use crate::spec::{
    CameraKind, CameraSpec, ChartSpec, DEFAULT_DIAGRAM_FONT_SIZE, DiagramEdge, DiagramNode,
    DiagramSpec, DiagramStyle, Dimension, EdgeEnd, EdgeStroke, MAX_DIAGRAM_FONT_SIZE,
    MAX_NODE_STROKE_WIDTH, NodeShape,
};
use crate::text::TextMeasure;
use crate::validate::{Issue, pointer};

/// The longer side of the plan, world units.
pub const PLAN_LONG: f64 = 2.0;
/// A plan smaller than this many layout units is scaled as if it were this long.
pub const MIN_PLAN: f64 = 480.0;
/// Empty floor round the drawing, layout units.
const MARGIN: f64 = 12.0;
/// How tall the nodes stand, in font sizes: blocks in the flat style, slabs in the glossy one.
const NODE_HEIGHT: f64 = 1.0;
const GLOSSY_NODE_HEIGHT: f64 = 0.7;
/// An isometric pedestal's whole height (see `diagram_isometric`).
const ISOMETRIC_NODE_HEIGHT: f64 = crate::diagram_isometric::STACK_HEIGHT;
/// Edges run this far above the node tops (layout units), so they never flicker against a
/// top face they cross the rim of.
const EDGE_LIFT: f64 = 0.6;
/// Arrowheads: length and width, layout units, and thickness.
const ARROW_LENGTH: f64 = 12.0;
const ARROW_WIDTH: f64 = 10.0;
const ARROW_HEIGHT: f64 = 2.5;
/// Radius of a `circle` end; half the arm of a `cross` end. Layout units.
const END_RADIUS: f64 = 4.0;
/// Dotted edges: dash and gap lengths, layout units.
const DASH: f64 = 5.0;
const GAP: f64 = 4.0;
/// Edge widths, CSS px.
const EDGE_WIDTH: f64 = 1.5;
const THICK_WIDTH: f64 = 3.5;
/// The rim round a node's top face, CSS px.
const RIM_WIDTH: f64 = 1.25;
/// Rounded corners: radius as a fraction of the shorter side.
const ROUNDING: f64 = 0.18;
/// Segments per quarter circle of a rounded outline.
const ARC_STEPS: usize = 6;
/// Line spacing of multi-line labels, in font sizes.
const LINE_HEIGHT: f64 = 1.25;
/// Edge labels are this much of the node label size.
const EDGE_LABEL_SCALE: f64 = 0.9;
/// Labels never get smaller or larger than this on screen at the starting camera, CSS px.
const MIN_LABEL_PX: f64 = 7.0;
const MAX_LABEL_PX: f64 = 32.0;

/// The starting camera of a flat diagram, where the spec leaves a field out: orthographic (no
/// perspective distortion: what documentation wants), square on to the page (turning it
/// tilts the ranks, which then no longer read as rows) and looking well down onto the floor.
pub const DIAGRAM_AZIMUTH: f64 = 0.0;
pub const DIAGRAM_ELEVATION: f64 = 50.0;
/// A glossy diagram's: through a perspective lens, turned and lower, like a product shot.
/// Its labels are printed on the slabs, so they turn with the page and stay readable.
pub const GLOSSY_AZIMUTH: f64 = 28.0;
pub const GLOSSY_ELEVATION: f64 = 40.0;
/// An isometric diagram's: the classic isometric view (45 degrees round, the elevation at
/// which the three axes are drawn equally foreshortened) through an orthographic lens.
pub const ISOMETRIC_AZIMUTH: f64 = 45.0;
pub const ISOMETRIC_ELEVATION: f64 = 35.26;
/// Share of the canvas (from the centre to each edge) the diagram may fill at the starting
/// distance.
const FILL: f64 = 0.92;
const PERSPECTIVE_FILL: f64 = 0.9;
/// Most CSS pixels per layout unit at the starting distance: a small diagram is drawn at
/// about its natural size (labels a little above `fontSize` px) instead of blown up to fill
/// the canvas.
const MAX_ZOOM: f64 = 1.15;

/// The diagram's default colours (light theme).
#[derive(Debug, Clone, Copy)]
struct Palette {
    node: Color,
    rim: Color,
    edge: Color,
    text: Color,
}

impl Palette {
    fn new(theme: &Theme) -> Self {
        let rgb =
            |hex: u32| Color::from_srgb8((hex >> 16) as u8, (hex >> 8) as u8, hex as u8, 0xff);
        Self {
            node: rgb(0xececff),
            rim: rgb(0x9370db),
            edge: rgb(0x4a4a5a),
            text: theme.text,
        }
    }
}

// --- validation -------------------------------------------------------------------------------

/// Everything wrong with `spec.diagram` (and with what a diagram spec must not have).
pub(crate) fn validate_diagram(spec: &ChartSpec, issues: &mut Vec<Issue>) {
    let Some(diagram) = &spec.diagram else {
        return;
    };
    if spec.dimension != Dimension::ThreeD {
        issues.push(Issue::new(
            pointer(&["diagram"]),
            "diagrams need `dimension: \"3d\"`",
        ));
    }
    for (name, present) in [
        ("layers", !spec.layers.is_empty()),
        ("axes", !spec.axes.is_empty()),
        ("data", !spec.data.is_empty()),
        ("legend", spec.legend.is_some()),
    ] {
        if present {
            issues.push(Issue::new(
                pointer(&[name]),
                format!("a diagram spec has no `{name}`"),
            ));
        }
    }
    if let Some(size) = diagram.font_size {
        if !(size.is_finite() && size > 0.0 && size <= MAX_DIAGRAM_FONT_SIZE) {
            issues.push(Issue::new(
                pointer(&["diagram", "fontSize"]),
                format!("fontSize must be greater than 0 and at most {MAX_DIAGRAM_FONT_SIZE}"),
            ));
        }
    }
    for (i, node) in diagram.nodes.iter().enumerate() {
        let n = i.to_string();
        let at = |field: &str| pointer(&["diagram", "nodes", &n, field]);
        if node.id.is_empty() {
            issues.push(Issue::new(at("id"), "a node id must not be empty"));
        } else if diagram.nodes[..i].iter().any(|m| m.id == node.id) {
            issues.push(Issue::new(
                at("id"),
                format!("duplicate node id {:?}", node.id),
            ));
        }
        for (field, v) in [("x", node.x), ("y", node.y)] {
            if !v.is_finite() {
                issues.push(Issue::new(
                    at(field),
                    format!("{field} must be a finite number"),
                ));
            }
        }
        for (field, v) in [("width", node.width), ("height", node.height)] {
            if !(v.is_finite() && v > 0.0) {
                issues.push(Issue::new(
                    at(field),
                    format!("{field} must be a finite number greater than 0"),
                ));
            }
        }
        for (field, value) in [
            ("color", &node.color),
            ("stroke", &node.stroke),
            ("textColor", &node.text_color),
        ] {
            if let Some(Err(e)) = value.as_deref().map(Color::from_hex) {
                issues.push(Issue::new(at(field), e.to_string()));
            }
        }
        if let Some(w) = node.stroke_width {
            if !(w.is_finite() && (0.0..=MAX_NODE_STROKE_WIDTH).contains(&w)) {
                issues.push(Issue::new(
                    at("strokeWidth"),
                    format!("strokeWidth must be between 0 and {MAX_NODE_STROKE_WIDTH}"),
                ));
            }
        }
    }
    for (i, edge) in diagram.edges.iter().enumerate() {
        let n = i.to_string();
        let at = |field: &str| pointer(&["diagram", "edges", &n, field]);
        for (field, id) in [("from", &edge.from), ("to", &edge.to)] {
            if !diagram.nodes.iter().any(|m| &m.id == id) {
                issues.push(Issue::new(at(field), format!("no node has the id {id:?}")));
            }
        }
        if edge.points.len() < 2 {
            issues.push(Issue::new(at("points"), "an edge needs at least 2 points"));
        }
        let finite = |p: &[f64; 2]| p.iter().all(|v| v.is_finite());
        if let Some(j) = edge.points.iter().position(|p| !finite(p)) {
            issues.push(Issue::new(
                pointer(&["diagram", "edges", &n, "points", &j.to_string()]),
                "points must be finite numbers",
            ));
        }
        if edge.label_position.as_ref().is_some_and(|p| !finite(p)) {
            issues.push(Issue::new(
                at("labelPosition"),
                "labelPosition must be finite numbers",
            ));
        }
    }
}

// --- outlines ---------------------------------------------------------------------------------

/// The outline of `shape` fitted into a `w` by `h` rectangle centred on the origin, in layout
/// units (y down), and whether it approximates a curve.
pub fn shape_outline(shape: NodeShape, w: f64, h: f64) -> (Vec<[f64; 2]>, bool) {
    let (hw, hh) = (w / 2.0, h / 2.0);
    let rounded = |r: f64| {
        let r = r.min(hw).min(hh);
        let mut out = Vec::with_capacity(4 * (ARC_STEPS + 1));
        // Corner centres, clockwise on screen from the top right, each swept a quarter turn.
        let corners = [
            (hw - r, -hh + r, -90.0f64),
            (hw - r, hh - r, 0.0),
            (-hw + r, hh - r, 90.0),
            (-hw + r, -hh + r, 180.0),
        ];
        for (cx, cy, start) in corners {
            for k in 0..=ARC_STEPS {
                let a = (start + 90.0 * k as f64 / ARC_STEPS as f64).to_radians();
                out.push([cx + r * a.cos(), cy + r * a.sin()]);
            }
        }
        dedup(out)
    };
    let ellipse = || {
        let n = 4 * ARC_STEPS * 2;
        (0..n)
            .map(|k| {
                let a = std::f64::consts::TAU * k as f64 / n as f64;
                [hw * a.cos(), hh * a.sin()]
            })
            .collect::<Vec<_>>()
    };
    // How far the slanted shapes lean: a quarter of the height, at most a third of the width.
    let lean = (h / 2.0).min(w / 3.0);
    match shape {
        NodeShape::Rect => (vec![[-hw, -hh], [hw, -hh], [hw, hh], [-hw, hh]], false),
        NodeShape::Rounded => (rounded(w.min(h) * ROUNDING), true),
        NodeShape::Stadium => (rounded(hh), true),
        NodeShape::Cylinder | NodeShape::Circle => (ellipse(), true),
        NodeShape::Diamond => (vec![[0.0, -hh], [hw, 0.0], [0.0, hh], [-hw, 0.0]], false),
        NodeShape::Hexagon => {
            let cut = (h / 2.0).min(w / 4.0);
            (
                vec![
                    [-hw + cut, -hh],
                    [hw - cut, -hh],
                    [hw, 0.0],
                    [hw - cut, hh],
                    [-hw + cut, hh],
                    [-hw, 0.0],
                ],
                false,
            )
        }
        NodeShape::Parallelogram => (
            vec![[-hw + lean, -hh], [hw, -hh], [hw - lean, hh], [-hw, hh]],
            false,
        ),
        NodeShape::ParallelogramAlt => (
            vec![[-hw, -hh], [hw - lean, -hh], [hw, hh], [-hw + lean, hh]],
            false,
        ),
        NodeShape::Trapezoid => (
            vec![[-hw + lean, -hh], [hw - lean, -hh], [hw, hh], [-hw, hh]],
            false,
        ),
        NodeShape::TrapezoidAlt => (
            vec![[-hw, -hh], [hw, -hh], [hw - lean, hh], [-hw + lean, hh]],
            false,
        ),
    }
}

/// Drops points that repeat their predecessor (where a rounded corner's arc meets a straight
/// side of length 0).
fn dedup(points: Vec<[f64; 2]>) -> Vec<[f64; 2]> {
    let mut out: Vec<[f64; 2]> = Vec::with_capacity(points.len());
    for p in points {
        let same = |q: &[f64; 2]| (q[0] - p[0]).abs() < 1e-9 && (q[1] - p[1]).abs() < 1e-9;
        if !out.last().is_some_and(same) {
            out.push(p);
        }
    }
    let n = out.len();
    if n > 1 && (out[0][0] - out[n - 1][0]).abs() < 1e-9 && (out[0][1] - out[n - 1][1]).abs() < 1e-9
    {
        out.pop();
    }
    out
}

// --- the plan and the camera ------------------------------------------------------------------

/// How layout units map to the world: `world = (layout - origin) * scale`.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Plan {
    origin: [f64; 2],
    pub(crate) scale: f64,
    /// World size of the floor, `[x, z]`.
    floor: [f64; 2],
    pub(crate) font: f64,
    /// Node height, in font sizes.
    node_height: f64,
}

impl Plan {
    pub(crate) fn new(diagram: &DiagramSpec) -> Self {
        let mut lo = [f64::INFINITY; 2];
        let mut hi = [f64::NEG_INFINITY; 2];
        let mut add = |p: [f64; 2]| {
            for k in 0..2 {
                if p[k].is_finite() {
                    lo[k] = lo[k].min(p[k]);
                    hi[k] = hi[k].max(p[k]);
                }
            }
        };
        for n in &diagram.nodes {
            add([n.x - n.width / 2.0, n.y - n.height / 2.0]);
            add([n.x + n.width / 2.0, n.y + n.height / 2.0]);
        }
        for e in &diagram.edges {
            e.points.iter().for_each(|&p| add(p));
            if let Some(p) = e.label_position {
                add(p);
            }
        }
        if !lo[0].is_finite() {
            lo = [0.0; 2];
            hi = [0.0; 2];
        }
        let origin = [lo[0] - MARGIN, lo[1] - MARGIN];
        let extent = [hi[0] - lo[0] + 2.0 * MARGIN, hi[1] - lo[1] + 2.0 * MARGIN];
        let scale = PLAN_LONG / extent[0].max(extent[1]).max(MIN_PLAN);
        Self {
            origin,
            scale,
            floor: [extent[0] * scale, extent[1] * scale],
            font: diagram.font_size.unwrap_or(DEFAULT_DIAGRAM_FONT_SIZE),
            node_height: match diagram.style {
                DiagramStyle::Flat => NODE_HEIGHT,
                DiagramStyle::Glossy => GLOSSY_NODE_HEIGHT,
                DiagramStyle::Isometric => ISOMETRIC_NODE_HEIGHT,
            },
        }
    }

    /// A layout point on the floor at world height `y`.
    pub(crate) fn world(&self, p: [f64; 2], y: f64) -> [f32; 3] {
        [
            ((p[0] - self.origin[0]) * self.scale) as f32,
            y as f32,
            ((p[1] - self.origin[1]) * self.scale) as f32,
        ]
    }

    pub(crate) fn floor_point(&self, p: [f64; 2]) -> [f32; 2] {
        let [x, _, z] = self.world(p, 0.0);
        [x, z]
    }

    /// A length in layout units, in world units.
    pub(crate) fn len(&self, v: f64) -> f64 {
        v * self.scale
    }

    pub(crate) fn node_top(&self) -> f64 {
        self.len(self.node_height * self.font)
    }

    /// The plot box: the floor, as high as the nodes.
    pub(crate) fn size(&self) -> [f32; 3] {
        [
            self.floor[0] as f32,
            self.node_top() as f32,
            self.floor[1] as f32,
        ]
    }
}

/// The camera distance (in box radii, see [`framing_radius`]) at which the diagram, seen from
/// `azimuth`/`elevation` degrees through `kind` of lens, fills `fill` of a canvas of
/// `viewport_px` device pixels, but no closer than [`MAX_ZOOM`] allows. Found by projecting the
/// corners of the plot box exactly as the renderer's camera does (the same eye, target and
/// lenses) for a candidate distance and bisecting.
fn fitting_distance(
    plan: &Plan,
    kind: CameraKind,
    azimuth: f64,
    elevation: f64,
    fill: f64,
    viewport_px: (u32, u32),
    dpr: f32,
) -> f64 {
    let size = plan.size().map(f64::from);
    let radius = f64::from(framing_radius(plan.size()));
    let centre = [size[0] / 2.0, size[1] / 2.0 - FRAMING_LIFT, size[2] / 2.0];
    let (az, el) = (azimuth.to_radians(), elevation.to_radians());
    let dir = [az.sin() * el.cos(), el.sin(), az.cos() * el.cos()];
    let (vw, vh) = (
        f64::from(viewport_px.0.max(1)),
        f64::from(viewport_px.1.max(1)),
    );
    let aspect = vw / vh;
    let tan = (FOV_Y_DEGREES / 2.0).to_radians().tan();
    // The camera's axes: looking along -dir, y up.
    let forward = [-dir[0], -dir[1], -dir[2]];
    let cross = |a: [f64; 3], b: [f64; 3]| {
        [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ]
    };
    let norm = |a: [f64; 3]| {
        let l = (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt().max(1e-12);
        [a[0] / l, a[1] / l, a[2] / l]
    };
    let right = norm(cross(forward, [0.0, 1.0, 0.0]));
    let up = cross(right, forward);
    let dot = |a: [f64; 3], b: [f64; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let corners: Vec<[f64; 3]> = (0..8)
        .map(|k| {
            [
                if k & 1 == 0 { 0.0 } else { size[0] },
                if k & 2 == 0 { 0.0 } else { size[1] },
                if k & 4 == 0 { 0.0 } else { size[2] },
            ]
        })
        .collect();
    // Whether every corner lands inside `fill` of the canvas at distance `k` radii.
    let fits = |k: f64| {
        let d = k * radius;
        let eye = [
            centre[0] + dir[0] * d,
            centre[1] + dir[1] * d,
            centre[2] + dir[2] * d,
        ];
        corners.iter().all(|c| {
            let v = [c[0] - eye[0], c[1] - eye[1], c[2] - eye[2]];
            let (x, y, z) = (dot(v, right), dot(v, up), dot(v, forward));
            let half = match kind {
                CameraKind::Perspective if z <= 1e-9 => return false,
                CameraKind::Perspective => z * tan,
                CameraKind::Orthographic => d * tan,
            };
            x.abs() <= half * aspect * fill && y.abs() <= half * fill
        })
    };
    let (mut lo, mut hi) = (0.3f64, 20.0f64);
    if fits(lo) {
        hi = lo;
    } else {
        for _ in 0..40 {
            let mid = (lo + hi) / 2.0;
            if fits(mid) {
                hi = mid;
            } else {
                lo = mid;
            }
        }
    }
    // At distance k (orthographic) a layout unit is `vh / (2 k radius tan) * scale` pixels.
    let s = f64::from(dpr).clamp(0.25, 16.0);
    let closest = vh * plan.scale / (2.0 * radius * tan * MAX_ZOOM * s);
    hi.max(closest)
}

impl ChartSpec {
    /// The camera to start from: the spec's own, with a diagram's defaults filling the fields
    /// it leaves out: orthographic, [`DIAGRAM_AZIMUTH`] and [`DIAGRAM_ELEVATION`] for a flat
    /// diagram, perspective, [`GLOSSY_AZIMUTH`] and [`GLOSSY_ELEVATION`] for a glossy one, and a
    /// distance that fits the diagram in a canvas of `viewport_px` device pixels at `dpr`.
    /// Charts start from their spec's camera as it is.
    pub fn start_camera(&self, viewport_px: (u32, u32), dpr: f32) -> Option<CameraSpec> {
        let Some(diagram) = &self.diagram else {
            return self.camera.clone();
        };
        let given = self.camera.clone().unwrap_or(CameraSpec {
            kind: None,
            azimuth: None,
            elevation: None,
            distance: None,
        });
        let (kind, azimuth, elevation) = match diagram.style {
            DiagramStyle::Flat => (CameraKind::Orthographic, DIAGRAM_AZIMUTH, DIAGRAM_ELEVATION),
            DiagramStyle::Glossy => (CameraKind::Perspective, GLOSSY_AZIMUTH, GLOSSY_ELEVATION),
            DiagramStyle::Isometric => (
                CameraKind::Orthographic,
                ISOMETRIC_AZIMUTH,
                ISOMETRIC_ELEVATION,
            ),
        };
        let kind = given.kind.unwrap_or(kind);
        let azimuth = given.azimuth.unwrap_or(azimuth);
        let elevation = given.elevation.unwrap_or(elevation);
        let fill = match kind {
            CameraKind::Orthographic => FILL,
            CameraKind::Perspective => PERSPECTIVE_FILL,
        };
        let distance = given.distance.unwrap_or_else(|| {
            fitting_distance(
                &Plan::new(diagram),
                kind,
                azimuth,
                elevation,
                fill,
                viewport_px,
                dpr,
            )
        });
        Some(CameraSpec {
            kind: Some(kind),
            azimuth: Some(azimuth),
            elevation: Some(elevation),
            distance: Some(distance),
        })
    }
}

// --- the scene --------------------------------------------------------------------------------

/// Builds the scene of `spec.diagram` (an empty scene if there is none). Like the chart
/// builders it is pure and lenient: an edge whose nodes are missing is skipped.
pub fn build_diagram_3d(
    spec: &ChartSpec,
    viewport_px: (u32, u32),
    dpr: f32,
    measure: &dyn TextMeasure,
    theme: &Theme,
) -> Scene3d {
    let s = f64::from(dpr).clamp(0.25, 16.0);
    let empty = DiagramSpec {
        nodes: Vec::new(),
        edges: Vec::new(),
        font_size: None,
        style: DiagramStyle::Flat,
    };
    let diagram = spec.diagram.as_ref().unwrap_or(&empty);
    match diagram.style {
        DiagramStyle::Flat => {}
        DiagramStyle::Glossy => {
            return crate::diagram_glossy::build(spec, diagram, viewport_px, dpr, measure, theme);
        }
        DiagramStyle::Isometric => {
            return crate::diagram_isometric::build(diagram, viewport_px, dpr, measure, theme);
        }
    }
    let plan = Plan::new(diagram);
    let palette = Palette::new(theme);
    let mut scene = Scene3d::empty(viewport_px, theme.background, plan.size());
    scene.halo_px = s.round().max(1.0) as f32;

    // How many screen pixels a layout unit is at the starting camera, to size the labels.
    let camera = spec.start_camera(viewport_px, dpr).unwrap_or(CameraSpec {
        kind: None,
        azimuth: None,
        elevation: None,
        distance: None,
    });
    let distance = camera.distance.unwrap_or(1.0).clamp(0.5, 20.0);
    let half_fov = (FOV_Y_DEGREES / 2.0).to_radians().tan();
    let px_per_world = f64::from(viewport_px.1.max(1))
        / (2.0 * distance * f64::from(framing_radius(scene.size)) * half_fov);
    let px_per_unit = px_per_world * plan.scale;
    let label_px = (plan.font * px_per_unit).clamp(MIN_LABEL_PX * s, MAX_LABEL_PX * s) as f32;

    let top = plan.node_top();
    for (i, node) in diagram.nodes.iter().enumerate() {
        push_node(&mut scene, &plan, &palette, i, node, top, s, label_px);
    }
    for (i, edge) in diagram.edges.iter().enumerate() {
        let known = |id: &str| diagram.nodes.iter().any(|n| n.id == id);
        if edge.points.len() < 2 || !known(&edge.from) || !known(&edge.to) {
            continue;
        }
        push_edge(&mut scene, &plan, &palette, i, edge, s, label_px);
    }
    scene
}

#[allow(clippy::too_many_arguments)]
fn push_node(
    scene: &mut Scene3d,
    plan: &Plan,
    palette: &Palette,
    index: usize,
    node: &DiagramNode,
    top: f64,
    s: f64,
    label_px: f32,
) {
    let (outline, smooth) = shape_outline(node.shape, node.width, node.height);
    let at = |p: &[f64; 2]| [node.x + p[0], node.y + p[1]];
    let color = parse_color(&node.color).unwrap_or(palette.node);
    scene.prisms.push(PrismMark {
        key: node.id.clone(),
        color,
        outline: outline.iter().map(|p| plan.floor_point(at(p))).collect(),
        base: 0.0,
        top: top as f32,
        smooth,
        gloss: 0.0,
        node: index,
    });
    // The rim: the top face's outline, a hair above it so the face does not hide it.
    let rim = parse_color(&node.stroke).unwrap_or(palette.rim);
    let rim_width = node.stroke_width.unwrap_or(RIM_WIDTH);
    push_rim(scene, plan, node, &outline, top, rim, rim_width * s);
    // The label, one Text3 per line, centred on the top face.
    let label = node.label.as_deref().unwrap_or(&node.id);
    let lines: Vec<&str> = label.split('\n').collect();
    let step = f64::from(label_px) * LINE_HEIGHT;
    let ink = parse_color(&node.text_color).unwrap_or_else(|| ink_on(color, palette.text));
    for (k, line) in lines.iter().enumerate() {
        if line.is_empty() {
            continue;
        }
        let dy = (k as f64 - (lines.len() as f64 - 1.0) / 2.0) * step;
        scene.texts.push(Text3 {
            key: format!("{}/label/{k}", node.id),
            text: (*line).to_owned(),
            anchor: plan.world([node.x, node.y], top),
            offset_px: [0.0, dy as f32],
            size_px: label_px,
            color: ink,
            align: Anchor::Middle,
            valign: VAlign::Middle,
            azimuths: None,
            role: LabelRole::Fixed,
            halo: false,
            flat: None,
        });
    }
}

/// A node's colour from the spec, if it gave a valid one.
pub(crate) fn parse_color(hex: &Option<String>) -> Option<Color> {
    hex.as_deref().and_then(|h| Color::from_hex(h).ok())
}

/// The dark text colour, or white where white reads better on `fill`.
pub(crate) fn ink_on(fill: Color, dark: Color) -> Color {
    let white = Color::linear(1.0, 1.0, 1.0, 1.0);
    if dark.contrast(fill) >= white.contrast(fill) {
        dark
    } else {
        white
    }
}

/// The rim round a node's top face, `width_px` device pixels, a hair above the face so it does
/// not hide it. Nothing for a width of 0.
pub(crate) fn push_rim(
    scene: &mut Scene3d,
    plan: &Plan,
    node: &DiagramNode,
    outline: &[[f64; 2]],
    top: f64,
    color: Color,
    width_px: f64,
) {
    if width_px <= 0.0 {
        return;
    }
    let at = |p: &[f64; 2]| [node.x + p[0], node.y + p[1]];
    let lift = top + plan.len(0.15);
    for k in 0..outline.len() {
        let (a, b) = (&outline[k], &outline[(k + 1) % outline.len()]);
        scene.lines.push(Line3 {
            key: format!("{}/rim/{k}", node.id),
            kind: RuleKind::Axis,
            a: plan.world(at(a), lift),
            b: plan.world(at(b), lift),
            width_px: width_px as f32,
            color,
            azimuths: None,
        });
    }
}

#[allow(clippy::too_many_arguments)]
fn push_edge(
    scene: &mut Scene3d,
    plan: &Plan,
    palette: &Palette,
    index: usize,
    edge: &DiagramEdge,
    s: f64,
    label_px: f32,
) {
    let key = format!("edge{index}");
    let lift = plan.node_top() + plan.len(EDGE_LIFT);
    let (tail, head) = edge.ends();
    // Pull the line back from the ends that carry an arrowhead, so its square end does not
    // poke out of the arrow's tip.
    let mut points = edge.points.clone();
    for (end, at_head) in [(head, true), (tail, false)] {
        if end == EdgeEnd::Arrow {
            trim_end(&mut points, ARROW_LENGTH * 0.8, at_head);
        }
    }
    let runs: Vec<Vec<[f32; 3]>> = match edge.stroke {
        EdgeStroke::Dotted => dashes(&points, DASH, GAP),
        _ => vec![points.clone()],
    }
    .into_iter()
    .map(|run| run.iter().map(|&p| plan.world(p, lift)).collect())
    .collect();
    let width = match edge.stroke {
        EdgeStroke::Thick => THICK_WIDTH,
        _ => EDGE_WIDTH,
    };
    scene.polylines.push(Polyline3 {
        key: key.clone(),
        color: palette.edge,
        width_px: (width * s) as f32,
        runs,
        area: None,
        layer: 0,
    });
    for (end, at_head) in [(head, true), (tail, false)] {
        let pts = &edge.points;
        let (tip, from) = if at_head {
            (
                pts[pts.len() - 1],
                previous_distinct(pts, pts.len() - 1, -1),
            )
        } else {
            (pts[0], previous_distinct(pts, 0, 1))
        };
        push_end(
            scene,
            plan,
            palette,
            &key,
            end,
            tip,
            from,
            width * s,
            at_head,
        );
    }

    if let Some(text) = edge.label.as_deref().filter(|t| !t.is_empty()) {
        let at = edge
            .label_position
            .unwrap_or_else(|| midpoint(&edge.points));
        let size = (f64::from(label_px) * EDGE_LABEL_SCALE) as f32;
        let lines: Vec<&str> = text.split('\n').collect();
        let step = f64::from(size) * LINE_HEIGHT;
        for (k, line) in lines.iter().enumerate() {
            let dy = (k as f64 - (lines.len() as f64 - 1.0) / 2.0) * step;
            scene.texts.push(Text3 {
                key: format!("{key}/label/{k}"),
                text: (*line).to_owned(),
                anchor: plan.world(at, lift),
                offset_px: [0.0, dy as f32],
                size_px: size,
                color: palette.text,
                align: Anchor::Middle,
                valign: VAlign::Middle,
                azimuths: None,
                role: LabelRole::Fixed,
                halo: true,
                flat: None,
            });
        }
    }
}

/// The nearest point before `points[i]` (stepping by `dir`) that is not on top of it, for the
/// direction an edge arrives in.
pub(crate) fn previous_distinct(points: &[[f64; 2]], i: usize, dir: isize) -> [f64; 2] {
    let p = points[i];
    let mut j = i as isize + dir;
    while j >= 0 && (j as usize) < points.len() {
        let q = points[j as usize];
        if (q[0] - p[0]).hypot(q[1] - p[1]) > 1e-6 {
            return q;
        }
        j += dir;
    }
    [p[0], p[1] - 1.0]
}

#[allow(clippy::too_many_arguments)]
fn push_end(
    scene: &mut Scene3d,
    plan: &Plan,
    palette: &Palette,
    key: &str,
    end: EdgeEnd,
    tip: [f64; 2],
    from: [f64; 2],
    width_px: f64,
    at_head: bool,
) {
    let (dx, dy) = (tip[0] - from[0], tip[1] - from[1]);
    let len = dx.hypot(dy).max(1e-9);
    let (ux, uy) = (dx / len, dy / len);
    let (nx, ny) = (-uy, ux);
    let name = if at_head { "head" } else { "tail" };
    match end {
        EdgeEnd::None => {}
        EdgeEnd::Arrow => {
            let base = [tip[0] - ux * ARROW_LENGTH, tip[1] - uy * ARROW_LENGTH];
            let w = ARROW_WIDTH / 2.0;
            let outline = [
                tip,
                [base[0] + nx * w, base[1] + ny * w],
                [base[0] - nx * w, base[1] - ny * w],
            ];
            scene.decor.push(PrismMark {
                key: format!("{key}/{name}"),
                color: palette.edge,
                outline: outline.iter().map(|&p| plan.floor_point(p)).collect(),
                base: plan.node_top() as f32,
                top: (plan.node_top() + plan.len(ARROW_HEIGHT)) as f32,
                smooth: false,
                gloss: 0.0,
                node: 0,
            });
        }
        EdgeEnd::Circle => {
            let centre = [tip[0] - ux * END_RADIUS, tip[1] - uy * END_RADIUS];
            let (outline, _) = shape_outline(NodeShape::Circle, 2.0 * END_RADIUS, 2.0 * END_RADIUS);
            scene.decor.push(PrismMark {
                key: format!("{key}/{name}"),
                color: palette.edge,
                outline: outline
                    .iter()
                    .map(|p| plan.floor_point([centre[0] + p[0], centre[1] + p[1]]))
                    .collect(),
                base: plan.node_top() as f32,
                top: (plan.node_top() + plan.len(ARROW_HEIGHT)) as f32,
                smooth: true,
                gloss: 0.0,
                node: 0,
            });
        }
        EdgeEnd::Cross => {
            let c = [
                tip[0] - ux * END_RADIUS * 1.5,
                tip[1] - uy * END_RADIUS * 1.5,
            ];
            let r = END_RADIUS;
            let diag =
                |a: f64, b: f64| [c[0] + (ux * a + nx * b) * r, c[1] + (uy * a + ny * b) * r];
            let lift = plan.node_top() + plan.len(EDGE_LIFT);
            let runs = vec![
                vec![
                    plan.world(diag(-1.0, -1.0), lift),
                    plan.world(diag(1.0, 1.0), lift),
                ],
                vec![
                    plan.world(diag(-1.0, 1.0), lift),
                    plan.world(diag(1.0, -1.0), lift),
                ],
            ];
            scene.polylines.push(Polyline3 {
                key: format!("{key}/{name}"),
                color: palette.edge,
                width_px: width_px as f32,
                runs,
                area: None,
                layer: 0,
            });
        }
    }
}

/// Shortens a path by `by` layout units at its last (or first) end, never by more than half
/// its length.
pub(crate) fn trim_end(points: &mut Vec<[f64; 2]>, by: f64, at_end: bool) {
    if !at_end {
        points.reverse();
    }
    let total = path_length(points);
    let mut left = by.min(total / 2.0);
    while left > 0.0 && points.len() >= 2 {
        let n = points.len();
        let (a, b) = (points[n - 2], points[n - 1]);
        let seg = (b[0] - a[0]).hypot(b[1] - a[1]);
        if seg > left {
            let t = (seg - left) / seg;
            points[n - 1] = [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
            break;
        }
        left -= seg;
        if n == 2 {
            points[1] = a;
            break;
        }
        points.pop();
    }
    if !at_end {
        points.reverse();
    }
}

pub(crate) fn path_length(points: &[[f64; 2]]) -> f64 {
    points
        .windows(2)
        .map(|w| (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1]))
        .sum()
}

/// The point halfway along a path.
pub(crate) fn midpoint(points: &[[f64; 2]]) -> [f64; 2] {
    let mut left = path_length(points) / 2.0;
    for w in points.windows(2) {
        let seg = (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1]);
        if seg >= left && seg > 0.0 {
            let t = left / seg;
            return [
                w[0][0] + (w[1][0] - w[0][0]) * t,
                w[0][1] + (w[1][1] - w[0][1]) * t,
            ];
        }
        left -= seg;
    }
    points[points.len() / 2]
}

/// A path cut into dashes `dash` long with `gap` between them, following its bends.
pub(crate) fn dashes(points: &[[f64; 2]], dash: f64, gap: f64) -> Vec<Vec<[f64; 2]>> {
    let mut out = Vec::new();
    let mut current: Vec<[f64; 2]> = vec![points[0]];
    let mut drawing = true;
    let mut left = dash;
    for w in points.windows(2) {
        let (mut a, b) = (w[0], w[1]);
        let mut seg = (b[0] - a[0]).hypot(b[1] - a[1]);
        while seg > left {
            let t = left / seg;
            let p = [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
            if drawing {
                current.push(p);
                out.push(std::mem::take(&mut current));
            } else {
                current = vec![p];
            }
            drawing = !drawing;
            seg -= left;
            a = p;
            left = if drawing { dash } else { gap };
        }
        left -= seg;
        if drawing {
            current.push(b);
        }
    }
    if drawing && current.len() >= 2 {
        out.push(current);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outlines_fit_their_rectangle_and_are_convex() {
        let shapes = [
            NodeShape::Rect,
            NodeShape::Rounded,
            NodeShape::Stadium,
            NodeShape::Cylinder,
            NodeShape::Circle,
            NodeShape::Diamond,
            NodeShape::Hexagon,
            NodeShape::Parallelogram,
            NodeShape::ParallelogramAlt,
            NodeShape::Trapezoid,
            NodeShape::TrapezoidAlt,
        ];
        for shape in shapes {
            for (w, h) in [(120.0, 40.0), (40.0, 40.0), (30.0, 90.0)] {
                let (outline, _) = shape_outline(shape, w, h);
                assert!(outline.len() >= 3, "{shape:?}");
                for p in &outline {
                    assert!(p[0].abs() <= w / 2.0 + 1e-9 && p[1].abs() <= h / 2.0 + 1e-9);
                }
                // Convex: every turn goes the same way.
                let n = outline.len();
                let turns: Vec<f64> = (0..n)
                    .map(|i| {
                        let (a, b, c) = (outline[i], outline[(i + 1) % n], outline[(i + 2) % n]);
                        (b[0] - a[0]) * (c[1] - b[1]) - (b[1] - a[1]) * (c[0] - b[0])
                    })
                    .filter(|t| t.abs() > 1e-9)
                    .collect();
                assert!(
                    turns.iter().all(|t| *t > 0.0) || turns.iter().all(|t| *t < 0.0),
                    "{shape:?} {w}x{h}"
                );
            }
        }
    }

    #[test]
    fn dashes_alternate_and_cover_the_path() {
        let d = dashes(&[[0.0, 0.0], [20.0, 0.0]], 5.0, 4.0);
        // 0-5, 9-14, 18-20
        assert_eq!(d.len(), 3);
        assert_eq!(d[0], vec![[0.0, 0.0], [5.0, 0.0]]);
        assert_eq!(d[1], vec![[9.0, 0.0], [14.0, 0.0]]);
        assert_eq!(d[2], vec![[18.0, 0.0], [20.0, 0.0]]);
        // Round a corner.
        let d = dashes(&[[0.0, 0.0], [3.0, 0.0], [3.0, 3.0]], 5.0, 4.0);
        assert_eq!(d[0], vec![[0.0, 0.0], [3.0, 0.0], [3.0, 2.0]]);
    }

    #[test]
    fn trimming_shortens_the_chosen_end() {
        let mut p = vec![[0.0, 0.0], [10.0, 0.0], [10.0, 2.0]];
        trim_end(&mut p, 4.0, true);
        assert_eq!(p, vec![[0.0, 0.0], [8.0, 0.0]]);
        let mut p = vec![[0.0, 0.0], [10.0, 0.0]];
        trim_end(&mut p, 4.0, false);
        assert_eq!(p, vec![[4.0, 0.0], [10.0, 0.0]]);
        // Never more than half.
        let mut p = vec![[0.0, 0.0], [4.0, 0.0]];
        trim_end(&mut p, 10.0, true);
        assert_eq!(p, vec![[0.0, 0.0], [2.0, 0.0]]);
    }

    #[test]
    fn midpoint_is_halfway_along() {
        assert_eq!(
            midpoint(&[[0.0, 0.0], [10.0, 0.0], [10.0, 10.0]]),
            [10.0, 0.0]
        );
        assert_eq!(midpoint(&[[0.0, 0.0], [4.0, 0.0]]), [2.0, 0.0]);
    }
}
