//! Layout for 3D charts: spec -> [`Scene3d`] in world units.
//!
//! **The box.** Bars stand on a floor of `nx * nz` square cells (`nx` x categories, `nz` z
//! categories, or 1 without `z`), each `CELL` wide, so a bar's footprint is square whatever
//! the data. The longer side is kept within `MIN_LONG..=MAX_LONG` by resizing the cell, so a
//! 3-category chart is not a sliver and a 40-category one still fits the screen. Height is
//! always 1: `y` maps the (nice) value domain onto `0..=1`.
//!
//! **Depth runs front to back.** The first z category is nearest the viewer at the default
//! camera (so a single row of bars is "at the front"): world `z = depth - band position`.
//!
//! **Axes depend on the viewpoint, the scene must not.** The camera can be anywhere, and an
//! axis should sit on an edge where its labels are readable. Instead of rebuilding per
//! camera move, we emit the axes once per 45 degree *sector* of azimuth, each tagged with the
//! [`AzimuthRange`] it is for, and the shader keeps only the sector the camera is in. The
//! walls are tagged the same way. The layout code below is written for one sector and
//! repeated eight times; see the comments there for which edge gets which axis and why.
//!
//! **Which labels show is not decided here.** Every tick label is emitted (up to a cap) and
//! tagged with a [`LabelRole`]; the renderer hides the ones that would overlap and pushes
//! the rest clear of the box, from the *current* camera, so an edge seen end-on thins out
//! its labels instead of piling them up (see `facetry-render`'s `labels` module).

use std::collections::HashMap;

use crate::Theme;
use crate::scale::{Band, Linear, format_number};
use crate::scene::{Anchor, RuleKind};
use crate::scene3d::*;
use crate::spec::{ChartSpec, Orient};
use crate::text::TextMeasure;
use crate::validate::{ColorSource, ResolvedLayer, resolve_layer};

/// Width and depth of one category cell in world units (the box is 1 high).
const CELL: f64 = 0.3;
/// Bounds of the longer horizontal side, so the box never gets a sliver or a monster.
const MIN_LONG: f64 = 0.9;
const MAX_LONG: f64 = 2.4;
/// Tick marks stick out of the box this far (world units).
const TICK_LEN: f64 = 0.03;
const BAND_PADDING_INNER: f64 = 0.2;
const BAND_PADDING_OUTER: f64 = 0.1;

// Pixel sizes, CSS px (multiplied by dpr).
const LABEL_SIZE: f64 = 12.0;
const TITLE_SIZE: f64 = 13.0;
const LABEL_GAP: f64 = 4.0;
const TITLE_GAP: f64 = 8.0;
/// Extra room for titles of axes that run diagonally on screen, so the labels at the low end
/// of the edge do not run into the title.
const TITLE_SLACK: f64 = 12.0;
/// The same for titles on the right-hand edge, where labels at both ends of the edge are
/// level with the title's row.
const SIDE_TITLE_SLACK: f64 = 30.0;
const TICK_SPACING: f64 = 50.0;
/// Category labels emitted per axis at most (every n-th beyond that). The renderer hides the
/// ones that overlap, but it can only keep track of so many, so a 500-category axis does not
/// get 500 candidates.
const MAX_LABELS_PER_AXIS: usize = 100;

/// The unit camera `distance` is measured in: the box's radius (half its diagonal), but at
/// least 1. A flat chart (one row of bars) has a small radius, and framing it by that would
/// zoom in until its 1-high bars no longer fit.
pub fn framing_radius(size: [f32; 3]) -> f32 {
    let diagonal = (size[0] * size[0] + size[1] * size[1] + size[2] * size[2]).sqrt();
    (diagonal / 2.0).max(1.0)
}

/// [`build_scene_3d_with_theme`] with the default (light) theme.
pub fn build_scene_3d(
    spec: &ChartSpec,
    viewport_px: (u32, u32),
    dpr: f32,
    measure: &dyn TextMeasure,
) -> Scene3d {
    build_scene_3d_with_theme(spec, viewport_px, dpr, measure, &Theme::default())
}

/// Builds the 3D scene. Like the 2D builder it is pure, deterministic and lenient: layers
/// that do not resolve are skipped. `viewport_px` and `dpr` only affect text and line sizes
/// and how many ticks and labels fit; the geometry itself is the same at any size.
pub fn build_scene_3d_with_theme(
    spec: &ChartSpec,
    viewport_px: (u32, u32),
    dpr: f32,
    measure: &dyn TextMeasure,
    theme: &Theme,
) -> Scene3d {
    let s = f64::from(dpr).clamp(0.25, 16.0);
    let layers: Vec<ResolvedLayer> = (0..spec.layers.len())
        .filter_map(|i| resolve_layer(spec, i).ok())
        .collect();
    let axis_of = |orient| spec.axes.iter().position(|a| a.orient == orient);
    let (x_axis, y_axis, z_axis) = (
        axis_of(Orient::Bottom),
        axis_of(Orient::Left),
        axis_of(Orient::Depth),
    );

    // --- the box ----------------------------------------------------------------------------
    let union = |dicts: &mut dyn Iterator<Item = &[String]>| {
        let mut domain: Vec<String> = Vec::new();
        for dict in dicts {
            for c in dict {
                if !domain.contains(c) {
                    domain.push(c.clone());
                }
            }
        }
        domain
    };
    let x_domain = union(
        &mut layers
            .iter()
            .filter(|l| Some(l.x_axis) == x_axis)
            .map(|l| l.x_dict),
    );
    let z_domain = union(
        &mut layers
            .iter()
            .filter(|l| l.z_axis.is_some() && l.z_axis == z_axis)
            .map(|l| l.z_dict),
    );
    let nx = x_domain.len().max(1);
    let nz = z_domain.len().max(1);
    let long = nx.max(nz) as f64;
    let cell = (CELL * long).clamp(MIN_LONG, MAX_LONG) / long;
    let (width, depth) = (nx as f64 * cell, nz as f64 * cell);
    let band_x = Band::new(
        x_domain,
        (0.0, width),
        BAND_PADDING_INNER,
        BAND_PADDING_OUTER,
    );
    let band_z = Band::new(
        if z_domain.is_empty() {
            vec![String::new()]
        } else {
            z_domain
        },
        (0.0, depth),
        BAND_PADDING_INNER,
        BAND_PADDING_OUTER,
    );
    // World z of a z band: first category at the front (large z).
    let z_hi = |i: usize| depth - band_z.position(i);
    let z_mid = |i: usize| depth - band_z.center(i);

    // How big the box will look, to decide how many ticks and labels fit. It is a guess
    // from the starting camera; zooming afterwards does not re-thin labels.
    let camera = spec.camera.as_ref();
    let distance = camera
        .and_then(|c| c.distance)
        .unwrap_or(DEFAULT_DISTANCE)
        .clamp(0.5, 20.0);
    let radius = f64::from(framing_radius([width as f32, 1.0, depth as f32]));
    let half_fov = (FOV_Y_DEGREES / 2.0).to_radians().tan();
    let px_per_unit = f64::from(viewport_px.1.max(1)) / (2.0 * distance * radius * half_fov);

    // --- y scale ----------------------------------------------------------------------------
    let y_plan = y_axis.map(|i| (i, &spec.axes[i]));
    let mut y_scale = Linear::new(
        y_plan.map_or((0.0, 1.0), |(i, axis)| {
            crate::build::y_domain(axis, layers.iter().filter(|l| l.y_axis == i))
        }),
        (0.0, 1.0),
    );
    let tick_count = ((px_per_unit / (TICK_SPACING * s)).round() as usize).clamp(2, 8);
    if let Some((_, axis)) = y_plan {
        let explicit = axis.scale.as_ref().is_some_and(|sc| sc.domain.is_some());
        if axis
            .scale
            .as_ref()
            .and_then(|sc| sc.nice)
            .unwrap_or(!explicit)
        {
            y_scale.nice(tick_count);
        }
    }
    let y_ticks = y_scale.ticks(tick_count);
    let y_labels = y_ticks.labels();
    let (d0, d1) = y_scale.domain();

    let mut scene = Scene3d {
        width: viewport_px.0,
        height: viewport_px.1,
        background: theme.background,
        size: [width as f32, 1.0, depth as f32],
        halo_px: s.round().max(1.0) as f32,
        boxes: Vec::new(),
        lines: Vec::new(),
        texts: Vec::new(),
    };

    // --- bars -------------------------------------------------------------------------------
    let ordinal = crate::build::color_scale(&layers, theme);
    let y_zero = y_scale.map(0.0_f64.clamp(d0, d1));
    for layer in &layers {
        let mut used_keys: HashMap<String, usize> = HashMap::new();
        for row in 0..layer.rows() {
            let category = &layer.x_dict[layer.x_codes[row] as usize];
            let value = layer.y[row];
            let z_category = layer
                .z_axis
                .map(|_| &layer.z_dict[layer.z_codes[row] as usize]);
            let bz = match z_category {
                Some(c) => band_z.index_of(c),
                None => Some(0),
            };
            let (Some(bx), Some(bz), true) = (band_x.index_of(category), bz, value.is_finite())
            else {
                continue;
            };
            let end = y_scale.map(value.clamp(d0, d1));
            let (x0, z1) = (band_x.position(bx), z_hi(bz));
            let color = match layer.color {
                ColorSource::Default => theme.palette[0],
                ColorSource::Fixed(c) => c,
                ColorSource::Field { codes, dict } => ordinal.color(&dict[codes[row] as usize]),
            };
            let mut base_key = format!(
                "layer{}/{}",
                layer.index,
                layer.key_text(row).unwrap_or_else(|| category.clone())
            );
            if let (None, Some(z)) = (layer.key, z_category) {
                base_key = format!("{base_key}/{z}");
            }
            // Duplicate keys get `#2`, `#3`, ... so every mark stays addressable.
            let n = used_keys.entry(base_key.clone()).or_insert(0);
            *n += 1;
            let key = if *n == 1 {
                base_key
            } else {
                format!("{base_key}#{n}")
            };
            scene.boxes.push(BoxMark {
                key,
                min: [
                    x0 as f32,
                    end.min(y_zero) as f32,
                    (z1 - band_z.bandwidth()) as f32,
                ],
                max: [
                    (x0 + band_x.bandwidth()) as f32,
                    end.max(y_zero) as f32,
                    z1 as f32,
                ],
                color,
            });
        }
    }

    // --- sizes ------------------------------------------------------------------------------
    let hair = s.round().max(1.0) as f32;
    let heavy = hair * 1.5;
    let label_size = (LABEL_SIZE * s) as f32;
    let title_size = (TITLE_SIZE * s) as f32;
    let label_gap = (LABEL_GAP * s) as f32;
    let title_gap = (TITLE_GAP * s) as f32;
    let label_h = measure.ascent(label_size) + measure.descent(label_size);
    let tick = TICK_LEN as f32;
    let (w, d) = (width as f32, depth as f32);
    let y_id = y_axis.map_or("y", |i| spec.axes[i].id.as_str());

    let mut line =
        |key: String, kind: RuleKind, a: [f32; 3], b: [f32; 3], azimuths: Option<AzimuthRange>| {
            let (width_px, color) = match kind {
                RuleKind::Grid => (hair, theme.grid),
                RuleKind::Axis | RuleKind::Tick => (heavy, theme.axis),
            };
            scene.lines.push(Line3 {
                key,
                kind,
                a,
                b,
                width_px,
                color,
                azimuths,
            });
        };

    // --- back walls and floor ---------------------------------------------------------------
    // A wall is a *back* wall while the camera is on the far side of the box centre from it:
    // the x = 0 wall while the camera is toward +x (azimuth 0..180), and so on. Exactly one
    // wall per pair is drawn, so nothing is ever drawn in front of the bars. The floor (the
    // camera is always above it: elevation is clamped) is permanent. The ceiling would only
    // be a back wall from below, which orbiting cannot reach.
    //
    // Azimuth ranges of the four walls and of the corners where two of them meet:
    let x0 = AzimuthRange::new(0.0, 180.0); // camera toward +x
    let x1 = AzimuthRange::new(180.0, 360.0);
    let z0 = AzimuthRange::new(270.0, 90.0); // camera toward +z (the front)
    let z1 = AzimuthRange::new(90.0, 270.0);
    let walls = [
        ("x0", 0.0, x0, true),
        ("x1", w, x1, true),
        ("z0", 0.0, z0, false),
        ("z1", d, z1, false),
    ];
    for (name, at, range, is_x) in walls {
        for (&v, label) in y_ticks.values.iter().zip(&y_labels) {
            let y = y_scale.map(v) as f32;
            // The floor edge is drawn below, by the floor itself.
            if y <= 0.0 {
                continue;
            }
            // Zero is drawn like the 2D zero baseline when negative values hang below it.
            let kind = if v == 0.0 && d0 < 0.0 {
                RuleKind::Axis
            } else {
                RuleKind::Grid
            };
            let (a, b) = if is_x {
                ([at, y, 0.0], [at, y, d])
            } else {
                ([0.0, y, at], [w, y, at])
            };
            line(
                format!("wall:{name}/{y_id}:{label}"),
                kind,
                a,
                b,
                Some(range),
            );
        }
        // The wall's bottom edge doubles as the floor's border.
        let (a, b) = if is_x {
            ([at, 0.0, 0.0], [at, 0.0, d])
        } else {
            ([0.0, 0.0, at], [w, 0.0, at])
        };
        line(format!("floor:{name}"), RuleKind::Grid, a, b, Some(range));
    }
    // Where two back walls meet.
    let corners = [
        ("x0z0", 0.0, 0.0, AzimuthRange::new(0.0, 90.0)),
        ("x0z1", 0.0, d, AzimuthRange::new(90.0, 180.0)),
        ("x1z1", w, d, AzimuthRange::new(180.0, 270.0)),
        ("x1z0", w, 0.0, AzimuthRange::new(270.0, 360.0)),
    ];
    for (name, x, z, range) in corners {
        line(
            format!("wall:corner:{name}"),
            RuleKind::Grid,
            [x, 0.0, z],
            [x, 1.0, z],
            Some(range),
        );
    }
    // Cell borders on the floor (the outer border is drawn above).
    for k in 1..nx {
        let x = (k as f64 * cell) as f32;
        let (a, b) = ([x, 0.0, 0.0], [x, 0.0, d]);
        line(format!("floor:x{k}"), RuleKind::Grid, a, b, None);
    }
    for k in 1..nz {
        let z = (k as f64 * cell) as f32;
        let (a, b) = ([0.0, 0.0, z], [w, 0.0, z]);
        line(format!("floor:z{k}"), RuleKind::Grid, a, b, None);
    }

    // --- axes, once per 45 degree sector ----------------------------------------------------
    let y_label_width = y_labels
        .iter()
        .map(|l| measure.width(l, label_size))
        .fold(0.0, f32::max);
    let x_centres: Vec<f32> = (0..band_x.domain().len())
        .map(|i| band_x.center(i) as f32)
        .collect();
    let z_centres: Vec<f32> = (0..band_z.domain().len())
        .map(|i| z_mid(i) as f32)
        .collect();
    // Each axis is a band scale along one floor edge.
    struct Edge<'a> {
        id: &'a str,
        /// [`AXIS_X`] or [`AXIS_Z`]: which group its labels belong to.
        group: u8,
        title: Option<&'a str>,
        domain: &'a [String],
        /// Centre of each category along the edge, in world units.
        centres: &'a [f32],
        length: f32,
        /// `true`: runs along x (at some z), `false`: along z (at some x).
        along_x: bool,
        /// Where the edge sits on the other horizontal axis, and which way is outward.
        fixed: f32,
        out: f32,
        /// Along the bottom of the picture (labels below) or up its side (labels beside it).
        bottom: bool,
        /// For a side edge: it runs to the left of the nearest corner, so its labels hang
        /// to the left (otherwise to the right).
        left: bool,
    }

    let mut text = |key: String,
                    text: &str,
                    anchor: [f32; 3],
                    offset: [f32; 2],
                    size: f32,
                    align,
                    valign,
                    azimuths,
                    role| {
        scene.texts.push(Text3 {
            key,
            text: text.to_owned(),
            anchor,
            offset_px: offset,
            size_px: size,
            color: if matches!(role, LabelRole::Title { .. }) {
                theme.text
            } else {
                theme.text_muted
            },
            align,
            valign,
            azimuths,
            role,
        });
    };

    for sector in 0..8 {
        let from = sector as f32 * 45.0;
        let range = Some(AzimuthRange::new(from, from + 45.0));
        let q = format!("{:03}", sector * 45);
        // Everything below is decided at the sector's middle azimuth, which is on the same
        // side of every boundary (0, 45, 90, ...) as the whole sector.
        let (sin, cos) = (from + 22.5).to_radians().sin_cos();
        // Facing the front or back of the box (as opposed to a side)?
        let facing = cos.abs() > sin.abs();
        // The screen's right vector is (cos, 0, -sin): the corner minimising that is the
        // leftmost one on screen (where the y axis stands). Nearest corner: toward the camera
        // in both x and z.
        let (left_x, left_z) = (
            if cos > 0.0 { 0.0 } else { w },
            if sin > 0.0 { d } else { 0.0 },
        );
        let (near_x, near_z) = (
            if sin > 0.0 { w } else { 0.0 },
            if cos > 0.0 { d } else { 0.0 },
        );
        // The two horizontal axes sit on the two floor edges that meet at the nearest corner
        // (the lower outline of the picture; the other two edges are hidden behind the bars,
        // and labels there would be drawn on top of them). Facing the front, x runs along
        // the bottom of the picture and depth up one side; seen from the side it is the
        // other way around, so the long axis is always the "bottom" one and the foreshortened
        // one runs up the side, with its labels beside it. Which side is the foreshortened
        // edge's: it runs from the nearest corner to the left corner or to the right one, and
        // its labels hang outward, away from the box.
        let outward = |at: f32| if at <= 0.0 { -1.0 } else { 1.0 };
        // The screen's right vector is (cos, 0, -sin); an edge runs to the left of the
        // nearest corner when its far end has a smaller component along it.
        let x_to_left = (w - 2.0 * near_x) * cos < 0.0;
        let z_to_left = (d - 2.0 * near_z) * sin > 0.0;
        let x_edge = |bottom| Edge {
            id: x_axis.map_or("x", |i| spec.axes[i].id.as_str()),
            group: AXIS_X,
            title: x_axis.and_then(|i| spec.axes[i].title.as_deref()),
            domain: band_x.domain(),
            centres: &x_centres,
            length: w,
            along_x: true,
            fixed: near_z,
            out: outward(near_z),
            bottom,
            left: x_to_left,
        };
        let z_edge = |bottom| Edge {
            id: z_axis.map_or("z", |i| spec.axes[i].id.as_str()),
            group: AXIS_Z,
            title: z_axis.and_then(|i| spec.axes[i].title.as_deref()),
            domain: band_z.domain(),
            centres: &z_centres,
            length: d,
            along_x: false,
            fixed: near_x,
            out: outward(near_x),
            bottom,
            left: z_to_left,
        };
        let mut edges = Vec::new();
        if x_axis.is_some() {
            edges.push(x_edge(facing));
        }
        if z_axis.is_some() && !z_domain_is_empty(&layers, z_axis) {
            edges.push(z_edge(!facing));
        }

        for e in &edges {
            let point = |t: f32, o: f32| {
                if e.along_x {
                    [t, 0.0, e.fixed + o]
                } else {
                    [e.fixed + o, 0.0, t]
                }
            };
            let id = e.id;
            line(
                format!("axis:{id}/line@{q}"),
                RuleKind::Axis,
                point(0.0, 0.0),
                point(e.length, 0.0),
                range,
            );
            let step = e.domain.len().div_ceil(MAX_LABELS_PER_AXIS).max(1);
            let count = e.domain.len().div_ceil(step) as u16;
            let widest = e
                .domain
                .iter()
                .map(|l| measure.width(l, label_size))
                .fold(0.0, f32::max);
            for (i, cat) in e.domain.iter().enumerate() {
                let t = e.centres[i];
                let end = point(t, e.out * tick);
                line(
                    format!("axis:{id}/tick:{cat}@{q}"),
                    RuleKind::Tick,
                    point(t, 0.0),
                    end,
                    range,
                );
                if i % step == 0 {
                    let (offset, align, valign) = if e.bottom {
                        ([0.0, label_gap], Anchor::Middle, VAlign::Top)
                    } else if e.left {
                        ([-label_gap, 0.0], Anchor::End, VAlign::Middle)
                    } else {
                        ([label_gap, 0.0], Anchor::Start, VAlign::Middle)
                    };
                    text(
                        format!("axis:{id}/label:{cat}@{q}"),
                        cat,
                        end,
                        offset,
                        label_size,
                        align,
                        valign,
                        range,
                        LabelRole::Tick {
                            group: e.group,
                            index: (i / step) as u16,
                            count,
                        },
                    );
                }
            }
            if let Some(t) = e.title {
                // Beyond the labels, with some slack for the labels at the low end of an
                // edge that runs diagonally on screen.
                let (offset, align, valign) = if e.bottom {
                    (
                        [
                            0.0,
                            label_gap + label_h + title_gap + (TITLE_SLACK * s) as f32,
                        ],
                        Anchor::Middle,
                        VAlign::Top,
                    )
                } else {
                    let reach = label_gap + widest + title_gap + (SIDE_TITLE_SLACK * s) as f32;
                    if e.left {
                        ([-reach, 0.0], Anchor::End, VAlign::Middle)
                    } else {
                        ([reach, 0.0], Anchor::Start, VAlign::Middle)
                    }
                };
                text(
                    format!("axis:{id}/title@{q}"),
                    t,
                    point(e.length / 2.0, e.out * tick),
                    offset,
                    title_size,
                    align,
                    valign,
                    range,
                    LabelRole::Title { group: e.group },
                );
            }
        }

        // y: up the vertical edge that is leftmost on screen, ticks and labels outward
        // (leftward). Facing the box the ticks run along x, from the side along z, so they
        // always have some length on screen.
        if let Some(i) = y_axis {
            let id = &spec.axes[i].id;
            let out_x = if left_x <= 0.0 { -1.0 } else { 1.0 };
            let out_z = if left_z >= d { 1.0 } else { -1.0 };
            let tick_dir = if facing {
                [out_x * tick, 0.0, 0.0]
            } else {
                [0.0, 0.0, out_z * tick]
            };
            let at = |y: f32, o: [f32; 3]| [left_x + o[0], y, left_z + o[2]];
            line(
                format!("axis:{id}/line@{q}"),
                RuleKind::Axis,
                at(0.0, [0.0; 3]),
                at(1.0, [0.0; 3]),
                range,
            );
            for (index, (&v, label)) in y_ticks.values.iter().zip(&y_labels).enumerate() {
                let y = y_scale.map(v) as f32;
                let end = at(y, tick_dir);
                let name = format_number(v, y_ticks.decimals, false);
                line(
                    format!("axis:{id}/tick:{name}@{q}"),
                    RuleKind::Tick,
                    at(y, [0.0; 3]),
                    end,
                    range,
                );
                text(
                    format!("axis:{id}/label:{name}@{q}"),
                    label,
                    end,
                    [-label_gap, 0.0],
                    label_size,
                    Anchor::End,
                    VAlign::Middle,
                    range,
                    LabelRole::Tick {
                        group: AXIS_Y,
                        index: index as u16,
                        count: y_labels.len() as u16,
                    },
                );
            }
            if let Some(t) = &spec.axes[i].title {
                text(
                    format!("axis:{id}/title@{q}"),
                    t,
                    at(0.5, tick_dir),
                    [-(label_gap + y_label_width + title_gap + title_gap), 0.0],
                    title_size,
                    Anchor::End,
                    VAlign::Middle,
                    range,
                    LabelRole::Title { group: AXIS_Y },
                );
            }
        }
    }
    scene
}

/// A depth axis with no `z` data has nothing to label, so it is not drawn.
fn z_domain_is_empty(layers: &[ResolvedLayer], z_axis: Option<usize>) -> bool {
    !layers
        .iter()
        .any(|l| l.z_axis.is_some() && l.z_axis == z_axis && !l.z_dict.is_empty())
}
