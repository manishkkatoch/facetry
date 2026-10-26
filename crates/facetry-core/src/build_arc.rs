//! Layout of a 2D pie or donut: spec + viewport -> [`Scene`] with one [`ArcMark`] per slice,
//! its value label, and leader lines for labels that do not fit inside their slice.
//!
//! A pie has no axes, so the layout is short: the circle is centred in the room that is left
//! (minus padding, and minus whatever the *outside* labels need), slices follow the colour
//! field's order clockwise from `startAngle`, and each label goes inside its slice when the
//! text fits there, else outside on a short leader line.
//!
//! **Labels and the radius depend on each other**: a bigger pie leaves less room for outside
//! labels, and the labels that must go outside depend on the pie's size. The loop at the
//! heart of [`build_pie`] settles it: it starts from the largest possible pie, finds which
//! labels do not fit inside, reserves room for those, shrinks the pie, and repeats until the
//! set of outside labels stops changing (a few rounds at most).

use std::f64::consts::TAU;

use crate::Color;
use crate::Theme;
use crate::build::{LABEL_SIZE, PADDING, VALUE_LABEL_GAP};
use crate::scale::{format_percent, format_value};
use crate::scene::{Anchor, ArcMark, MarkLabel, Polyline, Rotation, Scene, Text};
use crate::spec::{ChartSpec, LabelSpec};
use crate::text::TextMeasure;
use crate::validate::{ResolvedArc, arc_style};

/// Length of the leader line from the pie to an outside label, CSS px.
pub(crate) const LEADER: f64 = 10.0;
/// Where inside a *pie* (no hole) the label sits, as a fraction of the radius: far enough out
/// to leave room for the text, not so far that the slice's width is the limit.
const PIE_LABEL_AT: f64 = 0.62;
/// The pie is never smaller than this, however long the outside labels are, CSS px.
const MIN_RADIUS: f64 = 24.0;

/// One slice before it is drawn.
struct Slice {
    /// Index into the layer's cells.
    cell: usize,
    start: f64,
    end: f64,
    color: Color,
    text: Option<String>,
}

/// Where a label goes.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Place {
    Inside,
    Outside,
}

/// The size of a label's text, for the fit tests.
#[derive(Debug, Clone, Copy)]
pub(crate) struct TextBox {
    pub width: f64,
    /// Height of the digits (cap height): the visible part.
    pub height: f64,
}

/// Whether a `text`-sized box centred in the middle of the ring sector `span` (radians) of a
/// pie of `radius` with a hole of `inner` fits inside the slice.
///
/// The box is axis-aligned while the slice is turned by its middle angle, so its extent
/// along the slice's own axes follows from the angle: `radial` along the radius and
/// `tangent` across it. The slice is a wedge (narrower toward the centre), so the width it
/// offers is measured at the box's inner edge.
pub(crate) fn fits_inside(
    text: TextBox,
    span: (f64, f64),
    inner: f64,
    radius: f64,
    margin: f64,
) -> bool {
    let (a0, a1) = span;
    let mid = (a0 + a1) / 2.0;
    let (sin, cos) = (mid.sin().abs(), mid.cos().abs());
    let radial = sin * text.width + cos * text.height + 2.0 * margin;
    let tangent = cos * text.width + sin * text.height + 2.0 * margin;
    let at = middle_radius(inner, radius);
    // Room along the radius: the box is centred at `at`.
    if radial > 2.0 * (at - inner).min(radius - at) {
        return false;
    }
    let sweep = (a1 - a0).min(std::f64::consts::PI);
    let inner_edge = at - radial / 2.0;
    tangent <= 2.0 * inner_edge * (sweep / 2.0).sin()
}

/// The radius at which a label is centred: the middle of a donut's ring, or
/// [`PIE_LABEL_AT`] of a pie's radius.
pub(crate) fn middle_radius(inner: f64, radius: f64) -> f64 {
    if inner > 0.0 {
        (inner + radius) / 2.0
    } else {
        radius * PIE_LABEL_AT
    }
}

pub(crate) fn build_pie(
    spec: &ChartSpec,
    arc: &ResolvedArc,
    viewport_px: (u32, u32),
    dpr: f32,
    measure: &dyn TextMeasure,
    theme: &Theme,
) -> Scene {
    let s = f64::from(dpr).clamp(0.25, 16.0);
    let (w, h) = (f64::from(viewport_px.0), f64::from(viewport_px.1));
    let layer = &spec.layers[arc.index];
    let style = arc_style(layer);
    let label: Option<&LabelSpec> = layer.label.as_ref();
    let percent = label
        .and_then(|l| l.format.as_ref())
        .and_then(|f| f.percent)
        .unwrap_or(false);
    let decimals = label
        .and_then(|l| l.format.as_ref())
        .and_then(|f| f.decimals);

    let mut scene = Scene::empty();
    scene.width = viewport_px.0;
    scene.height = viewport_px.1;
    scene.background = theme.background;

    // --- slices, clockwise from the start angle, in the colour field's order ----------------
    let cells = arc.cells();
    let total: f64 = cells.iter().map(|c| c.value).sum();
    if total <= 0.0 {
        return scene;
    }
    let ordinal = crate::scale::Ordinal::new(arc.dict, theme.palette.to_vec());
    let mut angle = style.start;
    let slices: Vec<Slice> = cells
        .iter()
        .enumerate()
        .map(|(i, cell)| {
            let (start, end) = (angle, angle + cell.value / total * TAU);
            angle = end;
            Slice {
                cell: i,
                start,
                end,
                color: ordinal.color(&arc.dict[cell.series]),
                text: label.map(|_| {
                    if percent {
                        format_percent(cell.value / total, decimals)
                    } else {
                        format_value(cell.value, decimals)
                    }
                }),
            }
        })
        .collect();
    // The angles a slice really covers, after padding (as `ArcMark::span` computes them).
    let span = |sl: &Slice| {
        let sweep = sl.end - sl.start;
        if sweep >= TAU - 1e-4 {
            return (sl.start, sl.end);
        }
        let cut = (style.pad / 2.0).min(sweep / 4.0);
        (sl.start + cut, sl.end - cut)
    };

    // --- the room, and the radius it leaves -------------------------------------------------
    let pad = PADDING * s;
    let label_size = (LABEL_SIZE * s) as f32;
    let cap = f64::from(measure.cap_height(label_size));
    let desc = f64::from(measure.descent(label_size));
    let gap = VALUE_LABEL_GAP * s;
    let leader = LEADER * s;
    // The legend takes a band of the canvas; the pie is centred in the rest.
    let legend = crate::legend::layout(spec, theme, (w, h), s, pad, measure);
    let (ax0, ay0, ax1, ay1) = legend.as_ref().map_or((0.0, 0.0, w, h), |l| {
        (l.left, l.top, w - l.right, h - l.bottom)
    });
    let (room_w, room_h) = (
        (ax1 - ax0 - 2.0 * pad).max(1.0),
        (ay1 - ay0 - 2.0 * pad).max(1.0),
    );
    let centre = [(ax0 + ax1) / 2.0, (ay0 + ay1) / 2.0];
    let boxes: Vec<Option<TextBox>> = slices
        .iter()
        .map(|sl| {
            sl.text.as_ref().map(|t| TextBox {
                width: f64::from(measure.width(t, label_size)),
                height: cap,
            })
        })
        .collect();
    let largest = (room_w.min(room_h) / 2.0).max(1.0);
    let min_radius = (MIN_RADIUS * s).min(largest);
    let mut radius = largest;
    let mut places = vec![Place::Inside; slices.len()];
    for round in 0..5 {
        let now: Vec<Place> = slices
            .iter()
            .zip(&boxes)
            .map(|(sl, b)| match b {
                Some(b) if !fits_inside(*b, span(sl), style.inner * radius, radius, gap / 2.0) => {
                    Place::Outside
                }
                _ => Place::Inside,
            })
            .collect();
        if round > 0 && now == places {
            break;
        }
        places = now;
        // Room the outside labels need around the pie: a leader and the text beside it.
        let widest = places
            .iter()
            .zip(&boxes)
            .filter(|(p, _)| **p == Place::Outside)
            .filter_map(|(_, b)| b.map(|b| b.width))
            .fold(0.0, f64::max);
        let (reserve_x, reserve_y) = if widest > 0.0 {
            (leader + gap + widest, leader + gap + cap + desc)
        } else {
            (0.0, 0.0)
        };
        radius = (room_w / 2.0 - reserve_x)
            .min(room_h / 2.0 - reserve_y)
            .clamp(min_radius, largest);
    }

    // --- emit --------------------------------------------------------------------------------
    let inner = style.inner * radius;
    let mut used_keys = std::collections::HashMap::<String, usize>::new();
    for sl in &slices {
        let cell = &cells[sl.cell];
        let name = &arc.dict[cell.series];
        let base = format!(
            "layer{}/{}",
            arc.index,
            arc.key_text(cell.row).unwrap_or_else(|| name.clone())
        );
        let n = used_keys.entry(base.clone()).or_insert(0);
        *n += 1;
        let key = if *n == 1 { base } else { format!("{base}#{n}") };
        scene.arcs.push(ArcMark {
            key,
            color: sl.color,
            center: [centre[0] as f32, centre[1] as f32],
            inner: inner as f32,
            outer: radius as f32,
            start: sl.start as f32,
            end: sl.end as f32,
            pad: style.pad as f32,
            layer: arc.index,
            row: cell.row,
            sum: cell.sum(),
        });
    }

    if let Some(l) = legend {
        scene.rules.extend(l.rules);
        scene.texts.extend(l.texts);
        scene.legend_hits = l.hits;
    }

    // Labels. Inside ones are centred in the slice; outside ones sit past a leader line (only
    // `always` labels get a leader, since a hover label comes and goes). Outside labels that
    // would touch an earlier one are left out.
    let Some(label) = label else { return scene };
    let line = s.round().max(1.0);
    let mut kept: Vec<[f64; 4]> = Vec::new(); // x0, y0, x1, y1 of the outside labels placed
    for (i, (sl, place)) in slices.iter().zip(&places).enumerate() {
        let (Some(text), Some(b)) = (&sl.text, boxes[i]) else {
            continue;
        };
        let (a0, a1) = span(sl);
        let mid = (a0 + a1) / 2.0;
        let (sin, cos) = (mid.sin(), mid.cos());
        let key = format!("{}/label", scene.arcs[i].key);
        let put = |x: f64, y: f64, color: Color, anchor: Anchor| Text {
            key: key.clone(),
            text: text.clone(),
            x: x.round() as f32,
            y: y.round() as f32,
            size_px: label_size,
            color,
            anchor,
            rotation: Rotation::None,
        };
        let text = match place {
            Place::Inside => {
                let at = middle_radius(inner, radius);
                let (x, y) = (centre[0] + at * sin, centre[1] - at * cos);
                let color = if theme.text.contrast(sl.color) >= theme.background.contrast(sl.color)
                {
                    theme.text
                } else {
                    theme.background
                };
                put(x, y + cap / 2.0, color, Anchor::Middle)
            }
            Place::Outside => {
                let at = |r: f64| (centre[0] + r * sin, centre[1] - r * cos);
                let (x0, y0) = at(radius);
                let (x1, y1) = at(radius + leader);
                // Beside the leader's end on the side the slice faces; straight above or
                // below it for a slice at the top or bottom.
                let (tx, ty, anchor) = if sin.abs() < 0.2 {
                    let y = if cos > 0.0 { y1 - gap } else { y1 + gap + cap };
                    (x1, y, Anchor::Middle)
                } else if sin > 0.0 {
                    (x1 + gap, y1 + cap / 2.0, Anchor::Start)
                } else {
                    (x1 - gap, y1 + cap / 2.0, Anchor::End)
                };
                let (left, right) = match anchor {
                    Anchor::Start => (tx, tx + b.width),
                    Anchor::End => (tx - b.width, tx),
                    Anchor::Middle => (tx - b.width / 2.0, tx + b.width / 2.0),
                };
                let rect = [left, ty - cap, right, ty + desc];
                let overlaps = kept
                    .iter()
                    .any(|k| rect[0] < k[2] && k[0] < rect[2] && rect[1] < k[3] && k[1] < rect[3]);
                if label.show == crate::spec::LabelShow::Always {
                    if overlaps {
                        continue;
                    }
                    kept.push(rect);
                    scene.polylines.push(Polyline {
                        key: format!("{}/leader", scene.arcs[i].key),
                        color: theme.axis,
                        width: line as f32,
                        runs: vec![vec![[x0 as f32, y0 as f32], [x1 as f32, y1 as f32]]],
                        layer: arc.index,
                    });
                }
                put(tx, ty, theme.text, anchor)
            }
        };
        scene.labels.push(MarkLabel {
            mark: i,
            show: label.show,
            text,
        });
    }
    scene
}
