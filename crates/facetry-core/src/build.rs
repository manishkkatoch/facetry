//! Layout and scene building: spec + viewport -> positioned items.
//!
//! The order matters, because each step needs the previous one's answer:
//!
//! 1. Resolve layers and build one scale per axis (the data decides the domains).
//! 2. Vertical room: the bottom axis (ticks, labels, title) has a known height, which fixes
//!    the plot's top and bottom, hence the y-axis length, hence how many ticks fit. The top
//!    keeps room for half of the top tick label, which is centred on the plot's top edge.
//! 3. Horizontal room: the y tick labels' *measured widths* (plus titles) fix the left and
//!    right margins, hence the plot's left and right edges.
//! 4. Now the x band scale has its range, so we know whether x labels overlap and thin them.
//!    If the last label sticks out past the canvas, the plot is narrowed to make room.
//! 5. Emit gridlines, bars, axes and text.
//!
//! All sizes below are written in CSS pixels and multiplied by the device pixel ratio.

use std::collections::{HashMap, HashSet};

use crate::Theme;
use crate::scale::{Band, Linear, Ordinal, Ticks, format_number, format_percent, format_value};
use crate::scene::{Anchor, Bounds, MarkLabel, Rect, Rotation, Rule, RuleKind, Scene, Text};
use crate::spec::{AxisSpec, ChartSpec, LabelSpec, Orient, Position};
use crate::text::TextMeasure;
use crate::transform;
use crate::validate::{ColorSource, ResolvedLayer, resolve_layer};

// Layout constants, in CSS px.
const PADDING: f64 = 12.0;
const TICK_LENGTH: f64 = 6.0;
/// Between the end of a tick and its label.
const LABEL_GAP: f64 = 4.0;
/// Between tick labels and the axis title.
const TITLE_GAP: f64 = 8.0;
const LABEL_SIZE: f64 = 12.0;
const TITLE_SIZE: f64 = 13.0;
/// Smallest clear space between two neighbouring x labels.
const MIN_LABEL_GAP: f64 = 8.0;
/// Roughly one y tick per this much plot height.
const TICK_SPACING: f64 = 50.0;
const BAND_PADDING_INNER: f64 = 0.2;
const BAND_PADDING_OUTER: f64 = 0.1;
/// Space between the bars of a `group` layer, as a fraction of one bar's slot.
const GROUP_PADDING_INNER: f64 = 0.1;
/// Between a bar's end and its value label.
const VALUE_LABEL_GAP: f64 = 4.0;

/// Builds the scene with the default (light) theme.
///
/// Pure and deterministic: the same inputs always give the same scene. Call
/// [`crate::validate`] first; layers or axes that don't resolve are skipped rather than
/// reported, so an invalid spec draws what it can instead of panicking.
///
/// `viewport_px` is the canvas size in device pixels; `dpr` is device pixels per CSS pixel
/// (it scales fonts, margins and line widths).
pub fn build_scene(
    spec: &ChartSpec,
    viewport_px: (u32, u32),
    dpr: f32,
    measure: &dyn TextMeasure,
) -> Scene {
    build_scene_with_theme(spec, viewport_px, dpr, measure, &Theme::default())
}

enum AxisScale {
    Band(Band),
    Linear { scale: Linear, ticks: Ticks },
}

struct AxisPlan<'a> {
    /// Index into `spec.axes`; layers refer to axes by this.
    spec_index: usize,
    spec: &'a AxisSpec,
    scale: AxisScale,
    /// A numeric axis whose layers are all `normalize`d: ticks read `20%`, not `0.2`.
    percent: bool,
}

/// Measured sizes of one axis' text, used for both layout and drawing.
struct AxisText {
    labels: Vec<String>,
    max_label_width: f64,
}

fn plan_index(plans: &[AxisPlan], orient: Orient) -> Option<usize> {
    plans.iter().position(|p| p.spec.orient == orient)
}

/// [`build_scene`] with an explicit theme.
pub fn build_scene_with_theme(
    spec: &ChartSpec,
    viewport_px: (u32, u32),
    dpr: f32,
    measure: &dyn TextMeasure,
    theme: &Theme,
) -> Scene {
    let s = f64::from(dpr).clamp(0.25, 16.0);
    let (w, h) = (f64::from(viewport_px.0), f64::from(viewport_px.1));
    let layers: Vec<ResolvedLayer> = (0..spec.layers.len())
        .filter_map(|i| resolve_layer(spec, i).ok())
        .collect();

    let pad = PADDING * s;
    let tick_len = TICK_LENGTH * s;
    let label_gap = LABEL_GAP * s;
    let title_gap = TITLE_GAP * s;
    let label_size = (LABEL_SIZE * s) as f32;
    let title_size = (TITLE_SIZE * s) as f32;
    // Hairlines: 1 CSS px, at least 1 device px, always whole device pixels so they stay crisp.
    let line = s.round().max(1.0);
    let label_asc = f64::from(measure.ascent(label_size));
    let label_desc = f64::from(measure.descent(label_size));
    let label_h = label_asc + label_desc;
    let title_asc = f64::from(measure.ascent(title_size));
    let title_h = title_asc + f64::from(measure.descent(title_size));
    let cap = f64::from(measure.cap_height(label_size));

    // --- 1. axes and scales ---------------------------------------------------------------
    let mut plans: Vec<AxisPlan> = Vec::new();
    let mut seen = HashSet::new();
    for (i, axis) in spec.axes.iter().enumerate() {
        // One axis per side; `validate` reports duplicates, we keep the first.
        if axis.orient == Orient::Depth || !seen.insert(axis.orient as u8) {
            continue;
        }
        let scale = match axis.orient {
            Orient::Bottom => {
                // Union of every bound layer's categories, in first-seen order.
                let mut domain: Vec<String> = Vec::new();
                for layer in layers.iter().filter(|l| l.x_axis == i) {
                    for c in layer.x_dict {
                        if !domain.contains(c) {
                            domain.push(c.clone());
                        }
                    }
                }
                AxisScale::Band(Band::new(
                    domain,
                    (0.0, 1.0),
                    BAND_PADDING_INNER,
                    BAND_PADDING_OUTER,
                ))
            }
            _ => AxisScale::Linear {
                scale: Linear::new(
                    y_domain(axis, layers.iter().filter(|l| l.y_axis == i)),
                    (0.0, 1.0),
                ),
                ticks: Ticks {
                    values: Vec::new(),
                    decimals: 0,
                },
            },
        };
        let percent = matches!(scale, AxisScale::Linear { .. })
            && is_percent(layers.iter().filter(|l| l.y_axis == i));
        plans.push(AxisPlan {
            spec_index: i,
            spec: axis,
            scale,
            percent,
        });
    }

    // --- 2. vertical room -----------------------------------------------------------------
    // The top tick label is centred on the plot's top edge, so half of it sticks out above.
    let top = (pad + label_h / 2.0).round();
    let mut bottom_block = 0.0;
    if let Some(p) = plan_index(&plans, Orient::Bottom).map(|i| &plans[i]) {
        bottom_block = line + tick_len + label_gap + label_h;
        if p.spec.title.is_some() {
            bottom_block += title_gap + title_h;
        }
    }
    let plot_bottom = (h - pad - bottom_block).round().max(top + 1.0);
    let plot_h = plot_bottom - top;

    // --- 3. y scales, ticks, horizontal room ------------------------------------------------
    let tick_count = ((plot_h / (TICK_SPACING * s)).round() as usize).clamp(2, 10);
    let mut y_text: HashMap<usize, AxisText> = HashMap::new();
    for (pi, p) in plans.iter_mut().enumerate() {
        let AxisScale::Linear { scale, ticks } = &mut p.scale else {
            continue;
        };
        let explicit_domain = p.spec.scale.as_ref().is_some_and(|sc| sc.domain.is_some());
        let nice = p
            .spec
            .scale
            .as_ref()
            .and_then(|sc| sc.nice)
            .unwrap_or(!explicit_domain);
        if nice {
            scale.nice(tick_count);
        }
        scale.set_range((plot_bottom, top));
        *ticks = scale.ticks(tick_count);
        let labels = if p.percent {
            ticks.labels_percent()
        } else {
            ticks.labels()
        };
        let max_label_width = labels
            .iter()
            .map(|l| f64::from(measure.width(l, label_size)))
            .fold(0.0, f64::max);
        y_text.insert(
            pi,
            AxisText {
                labels,
                max_label_width,
            },
        );
    }
    let side_block = |orient: Orient| -> f64 {
        let Some(pi) = plan_index(&plans, orient) else {
            return 0.0;
        };
        let mut width = line + tick_len + label_gap + y_text[&pi].max_label_width;
        if plans[pi].spec.title.is_some() {
            width += title_gap + title_h;
        }
        width
    };
    let plot_left = (pad + side_block(Orient::Left)).round();
    let mut plot_right = (w - pad - side_block(Orient::Right))
        .round()
        .max(plot_left + 1.0);

    // --- 4. x band scale and label thinning -------------------------------------------------
    // Two passes: the labels depend on the band width, and if the last one then sticks out
    // past the canvas the plot is narrowed (which changes the band width) to fit it.
    let mut x_labels: Vec<(usize, String)> = Vec::new();
    for pass in 0..2 {
        for p in &mut plans {
            let AxisScale::Band(band) = &mut p.scale else {
                continue;
            };
            *band = Band::new(
                band.domain().to_vec(),
                (plot_left, plot_right),
                BAND_PADDING_INNER,
                BAND_PADDING_OUTER,
            );
            let widest = band
                .domain()
                .iter()
                .map(|l| f64::from(measure.width(l, label_size)))
                .fold(0.0, f64::max);
            // Show every n-th label, the smallest n for which neighbours keep a clear gap.
            let every = ((widest + MIN_LABEL_GAP * s) / band.step().max(1.0))
                .ceil()
                .max(1.0) as usize;
            x_labels = band
                .domain()
                .iter()
                .enumerate()
                .filter(|(i, _)| i % every == 0)
                .map(|(i, l)| (i, l.clone()))
                .collect();
        }
        let overflow = plans
            .iter()
            .find_map(|p| match (&p.scale, x_labels.last()) {
                (AxisScale::Band(band), Some((i, label))) => Some(
                    f64::from(measure.width(label, label_size)) / 2.0 + MIN_LABEL_GAP * s / 2.0
                        - (w - band.center(*i)),
                ),
                _ => None,
            })
            .unwrap_or(0.0);
        if pass == 1 || overflow <= 0.0 {
            break;
        }
        plot_right = (plot_right - overflow).ceil().max(plot_left + 1.0);
    }

    // --- 5. emit ----------------------------------------------------------------------------
    let mut scene = Scene {
        width: viewport_px.0,
        height: viewport_px.1,
        background: theme.background,
        rects: Vec::new(),
        rules: Vec::new(),
        texts: Vec::new(),
        labels: Vec::new(),
    };
    let snap = |v: f64| v.round();
    let rule = |key: String, kind, x: f64, y: f64, w: f64, h: f64, color| Rule {
        key,
        kind,
        bounds: Bounds::new(x, y, w, h),
        color,
    };
    let text =
        |key: String, text: String, x: f64, y: f64, size: f32, color, anchor, rotation| Text {
            key,
            text,
            x: x.round() as f32,
            y: y.round() as f32,
            size_px: size,
            color,
            anchor,
            rotation,
        };
    let has_left = plan_index(&plans, Orient::Left).is_some();
    let has_right = plan_index(&plans, Orient::Right).is_some();

    // Gridlines come from the first numeric axis in declaration order.
    let grid_plan = plans
        .iter()
        .position(|p| matches!(p.scale, AxisScale::Linear { .. }));
    if let Some(pi) = grid_plan {
        let id = &plans[pi].spec.id;
        let AxisScale::Linear { scale, ticks } = &plans[pi].scale else {
            unreachable!("grid_plan is a linear axis");
        };
        let (d0, _) = scale.domain();
        for &v in &ticks.values {
            let y = snap(scale.map(v)) - (line / 2.0).floor();
            let key = |what: &str| {
                format!(
                    "axis:{id}/{what}:{}",
                    format_number(v, ticks.decimals, false)
                )
            };
            // A tick at 0 with negative values below it doubles as the zero baseline, so it is
            // drawn in the axis colour and in front of the bars.
            if v == 0.0 && d0 < 0.0 {
                scene.rules.push(rule(
                    key("zero"),
                    RuleKind::Axis,
                    plot_left,
                    y,
                    plot_right - plot_left,
                    line,
                    theme.axis,
                ));
            } else {
                scene.rules.push(rule(
                    key("grid"),
                    RuleKind::Grid,
                    plot_left,
                    y,
                    plot_right - plot_left,
                    line,
                    theme.grid,
                ));
            }
        }
    }

    // Bars.
    let ordinal = color_scale(&layers, theme);
    for layer in &layers {
        let (Some(xp), Some(yp)) = (
            plans.iter().find(|p| p.spec_index == layer.x_axis),
            plans.iter().find(|p| p.spec_index == layer.y_axis),
        ) else {
            continue;
        };
        let (AxisScale::Band(band), AxisScale::Linear { scale, .. }) = (&xp.scale, &yp.scale)
        else {
            continue;
        };
        let (d0, d1) = scale.domain();
        let label_spec = spec.layers[layer.index].label.as_ref();
        // `group` splits each x band into one slot per series (all series, so a series
        // keeps its slot at x values where it has no row).
        let slots = (layer.position == Position::Group).then(|| {
            Band::new(
                (0..layer.series_count()).map(|i| i.to_string()).collect(),
                (0.0, band.bandwidth()),
                GROUP_PADDING_INNER,
                0.0,
            )
        });
        let mut used_keys: HashMap<String, usize> = HashMap::new();
        for bar in layer_bars(layer, band, &ordinal, theme, label_spec) {
            // Both ends snap to whole pixels (crisp edges, same rule as the axis lines), and
            // the end of one stacked segment is the start of the next, so they meet exactly.
            let a = snap(scale.map(bar.y0.clamp(d0, d1)));
            let b = snap(scale.map(bar.y1.clamp(d0, d1)));
            let (bar_top, bar_bottom) = (a.min(b), a.max(b));
            let x0 = band.position(bar.band);
            let (left, right) = match (&slots, bar.slot) {
                (Some(slots), Some(k)) => (
                    snap(x0 + slots.position(k)),
                    snap(x0 + slots.position(k) + slots.bandwidth()),
                ),
                _ => (snap(x0), snap(x0 + band.bandwidth())),
            };
            // Duplicate keys get `#2`, `#3`, ... so every mark stays addressable.
            let n = used_keys.entry(bar.key.clone()).or_insert(0);
            *n += 1;
            let key = if *n == 1 {
                bar.key.clone()
            } else {
                format!("{}#{n}", bar.key)
            };
            if let (Some(label), Some((label_text, place))) = (label_spec, &bar.label) {
                let contrast =
                    if theme.text.contrast(bar.color) >= theme.background.contrast(bar.color) {
                        theme.text
                    } else {
                        theme.background
                    };
                let gap = VALUE_LABEL_GAP * s;
                let width = f64::from(measure.width(label_text, label_size));
                // `Some((baseline, colour))`, or `None` when the label has no good place.
                let placed = match place {
                    // Above the bar's end (below a negative bar's), unless that leaves the
                    // plot: then inside the bar, in whichever of the theme's two colours
                    // contrasts more.
                    LabelPlace::End => {
                        let (outside, inside, fits) = if bar.y1 >= bar.y0 {
                            let y = bar_top - gap;
                            (y, bar_top + gap + cap, y - cap >= top)
                        } else {
                            let y = bar_bottom + gap + cap;
                            (y, bar_bottom - gap, y + label_desc <= plot_bottom)
                        };
                        // Side-by-side bars are narrow; a label wider than its slot would
                        // run into its neighbours' labels.
                        let crowded = slots.is_some() && width > (right - left) + gap;
                        (!crowded).then_some(if fits {
                            (outside, theme.text)
                        } else {
                            (inside, contrast)
                        })
                    }
                    // Centred in a stacked segment, if the text fits inside it.
                    LabelPlace::Inside => (bar_bottom - bar_top >= cap + 4.0 * s
                        && width <= (right - left) - 4.0 * s)
                        .then_some(((bar_top + bar_bottom + cap) / 2.0, contrast)),
                };
                if let Some((baseline_y, label_color)) = placed {
                    scene.labels.push(MarkLabel {
                        mark: scene.rects.len(),
                        show: label.show,
                        text: text(
                            format!("{key}/label"),
                            label_text.clone(),
                            (left + right) / 2.0,
                            baseline_y,
                            label_size,
                            label_color,
                            Anchor::Middle,
                            Rotation::None,
                        ),
                    });
                }
            }
            scene.rects.push(Rect {
                key,
                bounds: Bounds::new(left, bar_top, right - left, bar_bottom - bar_top),
                color: bar.color,
                layer: layer.index,
                row: bar.row,
                sum: bar.sum,
            });
        }
    }

    // Axes.
    for p in &plans {
        let id = &p.spec.id;
        let title = p.spec.title.as_deref();
        match (&p.scale, p.spec.orient) {
            (AxisScale::Band(band), Orient::Bottom) => {
                let x0 = plot_left - if has_left { line } else { 0.0 };
                let x1 = plot_right + if has_right { line } else { 0.0 };
                scene.rules.push(rule(
                    format!("axis:{id}/line"),
                    RuleKind::Axis,
                    x0,
                    plot_bottom,
                    x1 - x0,
                    line,
                    theme.axis,
                ));
                for (i, cat) in band.domain().iter().enumerate() {
                    let cx = band.center(i);
                    scene.rules.push(rule(
                        format!("axis:{id}/tick:{cat}"),
                        RuleKind::Tick,
                        snap(cx) - (line / 2.0).floor(),
                        plot_bottom + line,
                        line,
                        tick_len,
                        theme.axis,
                    ));
                }
                let baseline = plot_bottom + line + tick_len + label_gap + label_asc;
                for (i, label) in &x_labels {
                    scene.texts.push(text(
                        format!("axis:{id}/label:{label}"),
                        label.clone(),
                        band.center(*i),
                        baseline,
                        label_size,
                        theme.text_muted,
                        Anchor::Middle,
                        Rotation::None,
                    ));
                }
                if let Some(t) = title {
                    scene.texts.push(text(
                        format!("axis:{id}/title"),
                        t.to_owned(),
                        (plot_left + plot_right) / 2.0,
                        h - pad - f64::from(measure.descent(title_size)),
                        title_size,
                        theme.text,
                        Anchor::Middle,
                        Rotation::None,
                    ));
                }
            }
            (AxisScale::Linear { scale, ticks }, orient @ (Orient::Left | Orient::Right)) => {
                let left = orient == Orient::Left;
                // x of the axis line's left edge; ticks and labels grow away from the plot.
                let line_x = if left { plot_left - line } else { plot_right };
                let tick_x = if left {
                    line_x - tick_len
                } else {
                    line_x + line
                };
                let label_x = if left {
                    tick_x - label_gap
                } else {
                    tick_x + tick_len + label_gap
                };
                scene.rules.push(rule(
                    format!("axis:{id}/line"),
                    RuleKind::Axis,
                    line_x,
                    top,
                    line,
                    plot_bottom - top,
                    theme.axis,
                ));
                let pi = plans
                    .iter()
                    .position(|q| q.spec_index == p.spec_index)
                    .unwrap_or(0);
                for (&v, label) in ticks.values.iter().zip(&y_text[&pi].labels) {
                    let y = snap(scale.map(v));
                    let key = format!("tick:{}", format_number(v, ticks.decimals, false));
                    scene.rules.push(rule(
                        format!("axis:{id}/{key}"),
                        RuleKind::Tick,
                        tick_x,
                        y - (line / 2.0).floor(),
                        tick_len,
                        line,
                        theme.axis,
                    ));
                    scene.texts.push(text(
                        format!(
                            "axis:{id}/label:{}",
                            format_number(v, ticks.decimals, false)
                        ),
                        label.clone(),
                        label_x,
                        y + cap / 2.0,
                        label_size,
                        theme.text_muted,
                        if left { Anchor::End } else { Anchor::Start },
                        Rotation::None,
                    ));
                }
                if let Some(t) = title {
                    // Rotated text: glyph tops point left, so the baseline is the title's right edge.
                    let baseline_x = if left {
                        pad + title_asc
                    } else {
                        w - pad - f64::from(measure.descent(title_size))
                    };
                    scene.texts.push(text(
                        format!("axis:{id}/title"),
                        t.to_owned(),
                        baseline_x,
                        (top + plot_bottom) / 2.0,
                        title_size,
                        theme.text,
                        Anchor::Middle,
                        Rotation::Minus90,
                    ));
                }
            }
            _ => {}
        }
    }
    scene
}

/// Where a bar's value label goes.
#[derive(Debug, Clone, Copy)]
enum LabelPlace {
    /// Beyond the bar's end (inside it when there is no room outside).
    End,
    /// Centred in the bar; no label at all when the text does not fit.
    Inside,
}

/// A bar before it becomes pixels: its x band, the value range it spans and what it needs
/// to be drawn, hovered and labelled.
struct Bar {
    /// Index into the x band scale.
    band: usize,
    /// For `group`: the series' slot inside the band.
    slot: Option<usize>,
    /// The two ends of the bar in data units (not ordered: a negative bar starts at 0 and
    /// ends below it).
    y0: f64,
    y1: f64,
    /// The table row that stands for the bar, and the summed value if it stands for several.
    row: usize,
    sum: Option<f64>,
    color: crate::Color,
    key: String,
    label: Option<(String, LabelPlace)>,
}

/// One layer's bars, according to its `position`. `overlay` keeps one bar per row; the
/// others merge rows with the same x and series first (see [`transform`]).
fn layer_bars(
    layer: &ResolvedLayer,
    band: &Band,
    ordinal: &Ordinal,
    theme: &Theme,
    label: Option<&LabelSpec>,
) -> Vec<Bar> {
    let decimals = label
        .and_then(|l| l.format.as_ref())
        .and_then(|f| f.decimals);
    let value_text = |v: f64| label.map(|_| format_value(v, decimals));
    if layer.position == Position::Overlay {
        return (0..layer.rows())
            .filter_map(|row| {
                let category = &layer.x_dict[layer.x_codes[row] as usize];
                let value = layer.y[row];
                let band_index = band.index_of(category).filter(|_| value.is_finite())?;
                let color = match layer.color {
                    ColorSource::Default => theme.palette[0],
                    ColorSource::Fixed(c) => c,
                    ColorSource::Field { codes, dict } => ordinal.color(&dict[codes[row] as usize]),
                };
                Some(Bar {
                    band: band_index,
                    slot: None,
                    y0: 0.0,
                    y1: value,
                    row,
                    sum: None,
                    color,
                    key: format!(
                        "layer{}/{}",
                        layer.index,
                        layer.key_text(row).unwrap_or_else(|| category.clone())
                    ),
                    label: value_text(value).map(|t| (t, LabelPlace::End)),
                })
            })
            .collect();
    }

    let ColorSource::Field { dict, .. } = layer.color else {
        return Vec::new(); // `validate` rejects these positions without a colour field
    };
    let cells = layer.cells();
    let normalize = layer.position == Position::Normalize;
    let spans = match layer.position {
        Position::Stack | Position::Normalize => transform::stack(&cells, normalize),
        _ => cells
            .iter()
            .map(|c| transform::Span {
                y0: 0.0,
                y1: c.value,
            })
            .collect(),
    };
    cells
        .iter()
        .zip(&spans)
        .filter_map(|(cell, span)| {
            let category = &layer.x_dict[cell.x];
            let band_index = band.index_of(category)?;
            let text = if normalize {
                label.map(|_| format_percent(span.y1 - span.y0, decimals))
            } else {
                value_text(cell.value)
            };
            Some(Bar {
                band: band_index,
                slot: (layer.position == Position::Group).then_some(cell.series),
                y0: span.y0,
                y1: span.y1,
                row: cell.row,
                sum: cell.sum(),
                color: ordinal.color(&dict[cell.series]),
                key: format!(
                    "layer{}/{}/{}",
                    layer.index,
                    layer.key_text(cell.row).unwrap_or_else(|| category.clone()),
                    dict[cell.series]
                ),
                label: text.map(|t| {
                    let place = if layer.position == Position::Group {
                        LabelPlace::End
                    } else {
                        LabelPlace::Inside
                    };
                    (t, place)
                }),
            })
        })
        .collect()
}

/// Whether an axis shows percentages: it has layers and every one is `normalize`d. (`validate`
/// refuses to mix a normalised layer with others on one axis.)
pub(crate) fn is_percent<'a>(mut layers: impl Iterator<Item = &'a ResolvedLayer<'a>>) -> bool {
    let first = layers.next();
    first.is_some_and(|l| l.position == Position::Normalize)
        && layers.all(|l| l.position == Position::Normalize)
}

/// The y domain: a fixed one from the spec, or the extent of the bound layers' data
/// (extended to include 0 unless `zero: false`). Never degenerate, so scales stay invertible.
pub(crate) fn y_domain<'a>(
    axis: &AxisSpec,
    layers: impl Iterator<Item = &'a ResolvedLayer<'a>>,
) -> (f64, f64) {
    let overrides = axis.scale.as_ref();
    if let Some([lo, hi]) = overrides.and_then(|s| s.domain) {
        return (lo, hi);
    }
    let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
    for layer in layers {
        if let Some((a, b)) = layer.extent() {
            lo = lo.min(a);
            hi = hi.max(b);
        }
    }
    if overrides.and_then(|s| s.zero).unwrap_or(true) {
        lo = lo.min(0.0);
        hi = hi.max(0.0);
    }
    match (lo.is_finite(), hi.is_finite()) {
        // No data at all.
        (false, _) | (_, false) => (0.0, 1.0),
        _ if lo < hi => (lo, hi),
        // One distinct value: widen so the scale has something to map.
        _ if lo == 0.0 => (0.0, 1.0),
        _ => (lo - 1.0, hi + 1.0),
    }
}

/// One colour scale shared by every layer that colours by field, so the same category has
/// the same colour in every layer.
pub(crate) fn color_scale(layers: &[ResolvedLayer], theme: &Theme) -> Ordinal {
    let mut domain: Vec<String> = Vec::new();
    for layer in layers {
        if let ColorSource::Field { dict, .. } = layer.color {
            for c in dict {
                if !domain.contains(c) {
                    domain.push(c.clone());
                }
            }
        }
    }
    Ordinal::new(&domain, theme.palette.to_vec())
}
